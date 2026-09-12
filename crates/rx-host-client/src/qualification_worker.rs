//! One bounded coordinator per authenticated Host; native operation replay rules are unchanged.
use crate::HostClient;
use rx_application::{
    Identity,
    configuration_dispatch::{Issue, Phase},
    qualification_activation::{Emission, Task},
};
use rx_domain::{
    host_qualification::{CellTarget, Observation},
    types::*,
};
use rx_runtime::application::{ApplicationPort, Command, Reply};
use std::sync::Arc;
type Error = Box<dyn std::error::Error + Send + Sync>;
#[tonic::async_trait]
pub trait Transport: Send + Sync {
    fn host(&self) -> &Name;
    async fn inspect(&self) -> Result<Observation, tonic::Status>;
    async fn open(&self, cells: &[CellTarget], clock: &str) -> Result<(), tonic::Status>;
    async fn lookup(&self, id: &Id) -> Result<Observation, tonic::Status>;
    async fn apply(
        &self,
        request: &rx_domain::host_qualification::Request,
    ) -> Result<Observation, tonic::Status>;
}
#[tonic::async_trait]
impl Transport for HostClient {
    fn host(&self) -> &Name {
        &self.host_id
    }
    async fn inspect(&self) -> Result<Observation, tonic::Status> {
        self.inspect_qualification().await
    }
    async fn open(&self, cells: &[CellTarget], clock: &str) -> Result<(), tonic::Status> {
        for c in cells {
            self.open_configuration_cell(&c.cell, c.definition, clock)
                .await?;
        }
        Ok(())
    }
    async fn lookup(&self, id: &Id) -> Result<Observation, tonic::Status> {
        self.lookup_qualification(id).await
    }
    async fn apply(
        &self,
        r: &rx_domain::host_qualification::Request,
    ) -> Result<Observation, tonic::Status> {
        self.accept_qualification(r).await
    }
}
pub struct Coordinator {
    runtime: Arc<dyn ApplicationPort>,
    transport: Arc<dyn Transport>,
    identity: Identity,
    after: Option<Id>,
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
        })
    }
    async fn issue(&self, id: &Id, issue: Issue) -> Result<(), Error> {
        self.runtime
            .request(Command::QualificationTaskIssue {
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
        let Reply::QualificationTask(t) = self
            .runtime
            .request(Command::RecordQualificationObservation {
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
    pub async fn step(&mut self) -> Result<(), Error> {
        let Reply::QualificationTasks(tasks) = self
            .runtime
            .request(Command::GetQualificationTasks {
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
            if task.disputed || task.phase == Phase::Retired {
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
                    .request(Command::BindQualificationRequest {
                        identity: self.identity.clone(),
                        task: task.id.clone(),
                        observation: Box::new(observation),
                        read_started: started,
                    })
                    .await
                {
                    Ok(Reply::QualificationTask(v)) => task = *v,
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
                    || task.disputed
                    || task.issue != Some(Issue::ReceiptMissing)
                {
                    continue;
                }
                retry = true;
            }
            let now = self.now().await?;
            if self
                .transport
                .open(
                    &task
                        .request
                        .as_ref()
                        .ok_or("qualification request missing")?
                        .cells,
                    &now.clock_id,
                )
                .await
                .is_err()
            {
                self.issue(&task.id, Issue::TransportUnavailable).await?;
                continue;
            }
            let emission = match self
                .runtime
                .request(Command::EnterQualificationSend {
                    identity: self.identity.clone(),
                    task: task.id.clone(),
                    retry_missing: retry,
                })
                .await
            {
                Ok(Reply::QualificationEmission(v)) => v,
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
