use rx_domain::types::*;
use rx_ports::{Document, Repository};
use rx_runtime::writer::*;
use rx_storage::SqliteRepository;
use std::{
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

enum Command {
    Barrier(tokio::sync::oneshot::Sender<()>, mpsc::Receiver<()>),
    Write(u64),
    Control(u64),
    Panic,
}
struct Ledger {
    store: SqliteRepository,
    order: Arc<Mutex<Vec<u64>>>,
}
impl Processor for Ledger {
    type Command = Command;
    type Reply = ();
    type Error = rx_ports::StoreError;
    fn priority(c: &Command) -> Priority {
        match c {
            Command::Control(_) => Priority::Control,
            _ => Priority::Normal,
        }
    }
    fn process(&mut self, command: Command) -> Result<(), Self::Error> {
        let n = match command {
            Command::Barrier(started, release) => {
                started.send(()).unwrap();
                release.recv().unwrap();
                return Ok(());
            }
            Command::Write(n) | Command::Control(n) => n,
            Command::Panic => panic!("injected writer panic"),
        };
        self.store.transact(|tx| {
            tx.put(
                &Name::new(format!("entry/{n}")).unwrap(),
                None,
                &Document {
                    schema: Name::new("test/entity.v1").unwrap(),
                    value: serde_json::json!({"value":n.to_string()}),
                },
            )?;
            Ok(())
        })?;
        self.order.lock().unwrap().push(n);
        Ok(())
    }
}

async fn start() -> (tempfile::TempDir, Writer<Ledger>, Arc<Mutex<Vec<u64>>>) {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("writer.db");
    let order = Arc::new(Mutex::new(Vec::new()));
    let copy = order.clone();
    let writer = Writer::start(move || {
        Ok(Ledger {
            store: SqliteRepository::open(path)?,
            order: copy,
        })
    })
    .await
    .unwrap();
    (folder, writer, order)
}

#[tokio::test]
async fn dropped_response_still_commits_and_shutdown_drains() {
    let (folder, writer, _) = start().await;
    drop(writer.enqueue(Command::Write(1)).unwrap());
    writer.close();
    assert!(matches!(
        writer.enqueue(Command::Write(2)),
        Err(WriterError::Unavailable)
    ));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), writer.closed())
            .await
            .unwrap(),
        Status::Stopped
    );
    let mut store = SqliteRepository::open(folder.path().join("writer.db")).unwrap();
    assert_eq!(store.snapshot().unwrap().1.len(), 1);
}

#[tokio::test]
async fn normal_load_cannot_consume_control_reserve_and_control_is_scheduled_first() {
    let (_folder, writer, order) = start().await;
    let (started, started_rx) = tokio::sync::oneshot::channel();
    let (release, gate) = mpsc::channel();
    let first = writer.enqueue(Command::Barrier(started, gate)).unwrap();
    started_rx.await.unwrap();
    let mut pending = Vec::new();
    for n in 0..23 {
        pending.push(writer.enqueue(Command::Write(n)).unwrap());
    }
    assert!(matches!(
        writer.enqueue(Command::Write(100)),
        Err(WriterError::Busy)
    ));
    let controls: Vec<_> = (900..908)
        .map(|n| writer.enqueue(Command::Control(n)).unwrap())
        .collect();
    assert!(matches!(
        writer.enqueue(Command::Control(999)),
        Err(WriterError::Busy)
    ));
    release.send(()).unwrap();
    first.wait().await.unwrap();
    for control in controls {
        control.wait().await.unwrap();
    }
    for p in pending {
        p.wait().await.unwrap();
    }
    writer.close();
    assert_eq!(writer.closed().await, Status::Stopped);
    assert_eq!(
        &order.lock().unwrap()[..8],
        &[900, 901, 902, 903, 904, 905, 906, 907]
    );
    assert_eq!(order.lock().unwrap().len(), 31);
}

#[tokio::test]
async fn panic_closes_admission_without_retrying_the_command() {
    let (_folder, writer, _) = start().await;
    let pending = writer.enqueue(Command::Panic).unwrap();
    assert!(matches!(
        pending.wait().await,
        Err(WriterError::Unavailable)
    ));
    assert_eq!(writer.closed().await, Status::Faulted);
    assert!(matches!(
        writer.enqueue(Command::Write(1)),
        Err(WriterError::Unavailable)
    ));
}
