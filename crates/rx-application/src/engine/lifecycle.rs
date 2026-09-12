use super::*;
use crate::lifecycle::*;
const SCHEMA: &str = "rx.internal.runtime-lifecycle.v1";
fn lifecycle(tx: &mut dyn Transaction) -> Result<(Counter, Lifecycle)> {
    let row = tx
        .get(&name("runtime/lifecycle"))?
        .ok_or(StoreError::Integrity("runtime lifecycle missing".into()))?;
    Ok((row.revision, decode(&row, SCHEMA)?))
}
pub(super) fn boot(tx: &mut dyn Transaction, installation: &Installation) -> Result<()> {
    let k = name("runtime/lifecycle");
    let previous = tx.get(&k)?;
    if let Some(row) = &previous {
        let old: Lifecycle = decode(row, SCHEMA)?;
        save(tx, "runtimehistory", &old.runtime_boot, None, SCHEMA, &old)?;
    }
    let current = Lifecycle {
        runtime_boot: installation.runtime_boot.clone(),
        phase: Phase::Serving,
        stop_id: None,
        requested_at: None,
        stopped_at: None,
    };
    tx.put(&k, previous.map(|r| r.revision), &doc(SCHEMA, &current)?)?;
    Ok(())
}
pub(super) fn require_serving(tx: &mut dyn Transaction) -> Result<()> {
    if lifecycle(tx)?.1.phase != Phase::Serving {
        return reject(Reject::Busy);
    }
    Ok(())
}
fn report(tx: &mut dyn Transaction, lifecycle: Lifecycle) -> Result<StopReport> {
    let mut attention = vec![];
    let mut count = 0u64;
    let mut add = |item| {
        count += 1;
        if attention.len() < 128 {
            attention.push(item);
        }
    };
    for row in tx.scan("work/")? {
        let work: Work = decode(&row, WORK)?;
        if work.operation.outcome() != Outcome::NotExecuted
            && work.operation.disposition() != rx_domain::operation::Disposition::Released
        {
            add(Attention::WorkRetained {
                operation: work.operation.id().clone(),
            });
        }
    }
    for row in tx.scan("case/")? {
        let case: crate::intervention::Case = decode(&row, "rx.internal.intervention-case.v1")?;
        if case.state != crate::intervention::CaseState::Closed {
            add(Attention::OpenCase { case: case.id });
        }
    }
    let acks = tx
        .scan("fenceack/")?
        .iter()
        .map(|r| decode::<FenceAcknowledgment>(r, "rx.internal.fence-ack.v1"))
        .collect::<Result<Vec<_>>>()?;
    for row in tx.scan("host/")? {
        let host: HostRegistration = decode(&row, HOST)?;
        let (_, cell): (_, Cell) = load(tx, "cell", &host.cell, CELL)?;
        if !acks.iter().any(|a| {
            a.cell == host.cell
                && a.host_boot == host.boot_id
                && a.journal == host.delivery_journal
                && a.epoch == cell.epoch
                && a.scopes == cell.scope_epochs
        }) {
            add(Attention::HostFenceUnconfirmed {
                cell: host.cell,
                host: host.id,
            });
        }
    }
    Ok(StopReport {
        lifecycle,
        attention,
        truncated: count > 128,
        attention_count: Counter(count),
        physical_shutdown_assessed: false,
    })
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Local process owner only, not a public user/Host RPC. Revocation is durable before listeners stop.
    pub fn request_runtime_stop(&mut self) -> Result<StopReport> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (revision, mut current) = lifecycle(tx)?;
            if current.runtime_boot != meta.runtime_boot {
                return reject(Reject::StaleEpoch);
            }
            if current.phase == Phase::Serving {
                current.phase = Phase::StopRequested;
                current.stop_id = Some(id());
                current.requested_at = Some(clock.now());
                tx.put(
                    &name("runtime/lifecycle"),
                    Some(revision),
                    &doc(SCHEMA, &current)?,
                )?;
                let mut touched = BTreeSet::new();
                for row in tx.scan("cell/")? {
                    let cell: Cell = decode(&row, CELL)?;
                    if !touched.contains(&cell.configuration.id) {
                        touched.extend(invalidate_closure(
                            tx,
                            &cell.configuration.id,
                            BlockReason::AuthorityRevoked,
                        )?);
                    }
                }
                event(tx, "rx.event.runtime-stop-requested.v1", &current)?;
            }
            report(tx, current)
        })
    }
    pub fn runtime_lifecycle(&mut self) -> Result<Lifecycle> {
        self.repository.transact(|tx| Ok(lifecycle(tx)?.1))
    }
    /// Only after ingress/senders have drained. Unresolved physical work is retained and reported.
    pub fn commit_runtime_process_stop(&mut self) -> Result<StopReport> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (revision, mut current) = lifecycle(tx)?;
            if current.runtime_boot != meta.runtime_boot {
                return reject(Reject::StaleEpoch);
            }
            if current.phase == Phase::Serving {
                return reject(Reject::InvalidInput);
            }
            if current.phase == Phase::StopRequested {
                current.phase = Phase::StopCommitted;
                current.stopped_at = Some(clock.now());
                tx.put(
                    &name("runtime/lifecycle"),
                    Some(revision),
                    &doc(SCHEMA, &current)?,
                )?;
                event(tx, "rx.event.runtime-stop-committed.v1", &current)?;
            }
            report(tx, current)
        })
    }
}
