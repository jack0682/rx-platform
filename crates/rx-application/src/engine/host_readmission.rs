//! Durable ReleaseManager approval for a restarted Host generation. See crate::host_readmission.
use super::*;
use crate::{host_link::Plan, host_readmission as ra};
const RECORD: &str = "rx.internal.host-readmission.v1";
const REF: &str = "rx.internal.host-readmission-ref.v1";
const PLAN: &str = "rx.internal.host-link-plan.v1";

/// Current approval for a Host, if any. An approval stays current until every cell of the
/// replaced generation has been re-linked; a completed approval is history only.
pub(super) fn current(
    tx: &mut dyn Transaction,
    host: &Name,
) -> Result<Option<(Counter, ra::Record)>> {
    let Some(row) = tx.get(&key("hostreadmission-current", host))? else {
        return Ok(None);
    };
    let id: Id = decode(&row, REF)?;
    let (revision, r): (_, ra::Record) = load(tx, "hostreadmission", &id, RECORD)?;
    if r.id != id || &r.host != host {
        return Err(StoreError::Integrity(
            "Host readmission identity differs".into(),
        ));
    }
    Ok((!r.complete()).then_some((revision, r)))
}
pub(super) fn record(tx: &mut dyn Transaction, id: &Id) -> Result<ra::Record> {
    let (_, r): (_, ra::Record) = load(tx, "hostreadmission", id, RECORD)?;
    if &r.id != id {
        return Err(StoreError::Integrity(
            "Host readmission identity differs".into(),
        ));
    }
    Ok(r)
}
/// Binding transition that governs `boot` of `host` for `cell`: a current approval for a
/// boot other than the replaced one, or the re-admission this boot was already linked under.
pub(super) fn governing_binding(
    tx: &mut dyn Transaction,
    host: &Name,
    cell: &Name,
    boot: &Id,
) -> Result<Option<ra::BindingTransition>> {
    let mut governing = current(tx, host)?
        .map(|(_, r)| r)
        .filter(|r| &r.previous_boot != boot);
    if governing.is_none()
        && let Some(row) = tx.get(&key("host-link-current", (host, cell)))?
    {
        let previous: Id = decode(&row, "rx.internal.host-link-id.v1")?;
        let (_, plan): (_, Plan) = load(tx, "host-link-plan", &previous, PLAN)?;
        if &plan.host_boot == boot
            && let Some(r) = &plan.readmission
        {
            governing = Some(record(tx, r)?);
        }
    }
    Ok(governing
        .and_then(|r| r.binding)
        .filter(|t| &t.cell == cell))
}
/// Configuration a Host admitted under `readmission` must present for `cell`. Only a
/// binding transition for this cell whose change is still staged substitutes the change's
/// after configuration; an applied change already made it current, and any other state
/// falls back to the current configuration so a mismatching Host stays unadmitted.
pub(super) fn expected_configuration(
    tx: &mut dyn Transaction,
    readmission: Option<&ra::Record>,
    cell: &Cell,
) -> Result<CellConfiguration> {
    let Some(t) = readmission
        .and_then(|r| r.binding.as_ref())
        .filter(|t| t.cell == cell.configuration.id)
    else {
        return Ok(cell.configuration.clone());
    };
    let change = process_change::change(tx, &t.change, &t.cell)?;
    if change.state != crate::process_change::State::Staged {
        return Ok(cell.configuration.clone());
    }
    let after = process_change::read_config(tx, &change.after)?;
    if after.definition.sha256 != t.after_definition {
        return Err(StoreError::Integrity(
            "binding transition definition differs".into(),
        ));
    }
    Ok(after)
}
/// True when `previous` is the registration an approval replaces and `snapshot` is a
/// different boot of the same Host storage (both journals kept).
pub(super) fn covers(
    r: &ra::Record,
    previous: &HostRegistration,
    host_boot: &Id,
    delivery_journal: &Id,
    evidence_journal: &Id,
) -> bool {
    previous.id == r.host
        && previous.boot_id == r.previous_boot
        && previous.session == r.previous_session
        && previous.delivery_journal == r.delivery_journal
        && r.cells.get(&previous.cell) == Some(&None)
        && host_boot != &r.previous_boot
        && delivery_journal == &r.delivery_journal
        && evidence_journal == &r.evidence_journal
}
pub(super) fn consume(
    tx: &mut dyn Transaction,
    revision: Counter,
    mut r: ra::Record,
    cell: &Name,
    plan: &Id,
) -> Result<()> {
    match r.cells.get(cell) {
        Some(None) => {}
        _ => return reject(Reject::StaleRevision),
    }
    r.cells.insert(cell.clone(), Some(plan.clone()));
    save(tx, "hostreadmission", &r.id, Some(revision), RECORD, &r)?;
    event(
        tx,
        "rx.event.host-readmission-consumed.v1",
        &(&r, cell, plan),
    )
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn approve_host_readmission(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: ra::Approve,
    ) -> Result<ra::Record> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let mut registrations = vec![];
            for row in tx.scan("host/")? {
                let h: HostRegistration = decode(&row, HOST)?;
                if h.id == input.host {
                    registrations.push(h);
                }
            }
            let Some(first) = registrations.first().cloned() else {
                return reject(Reject::NotFound);
            };
            let mut principal = None;
            for h in &registrations {
                principal = Some(authorize(
                    tx,
                    identity,
                    meta,
                    &now,
                    Some(&h.cell),
                    Role::ReleaseManager,
                    true,
                )?);
            }
            let principal = principal.expect("non-empty");
            let (scope, fp) = request(
                meta,
                &principal,
                "HostReadmission.Approve",
                key_.as_str(),
                &input,
            )?;
            if let Some(id) = prior::<Id>(tx, &scope, fp, REF)? {
                return Ok(load::<ra::Record>(tx, "hostreadmission", &id, RECORD)?.1);
            }
            if current(tx, &input.host)?.is_some() {
                return reject(Reject::Busy);
            }
            let cells: BTreeSet<_> = registrations.iter().map(|h| h.cell.clone()).collect();
            for h in &registrations {
                if h.boot_id != input.previous_boot
                    || h.delivery_journal != input.delivery_journal
                    || h.session != first.session
                {
                    return reject(Reject::ContinuityUnproven);
                }
                // The evidence journal of the replaced generation comes from its bound link.
                let row = tx
                    .get(&key("host-link-current", (&h.id, &h.cell)))?
                    .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?;
                let plan_id: Id = decode(&row, "rx.internal.host-link-id.v1")?;
                let (_, plan): (_, Plan) = load(tx, "host-link-plan", &plan_id, PLAN)?;
                if !plan.bound
                    || plan.host_boot != h.boot_id
                    || plan.evidence_journal != input.evidence_journal
                {
                    return reject(Reject::ContinuityUnproven);
                }
            }
            // Work the replaced generation may still hold must be resolved first; a restart
            // never proves that an accepted command did not act.
            for row in tx.scan("run/")? {
                let run: Run = decode(&row, RUN)?;
                if cells.contains(&run.cell) && run.state == RunState::Executing {
                    return reject(Reject::Busy);
                }
            }
            for row in tx.scan("work/")? {
                let w: Work = decode(&row, WORK)?;
                if cells.contains(&w.cell)
                    && (matches!(w.operation.outcome(), Outcome::None | Outcome::Unresolved)
                        || w.operation.integrity() == rx_domain::operation::Integrity::Disputed)
                {
                    return reject(Reject::Busy);
                }
            }
            let binding = match &input.binding_intent {
                None => None,
                Some(intent_id) => {
                    let (_, b) = super::host_binding_transition::read(tx, intent_id)?;
                    let change = process_change::change(tx, &b.intent.change, &b.intent.cell)?;
                    let baseline = b
                        .baseline
                        .as_ref()
                        .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?;
                    if b.intent.host != input.host
                        || b.intent.runtime_boot != meta.runtime_boot
                        || change.state != crate::process_change::State::Staged
                        || change.preparation.is_none()
                        || change
                            .host_binding_plan
                            .as_ref()
                            .and_then(|p| p.digest().ok())
                            != Some(b.intent.host_plan_digest)
                        || b.intent
                            .before_cells
                            .keys()
                            .cloned()
                            .collect::<BTreeSet<_>>()
                            != cells
                    {
                        return reject(Reject::StaleRevision);
                    }
                    // The baseline must describe exactly the generation being replaced.
                    if baseline.snapshot.host_boot != input.previous_boot
                        || baseline.snapshot.delivery_journal != input.delivery_journal
                        || baseline.snapshot.evidence_journal.as_ref()
                            != Some(&input.evidence_journal)
                        || b.phase != crate::host_binding_transition::Phase::BaselineRecorded
                    {
                        return reject(Reject::ContinuityUnproven);
                    }
                    Some(ra::BindingTransition {
                        intent: intent_id.clone(),
                        change: change.id.clone(),
                        cell: b.intent.cell.clone(),
                        after_definition: b.intent.after_cells[&b.intent.cell].definition,
                    })
                }
            };
            let record = ra::Record {
                schema: name(ra::SCHEMA),
                id: id(),
                host: input.host.clone(),
                previous_session: first.session.clone(),
                previous_boot: input.previous_boot.clone(),
                delivery_journal: input.delivery_journal.clone(),
                evidence_journal: input.evidence_journal.clone(),
                cells: cells.into_iter().map(|c| (c, None)).collect(),
                approved_by: principal.id.clone(),
                approved_at: now,
                binding,
            };
            save(tx, "hostreadmission", &record.id, None, RECORD, &record)?;
            let pointer = key("hostreadmission-current", &record.host);
            let previous = tx.get(&pointer)?;
            tx.put(
                &pointer,
                previous.map(|r| r.revision),
                &doc(REF, &record.id)?,
            )?;
            remember(tx, &scope, fp, REF, &record.id)?;
            event(tx, "rx.event.host-readmission-approved.v1", &record)?;
            Ok(record)
        })
    }
}
