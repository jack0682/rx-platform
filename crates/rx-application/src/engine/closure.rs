use super::procedure::{reference, same};
use super::*;
use crate::{
    closure::*,
    intervention::{Case, CaseState},
    procedure::{Action, Certainty, StoredRecord},
};
const CASE: &str = "rx.internal.intervention-case.v1";
const POLICY: &str = "rx.internal.close-policy-admission.v1";
const CLEARANCE: &str = "rx.internal.clearance.v1";
const RECEIPT: &str = "rx.internal.close-receipt.v1";

fn unique<T: Ord>(items: &[T]) -> bool {
    items.iter().collect::<BTreeSet<_>>().len() == items.len()
}
fn validate_cohort(cases: &[CaseRevision]) -> Result<()> {
    if cases.is_empty()
        || cases.len() > 64
        || cases.iter().any(|c| c.revision.0 == 0)
        || !unique(&cases.iter().map(|c| &c.case).collect::<Vec<_>>())
    {
        return reject(Reject::InvalidInput);
    }
    Ok(())
}
fn actor(
    tx: &mut dyn Transaction,
    context: &ProcessingContext<'_>,
    cell: &Name,
    cases: &[CaseRevision],
) -> Result<Principal> {
    let principal = authorize(
        tx,
        context.identity,
        context.meta,
        context.now,
        Some(cell),
        Role::RecoveryLead,
        false,
    )?;
    if !crate::procedure::can_report(&principal.roles) {
        return reject(Reject::Forbidden);
    }
    validate_cohort(cases)?;
    // Current membership is checked before a cached result, including after closure.
    for requested in cases {
        let (_, case): (_, Case) = load(tx, "case", &requested.case, CASE)?;
        if case.cell != *cell
            || case.lead != principal.id
            || !case
                .effective_cells
                .iter()
                .all(|c| principal.cells.contains(c))
        {
            return reject(Reject::Forbidden);
        }
    }
    Ok(principal)
}
fn bound_cells(tx: &mut dyn Transaction, cell: &Name) -> Result<Vec<BoundCell>> {
    affected_cells(tx, cell)?
        .iter()
        .map(|id| {
            let (revision, cell): (_, Cell) = load(tx, "cell", id, CELL)?;
            Ok(BoundCell {
                reference: intervention::reference(&cell),
                revision,
                envelope: cell.configuration.envelope.sha256,
            })
        })
        .collect()
}
fn restrict(until: &mut TimePoint, candidate: &TimePoint, now: &TimePoint) -> Result<()> {
    if candidate.clock_id != now.clock_id || candidate.ticks_ns <= now.ticks_ns {
        return reject(Reject::Expired);
    }
    if candidate.ticks_ns < until.ticks_ns {
        *until = candidate.clone();
    }
    Ok(())
}
struct Verified {
    contexts: Vec<BoundCell>,
    policies: Vec<ArtifactRef>,
    evidence_ids: Vec<Id>,
    valid_until: TimePoint,
}
fn verify<A: QualificationAuthority>(
    tx: &mut dyn Transaction,
    authority: &A,
    context: &ProcessingContext<'_>,
    cell_id: &Name,
    cohort: &[CaseRevision],
) -> Result<Verified> {
    let now = context.now;
    let contexts = bound_cells(tx, cell_id)?;
    let (_, session): (_, Session) = load(tx, "session", &context.identity.session, SESSION)?;
    let mut valid_until = session.expires_at;
    let mut evidence_ids = BTreeSet::new();
    let mut policies = BTreeMap::new();
    for requested in cohort {
        let detail = intervention::detail(tx, &requested.case)?;
        check_revision(detail.snapshot.revision, requested.revision)?;
        let case = &detail.snapshot.case;
        let progress = &detail.procedure_progress;
        if case.state != CaseState::Revalidating
            || case.scope_uncertain
            || progress.configuration_changed
            || progress.people.is_empty()
            || progress
                .people
                .values()
                .any(|p| p.active || !p.finished || !p.accounted || !p.handover)
            || case.participants.iter().collect::<BTreeSet<_>>() != progress.people.keys().collect()
            || case.effective_cells.iter().collect::<BTreeSet<_>>()
                != contexts.iter().map(|c| &c.reference.cell).collect()
        {
            return reject(Reject::ConditionUnknown);
        }
        let (_, admitted): (_, Admission) = load(tx, "closepolicy", case.procedure.sha256, POLICY)
            .map_err(|e| missing_as(e, Reject::QualificationRequired))?;
        let policy = &admitted.policy;
        if policy.procedure != case.procedure
            || policy.cell != *cell_id
            || admitted.reference != reference("rx.close-policy.v1", policy)?
            || policy
                .contexts
                .iter()
                .map(|c| &c.cell)
                .collect::<BTreeSet<_>>()
                != contexts.iter().map(|c| &c.reference.cell).collect()
        {
            return reject(Reject::QualificationRequired);
        }
        let (_, procedure): (_, crate::procedure::Admission) = load(
            tx,
            "procedurepolicy",
            case.procedure.sha256,
            "rx.internal.procedure-admission.v1",
        )?;
        let (_, origin): (_, Cell) = load(tx, "cell", cell_id, CELL)?;
        if procedure.reference != case.procedure
            || !authority.verify_procedure(
                &origin.configuration,
                &procedure.reference,
                &procedure.policy,
            )
        {
            return reject(Reject::QualificationRequired);
        }
        let ttl = now
            .ticks_ns
            .0
            .checked_add(policy.maximum_validity_ns.0)
            .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
        restrict(
            &mut valid_until,
            &TimePoint {
                clock_id: now.clock_id.clone(),
                ticks_ns: Counter(ttl),
            },
            now,
        )?;
        let since = progress
            .last_external_change
            .as_ref()
            .unwrap_or(&case.opened_at);
        for (action, record_id) in [
            (Action::PersonnelAccounted, &progress.personnel_record),
            (Action::HandoverAccepted, &progress.handover_record),
        ] {
            let record_id = record_id
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
            let record: &StoredRecord = detail
                .procedure_records
                .iter()
                .find(|r| &r.record.id == record_id)
                .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
            let a = &record.assertions;
            let observed = a
                .observed_at
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
            let until = a
                .valid_until
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
            if record.record.action != action
                || record.record.actor != case.lead
                || a.certainty != Some(Certainty::Reported)
                || a.people.iter().collect::<BTreeSet<_>>() != progress.people.keys().collect()
                || record.recorded_at.clock_id != since.clock_id
                || record.recorded_at.ticks_ns < since.ticks_ns
                || observed.clock_id != since.clock_id
                || observed.ticks_ns < since.ticks_ns
                || now
                    .age_ns(observed)
                    .is_none_or(|age| age > procedure.policy.maximum_report_age_ns.0)
            {
                return reject(Reject::ConditionUnknown);
            }
            restrict(&mut valid_until, until, now)?;
            let report_expiry = observed
                .ticks_ns
                .0
                .checked_add(procedure.policy.maximum_report_age_ns.0)
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            restrict(
                &mut valid_until,
                &TimePoint {
                    clock_id: observed.clock_id.clone(),
                    ticks_ns: Counter(report_expiry),
                },
                now,
            )?;
            evidence_ids.insert(record_id.clone());
        }
        for scope in &policy.contexts {
            let (_, cell): (_, Cell) = load(tx, "cell", &scope.cell, CELL)?;
            if scope.definition != cell.configuration.definition.sha256
                || scope.envelope != cell.configuration.envelope.sha256
                || !authority.verify_close_policy(&cell.configuration, &admitted.reference, policy)
            {
                return reject(Reject::QualificationRequired);
            }
            procedure::require_fences(tx, &scope.cell)?;
            let proof = evaluate(tx, &cell, &scope.conditions, now)?;
            let expiry = proof
                .valid_until
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
            restrict(&mut valid_until, expiry, now)?;
            for id in proof.evidence_ids {
                let (_, evidence): (_, ObservationEvidence) = load(
                    tx,
                    "factevidence",
                    &id,
                    "rx.internal.observation-evidence.v1",
                )?;
                if evidence.acquired_at.clock_id != since.clock_id
                    || evidence
                        .acquired_at
                        .ticks_ns
                        .0
                        .checked_sub(evidence.acquisition_uncertainty_ns.0)
                        .is_none_or(|earliest| earliest < since.ticks_ns.0)
                {
                    return reject(Reject::ConditionUnknown);
                }
                evidence_ids.insert(id);
            }
        }
        policies.insert(admitted.reference.sha256, admitted.reference);
    }
    Ok(Verified {
        contexts,
        policies: policies.into_values().collect(),
        evidence_ids: evidence_ids.into_iter().collect(),
        valid_until,
    })
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Package/release integration only. No public caller-supplied approval boolean.
    pub fn admit_close_policy(
        &mut self,
        identity: &Identity,
        policy: Policy,
        artifact: ArtifactRef,
    ) -> Result<()> {
        let meta = &self.installation;
        let clock = &self.clock;
        let authority = &self.authority;
        self.repository.transact(|tx| {
            let now = clock.now();
            let principal = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&policy.cell),
                Role::Engineer,
                false,
            )?;
            if policy.schema.as_str() != "rx.close-policy.v1"
                || artifact != reference("rx.close-policy.v1", &policy)?
                || policy.maximum_validity_ns.0 == 0
                || policy.contexts.is_empty()
                || policy.contexts.len() > 64
                || policy.dependencies.is_empty()
                || policy.dependencies.len() > 128
                || policy.external_procedure.size_bytes.0 == 0
                || policy.dependencies.iter().any(|d| d.size_bytes.0 == 0)
                || !unique(&policy.contexts.iter().map(|s| &s.cell).collect::<Vec<_>>())
            {
                return reject(Reject::InvalidInput);
            }
            let (_, procedure): (_, crate::procedure::Admission) = load(
                tx,
                "procedurepolicy",
                policy.procedure.sha256,
                "rx.internal.procedure-admission.v1",
            )?;
            if procedure.reference != policy.procedure || procedure.policy.cell != policy.cell {
                return reject(Reject::InvalidInput);
            }
            let contexts = bound_cells(tx, &policy.cell)?;
            if policy
                .contexts
                .iter()
                .map(|c| &c.cell)
                .collect::<BTreeSet<_>>()
                != contexts.iter().map(|c| &c.reference.cell).collect()
            {
                return reject(Reject::InvalidInput);
            }
            for context in &policy.contexts {
                let (_, cell): (_, Cell) = load(tx, "cell", &context.cell, CELL)?;
                if !principal.cells.contains(&context.cell) {
                    return reject(Reject::Forbidden);
                }
                if context.definition != cell.configuration.definition.sha256
                    || context.envelope != cell.configuration.envelope.sha256
                {
                    return reject(Reject::QualificationRequired);
                }
                evaluate_raw(tx, &cell, &context.conditions, &now)?;
                if !authority.verify_close_policy(&cell.configuration, &artifact, &policy) {
                    return reject(Reject::QualificationRequired);
                }
            }
            let k = key("closepolicy", policy.procedure.sha256);
            if let Some(old) = tx.get(&k)? {
                let admitted: Admission = decode(&old, POLICY)?;
                if admitted.reference != artifact {
                    return Err(StoreError::KeyConflict);
                }
                return Ok(());
            }
            let admitted = Admission {
                reference: artifact,
                policy,
                admitted_by: principal.id,
            };
            tx.put(&k, None, &doc(POLICY, &admitted)?)?;
            event(tx, "rx.event.close-policy-admitted.v1", &admitted)
        })
    }
    pub fn prepare_close(
        &mut self,
        identity: &Identity,
        key_: &str,
        command: PrepareClose,
    ) -> Result<Clearance> {
        let meta = &self.installation;
        let clock = &self.clock;
        let authority = &self.authority;
        self.repository.transact(|tx| {
            let now = clock.now();
            let context = ProcessingContext {
                identity,
                meta,
                now: &now,
            };
            let principal = actor(tx, &context, &command.cell, &command.case_revisions)?;
            let (scope, fingerprint) =
                request(meta, &principal, "Cell.PrepareClose", key_, &command)?;
            if let Some(saved) = prior(tx, &scope, fingerprint, CLEARANCE)? {
                return Ok(saved);
            }
            if command.expected_cell.0 == 0
                || command.evidence_ids.is_empty()
                || command.evidence_ids.len() > 1024
                || !unique(&command.evidence_ids)
            {
                return reject(Reject::InvalidInput);
            }
            let (revision, _): (_, Cell) = load(tx, "cell", &command.cell, CELL)?;
            check_revision(revision, command.expected_cell)?;
            let verified = verify(
                tx,
                authority,
                &context,
                &command.cell,
                &command.case_revisions,
            )?;
            if command.evidence_ids.iter().collect::<BTreeSet<_>>()
                != verified.evidence_ids.iter().collect()
            {
                return reject(Reject::ConditionUnknown);
            }
            let clearance = Clearance {
                id: id(),
                cell: command.cell,
                case_revisions: command.case_revisions,
                contexts: verified.contexts,
                policies: verified.policies,
                evidence_ids: verified.evidence_ids,
                prepared_by: principal.id,
                prepared_at: now,
                valid_until: verified.valid_until,
                disposition: Disposition::RemainOutOfService,
                consumed_by: None,
            };
            save(tx, "clearance", &clearance.id, None, CLEARANCE, &clearance)?;
            event(tx, "rx.event.close-prepared.v1", &clearance)?;
            remember(tx, &scope, fingerprint, CLEARANCE, &clearance)?;
            Ok(clearance)
        })
    }
    pub fn close_without_restart(
        &mut self,
        identity: &Identity,
        key_: &str,
        command: CloseWithoutRestart,
    ) -> Result<Receipt> {
        let meta = &self.installation;
        let clock = &self.clock;
        let authority = &self.authority;
        self.repository.transact(|tx| {
            let now = clock.now();
            let context = ProcessingContext {
                identity,
                meta,
                now: &now,
            };
            let principal = actor(tx, &context, &command.cell, &command.case_revisions)?;
            let (scope, fingerprint) =
                request(meta, &principal, "Cell.CloseWithoutRestart", key_, &command)?;
            if let Some(saved) = prior(tx, &scope, fingerprint, RECEIPT)? {
                return Ok(saved);
            }
            let (revision, _): (_, Cell) = load(tx, "cell", &command.cell, CELL)?;
            check_revision(revision, command.expected_cell)?;
            let (clearance_revision, mut clearance): (_, Clearance) =
                load(tx, "clearance", &command.clearance, CLEARANCE)?;
            if clearance.consumed_by.is_some()
                || clearance.case_revisions != command.case_revisions
                || clearance.cell != command.cell
                || clearance.prepared_by != principal.id
            {
                return reject(Reject::StaleRevision);
            }
            let mut until = clearance.valid_until.clone();
            restrict(&mut until, &clearance.valid_until, &now)?;
            let verified = verify(
                tx,
                authority,
                &context,
                &command.cell,
                &command.case_revisions,
            )?;
            if !same(&clearance.contexts, &verified.contexts)?
                || clearance.policies != verified.policies
                || clearance.evidence_ids != verified.evidence_ids
            {
                return reject(Reject::StaleEpoch);
            }
            let receipt_id = id();
            let mut case_revisions = vec![];
            // Removing only the target cases' owned blocks must never remove another case's restriction.
            let mut removable = BTreeSet::new();
            for requested in &command.case_revisions {
                let (_, case): (_, Case) = load(tx, "case", &requested.case, CASE)?;
                removable.extend(case.block_ids);
            }
            let targets: BTreeSet<_> = command
                .case_revisions
                .iter()
                .map(|c| c.case.clone())
                .collect();
            for row in tx.scan("case/")? {
                let case: Case = decode(&row, CASE)?;
                if !targets.contains(&case.id) && case.state != CaseState::Closed {
                    for id in case.block_ids {
                        removable.remove(&id);
                    }
                }
            }
            invalidate_closure(tx, &command.cell, BlockReason::OutOfService)?;
            let mut cells = vec![];
            let mut retained_block_ids = vec![];
            for bound in &verified.contexts {
                let (revision, mut cell): (_, Cell) =
                    load(tx, "cell", &bound.reference.cell, CELL)?;
                cell.mode = Some(OperatingMode::Maintenance);
                cell.blocks.retain(|b| !removable.contains(&b.id));
                cell.open_cases.retain(|c| !targets.contains(c));
                retained_block_ids.extend(cell.blocks.iter().map(|b| b.id.clone()));
                let revision = save(
                    tx,
                    "cell",
                    &bound.reference.cell,
                    Some(revision),
                    CELL,
                    &cell,
                )?;
                cells.push(ClosedCell { revision, cell });
            }
            for requested in &command.case_revisions {
                let (revision, mut case): (_, Case) = load(tx, "case", &requested.case, CASE)?;
                case.state = CaseState::Closed;
                let revision = save(tx, "case", &case.id, Some(revision), CASE, &case)?;
                case_revisions.push(CaseRevision {
                    case: case.id,
                    revision,
                });
            }
            clearance.consumed_by = Some(receipt_id.clone());
            save(
                tx,
                "clearance",
                &clearance.id,
                Some(clearance_revision),
                CLEARANCE,
                &clearance,
            )?;
            let receipt = Receipt {
                id: receipt_id,
                clearance: clearance.id,
                disposition: Disposition::RemainOutOfService,
                case_revisions,
                cells,
                retained_block_ids,
                closed_by: principal.id,
                closed_at: now,
            };
            save(tx, "closure", &receipt.id, None, RECEIPT, &receipt)?;
            event(tx, "rx.event.case-closed-out-of-service.v1", &receipt)?;
            remember(tx, &scope, fingerprint, RECEIPT, &receipt)?;
            Ok(receipt)
        })
    }
}
