//! One bounded coordinator per authenticated Host; native operation replay rules are unchanged.
use crate::HostClient;
use rx_application::{
    Identity,
    configuration_dispatch::{CellProjection, Emission, Issue, Phase, Task},
};
use rx_domain::{host_configuration::Observation, types::*};
use rx_runtime::application::{ApplicationPort, Command, Reply};
use std::sync::Arc;
type Error = Box<dyn std::error::Error + Send + Sync>;
#[tonic::async_trait]
pub trait Transport: Send + Sync {
    fn host(&self) -> &Name;
    async fn inspect(&self) -> Result<Observation, tonic::Status>;
    async fn open(&self, cells: &[CellProjection], clock: &str) -> Result<(), tonic::Status>;
    async fn lookup(&self, id: &Id) -> Result<Observation, tonic::Status>;
    async fn apply(
        &self,
        request: &rx_domain::host_configuration::Request,
    ) -> Result<Observation, tonic::Status>;
}
#[tonic::async_trait]
impl Transport for HostClient {
    fn host(&self) -> &Name {
        &self.host_id
    }
    async fn inspect(&self) -> Result<Observation, tonic::Status> {
        self.inspect_process_configuration().await
    }
    async fn open(&self, cells: &[CellProjection], clock: &str) -> Result<(), tonic::Status> {
        for c in cells {
            self.open_configuration_cell(&c.cell, c.definition, clock)
                .await?;
        }
        Ok(())
    }
    async fn lookup(&self, id: &Id) -> Result<Observation, tonic::Status> {
        self.lookup_process_configuration(id).await
    }
    async fn apply(
        &self,
        r: &rx_domain::host_configuration::Request,
    ) -> Result<Observation, tonic::Status> {
        self.accept_process_configuration(r).await
    }
}
pub struct Coordinator {
    runtime: Arc<dyn ApplicationPort>,
    transport: Arc<dyn Transport>,
    identity: Identity,
    after: Option<Id>,
    binding_after: Option<Id>,
}
impl Coordinator {
    pub fn new(
        runtime: Arc<dyn ApplicationPort>,
        client: HostClient,
        identity: Identity,
    ) -> Result<Self, Error> {
        Self::with_transport(runtime, Arc::new(client), identity)
    }
    pub fn with_transport(
        runtime: Arc<dyn ApplicationPort>,
        transport: Arc<dyn Transport>,
        identity: Identity,
    ) -> Result<Self, Error> {
        if transport.host() != &identity.principal {
            return Err("configuration coordinator Host differs".into());
        }
        Ok(Self {
            runtime,
            transport,
            identity,
            after: None,
            binding_after: None,
        })
    }
    async fn issue(&self, id: &Id, issue: Issue) -> Result<(), Error> {
        self.runtime
            .request(Command::HostConfigurationIssue {
                identity: self.identity.clone(),
                task: id.clone(),
                issue,
            })
            .await?;
        Ok(())
    }
    async fn record(
        &self,
        id: &Id,
        observation: Observation,
        read_started: TimePoint,
    ) -> Result<Task, Error> {
        let Reply::HostConfigurationTask(t) = self
            .runtime
            .request(Command::RecordHostConfiguration {
                identity: self.identity.clone(),
                task: id.clone(),
                read_started,
                observation: Box::new(observation),
            })
            .await?
        else {
            return Err("configuration record reply".into());
        };
        Ok(*t)
    }
    async fn now(&self) -> Result<TimePoint, Error> {
        let Reply::Time(t) = self.runtime.request(Command::CurrentTime).await? else {
            return Err("clock reply".into());
        };
        Ok(t)
    }
    async fn binding_reads(&mut self) -> Result<(), Error> {
        use rx_application::host_binding_transition::Rejection;
        let Reply::HostBindingIntents(records) = self
            .runtime
            .request(Command::HostBindingIntents {
                identity: self.identity.clone(),
                after: self.binding_after.clone(),
            })
            .await?
        else {
            return Err("Host binding intent list reply".into());
        };
        self.binding_after = if records.len() == 16 {
            records.last().map(|r| r.intent.request.clone())
        } else {
            None
        };
        if records.is_empty() {
            return Ok(());
        }
        let started = self.now().await?;
        let observation = self.transport.inspect().await;
        for record in records {
            let command = match &observation {
                Ok(value) => Command::ObserveHostBindingIntent {
                    identity: self.identity.clone(),
                    request: record.intent.request.clone(),
                    observation: Box::new(value.clone()),
                    read_started: started.clone(),
                },
                Err(_) => Command::HostBindingReadIssue {
                    identity: self.identity.clone(),
                    request: record.intent.request.clone(),
                    issue: Rejection::TransportUnavailable,
                },
            };
            match self.runtime.request(command).await {
                Ok(Reply::HostBindingIntent(_)) => {}
                Err(rx_runtime::writer::WriterError::Rejected(rx_ports::StoreError::Rejected(
                    _,
                ))) => {
                    match self
                        .runtime
                        .request(Command::HostBindingReadIssue {
                            identity: self.identity.clone(),
                            request: record.intent.request,
                            issue: Rejection::AuthorizationChanged,
                        })
                        .await
                    {
                        Ok(Reply::HostBindingIntent(_))
                        | Err(rx_runtime::writer::WriterError::Rejected(
                            rx_ports::StoreError::Rejected(_),
                        )) => {}
                        Err(e) => return Err(Box::new(e)),
                        _ => return Err("Host binding issue reply".into()),
                    }
                }
                Err(e) => return Err(Box::new(e)),
                _ => return Err("Host binding observation reply".into()),
            }
        }
        Ok(())
    }
    pub async fn step(&mut self) -> Result<(), Error> {
        let Reply::HostConfigurationTasks(tasks) = self
            .runtime
            .request(Command::HostConfigurationTasks {
                identity: self.identity.clone(),
                after: self.after.clone(),
            })
            .await?
        else {
            return Err("configuration task list".into());
        };
        self.after = if tasks.len() == 16 {
            tasks.last().map(|t| t.id.clone())
        } else {
            None
        };
        for mut task in tasks {
            if task.integrity_disputed || task.phase == Phase::Retired {
                continue;
            }
            if task.phase == Phase::AwaitingSnapshot {
                let started = self.now().await?;
                let observation = match self.transport.inspect().await {
                    Ok(v) => v,
                    Err(_) => {
                        self.issue(&task.id, Issue::TransportUnavailable).await?;
                        continue;
                    }
                };
                match self
                    .runtime
                    .request(Command::BindHostConfiguration {
                        identity: self.identity.clone(),
                        task: task.id.clone(),
                        observation: Box::new(observation),
                        read_started: started,
                    })
                    .await
                {
                    Ok(Reply::HostConfigurationTask(v)) => task = *v,
                    Err(rx_runtime::writer::WriterError::Rejected(
                        rx_ports::StoreError::Rejected(_),
                    )) => {
                        self.issue(&task.id, Issue::AuthorizationChanged).await?;
                        continue;
                    }
                    Err(e) => return Err(Box::new(e)),
                    _ => return Err("configuration binding reply".into()),
                }
            }
            let mut retry = false;
            if task.phase == Phase::SendEntered {
                let read_started = self.now().await?;
                let observation = match self.transport.lookup(&task.id).await {
                    Ok(v) => v,
                    Err(_) => {
                        self.issue(&task.id, Issue::TransportUnavailable).await?;
                        continue;
                    }
                };
                task = self.record(&task.id, observation, read_started).await?;
                if task.receipt.is_some()
                    || task.integrity_disputed
                    || task.issue != Some(Issue::ReceiptMissing)
                {
                    continue;
                }
                retry = true;
            }
            let now = self.now().await?;
            if self
                .transport
                .open(&task.cells, &now.clock_id)
                .await
                .is_err()
            {
                self.issue(&task.id, Issue::TransportUnavailable).await?;
                continue;
            }
            let emission = match self
                .runtime
                .request(Command::EnterHostConfigurationSend {
                    identity: self.identity.clone(),
                    task: task.id.clone(),
                    retry_missing: retry,
                })
                .await
            {
                Ok(Reply::HostConfigurationEmission(v)) => v,
                Err(rx_runtime::writer::WriterError::Rejected(rx_ports::StoreError::Rejected(
                    _,
                ))) => {
                    self.issue(&task.id, Issue::AuthorizationChanged).await?;
                    continue;
                }
                Err(e) => return Err(Box::new(e)),
                _ => return Err("configuration emission reply".into()),
            };
            if let Emission::Send { request } = emission {
                let read_started = self.now().await?;
                match self.transport.apply(&request).await {
                    Ok(v) => {
                        self.record(&task.id, v, read_started).await?;
                    }
                    Err(_) => {
                        self.issue(&task.id, Issue::TransportUnavailable).await?;
                    }
                }
            }
        }
        self.binding_reads().await?;
        Ok(())
    }
    pub async fn run(mut self, mut stop: tokio::sync::watch::Receiver<bool>) -> Result<(), Error> {
        let mut timer = tokio::time::interval(std::time::Duration::from_millis(500));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if *stop.borrow() || stop.has_changed().is_err() {
                break;
            }
            tokio::select! {
                _ = timer.tick() => {
                    tokio::select! {
                        result = self.step() => result?,
                        _ = stop.changed() => break,
                    }
                },
                changed = stop.changed() => { if changed.is_err() || *stop.borrow() { break; } }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rx_runtime::{
        application::CallFuture,
        writer::{Status, WriterError},
    };
    struct Port {
        entered: tokio::sync::Notify,
        fail: bool,
    }
    impl ApplicationPort for Port {
        fn request(&self, command: Command) -> CallFuture<'_> {
            assert!(matches!(command, Command::HostConfigurationTasks { .. }));
            self.entered.notify_one();
            Box::pin(async move {
                if self.fail {
                    Err(WriterError::Rejected(rx_ports::StoreError::Unavailable(
                        "test writer unavailable".into(),
                    )))
                } else {
                    std::future::pending().await
                }
            })
        }
        fn status(&self) -> Status {
            Status::Running
        }
    }
    struct NeverTransport(Name);
    #[tonic::async_trait]
    impl Transport for NeverTransport {
        fn host(&self) -> &Name {
            &self.0
        }
        async fn inspect(&self) -> Result<Observation, tonic::Status> {
            panic!("no task authorized")
        }
        async fn open(&self, _: &[CellProjection], _: &str) -> Result<(), tonic::Status> {
            panic!("no task authorized")
        }
        async fn lookup(&self, _: &Id) -> Result<Observation, tonic::Status> {
            panic!("no task authorized")
        }
        async fn apply(
            &self,
            _: &rx_domain::host_configuration::Request,
        ) -> Result<Observation, tonic::Status> {
            panic!("no task authorized")
        }
    }
    fn coordinator(port: Arc<Port>) -> Coordinator {
        let host = Name::new("host/test").unwrap();
        Coordinator::with_transport(
            port,
            Arc::new(NeverTransport(host.clone())),
            Identity {
                principal: host,
                session: Id::new(uuid::Uuid::new_v4().to_string()).unwrap(),
                terminal: None,
            },
        )
        .unwrap()
    }
    #[tokio::test]
    async fn configuration_worker_stops_while_a_writer_reply_is_pending() {
        let port = Arc::new(Port {
            entered: tokio::sync::Notify::new(),
            fail: false,
        });
        let (stop, rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(coordinator(port.clone()).run(rx));
        tokio::time::timeout(std::time::Duration::from_secs(2), port.entered.notified())
            .await
            .unwrap();
        stop.send(true).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn configuration_worker_propagates_writer_failure_and_never_contacts_host() {
        let port = Arc::new(Port {
            entered: tokio::sync::Notify::new(),
            fail: true,
        });
        assert!(coordinator(port).step().await.is_err());
    }
}

#[cfg(test)]
mod binding_reader_tests {
    use super::*;
    use rx_application::host_binding_transition as binding;
    use rx_domain::host_configuration as data;
    use rx_runtime::{application::CallFuture, writer::Status};
    use std::{
        collections::BTreeMap,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };
    fn n(s: &str) -> Name {
        Name::new(s).unwrap()
    }
    fn id(v: u8) -> Id {
        Id::new(format!("00000000-0000-4000-8000-{v:012}")).unwrap()
    }
    fn at() -> TimePoint {
        TimePoint {
            clock_id: "test/clock".into(),
            ticks_ns: Counter(100),
        }
    }
    fn observation() -> Observation {
        Observation {
            schema: n("rx.host-process-configuration-observation.v1"),
            snapshot: data::Snapshot {
                schema: n("rx.host-process-configuration-snapshot.v1"),
                host: n("host/test"),
                host_boot: id(1),
                delivery_journal: id(2),
                evidence_journal: Some(id(3)),
                installation_identity: Some(Digest::from_bytes([1; 32])),
                binding_commit: None,
                binding_digest: Digest::from_bytes([2; 32]),
                cells: vec![data::CellObservation {
                    cell: n("cell/a"),
                    definition: Digest::from_bytes([3; 32]),
                    envelope: Digest::from_bytes([4; 32]),
                    environment: n("SIMULATION"),
                    epoch: Counter(1),
                    scopes: BTreeMap::from([(n("scope/a"), Counter(1))]),
                    blocked: vec![],
                    applied: None,
                }],
            },
            receipt: None,
            context_matches_current_host: false,
            activation_authorized: false,
        }
    }
    fn record(i: u8) -> binding::Record {
        let cells = BTreeMap::from([(
            n("cell/a"),
            binding::CellIdentity {
                definition: Digest::from_bytes([3; 32]),
                envelope: Digest::from_bytes([4; 32]),
                environment: n("SIMULATION"),
            },
        )]);
        binding::Record {
            intent: binding::Intent {
                request: id(i),
                change: id(20),
                host: n("host/test"),
                cell: n("cell/a"),
                host_plan_digest: Digest::from_bytes([5; 32]),
                before_configuration: Digest::from_bytes([6; 32]),
                after_configuration: Digest::from_bytes([7; 32]),
                before_cells: cells.clone(),
                after_cells: cells,
                runtime_boot: id(21),
                created_at: at(),
            },
            phase: binding::Phase::AwaitingBaseline,
            baseline: None,
            observation: None,
            read_started: None,
            observed_session: None,
            issue: None,
            created_by: n("release"),
            activation_authorized: false,
            adopted_from: vec![],
        }
    }
    struct Port {
        records: Vec<binding::Record>,
        seen: Mutex<Vec<(&'static str, Id)>>,
    }
    impl ApplicationPort for Port {
        fn status(&self) -> Status {
            Status::Running
        }
        fn request(&self, command: Command) -> CallFuture<'_> {
            Box::pin(async move {
                Ok(match command {
                    Command::HostConfigurationTasks { .. } => Reply::HostConfigurationTasks(vec![]),
                    Command::HostBindingIntents { identity, .. } => {
                        assert_eq!(identity.principal, n("host/test"));
                        Reply::HostBindingIntents(self.records.clone())
                    }
                    Command::CurrentTime => Reply::Time(at()),
                    Command::ObserveHostBindingIntent {
                        request,
                        observation,
                        read_started,
                        ..
                    } => {
                        assert_eq!(read_started, at());
                        observation.validate().unwrap();
                        self.seen.lock().unwrap().push(("OBSERVE", request.clone()));
                        Reply::HostBindingIntent(Box::new(
                            self.records
                                .iter()
                                .find(|r| r.intent.request == request)
                                .unwrap()
                                .clone(),
                        ))
                    }
                    Command::HostBindingReadIssue { request, issue, .. } => {
                        assert_eq!(issue, binding::Rejection::TransportUnavailable);
                        self.seen
                            .lock()
                            .unwrap()
                            .push(("UNAVAILABLE", request.clone()));
                        Reply::HostBindingIntent(Box::new(
                            self.records
                                .iter()
                                .find(|r| r.intent.request == request)
                                .unwrap()
                                .clone(),
                        ))
                    }
                    _ => panic!("binding reader must never send a configuration/native mutation"),
                })
            })
        }
    }
    struct Peer {
        host: Name,
        fail: bool,
        reads: AtomicUsize,
    }
    #[tonic::async_trait]
    impl Transport for Peer {
        fn host(&self) -> &Name {
            &self.host
        }
        async fn inspect(&self) -> Result<Observation, tonic::Status> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err(tonic::Status::unavailable("test reply lost"))
            } else {
                Ok(observation())
            }
        }
        async fn open(&self, _: &[CellProjection], _: &str) -> Result<(), tonic::Status> {
            panic!("no open for metadata reads")
        }
        async fn lookup(&self, _: &Id) -> Result<Observation, tonic::Status> {
            panic!("no configuration lookup")
        }
        async fn apply(&self, _: &data::Request) -> Result<Observation, tonic::Status> {
            panic!("no mutation")
        }
    }
    #[tokio::test]
    async fn one_bounded_read_serves_host_batch_and_transport_loss_is_not_a_match() {
        for fail in [false, true] {
            let port = Arc::new(Port {
                records: vec![record(5), record(6)],
                seen: Mutex::new(vec![]),
            });
            let peer = Arc::new(Peer {
                host: n("host/test"),
                fail,
                reads: AtomicUsize::new(0),
            });
            let mut coordinator = Coordinator::with_transport(
                port.clone(),
                peer.clone(),
                Identity {
                    principal: n("host/test"),
                    session: id(10),
                    terminal: None,
                },
            )
            .unwrap();
            coordinator.step().await.unwrap();
            assert_eq!(peer.reads.load(Ordering::SeqCst), 1);
            let seen = port.seen.lock().unwrap();
            assert_eq!(seen.len(), 2);
            assert!(
                seen.iter()
                    .all(|(kind, _)| *kind == if fail { "UNAVAILABLE" } else { "OBSERVE" })
            );
            assert_eq!(seen[0].1, id(5));
            assert_eq!(seen[1].1, id(6));
        }
    }
}
