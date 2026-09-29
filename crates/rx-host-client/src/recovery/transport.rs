//! No deref/accessor exposes the unrestricted HostClient to recovery coordination.
use super::*;
use rx_domain::{host_configuration, host_snapshot::HostSnapshot};

pub(super) struct RestrictedHost {
    client: HostClient,
}
impl RestrictedHost {
    pub(super) async fn connect_rejoin(
        slot: &Slot,
        context: &rx_application::host_rejoin::Context,
        hello: Hello,
    ) -> Result<Self, WorkerError> {
        let client = HostClient::connect_pinned(
            slot.configuration.endpoint.clone(),
            context.host.clone(),
            hello,
            slot.configuration.server_pin,
        )
        .await
        .map_err(|_| WorkerError::Unavailable)?;
        if client.transport_pin() != context.transport.as_ref() {
            return Err(WorkerError::InvalidRead);
        }
        for (cell, cut) in &context.cells {
            if cut.cell.configuration.hosts.contains(&context.host) {
                client
                    .open_configuration_cell(
                        cell,
                        cut.cell.configuration.definition.sha256,
                        &context.clock_id,
                    )
                    .await
                    .map_err(map_rpc)?;
            }
        }
        Ok(Self { client })
    }
    pub(super) async fn connect(
        slot: &Slot,
        context: &data::Context,
        hello: Hello,
    ) -> Result<Self, WorkerError> {
        let client = HostClient::connect_pinned(
            slot.configuration.endpoint.clone(),
            context.host.clone(),
            hello,
            slot.configuration.server_pin,
        )
        .await
        .map_err(|_| WorkerError::Unavailable)?;
        if client.transport_pin() != Some(&context.transport) {
            return Err(WorkerError::InvalidRead);
        }
        for cell in &context.host_cells {
            let cut = context.cells.get(cell).ok_or(WorkerError::InvalidRead)?;
            client
                .open_configuration_cell(
                    cell,
                    cut.configuration.definition.sha256,
                    &context.clock_id,
                )
                .await
                .map_err(map_rpc)?;
        }
        Ok(Self { client })
    }
    pub(super) fn pin(&self) -> &data::TransportPin {
        self.client.transport_pin().expect("pinned recovery client")
    }
    pub(super) fn session(&self) -> Result<Id, WorkerError> {
        Id::new(&self.client.session.session_id).map_err(|_| WorkerError::InvalidRead)
    }
    pub(super) async fn configuration(
        &self,
    ) -> Result<host_configuration::Observation, WorkerError> {
        self.client
            .inspect_process_configuration()
            .await
            .map_err(map_rpc)
    }
    pub(super) async fn snapshot(
        &self,
        cell: &Name,
        sources: Vec<Name>,
    ) -> Result<HostSnapshot, WorkerError> {
        self.client
            .read_bootstrap(cell, sources)
            .await
            .map_err(map_rpc)
    }
    pub(super) async fn fence(
        &self,
        task: &data::FenceTask,
    ) -> Result<rx_application::FenceAcknowledgment, WorkerError> {
        let reply = self
            .client
            .fence(
                &task.request,
                &task.cell,
                task.epoch,
                &task.scopes,
                &task.block_ids,
            )
            .await
            .map_err(map_rpc)?;
        Ok(rx_application::FenceAcknowledgment {
            cell: task.cell.clone(),
            invalidation: task.request.clone(),
            epoch: task.epoch,
            scopes: task.scopes.clone(),
            host_boot: Id::new(reply.host_boot_id).map_err(|_| WorkerError::InvalidRead)?,
            journal: Id::new(reply.delivery_journal_id).map_err(|_| WorkerError::InvalidRead)?,
            sequence: Counter(reply.seq),
        })
    }
    pub(super) async fn handover(
        &self,
        operation: &Id,
    ) -> Result<Vec<rx_application::HandoverObservation>, WorkerError> {
        self.client.handover(operation).await.map_err(map_rpc)
    }
    pub(super) async fn receipt(
        &self,
        operation: &Id,
    ) -> Result<rx_application::HostReceipt, tonic::Status> {
        self.client.receipt(operation).await
    }
    pub(super) async fn lookup(
        &self,
        operation: &Id,
    ) -> Result<rx_application::EvidenceBatch, tonic::Status> {
        self.client.reconcile(operation).await
    }
}
pub(super) fn map_rpc(error: tonic::Status) -> WorkerError {
    match error.code() {
        tonic::Code::Unavailable
        | tonic::Code::DeadlineExceeded
        | tonic::Code::Cancelled
        | tonic::Code::ResourceExhausted => WorkerError::Unavailable,
        _ => WorkerError::InvalidRead,
    }
}

/// Only explicit rebind coordination gets this grant-capable wrapper. Ordinary recovery
/// retains RestrictedHost and cannot access a generic HostClient or native admission methods.
pub(super) struct RebindingHost {
    read: RestrictedHost,
}
impl RebindingHost {
    pub(super) fn new(read: RestrictedHost) -> Self {
        Self { read }
    }
    pub(super) fn reader(&self) -> &RestrictedHost {
        &self.read
    }
    pub(super) async fn grant(
        &self,
        plan: &rx_application::host_link::Plan,
    ) -> Result<(rx_application::Grant, Id), WorkerError> {
        self.read
            .client
            .acquire_grant(
                &plan.grant_request,
                plan.resources.clone(),
                plan.fence,
                plan.ttl_ms,
                plan.prepared_at.clone(),
            )
            .await
            .map_err(map_rpc)
    }
}
