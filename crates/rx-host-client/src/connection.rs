//! Automated bootstrap, distinct from qualification, Arm and observation acceptance.
use crate::{Hello, HostClient, TlsEndpoint};
use rx_application::{
    CellConfiguration, FenceAcknowledgment, HostRegistration, Identity, host_link,
};
use rx_domain::types::*;
use rx_runtime::application::{ApplicationPort, Command, Reply};
use std::sync::Arc;
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub struct ConnectedHost {
    pub client: HostClient,
    pub plan: host_link::Plan,
    pub registration: HostRegistration,
    runtime: Arc<dyn ApplicationPort>,
    pending_commit: Option<host_link::Commit>,
    observation_interval: std::time::Duration,
}
async fn now(runtime: &Arc<dyn ApplicationPort>) -> Result<TimePoint, Error> {
    let Reply::Time(time) = runtime.request(Command::CurrentTime).await? else {
        return Err("clock reply".into());
    };
    Ok(time)
}
impl ConnectedHost {
    /// Both directions must already identify the same Host incarnation; missing publisher stays unprepared.
    pub async fn establish(
        runtime: Arc<dyn ApplicationPort>,
        endpoint: TlsEndpoint,
        server_pin: Digest,
        hello: Hello,
        host: Name,
        configuration: &CellConfiguration,
        ttl_ms: Counter,
    ) -> Result<Self, Error> {
        let client = HostClient::connect_pinned(endpoint, host.clone(), hello, server_pin).await?;
        client
            .open_cell(configuration, &now(&runtime).await?.clock_id)
            .await?;
        Self::from_client(runtime, client, configuration, ttl_ms).await
    }
    pub async fn from_client(
        runtime: Arc<dyn ApplicationPort>,
        client: HostClient,
        configuration: &CellConfiguration,
        ttl_ms: Counter,
    ) -> Result<Self, Error> {
        let interval_ns = configuration
            .fact_specs
            .iter()
            .filter(|s| s.host == client.host_id)
            .map(|s| s.maximum_age_ns.0 / 3)
            .min()
            .unwrap_or(250_000_000)
            .clamp(1_000_000, 250_000_000);
        let observation_interval = std::time::Duration::from_nanos(interval_ns);
        let read_started = now(&runtime).await?;
        let snapshot = client
            .read_bootstrap(
                &configuration.id,
                configuration
                    .fact_specs
                    .iter()
                    .filter(|s| s.host == client.host_id)
                    .map(|s| s.id.clone())
                    .collect(),
            )
            .await?;
        let Reply::HostLink(plan) = runtime
            .request(Command::PrepareHostLink(Box::new(host_link::Prepare {
                host: client.host_id.clone(),
                platform_session: Id::new(&client.session.session_id)?,
                snapshot,
                read_started,
                ttl_ms,
            })))
            .await?
        else {
            return Err("link plan reply".into());
        };
        if plan.bound {
            let Reply::HostRegistration(registration) = runtime
                .request(Command::BoundHostLink(plan.id.clone()))
                .await?
            else {
                return Err("link registration reply".into());
            };
            return Ok(Self {
                runtime,
                client,
                plan: *plan,
                registration: *registration,
                pending_commit: None,
                observation_interval,
            });
        }
        let fence = client
            .fence(
                &plan.fence_request,
                &plan.cell,
                plan.epoch,
                &plan.scopes,
                &plan.block_ids,
            )
            .await?;
        // Persisted before either outbound request. Retries of an existing grant must not
        // move its conservative P expiry forward using a later local send time.
        let sent_at = plan.prepared_at.clone();
        let (grant, host_boot) = client
            .acquire_grant(
                &plan.grant_request,
                plan.resources.clone(),
                plan.fence,
                plan.ttl_ms,
                sent_at.clone(),
            )
            .await?;
        if host_boot != plan.host_boot {
            return Err("Host changed during bootstrap".into());
        }
        let command = host_link::Commit {
            plan: plan.id.clone(),
            fence_receipt: FenceAcknowledgment {
                cell: plan.cell.clone(),
                invalidation: plan.fence_request.clone(),
                epoch: plan.epoch,
                scopes: plan.scopes.clone(),
                host_boot: Id::new(fence.host_boot_id)?,
                journal: Id::new(fence.delivery_journal_id)?,
                sequence: Counter(fence.seq),
            },
            grant,
            grant_sent_at: sent_at,
        };
        let Reply::HostRegistration(registration) = runtime
            .request(Command::CommitHostLink(Box::new(command.clone())))
            .await?
        else {
            return Err("link commit reply".into());
        };
        Ok(Self {
            runtime,
            client,
            plan: *plan,
            registration: *registration,
            pending_commit: Some(command),
            observation_interval,
        })
    }
    pub async fn observe(&self) -> Result<rx_application::observation::BatchReceipt, Error> {
        crate::observation::read_once(
            &self.runtime,
            &self.client,
            &self.plan.id,
            &self
                .plan
                .source_sessions
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
        )
        .await
    }
    pub fn identity(&self) -> Identity {
        Identity {
            principal: self.registration.id.clone(),
            session: self.registration.session.clone(),
            terminal: None,
        }
    }
    pub async fn renew(&mut self) -> Result<(), Error> {
        let Reply::HostRenewal(renewal) = self
            .runtime
            .request(Command::PrepareHostRenewal(self.plan.id.clone()))
            .await?
        else {
            return Err("renewal reply".into());
        };
        let (grant, boot) = self.client.renew_bootstrap(&renewal).await?;
        if boot != self.plan.host_boot {
            return Err("Host changed during renewal".into());
        }
        let Reply::HostRegistration(registration) = self
            .runtime
            .request(Command::CommitHostRenewal { renewal, grant })
            .await?
        else {
            return Err("renewal commit reply".into());
        };
        self.registration = *registration;
        Ok(())
    }
    pub fn initial_commit(&self) -> Option<&host_link::Commit> {
        self.pending_commit.as_ref()
    }
}

#[derive(Clone, Debug)]
pub enum ConnectionStatus {
    WaitingForProducer,
    Connecting,
    Bound {
        host_boot: Id,
        valid_until: TimePoint,
    },
    Attention,
    Stopped,
}
#[derive(Clone)]
pub struct ConnectionConfiguration {
    pub host: Name,
    pub cell: Name,
    pub endpoint: TlsEndpoint,
    pub server_pin: Digest,
    pub release: Digest,
    pub ttl_ms: Counter,
}
/// Lifetime owner for one initial connection. It never adopts a changed incarnation after binding.
pub struct ConnectionService {
    runtime: Arc<dyn ApplicationPort>,
    configuration: ConnectionConfiguration,
    status: tokio::sync::watch::Sender<ConnectionStatus>,
    observation_status: tokio::sync::watch::Sender<crate::observation::ObservationStatus>,
    delivery_status: tokio::sync::watch::Sender<crate::delivery::Report>,
}
impl ConnectionService {
    pub fn new(
        runtime: Arc<dyn ApplicationPort>,
        configuration: ConnectionConfiguration,
    ) -> Result<Self, Error> {
        if configuration.ttl_ms.0 < 1000 || configuration.ttl_ms.0 > 30_000 {
            return Err("connection lease must be 1–30 seconds".into());
        }
        let (status, _) = tokio::sync::watch::channel(ConnectionStatus::WaitingForProducer);
        let (observation_status, _) =
            tokio::sync::watch::channel(crate::observation::ObservationStatus::Waiting);
        let (delivery_status, _) = tokio::sync::watch::channel(crate::delivery::Report::default());
        Ok(Self {
            runtime,
            configuration,
            observation_status,
            delivery_status,
            status,
        })
    }
    pub(crate) fn target(&self) -> rx_application::service_health::Target {
        rx_application::service_health::Target {
            host: self.configuration.host.clone(),
            cell: self.configuration.cell.clone(),
        }
    }
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<ConnectionStatus> {
        self.status.subscribe()
    }
    pub fn subscribe_observations(
        &self,
    ) -> tokio::sync::watch::Receiver<crate::observation::ObservationStatus> {
        self.observation_status.subscribe()
    }
    pub fn subscribe_delivery(&self) -> tokio::sync::watch::Receiver<crate::delivery::Report> {
        self.delivery_status.subscribe()
    }
    pub async fn run(self, mut stopped: tokio::sync::watch::Receiver<bool>) -> Result<(), Error> {
        let mut delay = 100u64;
        let mut connected = loop {
            if *stopped.borrow() || stopped.has_changed().is_err() {
                self.status.send_replace(ConnectionStatus::Stopped);
                self.observation_status
                    .send_replace(crate::observation::ObservationStatus::Stopped);
                return Ok(());
            }
            let attempt = async {
                let producer = match self
                    .runtime
                    .request(Command::CurrentEvidenceProducer(
                        self.configuration.host.clone(),
                    ))
                    .await
                {
                    Ok(Reply::Producer(producer)) => producer,
                    Err(rx_runtime::writer::WriterError::Rejected(
                        rx_ports::StoreError::Rejected(rx_domain::fault::Rejection::NotFound),
                    )) => return Ok(None),
                    Err(error) => return Err::<Option<ConnectedHost>, Error>(error.into()),
                    _ => return Err("producer reply".into()),
                };
                let identity = Identity {
                    principal: producer.principal.clone(),
                    session: producer.session.clone(),
                    terminal: None,
                };
                let Reply::Cell(_, cell) = self
                    .runtime
                    .request(Command::InspectCell {
                        identity,
                        cell: self.configuration.cell.clone(),
                    })
                    .await?
                else {
                    return Err("cell reply".into());
                };
                let Reply::Installation(installation) =
                    self.runtime.request(Command::Installation).await?
                else {
                    return Err("installation reply".into());
                };
                self.status.send_replace(ConnectionStatus::Connecting);
                ConnectedHost::establish(
                    self.runtime.clone(),
                    self.configuration.endpoint.clone(),
                    self.configuration.server_pin,
                    Hello {
                        peer_id: Name::new(installation.id.as_str())?,
                        boot_id: installation.runtime_boot,
                        installation: installation.id,
                        store_generation: installation.store_generation,
                        release_digest: self.configuration.release,
                        clock_id: installation.clock_id,
                    },
                    self.configuration.host.clone(),
                    &cell.configuration,
                    self.configuration.ttl_ms,
                )
                .await
                .map(Some)
            };
            let result = tokio::select! {result=attempt=>result,_=stopped.changed()=>{continue;}};
            match result {
                Ok(Some(connected)) => break connected,
                Ok(None) => {
                    self.status
                        .send_replace(ConnectionStatus::WaitingForProducer);
                }
                Err(_) => {
                    self.status.send_replace(ConnectionStatus::Attention);
                }
            }
            tokio::select! {_=tokio::time::sleep(std::time::Duration::from_millis(delay))=>{},_=stopped.changed()=>{}}
            delay = (delay * 2).min(2000);
        };
        self.status.send_replace(ConnectionStatus::Bound {
            host_boot: connected.plan.host_boot.clone(),
            valid_until: connected.registration.grant.valid_until.clone(),
        });
        let dispatcher = crate::delivery::Dispatcher::new(
            self.runtime.clone(),
            connected.client.clone(),
            connected.identity(),
        )?
        .with_report(self.delivery_status.clone());
        let reader = crate::observation::ObservationReader::new(
            self.runtime.clone(),
            connected.client.clone(),
            connected.plan.id.clone(),
            connected.plan.source_sessions.keys().cloned().collect(),
            connected.observation_interval,
        )?
        .with_status(self.observation_status.clone());
        let configuration_worker = crate::configuration_worker::Coordinator::new(
            self.runtime.clone(),
            connected.client.clone(),
            connected.identity(),
        )?;
        let qualification_worker = crate::qualification_worker::Coordinator::new(
            self.runtime.clone(),
            connected.client.clone(),
            connected.identity(),
        )?;
        let (stop_children, children_stopped) = tokio::sync::watch::channel(false);
        let mut children = tokio::task::JoinSet::<Result<(), Error>>::new();
        let dispatch_stopped = children_stopped.clone();
        children.spawn(async move {
            dispatcher
                .run(dispatch_stopped)
                .await
                .map_err(|e| Box::new(e) as Error)
        });
        children.spawn(configuration_worker.run(children_stopped.clone()));
        children.spawn(qualification_worker.run(children_stopped.clone()));
        children.spawn(reader.run(children_stopped));
        let mut failure: Option<Error> = None;
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(
            (self.configuration.ttl_ms.0 / 3).clamp(100, 1000),
        ));
        loop {
            tokio::select! {
                changed=stopped.changed()=>{if changed.is_err() || *stopped.borrow(){break;}},
                result=children.join_next()=>{
                    failure=Some(match result {Some(Ok(Err(e)))=>e,Some(Err(e))=>Box::new(e),_=>"Host worker stopped unexpectedly".into()});break;
                },
                _=tick.tick()=>{
                    let current=match now(&self.runtime).await {Ok(time)=>time,Err(_)=>{self.status.send_replace(ConnectionStatus::Attention);continue;}};
                    let remaining=connected.registration.grant.valid_until.ticks_ns.0.saturating_sub(current.ticks_ns.0);
                    if current.clock_id!=connected.registration.grant.valid_until.clock_id || remaining<=self.configuration.ttl_ms.0*500_000 {
                        // Retain the sender for fencing/evidence if lease renewal is no longer permitted.
                        // Do not acquire a replacement grant or adopt a new Host boot automatically.
                        if connected.renew().await.is_err() {self.status.send_replace(ConnectionStatus::Attention);}
                        else {self.status.send_replace(ConnectionStatus::Bound {host_boot:connected.plan.host_boot.clone(),valid_until:connected.registration.grant.valid_until.clone()});}
                    }
                }
            }
        }
        let _ = stop_children.send(true);
        while let Some(result) = children.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    failure.get_or_insert(e);
                }
                Err(e) => {
                    failure.get_or_insert(Box::new(e));
                }
            }
        }
        if let Some(error) = failure {
            self.status.send_replace(ConnectionStatus::Attention);
            Err(error)
        } else {
            self.status.send_replace(ConnectionStatus::Stopped);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rx_runtime::{
        application::CallFuture,
        writer::{Status, WriterError},
    };
    struct MissingProducer;
    impl ApplicationPort for MissingProducer {
        fn request(&self, _: Command) -> CallFuture<'_> {
            Box::pin(async {
                Err(WriterError::Rejected(rx_ports::StoreError::Rejected(
                    rx_domain::fault::Rejection::NotFound,
                )))
            })
        }
        fn status(&self) -> Status {
            Status::Running
        }
    }
    #[tokio::test]
    async fn closed_owner_while_waiting_for_producer_stops_without_connecting() {
        let configuration = ConnectionConfiguration {
            host: Name::new("host/test").unwrap(),
            cell: Name::new("cell/a").unwrap(),
            server_pin: Digest::from_bytes([0; 32]),
            release: Digest::from_bytes([1; 32]),
            ttl_ms: Counter(1000),
            endpoint: TlsEndpoint {
                uri: "https://127.0.0.1:1".into(),
                server_name: "localhost".into(),
                server_ca_pem: vec![],
                client_certificate_pem: vec![],
                client_key_pem: vec![],
            },
        };
        let service = ConnectionService::new(Arc::new(MissingProducer), configuration).unwrap();
        let status = service.subscribe();
        let (owner, stopped) = tokio::sync::watch::channel(false);
        drop(owner);
        tokio::time::timeout(std::time::Duration::from_secs(1), service.run(stopped))
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(*status.borrow(), ConnectionStatus::Stopped));
    }
}
