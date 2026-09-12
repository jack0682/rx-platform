use super::*;
use rx_application::{device_binding as binding, device_review};
impl Worker {
    pub async fn prepare_device_binding(
        self: &Arc<Self>,
        ticket: binding::Ticket,
    ) -> Result<binding::Prepared, String> {
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
                .map_err(|_| "package store owner failed")?
                .verify_owned(&object, &policy)
                .map_err(|e| e.to_string())?;
            let verified = device_review::Validated::check(
                ticket.job(),
                stored,
                &authority,
                ticket.version().report.clone(),
                ticket.version().signature.clone(),
            )?;
            binding::Prepared::new(ticket, verified)
        })
        .await
        .map_err(|_| "device binding worker stopped")?
    }
}
