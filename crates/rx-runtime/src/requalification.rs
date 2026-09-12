//! Bounded filesystem/crypto work for qualification evidence; authoritative review stays in P.
use rx_application::requalification as q;
use rx_domain::{canonical, types::*};
use rx_package::PackagePath;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
pub struct Worker {
    root: PathBuf,
    policy_file: PathBuf,
    pin: Digest,
    policy: q::Policy,
    permit: Arc<tokio::sync::Semaphore>,
}
impl Worker {
    pub fn new(root: PathBuf, policy_file: PathBuf, pin: Digest) -> Result<Arc<Self>, String> {
        if !root.is_absolute() || !policy_file.is_absolute() {
            return Err("absolute qualification paths required".into());
        }
        let (policy, digest) = rx_package::policy::read_with_digest::<q::Policy>(&policy_file)
            .map_err(|e| e.to_string())?;
        if digest != pin {
            return Err("qualification policy pin mismatch".into());
        }
        policy.digest()?;
        Ok(Arc::new(Self {
            root,
            policy_file,
            pin,
            policy,
            permit: Arc::new(tokio::sync::Semaphore::new(1)),
        }))
    }
    pub fn policy(&self) -> q::Policy {
        self.policy.clone()
    }
    pub(crate) fn current_policy(&self) -> Result<q::Policy, String> {
        let (p, h) = rx_package::policy::read_with_digest::<q::Policy>(&self.policy_file)
            .map_err(|e| e.to_string())?;
        if h != self.pin || p.digest()? != self.policy.digest()? {
            return Err("qualification policy changed".into());
        }
        Ok(p)
    }
    pub async fn prepare(self: &Arc<Self>, ticket: q::Ticket) -> Result<q::Prepared, String> {
        let permit = self
            .permit
            .clone()
            .try_acquire_owned()
            .map_err(|_| "qualification worker busy")?;
        let worker = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let policy = worker.current_policy()?;
            let read = |path: &str| -> Result<Vec<u8>, String> {
                let path = PackagePath::new(format!("{}/{}", ticket.directory().as_str(), path))?;
                rx_package::directory::read_relative_file(&worker.root, &path, 1_048_576)
                    .map_err(|e| e.to_string())
            };
            let report: q::Report =
                canonical::decode_json(&read("qualification.json")?).map_err(|e| e.to_string())?;
            let signature: rx_package::SignatureEnvelope =
                canonical::decode_json(&read("qualification.sig.json")?)
                    .map_err(|e| e.to_string())?;
            let refs = report.references();
            let mut blobs = BTreeMap::new();
            let mut total = 0u64;
            for r in refs {
                if blobs.contains_key(&r.sha256) {
                    continue;
                }
                if r.size_bytes.0 == 0 || r.size_bytes.0 > q::MAX_ARTIFACT {
                    return Err("qualification artifact bound".into());
                }
                total = total
                    .checked_add(r.size_bytes.0)
                    .ok_or("qualification size overflow")?;
                if total > q::MAX_TOTAL {
                    return Err("qualification bundle too large".into());
                }
                let p = PackagePath::new(format!(
                    "{}/artifacts/{}.bin",
                    ticket.directory().as_str(),
                    r.sha256
                ))?;
                let bytes =
                    rx_package::directory::read_relative_file(&worker.root, &p, r.size_bytes.0)
                        .map_err(|e| e.to_string())?;
                blobs.insert(r.sha256, bytes);
            }
            let verified = q::Verified::check(ticket.job(), &policy, report, signature, blobs)?;
            q::Prepared::new(ticket, verified)
        })
        .await
        .map_err(|_| "qualification worker stopped")?
    }
    pub async fn decide(
        self: &Arc<Self>,
        ticket: q::DecisionTicket,
    ) -> Result<q::PreparedDecision, String> {
        let permit = self
            .permit
            .clone()
            .try_acquire_owned()
            .map_err(|_| "qualification worker busy")?;
        let worker = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            ticket.verify(&worker.current_policy()?)
        })
        .await
        .map_err(|_| "qualification review worker stopped")?
    }
}
