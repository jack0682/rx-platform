//! Read-only sampling. Ingestion never issues native commands or grants qualification.
use crate::{HostClient, connection::Error};
use rx_application::observation::{BatchReceipt, HostRead};
use rx_domain::types::*;
use rx_runtime::application::{ApplicationPort, Command, Reply};
use std::{sync::Arc, time::Duration};
#[derive(Clone, Debug)]
pub enum ObservationStatus {
    Waiting,
    Received(BatchReceipt),
    Unavailable,
    Stopped,
}
pub async fn read_once(
    runtime: &Arc<dyn ApplicationPort>,
    client: &HostClient,
    plan: &Id,
    sources: &[Name],
) -> Result<BatchReceipt, Error> {
    let Reply::Time(read_started) = runtime.request(Command::CurrentTime).await? else {
        return Err("clock reply".into());
    };
    let snapshot = client
        .read_bootstrap(&plan_cell(runtime, plan).await?, sources.to_vec())
        .await?;
    let Reply::Observations(receipt) = runtime
        .request(Command::IngestHostRead(Box::new(HostRead {
            plan: plan.clone(),
            snapshot,
            read_started,
        })))
        .await?
    else {
        return Err("observation reply".into());
    };
    Ok(receipt)
}
async fn plan_cell(runtime: &Arc<dyn ApplicationPort>, plan: &Id) -> Result<Name, Error> {
    let Reply::HostRegistration(registration) = runtime
        .request(Command::BoundHostLink(plan.clone()))
        .await?
    else {
        return Err("Host registration reply".into());
    };
    Ok(registration.cell)
}
pub struct ObservationReader {
    runtime: Arc<dyn ApplicationPort>,
    client: HostClient,
    plan: Id,
    sources: Vec<Name>,
    interval: Duration,
    status: tokio::sync::watch::Sender<ObservationStatus>,
}
impl ObservationReader {
    pub fn new(
        runtime: Arc<dyn ApplicationPort>,
        client: HostClient,
        plan: Id,
        sources: Vec<Name>,
        interval: Duration,
    ) -> Result<Self, Error> {
        if interval < Duration::from_millis(1) || interval > Duration::from_secs(1) {
            return Err("observation interval must be 1–1000 ms".into());
        }
        let (status, _) = tokio::sync::watch::channel(ObservationStatus::Waiting);
        Ok(Self {
            runtime,
            client,
            plan,
            sources,
            interval,
            status,
        })
    }
    pub(crate) fn with_status(
        mut self,
        status: tokio::sync::watch::Sender<ObservationStatus>,
    ) -> Self {
        self.status = status;
        self
    }
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<ObservationStatus> {
        self.status.subscribe()
    }
    pub async fn run(self, mut stopped: tokio::sync::watch::Receiver<bool>) -> Result<(), Error> {
        let mut tick = tokio::time::interval(self.interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if *stopped.borrow() || stopped.has_changed().is_err() {
                break;
            }
            tokio::select! {
                _=stopped.changed()=>{},
                _=tick.tick()=>{
                    // Read/ingest can be abandoned on stop. No native effect is sent here.
                    let receipt=tokio::select! {
                        _=stopped.changed()=>continue,
                        result=read_once(&self.runtime,&self.client,&self.plan,&self.sources)=>result,
                    };
                    self.status.send_replace(match receipt {Ok(value)=>ObservationStatus::Received(value),Err(_)=>ObservationStatus::Unavailable});
                }
            }
        }
        self.status.send_replace(ObservationStatus::Stopped);
        Ok(())
    }
}
/// How long the writer may refuse the watchdog as busy before the watchdog gives up. While the
/// writer is saturated no new permit can be issued either, so only permits already issued are
/// unchecked; this matches the shortest dispatch-permit window of the simulation envelope.
const WRITER_BUSY_TOLERANCE: Duration = Duration::from_secs(1);

/// Independent of network sampling: a blocked or failed read cannot postpone expiry checks.
/// A full writer queue is a missed tick, not a failure, until it lasts longer than
/// `WRITER_BUSY_TOLERANCE`; any other writer error ends the watchdog, which stops the runtime.
pub async fn monitor_maintained(
    runtime: Arc<dyn ApplicationPort>,
    stopped: tokio::sync::watch::Receiver<bool>,
) -> Result<(), Error> {
    monitor_maintained_within(runtime, stopped, WRITER_BUSY_TOLERANCE).await
}
async fn monitor_maintained_within(
    runtime: Arc<dyn ApplicationPort>,
    mut stopped: tokio::sync::watch::Receiver<bool>,
    busy_tolerance: Duration,
) -> Result<(), Error> {
    let mut tick = tokio::time::interval(Duration::from_millis(25));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut busy_since = None;
    loop {
        if *stopped.borrow() || stopped.has_changed().is_err() {
            return Ok(());
        }
        tokio::select! {
            _=stopped.changed()=>{},
            _=tick.tick()=>{
                match runtime.request(Command::CheckMaintainedConditions).await {
                    Ok(Reply::MaintainedRevoked(_)) => busy_since = None,
                    Ok(_) => return Err("maintained watchdog reply".into()),
                    Err(rx_runtime::writer::WriterError::Busy) => {
                        let since = *busy_since.get_or_insert_with(tokio::time::Instant::now);
                        if since.elapsed() > busy_tolerance {
                            return Err("maintained watchdog could not reach the busy writer".into());
                        }
                    }
                    Err(error) => return Err(error.into()),
                }
            }
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct MonitorPort {
        calls: AtomicUsize,
        fail: bool,
        /// Answer Busy for the first `busy` calls.
        busy: usize,
    }
    impl ApplicationPort for MonitorPort {
        fn request(&self, command: Command) -> CallFuture<'_> {
            assert!(matches!(command, Command::CheckMaintainedConditions));
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if call < self.busy {
                    Err(WriterError::Busy)
                } else if self.fail {
                    Err(WriterError::Rejected(rx_ports::StoreError::Unavailable(
                        "injected writer failure".into(),
                    )))
                } else {
                    Ok(Reply::MaintainedRevoked(vec![]))
                }
            })
        }
        fn status(&self) -> Status {
            Status::Running
        }
    }
    #[tokio::test]
    async fn maintained_monitor_polls_independently_and_obeys_owner_stop() {
        let port = Arc::new(MonitorPort {
            calls: AtomicUsize::new(0),
            fail: false,
            busy: 0,
        });
        let (owner, stopped) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(monitor_maintained(port.clone(), stopped));
        tokio::time::timeout(Duration::from_secs(2), async {
            while port.calls.load(Ordering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        owner.send(true).unwrap();
        task.await.unwrap().unwrap();
    }
    #[tokio::test]
    async fn maintained_monitor_does_not_hide_writer_failure_or_run_after_owner_disappears() {
        let port = Arc::new(MonitorPort {
            calls: AtomicUsize::new(0),
            fail: true,
            busy: 0,
        });
        let (owner, stopped) = tokio::sync::watch::channel(false);
        assert!(monitor_maintained(port.clone(), stopped).await.is_err());
        assert_eq!(port.calls.load(Ordering::SeqCst), 1);
        drop(owner);
        let (owner, stopped) = tokio::sync::watch::channel(false);
        drop(owner);
        monitor_maintained(port.clone(), stopped).await.unwrap();
        assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn a_briefly_busy_writer_is_a_missed_tick_but_a_lasting_one_ends_the_watchdog() {
        // Busy for a few ticks, then answering: the watchdog keeps running.
        let port = Arc::new(MonitorPort {
            calls: AtomicUsize::new(0),
            fail: false,
            busy: 3,
        });
        let (owner, stopped) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(monitor_maintained_within(
            port.clone(),
            stopped,
            Duration::from_millis(500),
        ));
        tokio::time::timeout(Duration::from_secs(5), async {
            while port.calls.load(Ordering::SeqCst) < 6 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!task.is_finished());
        owner.send(true).unwrap();
        task.await.unwrap().unwrap();
        // Busy for longer than the tolerance: the watchdog fails, so the runtime stops.
        let port = Arc::new(MonitorPort {
            calls: AtomicUsize::new(0),
            fail: false,
            busy: usize::MAX,
        });
        let (_owner, stopped) = tokio::sync::watch::channel(false);
        let started = tokio::time::Instant::now();
        assert!(
            monitor_maintained_within(port.clone(), stopped, Duration::from_millis(100))
                .await
                .is_err()
        );
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(port.calls.load(Ordering::SeqCst) > 1);
    }
}
