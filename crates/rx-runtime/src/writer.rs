use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::{oneshot, watch};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    Control,
    Normal,
}

/// Only application code classifies commands. A wire caller cannot choose its own priority.
pub trait Processor: Send + 'static {
    type Command: Send + 'static;
    type Reply: Send + 'static;
    type Error: Send + 'static;
    fn priority(command: &Self::Command) -> Priority;
    /// Synchronous state/transaction work only; no device/network wait.
    fn process(&mut self, command: Self::Command) -> Result<Self::Reply, Self::Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum WriterError<E> {
    #[error("state writer queue is full")]
    Busy,
    #[error("state writer unavailable; recover the request by its original key")]
    Unavailable,
    #[error("state writer could not start: {0}")]
    ThreadStart(String),
    #[error("application rejected command")]
    Rejected(E),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Starting,
    Running,
    Draining,
    Stopped,
    Faulted,
}

struct Task<P: Processor> {
    priority: Priority,
    command: P::Command,
    response: oneshot::Sender<Result<P::Reply, P::Error>>,
}
struct Queue<P: Processor> {
    control: VecDeque<Task<P>>,
    normal: VecDeque<Task<P>>,
    accepting: bool,
    executing: Option<Priority>,
}
struct Shared<P: Processor> {
    queue: Mutex<Queue<P>>,
    wake: Condvar,
    handles: AtomicUsize,
    status: watch::Sender<Status>,
}
impl<P: Processor> Shared<P> {
    fn lock(&self) -> MutexGuard<'_, Queue<P>> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn close(&self) {
        let mut q = self.lock();
        if q.accepting {
            q.accepting = false;
            self.status.send_replace(Status::Draining);
        }
        self.wake.notify_all();
    }
}
pub struct Writer<P: Processor> {
    shared: Arc<Shared<P>>,
}
impl<P: Processor> Clone for Writer<P> {
    fn clone(&self) -> Self {
        self.shared.handles.fetch_add(1, Ordering::Relaxed);
        Self {
            shared: self.shared.clone(),
        }
    }
}
impl<P: Processor> Drop for Writer<P> {
    fn drop(&mut self) {
        if self.shared.handles.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.shared.close();
        }
    }
}
pub struct Pending<R, E> {
    response: oneshot::Receiver<Result<R, E>>,
}
pub type Submission<P> = Result<
    Pending<<P as Processor>::Reply, <P as Processor>::Error>,
    WriterError<<P as Processor>::Error>,
>;
impl<R, E> Pending<R, E> {
    /// Dropping this receiver never cancels an admitted state transaction.
    pub async fn wait(self) -> Result<R, WriterError<E>> {
        self.response
            .await
            .map_err(|_| WriterError::Unavailable)?
            .map_err(WriterError::Rejected)
    }
}
impl<P: Processor> Writer<P> {
    pub async fn start(
        initialize: impl FnOnce() -> Result<P, P::Error> + Send + 'static,
    ) -> Result<Self, WriterError<P::Error>> {
        let (status, _) = watch::channel(Status::Starting);
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                control: VecDeque::new(),
                normal: VecDeque::new(),
                accepting: true,
                executing: None,
            }),
            wake: Condvar::new(),
            handles: AtomicUsize::new(1),
            status,
        });
        let state = shared.clone();
        let (ready, initialized) = oneshot::channel();
        std::thread::Builder::new()
            .name("rx-state-writer".into())
            .spawn(move || {
                let mut guard = ExitGuard {
                    shared: state.clone(),
                    drained: false,
                };
                let mut processor = match initialize() {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                };
                state.status.send_replace(Status::Running);
                if ready.send(Ok(())).is_err() {
                    state.close();
                }
                let mut burst = 0;
                loop {
                    let task = {
                        let mut q = state.lock();
                        while q.control.is_empty() && q.normal.is_empty() && q.accepting {
                            q = state.wake.wait(q).unwrap_or_else(|e| e.into_inner());
                        }
                        if q.control.is_empty() && q.normal.is_empty() && !q.accepting {
                            break;
                        }
                        let task = if !q.control.is_empty() && (burst < 8 || q.normal.is_empty()) {
                            burst = (burst + 1).min(8);
                            q.control.pop_front()
                        } else {
                            burst = 0;
                            q.normal.pop_front()
                        };
                        let task = task.expect("nonempty writer lane");
                        q.executing = Some(task.priority);
                        task
                    };
                    let result = processor.process(task.command);
                    // Reply loss cannot roll back a transaction that the processor already committed.
                    let _ = task.response.send(result);
                    state.lock().executing = None;
                }
                guard.drained = true;
            })
            .map_err(|e| WriterError::ThreadStart(e.to_string()))?;
        initialized
            .await
            .map_err(|_| WriterError::Unavailable)?
            .map_err(WriterError::Rejected)?;
        Ok(Self { shared })
    }
    pub fn status(&self) -> Status {
        *self.shared.status.borrow()
    }
    pub fn close(&self) {
        self.shared.close();
    }
    pub async fn closed(&self) -> Status {
        let mut status = self.shared.status.subscribe();
        let _ = status
            .wait_for(|s| matches!(s, Status::Stopped | Status::Faulted))
            .await;
        *status.borrow()
    }
    pub fn enqueue(&self, command: P::Command) -> Submission<P> {
        let priority = P::priority(&command);
        let (response, receiver) = oneshot::channel();
        let mut q = self.shared.lock();
        if !q.accepting {
            return Err(WriterError::Unavailable);
        }
        let total = q.control.len() + q.normal.len() + usize::from(q.executing.is_some());
        let lane_full = match priority {
            Priority::Control => {
                q.control.len() + usize::from(q.executing == Some(Priority::Control)) >= 8
            }
            Priority::Normal => {
                q.normal.len() + usize::from(q.executing == Some(Priority::Normal)) >= 24
            }
        };
        if total >= 32 || lane_full {
            return Err(WriterError::Busy);
        }
        let task = Task {
            priority,
            command,
            response,
        };
        match priority {
            Priority::Control => q.control.push_back(task),
            Priority::Normal => q.normal.push_back(task),
        }
        self.shared.wake.notify_one();
        Ok(Pending { response: receiver })
    }
}

struct ExitGuard<P: Processor> {
    shared: Arc<Shared<P>>,
    drained: bool,
}
impl<P: Processor> Drop for ExitGuard<P> {
    fn drop(&mut self) {
        let mut q = self.shared.lock();
        q.accepting = false;
        q.executing = None;
        q.control.clear();
        q.normal.clear();
        self.shared.status.send_replace(if self.drained {
            Status::Stopped
        } else {
            Status::Faulted
        });
        self.shared.wake.notify_all();
    }
}
