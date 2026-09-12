//! One bounded sender/reconciler per authenticated Host. No native re-submit after an uncertain send.
use crate::HostClient;
mod diagnostic;
mod reconciliation;
use rx_application::{engine::authorization_delivery_id, *};
use rx_domain::types::*;
use rx_ports::StoreError;
use rx_runtime::{
    application::{ApplicationPort, Command, Reply},
    writer::WriterError,
};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tonic::{Code, Status};

#[tonic::async_trait]
pub trait DeliveryTransport: Send + Sync {
    fn host_id(&self) -> &Name;
    async fn arm(
        &self,
        key: &Id,
        attempt: &Id,
        cell: &Name,
        epoch: Counter,
        scopes: &BTreeMap<Name, Counter>,
    ) -> Result<ArmAcknowledgment, Status>;
    async fn arm_clearing(
        &self,
        key: &Id,
        attempt: &Id,
        cell: &Name,
        epoch: Counter,
        scopes: &BTreeMap<Name, Counter>,
        clear: &[Id],
    ) -> Result<ArmAcknowledgment, Status> {
        if !clear.is_empty() {
            return Err(Status::failed_precondition(
                "transport does not support approved block clearance",
            ));
        }
        self.arm(key, attempt, cell, epoch, scopes).await
    }
    async fn fence(
        &self,
        key: &Id,
        cell: &Name,
        epoch: Counter,
        scopes: &BTreeMap<Name, Counter>,
        blocks: &[Id],
    ) -> Result<rx_protocol::cell::FenceReceipt, Status>;
    async fn prepare(&self, key: &Id, work: &Work, permit: &Permit) -> Result<HostReceipt, Status>;
    async fn authorize(
        &self,
        key: &Id,
        work: &Work,
        permit: &Permit,
    ) -> Result<HostReceipt, Status>;
    async fn receipt(&self, operation: &Id) -> Result<HostReceipt, Status>;
    async fn reconcile(&self, operation: &Id) -> Result<EvidenceBatch, Status>;
    async fn handover(&self, operation: &Id) -> Result<Vec<HandoverObservation>, Status>;
}
#[tonic::async_trait]
impl DeliveryTransport for HostClient {
    fn host_id(&self) -> &Name {
        &self.host_id
    }
    async fn arm(
        &self,
        key: &Id,
        attempt: &Id,
        cell: &Name,
        epoch: Counter,
        scopes: &BTreeMap<Name, Counter>,
    ) -> Result<ArmAcknowledgment, Status> {
        HostClient::arm(self, key, attempt, cell, epoch, scopes).await
    }
    async fn arm_clearing(
        &self,
        key: &Id,
        attempt: &Id,
        cell: &Name,
        epoch: Counter,
        scopes: &BTreeMap<Name, Counter>,
        clear: &[Id],
    ) -> Result<ArmAcknowledgment, Status> {
        HostClient::arm_clearing(self, key, attempt, cell, epoch, scopes, clear).await
    }
    async fn fence(
        &self,
        key: &Id,
        cell: &Name,
        epoch: Counter,
        scopes: &BTreeMap<Name, Counter>,
        blocks: &[Id],
    ) -> Result<rx_protocol::cell::FenceReceipt, Status> {
        HostClient::fence(self, key, cell, epoch, scopes, blocks).await
    }
    async fn prepare(&self, key: &Id, work: &Work, permit: &Permit) -> Result<HostReceipt, Status> {
        HostClient::prepare(self, key, work, permit).await
    }
    async fn authorize(
        &self,
        key: &Id,
        work: &Work,
        permit: &Permit,
    ) -> Result<HostReceipt, Status> {
        HostClient::authorize(self, key, work, permit).await
    }
    async fn receipt(&self, operation: &Id) -> Result<HostReceipt, Status> {
        HostClient::receipt(self, operation).await
    }
    async fn reconcile(&self, operation: &Id) -> Result<EvidenceBatch, Status> {
        HostClient::reconcile(self, operation).await
    }
    async fn handover(&self, operation: &Id) -> Result<Vec<HandoverObservation>, Status> {
        HostClient::handover(self, operation).await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("state writer: {0}")]
    Writer(#[from] WriterError<StoreError>),
    #[error("Host RPC: {0}")]
    Rpc(#[from] Status),
    #[error("delivery boundary: {0}")]
    Protocol(&'static str),
}
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub last_pass_at: Option<TimePoint>,
    pub delivered: u64,
    pub reconciled: u64,
    pub attention: u64,
    pub last_error: Option<String>,
}
struct Retry {
    not_before: Instant,
    delay: Duration,
}
pub struct Dispatcher {
    runtime: Arc<dyn ApplicationPort>,
    client: Arc<dyn DeliveryTransport>,
    identity: Identity,
    after: Option<Id>,
    work_after: Option<Id>,
    plan_after: Option<Id>,
    retries: BTreeMap<Id, Retry>,
    report: tokio::sync::watch::Sender<Report>,
    diagnostic: std::sync::Mutex<diagnostic::Changes>,
}
impl Dispatcher {
    pub fn new(
        runtime: Arc<dyn ApplicationPort>,
        client: HostClient,
        identity: Identity,
    ) -> Result<Self, Error> {
        Self::with_transport(runtime, Arc::new(client), identity)
    }
    pub fn with_transport(
        runtime: Arc<dyn ApplicationPort>,
        client: Arc<dyn DeliveryTransport>,
        identity: Identity,
    ) -> Result<Self, Error> {
        if client.host_id() != &identity.principal {
            return Err(Error::Protocol("Host identity mismatch"));
        }
        let (report, _) = tokio::sync::watch::channel(Report::default());
        Ok(Self {
            runtime,
            client,
            identity,
            after: None,
            work_after: None,
            plan_after: None,
            retries: BTreeMap::new(),
            report,
            diagnostic: std::sync::Mutex::new(diagnostic::Changes::default()),
        })
    }
    pub(crate) fn with_report(mut self, report: tokio::sync::watch::Sender<Report>) -> Self {
        self.report = report;
        self
    }
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<Report> {
        self.report.subscribe()
    }
    async fn call(&self, command: Command) -> Result<Reply, Error> {
        Ok(self.runtime.request(command).await?)
    }
    async fn attention(&self, message: &Id, issue: DeliveryIssue) -> Result<(), Error> {
        self.call(Command::DeliveryAttention {
            message: message.clone(),
            issue,
        })
        .await?;
        Ok(())
    }
    fn report_error(&self, error: &Error) {
        self.report.send_modify(|r| {
            r.attention = r.attention.saturating_add(1);
            r.last_error = Some(error.to_string());
        });
        if let Some(line) = self
            .diagnostic
            .lock()
            .ok()
            .and_then(|mut previous| previous.next(&self.identity.principal, error))
        {
            // Diagnostics never change delivery/retry state, even if stderr is unavailable.
            use std::io::Write;
            let _ = writeln!(std::io::stderr().lock(), "{line}");
        }
    }
    fn delayed(&self, message: &Id) -> bool {
        self.retries
            .get(message)
            .is_some_and(|r| r.not_before > Instant::now())
    }
    fn retry(&mut self, message: &Id) {
        let delay = self
            .retries
            .get(message)
            .map(|r| (r.delay * 2).min(Duration::from_secs(5)))
            .unwrap_or(Duration::from_millis(100));
        // Bound diagnostic scheduling memory; durable pending work remains in SQLite.
        if self.retries.len() >= 1024
            && !self.retries.contains_key(message)
            && let Some(key) = self.retries.keys().next().cloned()
        {
            self.retries.remove(&key);
        }
        self.retries.insert(
            message.clone(),
            Retry {
                not_before: Instant::now() + delay,
                delay,
            },
        );
    }
    pub async fn tick(&mut self) -> Result<(), Error> {
        let Reply::PendingDeliveries(mut messages) = self
            .call(Command::PendingDeliveries {
                after: self.after.clone(),
                limit: 8,
            })
            .await?
        else {
            return Err(Error::Protocol("pending delivery reply"));
        };
        self.after = messages.last().map(|m| m.id.clone());
        messages.sort_by_key(|m| {
            if matches!(m.payload, Delivery::Fence { .. }) {
                0
            } else {
                1
            }
        });
        for message in messages {
            let host = match &message.payload {
                Delivery::Arm { host, .. }
                | Delivery::Prepare { host, .. }
                | Delivery::Authorize { host, .. }
                | Delivery::Fence { host, .. } => host,
            };
            if host != &self.identity.principal || self.delayed(&message.id) {
                continue;
            }
            match self.send(&message.id).await {
                Ok(done) => {
                    if done {
                        self.retries.remove(&message.id);
                    } else {
                        self.retry(&message.id);
                    }
                }
                Err(error) => {
                    let issue = match &error {
                        Error::Rpc(status) if status.code() == Code::Unauthenticated => {
                            DeliveryIssue::PeerChanged
                        }
                        Error::Writer(WriterError::Rejected(_)) => DeliveryIssue::PolicyRejected,
                        _ => DeliveryIssue::ResponseUnknown,
                    };
                    let _ = self.attention(&message.id, issue).await;
                    self.report_error(&error);
                    self.retry(&message.id);
                }
            }
        }
        let Reply::ReconciliationWork(work) = self
            .call(Command::ReconciliationWork {
                identity: self.identity.clone(),
                after: self.work_after.clone(),
                limit: 8,
            })
            .await?
        else {
            return Err(Error::Protocol("reconciliation reply"));
        };
        self.work_after = work.last().map(|w| w.operation.id().clone());
        for work in work {
            let message = authorization_delivery_id(work.operation.id());
            if self.delayed(&message) {
                continue;
            }
            if let Err(error) = self.reconcile(&message, &work).await {
                self.report_error(&error);
                self.retry(&message);
            }
        }
        self.query_tick().await?;
        Ok(())
    }
    async fn send(&mut self, message: &Id) -> Result<bool, Error> {
        let Reply::DeliveryPlan(plan) = self
            .call(Command::PlanDelivery {
                identity: self.identity.clone(),
                message: message.clone(),
            })
            .await?
        else {
            return Err(Error::Protocol("delivery plan reply"));
        };
        match &plan.payload {
            Delivery::Arm {
                attempt,
                epoch,
                scopes,
                clear_blocks,
                ..
            } => {
                let ack = self
                    .client
                    .arm_clearing(message, attempt, &plan.cell, *epoch, scopes, clear_blocks)
                    .await?;
                self.call(Command::FinishArm {
                    identity: self.identity.clone(),
                    message: message.clone(),
                    ack,
                })
                .await?;
            }
            Delivery::Fence {
                cell,
                epoch,
                scopes,
                block_ids,
                ..
            } => {
                let receipt = self
                    .client
                    .fence(message, cell, *epoch, scopes, block_ids)
                    .await?;
                let ack = FenceAcknowledgment {
                    cell: cell.clone(),
                    invalidation: message.clone(),
                    epoch: *epoch,
                    scopes: scopes.clone(),
                    host_boot: Id::new(receipt.host_boot_id)
                        .map_err(|_| Error::Protocol("fence boot identity"))?,
                    journal: Id::new(receipt.delivery_journal_id)
                        .map_err(|_| Error::Protocol("fence journal"))?,
                    sequence: Counter(receipt.seq),
                };
                self.call(Command::FinishFence {
                    identity: self.identity.clone(),
                    message: message.clone(),
                    ack,
                })
                .await?;
            }
            Delivery::Prepare { .. } | Delivery::Authorize { .. } => {
                let work = plan
                    .work
                    .as_deref()
                    .ok_or(Error::Protocol("operation delivery without work"))?;
                let permit = plan
                    .permit
                    .as_deref()
                    .ok_or(Error::Protocol("operation delivery without permit"))?;
                let receipt = if plan.first_emission {
                    match plan.payload {
                        Delivery::Prepare { .. } => {
                            self.client.prepare(message, work, permit).await?
                        }
                        _ => self.client.authorize(message, work, permit).await?,
                    }
                } else {
                    match self.client.receipt(work.operation.id()).await {
                        Ok(receipt) => receipt,
                        Err(error) if error.code() == Code::NotFound => {
                            self.attention(message, DeliveryIssue::RemoteNotFound)
                                .await?;
                            return Ok(false);
                        }
                        Err(error) => return Err(error.into()),
                    }
                };
                let prepared = receipt.state == ReceiptState::Prepared;
                self.call(Command::RecordReceipt {
                    identity: self.identity.clone(),
                    message: message.clone(),
                    receipt,
                })
                .await?;
                if prepared && matches!(plan.payload, Delivery::Authorize { .. }) {
                    // An entered authorization is not replayed with its old grant/permit.
                    self.attention(message, DeliveryIssue::ReauthorizationRequired)
                        .await?;
                    return Ok(false);
                }
            }
        }
        self.report
            .send_modify(|r| r.delivered = r.delivered.saturating_add(1));
        Ok(true)
    }
    async fn reconcile(&self, message: &Id, work: &Work) -> Result<(), Error> {
        let receipt = self.client.receipt(work.operation.id()).await?;
        if receipt.state == ReceiptState::Prepared {
            return Ok(());
        }
        self.call(Command::RecordReceipt {
            identity: self.identity.clone(),
            message: message.clone(),
            receipt,
        })
        .await?;
        let batch = self.client.reconcile(work.operation.id()).await?;
        if !batch.records.is_empty() {
            self.call(Command::IngestEvidence {
                identity: self.identity.clone(),
                batch,
            })
            .await?;
        }
        self.report
            .send_modify(|r| r.reconciled = r.reconciled.saturating_add(1));
        Ok(())
    }
    pub async fn run(mut self, mut stop: tokio::sync::watch::Receiver<bool>) -> Result<(), Error> {
        loop {
            if *stop.borrow() {
                return Ok(());
            }
            // Drain the current bounded pass: do not cancel a claimed send between its CAS and RPC.
            if let Err(error) = self.tick().await {
                self.report_error(&error);
            }
            if let Ok(Reply::Time(now)) = self.call(Command::CurrentTime).await {
                self.report.send_modify(|r| r.last_pass_at = Some(now));
            }
            tokio::select! {_=tokio::time::sleep(Duration::from_millis(100))=>{},changed=stop.changed()=>{if changed.is_err() {return Ok(());}}}
        }
    }
}
