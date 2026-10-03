//! Explicit protocol declaration by the existing registered Executor peer.
use super::*;
use rx_process_contract::execution_v2::executor as v2;
const SESSION_V2: &str = "rx.execution-session.v2";
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn negotiate_execution_session(
        &mut self,
        identity: &Identity,
        cell: &Name,
        binding: Digest,
    ) -> Result<v2::Session> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (actor, _, current) = executor_peer::executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                cell,
            )?;
            if binding != v2::binding_hash() {
                return reject(Reject::UnsupportedSchema);
            }
            let value = v2::Session {
                schema: name(v2::SESSION_SCHEMA),
                installation: meta.id.clone(),
                store_generation: meta.store_generation.clone(),
                runtime_boot: meta.runtime_boot.clone(),
                principal: actor.id,
                session: identity.session.clone(),
                cell: cell.clone(),
                definition: current.configuration.definition.sha256,
                binding,
                declared_at: now,
            };
            let key = key("executionsession", (&identity.session, cell));
            let previous = tx.get(&key)?;
            tx.require_workflow_execution_reader()?;
            tx.put(
                &key,
                previous.map(|p| p.revision),
                &doc(SESSION_V2, &value)?,
            )?;
            event(tx, "rx.event.execution-session-negotiated.v2", &value)?;
            Ok(value)
        })
    }
}
pub(super) fn require(
    tx: &mut dyn Transaction,
    identity: &Identity,
    meta: &Installation,
    now: &TimePoint,
    cell: &Cell,
) -> Result<()> {
    executor_peer::executor_scope(
        tx,
        ProcessingContext {
            identity,
            meta,
            now,
        },
        &cell.configuration.id,
    )?;
    let row = tx
        .get(&key(
            "executionsession",
            (&identity.session, &cell.configuration.id),
        ))?
        .ok_or(StoreError::Rejected(Reject::UnsupportedSchema))?;
    let value: v2::Session = decode(&row, SESSION_V2)?;
    if value.schema.as_str() != v2::SESSION_SCHEMA
        || value.installation != meta.id
        || value.store_generation != meta.store_generation
        || value.runtime_boot != meta.runtime_boot
        || value.principal != identity.principal
        || value.session != identity.session
        || value.cell != cell.configuration.id
        || value.definition != cell.configuration.definition.sha256
        || value.binding != v2::binding_hash()
    {
        return reject(Reject::UnsupportedSchema);
    }
    Ok(())
}
