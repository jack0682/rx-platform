//! Bounded off-writer filesystem/crypto work. Every result still requires authoritative commit.
use rx_application::package_intake::{Prepared, Ticket};
use rx_domain::types::*;
use rx_package::{VerificationPolicy, store::Store};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::Semaphore;
pub struct Worker {
    device_review: Option<(PathBuf, Digest, Digest)>,
    qualification: Option<Arc<crate::requalification::Worker>>,
    review: Option<(PathBuf, Digest, Digest)>,
    root: PathBuf,
    store: Mutex<Store>,
    policy: VerificationPolicy,
    owner: Id,
    fingerprint: Digest,
    policy_file_digest: Digest,
    policy_file: Option<PathBuf>,
    permit: Arc<Semaphore>,
}
impl Worker {
    pub fn new(
        root: PathBuf,
        store: Store,
        policy: VerificationPolicy,
        policy_file_digest: Digest,
    ) -> Result<Arc<Self>, String> {
        Self::build(root, store, policy, policy_file_digest, None)
    }
    pub fn new_pinned(
        root: PathBuf,
        store: Store,
        policy: VerificationPolicy,
        policy_file_digest: Digest,
        policy_file: PathBuf,
    ) -> Result<Arc<Self>, String> {
        if !policy_file.is_absolute() {
            return Err("absolute policy source required".into());
        }
        Self::build(root, store, policy, policy_file_digest, Some(policy_file))
    }
    fn build(
        root: PathBuf,
        store: Store,
        policy: VerificationPolicy,
        policy_file_digest: Digest,
        policy_file: Option<PathBuf>,
    ) -> Result<Arc<Self>, String> {
        if !root.is_absolute() {
            return Err("absolute package import root required".into());
        }
        let owner = store.owner().clone();
        let fingerprint = policy.fingerprint().map_err(|e| e.to_string())?;
        Ok(Arc::new(Self {
            device_review: None,
            qualification: None,
            review: None,
            root,
            store: Mutex::new(store),
            policy,
            owner,
            fingerprint,
            policy_file_digest,
            policy_file,
            permit: Arc::new(Semaphore::new(1)),
        }))
    }
    pub fn with_review(
        mut worker: Arc<Self>,
        path: PathBuf,
        pin: Digest,
    ) -> Result<Arc<Self>, String> {
        if !path.is_absolute() {
            return Err("absolute review authority path required".into());
        }
        let (authority, digest) = rx_package::policy::read_with_digest::<
            rx_application::process_review::Authority,
        >(&path)
        .map_err(|e| e.to_string())?;
        if digest != pin {
            return Err("review authority pin differs".into());
        }
        let digest = authority.digest()?;
        Arc::get_mut(&mut worker)
            .ok_or("review authority must be configured before sharing worker")?
            .review = Some((path, pin, digest));
        Ok(worker)
    }
    pub fn with_requalification(
        mut worker: Arc<Self>,
        path: PathBuf,
        pin: Digest,
    ) -> Result<Arc<Self>, String> {
        let qualification = crate::requalification::Worker::new(worker.root.clone(), path, pin)?;
        Arc::get_mut(&mut worker)
            .ok_or("qualification worker must be configured before sharing")?
            .qualification = Some(qualification);
        Ok(worker)
    }
    pub fn requalification(&self) -> Option<&Arc<crate::requalification::Worker>> {
        self.qualification.as_ref()
    }
    pub fn review_digest(&self) -> Option<Digest> {
        self.review.as_ref().map(|v| v.2)
    }
    fn load_policy(&self) -> Result<VerificationPolicy, String> {
        if let Some(path) = &self.policy_file {
            let (document, digest) =
                rx_package::policy::read_with_digest::<rx_package::policy::Policy>(path)
                    .map_err(|e| e.to_string())?;
            if digest != self.policy_file_digest {
                return Err("package policy file changed".into());
            }
            let policy = document.load().map_err(|e| e.to_string())?;
            if policy.fingerprint().map_err(|e| e.to_string())? != self.fingerprint {
                return Err("package policy meaning changed".into());
            }
            Ok(policy)
        } else {
            Ok(self.policy.clone())
        }
    }
    fn load_review_authority(&self) -> Result<rx_application::process_review::Authority, String> {
        let (path, pin, meaning) = self
            .review
            .as_ref()
            .ok_or("review authority not configured")?;
        let (authority, digest) =
            rx_package::policy::read_with_digest::<rx_application::process_review::Authority>(path)
                .map_err(|e| e.to_string())?;
        if &digest != pin || authority.digest()? != *meaning {
            return Err("review authority changed".into());
        }
        Ok(authority)
    }
    pub async fn prepare_review(
        self: &Arc<Self>,
        ticket: rx_application::process_review::Ticket,
    ) -> Result<rx_application::process_review::Prepared, String> {
        let permit = self
            .permit
            .clone()
            .try_acquire_owned()
            .map_err(|_| "package worker busy")?;
        let worker = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if ticket.registration().store_owner != worker.owner {
                return Err("review Store owner differs".into());
            }
            let policy = worker.load_policy()?;
            let authority = worker.load_review_authority()?;
            let file = |name: &str| -> Result<Vec<u8>, String> {
                let path = rx_package::PackagePath::new(format!(
                    "{}/{}",
                    ticket.directory().as_str(),
                    name
                ))?;
                rx_package::directory::read_relative_file(&worker.root, &path, 1_048_576)
                    .map_err(|e| e.to_string())
            };
            let report: rx_process_contract::package_review::Report =
                rx_domain::canonical::decode_json(&file("verification.json")?)
                    .map_err(|e| e.to_string())?;
            let signature: rx_package::SignatureEnvelope =
                rx_domain::canonical::decode_json(&file("verification.sig.json")?)
                    .map_err(|e| e.to_string())?;
            let resolved = if report.resolved.is_some() {
                Some(file("resolved.json")?)
            } else {
                None
            };
            let object = rx_package::store::ObjectId {
                manifest: ticket.job().request.package_manifest,
                signature: ticket.job().request.package_signature,
            };
            let stored = worker
                .store
                .lock()
                .map_err(|_| "store owner failed")?
                .verify_owned(&object, &policy)
                .map_err(|e| e.to_string())?;
            let devices = worker.verify_process_devices(ticket.job(), &policy)?;
            let validated = rx_application::process_review::Validated::check_with_devices(
                ticket.job(),
                stored,
                &authority,
                report,
                signature,
                resolved.as_deref(),
                devices,
            )?;
            rx_application::process_review::Prepared::new(ticket, validated)
        })
        .await
        .map_err(|_| "review worker failed")?
    }
    pub async fn prepare_review_decision(
        self: &Arc<Self>,
        ticket: rx_application::process_review::DecisionTicket,
    ) -> Result<rx_application::process_review::PreparedDecision, String> {
        if !ticket.requires_verification() {
            return rx_application::process_review::PreparedDecision::reject(ticket);
        }
        let permit = self
            .permit
            .clone()
            .try_acquire_owned()
            .map_err(|_| "package worker busy")?;
        let worker = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let policy = worker.load_policy()?;
            let authority = worker.load_review_authority()?;
            let object = rx_package::store::ObjectId {
                manifest: ticket.job().request.package_manifest,
                signature: ticket.job().request.package_signature,
            };
            let stored = worker
                .store
                .lock()
                .map_err(|_| "store owner failed")?
                .verify_owned(&object, &policy)
                .map_err(|e| e.to_string())?;
            let resolved = ticket
                .resolved()
                .map(rx_domain::canonical::bytes)
                .transpose()
                .map_err(|e| e.to_string())?;
            let devices = worker.verify_process_devices(ticket.job(), &policy)?;
            let validated = rx_application::process_review::Validated::check_with_devices(
                ticket.job(),
                stored,
                &authority,
                ticket.version().report.clone(),
                ticket.version().signature.clone(),
                resolved.as_deref(),
                devices,
            )?;
            rx_application::process_review::PreparedDecision::approve(ticket, validated)
        })
        .await
        .map_err(|_| "review decision worker failed")?
    }
    pub async fn prepare_process_change(
        self: &Arc<Self>,
        ticket: rx_application::process_change::Ticket,
    ) -> Result<rx_application::process_change::Prepared, String> {
        let permit = self
            .permit
            .clone()
            .try_acquire_owned()
            .map_err(|_| "package worker busy")?;
        let worker = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let policy = worker.load_policy()?;
            let authority = worker.load_review_authority()?;
            let object = rx_package::store::ObjectId {
                manifest: ticket.job().request.package_manifest,
                signature: ticket.job().request.package_signature,
            };
            let stored = worker
                .store
                .lock()
                .map_err(|_| "store owner failed")?
                .verify_owned(&object, &policy)
                .map_err(|e| e.to_string())?;
            let resolved =
                rx_domain::canonical::bytes(ticket.resolved()).map_err(|e| e.to_string())?;
            let verified = rx_application::process_review::Validated::check(
                ticket.job(),
                stored,
                &authority,
                ticket.version().report.clone(),
                ticket.version().signature.clone(),
                Some(&resolved),
            )?;
            rx_application::process_change::Prepared::new(ticket, verified)
        })
        .await
        .map_err(|_| "change preparation worker failed")?
    }
    pub async fn prepare_qualification_activation(
        self: &Arc<Self>,
        ticket: rx_application::qualification_activation::Ticket,
    ) -> Result<rx_application::qualification_activation::Prepared, String> {
        let permit = self
            .permit
            .clone()
            .try_acquire_owned()
            .map_err(|_| "qualification issuance worker busy")?;
        let worker = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let qualifier = worker
                .qualification
                .as_ref()
                .ok_or("qualification policy not configured")?;
            let policy = qualifier.current_policy()?;
            let packages = worker.load_policy()?;
            let stored = worker
                .store
                .lock()
                .map_err(|_| "package owner failed")?
                .verify_owned(ticket.package(), &packages)
                .map_err(|e| e.to_string())?;
            rx_application::qualification_activation::Prepared::verify(ticket, &policy, stored)
        })
        .await
        .map_err(|_| "qualification issuance worker stopped")?
    }
    pub fn registration(&self) -> (Id, Digest, Digest) {
        (
            self.owner.clone(),
            self.fingerprint,
            self.policy_file_digest,
        )
    }
    pub async fn prepare(self: &Arc<Self>, ticket: Ticket) -> Result<Prepared, String> {
        // No unbounded blocking-task queue; one acquisition includes at most the policy limits.
        let permit = self
            .permit
            .clone()
            .try_acquire_owned()
            .map_err(|_| "package intake worker is busy")?;
        let worker = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if ticket.registration().store_owner != worker.owner
                || ticket.registration().policy_fingerprint != worker.fingerprint
                || ticket.registration().policy_file_digest != worker.policy_file_digest
            {
                return Err("package intake worker registration differs".into());
            }
            let refreshed = worker.load_policy()?;
            let policy = &refreshed;
            let package = rx_package::directory::verify_relative(
                &worker.root,
                ticket.relative_path(),
                policy,
            )
            .map_err(|e| e.to_string())?;
            let expected = ticket.object();
            let signature = rx_package::content_digest(
                &rx_domain::canonical::bytes(package.signature()).map_err(|e| e.to_string())?,
            );
            if package.digest() != expected.manifest || signature != expected.signature {
                return Err("import source does not match requested object".into());
            }
            let mut store = worker
                .store
                .lock()
                .map_err(|_| "package store owner failed")?;
            let actual = store.put(&package).map_err(|e| e.to_string())?;
            let stored = store
                .verify_owned(&actual, policy)
                .map_err(|e| e.to_string())?;
            Prepared::new(ticket, stored)
        })
        .await
        .map_err(|_| "package intake worker task failed")?
    }
}
mod device_binding;
mod device_review;

impl Worker {
    fn verify_process_devices(
        &self,
        job: &rx_application::process_review::Job,
        policy: &rx_package::VerificationPolicy,
    ) -> Result<Vec<rx_application::device_review::Validated>, String> {
        let Some(context) = &job.device_context else {
            return Ok(vec![]);
        };
        let authority = self.load_device_authority()?;
        context
            .dependencies
            .iter()
            .map(|d| {
                let stored = self
                    .store
                    .lock()
                    .map_err(|_| "store owner failed")?
                    .verify_owned(&d.plan.definition.object, policy)
                    .map_err(|e| e.to_string())?;
                rx_application::device_review::Validated::check(
                    &d.job,
                    stored,
                    &authority,
                    d.version.report.clone(),
                    d.version.signature.clone(),
                )
            })
            .collect()
    }
}
