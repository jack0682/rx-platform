//! Release-configured recovery worker. Its transport surface excludes operation admission.
mod progress;
mod query;
mod transport;

use crate::{Hello, HostClient, connection::ConnectionConfiguration};
use rx_application::{Identity, host_recovery as data};
use rx_domain::{fault::Rejection, types::*};
use rx_ports::StoreError;
use rx_runtime::{
    application::{ApplicationPort, Command, Reply},
    host_recovery::{Service, ServiceFuture, WorkerError},
    writer::WriterError,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tokio::sync::Mutex;
use transport::RestrictedHost;

struct Slot {
    configuration: ConnectionConfiguration,
    pin: data::TransportPin,
    cells: BTreeSet<Name>,
    serial: Mutex<()>,
}

pub struct Worker {
    runtime: Arc<dyn ApplicationPort>,
    slots: BTreeMap<Name, Arc<Slot>>,
}
fn reject(reason: Rejection) -> WorkerError {
    WorkerError::Writer(WriterError::Rejected(StoreError::Rejected(reason)))
}
impl Worker {
    /// Inputs are the daemon's already loaded immutable deployment files, never HTTP fields.
    pub fn new(
        runtime: Arc<dyn ApplicationPort>,
        configurations: Vec<ConnectionConfiguration>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut slots: BTreeMap<Name, Slot> = BTreeMap::new();
        for configuration in configurations {
            let pin = HostClient::expected_transport_pin(
                &configuration.endpoint,
                configuration.release,
                configuration.server_pin,
            )?;
            if let Some(previous) = slots.get_mut(&configuration.host) {
                if previous.pin != pin || !previous.cells.insert(configuration.cell.clone()) {
                    return Err("duplicate Host cell or conflicting recovery transport pins".into());
                }
            } else {
                slots.insert(
                    configuration.host.clone(),
                    Slot {
                        cells: BTreeSet::from([configuration.cell.clone()]),
                        configuration,
                        pin,
                        serial: Mutex::new(()),
                    },
                );
            }
        }
        Ok(Self {
            runtime,
            slots: slots
                .into_iter()
                .map(|(key, value)| (key, Arc::new(value)))
                .collect(),
        })
    }
    /// Register expected transport declarations. This performs no network I/O or Host operation.
    pub async fn register(&self) -> Result<(), WorkerError> {
        for (host, slot) in &self.slots {
            match self
                .runtime
                .request(Command::RegisterHostRecoveryTransport {
                    host: host.clone(),
                    pin: slot.pin.clone(),
                })
                .await?
            {
                Reply::Done => {}
                _ => return Err(WorkerError::InvalidRead),
            }
        }
        Ok(())
    }
    fn slot(&self, host: &Name) -> Result<Arc<Slot>, WorkerError> {
        self.slots
            .get(host)
            .cloned()
            .ok_or(WorkerError::Unavailable)
    }
    async fn now(&self) -> Result<TimePoint, WorkerError> {
        match self.runtime.request(Command::CurrentTime).await? {
            Reply::Time(value) => Ok(value),
            _ => Err(WorkerError::InvalidRead),
        }
    }
    async fn view(&self, identity: Identity, id: Id) -> Result<data::View, WorkerError> {
        match self
            .runtime
            .request(Command::GetHostRecovery { identity, id })
            .await?
        {
            Reply::HostRecoveryView(value) => Ok(*value),
            _ => Err(WorkerError::InvalidRead),
        }
    }
    async fn context_value(
        &self,
        identity: Identity,
        host: Name,
        origin: Name,
    ) -> Result<data::Context, WorkerError> {
        match self
            .runtime
            .request(Command::HostRecoveryContext {
                identity,
                host,
                origin,
            })
            .await?
        {
            Reply::HostRecoveryContext(value) => Ok(*value),
            _ => Err(WorkerError::InvalidRead),
        }
    }
    async fn connect(
        &self,
        slot: &Slot,
        context: &data::Context,
    ) -> Result<RestrictedHost, WorkerError> {
        if slot.pin != context.transport
            || context.host != slot.configuration.host
            || context
                .host_cells
                .iter()
                .any(|cell| !slot.cells.contains(cell))
        {
            return Err(reject(Rejection::ContinuityUnproven));
        }
        let installation = match self.runtime.request(Command::Installation).await? {
            Reply::Installation(value) => value,
            _ => return Err(WorkerError::InvalidRead),
        };
        if installation.id != context.installation
            || installation.store_generation != context.store_generation
            || installation.runtime_boot != context.runtime_boot
            || installation.clock_id != context.clock_id
        {
            return Err(reject(Rejection::StaleRevision));
        }
        RestrictedHost::connect(
            slot,
            context,
            Hello {
                peer_id: Name::new(installation.id.as_str())
                    .map_err(|_| WorkerError::InvalidRead)?,
                boot_id: installation.runtime_boot,
                installation: installation.id,
                store_generation: installation.store_generation,
                release_digest: slot.configuration.release,
                clock_id: installation.clock_id,
            },
        )
        .await
    }
    async fn read(
        &self,
        remote: &RestrictedHost,
        context: &data::Context,
    ) -> Result<data::VerifiedRead, WorkerError> {
        let configuration_started = self.now().await?;
        let configuration = remote.configuration().await?;
        let configuration_finished = self.now().await?;
        let mut cells = BTreeMap::new();
        for cell in &context.host_cells {
            let cut = context.cells.get(cell).ok_or(WorkerError::InvalidRead)?;
            let sources = cut
                .configuration
                .fact_specs
                .iter()
                .filter(|source| source.host == context.host)
                .map(|source| source.id.clone())
                .collect();
            let started = self.now().await?;
            let snapshot = remote.snapshot(cell, sources).await?;
            let finished = self.now().await?;
            cells.insert(
                cell.clone(),
                data::SnapshotRead {
                    snapshot,
                    started,
                    finished,
                },
            );
        }
        data::VerifiedRead::new(
            remote.session()?,
            remote.pin().clone(),
            configuration,
            configuration_started,
            configuration_finished,
            cells,
        )
        .map_err(|_| WorkerError::InvalidRead)
    }
}
impl Service for Worker {
    fn context(
        &self,
        identity: Identity,
        host: Name,
        origin: Name,
    ) -> ServiceFuture<'_, data::Context> {
        Box::pin(self.context_value(identity, host, origin))
    }
    fn list(
        &self,
        identity: Identity,
        host: Name,
        after: Option<Id>,
        limit: usize,
    ) -> ServiceFuture<'_, data::RecoveryPage> {
        Box::pin(async move {
            match self
                .runtime
                .request(Command::ListHostRecoveries {
                    identity,
                    host,
                    after,
                    limit,
                })
                .await?
            {
                Reply::HostRecoveryPage(value) => Ok(*value),
                _ => Err(WorkerError::InvalidRead),
            }
        })
    }
    fn get(&self, identity: Identity, id: Id) -> ServiceFuture<'_, data::View> {
        Box::pin(self.view(identity, id))
    }
    fn propose(
        &self,
        identity: Identity,
        key: Id,
        input: data::Prepare,
    ) -> ServiceFuture<'_, data::View> {
        Box::pin(async move {
            match self
                .runtime
                .request(Command::LookupHostRecoveryProposal {
                    identity: identity.clone(),
                    key: key.clone(),
                    input: input.clone(),
                })
                .await?
            {
                Reply::OptionalHostRecoveryBinding(Some(value)) => {
                    return self.view(identity, value.id.clone()).await;
                }
                Reply::OptionalHostRecoveryBinding(None) => {}
                _ => return Err(WorkerError::InvalidRead),
            }
            let context = self
                .context_value(identity.clone(), input.host.clone(), input.origin.clone())
                .await?;
            if context.digest().map_err(|_| WorkerError::InvalidRead)? != input.expected_context
                || context.expected_cells() != input.expected_cells
            {
                return Err(reject(Rejection::StaleRevision));
            }
            if !context.blockers.is_empty() {
                return Err(reject(Rejection::ContinuityUnproven));
            }
            let slot = self.slot(&context.host)?;
            let _held = slot.serial.try_lock().map_err(|_| WorkerError::Busy)?;
            let remote = self.connect(&slot, &context).await?;
            let read = self.read(&remote, &context).await?;
            match self
                .runtime
                .request(Command::ProposeHostRecovery {
                    identity: identity.clone(),
                    key,
                    input,
                    read: Box::new(read),
                })
                .await?
            {
                Reply::HostRecoveryBinding(value) => self.view(identity, value.id.clone()).await,
                _ => Err(WorkerError::InvalidRead),
            }
        })
    }
    fn approve(
        &self,
        identity: Identity,
        key: Id,
        input: data::Approve,
    ) -> ServiceFuture<'_, data::View> {
        Box::pin(async move {
            let initial = self.view(identity.clone(), input.id.clone()).await?;
            let slot = self.slot(&initial.binding.context.host)?;
            let _held = slot.serial.try_lock().map_err(|_| WorkerError::Busy)?;
            let binding = match self
                .runtime
                .request(Command::ApproveHostRecovery {
                    identity: identity.clone(),
                    key,
                    input,
                })
                .await?
            {
                Reply::HostRecoveryBinding(value) => *value,
                _ => return Err(WorkerError::InvalidRead),
            };
            self.progress_locked(identity, &slot, binding).await
        })
    }
    fn progress(&self, identity: Identity, id: Id) -> ServiceFuture<'_, data::View> {
        Box::pin(async move {
            let view = self.view(identity.clone(), id).await?;
            let slot = self.slot(&view.binding.context.host)?;
            let _held = slot.serial.try_lock().map_err(|_| WorkerError::Busy)?;
            self.progress_locked(identity, &slot, view.binding).await
        })
    }
    fn query(
        &self,
        identity: Identity,
        id: Id,
        operation: Id,
    ) -> ServiceFuture<'_, data::QueryResult> {
        Box::pin(self.query_operation(identity, id, operation))
    }
}
