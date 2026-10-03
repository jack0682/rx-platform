use super::*;
use rx_application::workflow_publication::{PreparedPublication, PublishTicket};
impl Worker {
    pub async fn prepare_workflow_publication(
        self: &Arc<Self>,
        ticket: PublishTicket,
    ) -> Result<PreparedPublication, String> {
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
                return Err("publication package root differs".into());
            }
            let policy = worker.load_policy()?;
            let store = worker
                .store
                .lock()
                .map_err(|_| "package store owner failed")?;
            let mut packages = std::collections::BTreeMap::new();
            for (id, object) in ticket.objects() {
                packages.insert(
                    id,
                    store
                        .verify_owned(&object, &policy)
                        .map_err(|e| e.to_string())?,
                );
            }
            PreparedPublication::verify(ticket, packages)
        })
        .await
        .map_err(|_| "publication package worker stopped")?
    }
}
