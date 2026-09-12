use super::*;
use rx_application::device_review as review;
impl Worker {
    pub fn with_device_review(
        mut worker: Arc<Self>,
        path: PathBuf,
        pin: Digest,
    ) -> Result<Arc<Self>, String> {
        if !path.is_absolute() {
            return Err("absolute device review authority path required".into());
        }
        let (a, d) = rx_package::policy::read_with_digest::<review::Authority>(&path)
            .map_err(|e| e.to_string())?;
        if d != pin {
            return Err("device authority pin differs".into());
        }
        Arc::get_mut(&mut worker)
            .ok_or("configure device authority before sharing worker")?
            .device_review = Some((path, pin, a.digest()?));
        Ok(worker)
    }
    pub fn device_review_digest(&self) -> Option<Digest> {
        self.device_review.as_ref().map(|v| v.2)
    }
    pub(super) fn load_device_authority(&self) -> Result<review::Authority, String> {
        let (path, pin, meaning) = self
            .device_review
            .as_ref()
            .ok_or("device review authority not configured")?;
        let (a, d) = rx_package::policy::read_with_digest::<review::Authority>(path)
            .map_err(|e| e.to_string())?;
        if &d != pin || a.digest()? != *meaning {
            return Err("device authority changed".into());
        }
        Ok(a)
    }
    pub async fn prepare_device_report(
        self: &Arc<Self>,
        ticket: review::Ticket,
    ) -> Result<review::Prepared, String> {
        let permit = self
            .permit
            .clone()
            .try_acquire_owned()
            .map_err(|_| "package worker busy")?;
        let worker = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if ticket.registration().store_owner != worker.owner
                || ticket.registration().policy_fingerprint != worker.fingerprint
                || ticket.registration().policy_file_digest != worker.policy_file_digest
            {
                return Err("device worker registration differs".into());
            }
            let policy = worker.load_policy()?;
            let authority = worker.load_device_authority()?;
            let read = |name: &str| max_file(&worker.root, ticket.directory(), name);
            let report: rx_process_contract::device_review::Report =
                rx_domain::canonical::decode_json(&read("verification.json")?)
                    .map_err(|e| e.to_string())?;
            let signature: rx_package::SignatureEnvelope =
                rx_domain::canonical::decode_json(&read("verification.sig.json")?)
                    .map_err(|e| e.to_string())?;
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
            let verified =
                review::Validated::check(ticket.job(), stored, &authority, report, signature)?;
            review::Prepared::new(ticket, verified)
        })
        .await
        .map_err(|_| "device report worker stopped")?
    }
    pub async fn prepare_device_decision(
        self: &Arc<Self>,
        ticket: review::DecisionTicket,
    ) -> Result<review::PreparedDecision, String> {
        if !ticket.requires_verification() {
            return review::PreparedDecision::reject(ticket);
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
            let authority = worker.load_device_authority()?;
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
            let verified = review::Validated::check(
                ticket.job(),
                stored,
                &authority,
                ticket.version().report.clone(),
                ticket.version().signature.clone(),
            )?;
            review::PreparedDecision::approve(ticket, verified)
        })
        .await
        .map_err(|_| "device decision worker stopped")?
    }
}
fn max_file(
    root: &std::path::Path,
    dir: &rx_package::PackagePath,
    name: &str,
) -> Result<Vec<u8>, String> {
    let path = rx_package::PackagePath::new(format!("{}/{}", dir.as_str(), name))?;
    rx_package::directory::read_relative_file(root, &path, 131_072).map_err(|e| e.to_string())
}
