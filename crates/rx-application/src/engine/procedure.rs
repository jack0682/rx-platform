use super::*;
use crate::{
    intervention::{Case, CaseState, CaseType},
    procedure::*,
};
const CASE: &str = "rx.internal.intervention-case.v1";
const POLICY: &str = "rx.internal.procedure-admission.v1";
const REPORT: &str = "rx.internal.procedure-record.v1";
const RECEIPT: &str = "rx.internal.procedure-receipt.v1";
const PROGRESS: &str = "rx.internal.procedure-progress.v1";
pub(super) fn same<T: Serialize>(a: &T, b: &T) -> Result<bool> {
    Ok(canonical::bytes(a).map_err(domain_error)? == canonical::bytes(b).map_err(domain_error)?)
}
pub(super) fn reference<T: Serialize>(schema: &str, value: &T) -> Result<ArtifactRef> {
    use sha2::Digest as _;
    let bytes = canonical::bytes(value).map_err(domain_error)?;
    Ok(ArtifactRef {
        sha256: Digest::from_bytes(sha2::Sha256::digest(&bytes).into()),
        schema_id: name(schema),
        size_bytes: Counter(bytes.len() as u64),
    })
}
fn unique<T: Ord>(values: &[T]) -> bool {
    values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
fn utc(value: &str) -> Result<()> {
    let parsed = time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map_err(|_| StoreError::Rejected(Reject::InvalidInput))?;
    if value.len() > 128 || !parsed.offset().is_utc() || value.ends_with("-00:00") {
        return reject(Reject::InvalidInput);
    }
    Ok(())
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Trusted package/release integration, denied by the default authority implementation.
    pub fn admit_procedure(
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
            authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&policy.cell),
                Role::Engineer,
                false,
            )?;
            let (_, cell): (_, Cell) = load(tx, "cell", &policy.cell, CELL)?;
            if artifact != reference("rx.procedure-policy.v1", &policy)?
                || policy.schema.as_str() != "rx.procedure-policy.v1"
                || policy.definition != cell.configuration.definition.sha256
                || policy.envelope != cell.configuration.envelope.sha256
                || policy.external_procedure.size_bytes.0 == 0
                || policy.dependencies.is_empty()
                || policy.dependencies.iter().any(|a| a.size_bytes.0 == 0)
                || policy.steps.is_empty()
                || policy.steps.len() > 64
                || policy.entry_conditions.is_empty()
                || policy.maximum_report_age_ns.0 == 0
                || policy.case_types.is_empty()
                || !unique(&policy.steps.iter().map(|s| &s.id).collect::<Vec<_>>())
            {
                return reject(Reject::InvalidInput);
            }
            evaluate_raw(tx, &cell, &policy.entry_conditions, &now)?;
            for action in [
                Action::EntryConditionsReported,
                Action::WorkStarted,
                Action::WorkFinished,
                Action::PersonnelAccounted,
                Action::HandoverAccepted,
            ] {
                if !policy.steps.iter().any(|s| s.action == action) {
                    return reject(Reject::InvalidInput);
                }
            }
            for step in &policy.steps {
                if !step.conditions.is_empty() {
                    evaluate_raw(tx, &cell, &step.conditions, &now)?;
                }
                if step.actors.is_empty()
                    || !unique(&step.actors)
                    || !unique(&step.required_evidence)
                {
                    return reject(Reject::InvalidInput);
                }
                for actor in &step.actors {
                    let (_, p): (_, Principal) = load(tx, "principal", actor, PRINCIPAL)?;
                    if !p.active || !can_report(&p.roles) || !p.cells.contains(&policy.cell) {
                        return reject(Reject::Forbidden);
                    }
                }
                if step
                    .required_evidence
                    .iter()
                    .any(|id| !cell.configuration.fact_specs.iter().any(|f| &f.id == id))
                {
                    return reject(Reject::InvalidInput);
                }
            }
            if !authority.verify_procedure(&cell.configuration, &artifact, &policy) {
                return reject(Reject::QualificationRequired);
            }
            let k = key("procedurepolicy", artifact.sha256);
            let value = Admission {
                policy,
                reference: artifact,
                admitted_by: identity.principal.clone(),
            };
            if let Some(old) = tx.get(&k)? {
                let old: Admission = decode(&old, POLICY)?;
                if !same(&old.policy, &value.policy)? {
                    return Err(StoreError::Integrity("procedure artifact conflict".into()));
                }
                return Ok(());
            }
            tx.put(&k, None, &doc(POLICY, &value)?)?;
            event(tx, "rx.event.procedure-policy-admitted.v1", &value)
        })
    }
    pub fn record_procedure(
        &mut self,
        identity: &Identity,
        key_: &str,
        command: Submission,
    ) -> Result<Receipt> {
        let clock = &self.clock;
        let meta = &self.installation;
        let authority = &self.authority;
        self.repository.transact(|tx| {
            let now = clock.now();
            let actor = authorize_read(tx, identity, meta, &now, &command.cell)?;
            if !can_report(&actor.roles) || command.record.actor != actor.id {
                return reject(Reject::Forbidden);
            }
            let assertions = if let Some(value) = &command.assertions {
                if reference("rx.procedure-assertions.v1", value)? != command.record.assertions {
                    return reject(Reject::InvalidInput);
                }
                value.clone()
            } else {
                let row = tx
                    .get(&key(
                        "procedureassertions",
                        command.record.assertions.sha256,
                    ))?
                    .ok_or(StoreError::Rejected(Reject::NotFound))?;
                let value: Assertions = decode(&row, "rx.procedure-assertions.v1")?;
                if reference("rx.procedure-assertions.v1", &value)? != command.record.assertions {
                    return Err(StoreError::Integrity(
                        "procedure assertion bytes differ".into(),
                    ));
                }
                value
            };
            let (scope, fingerprint) = request(
                meta,
                &actor,
                "Cell.RecordProcedure",
                key_,
                &(
                    &command.cell,
                    command.expected_cell,
                    command.expected_case,
                    &command.record,
                ),
            )?;
            if let Some(saved) = prior(tx, &scope, fingerprint, RECEIPT)? {
                return Ok(Ok(saved));
            }
            let (case_revision, mut case): (_, Case) =
                load(tx, "case", &command.record.case, CASE)?;
            let was_closed = case.state == CaseState::Closed;
            if !case.effective_cells.contains(&command.cell) {
                return reject(Reject::Forbidden);
            }
            validate_report(&command, &assertions, &case, &actor)?;
            let mut progress = load_progress(tx, &case.id)?;
            let before_context = current_context(tx, &case)?;
            let (cell_revision, _): (_, Cell) = load(tx, "cell", &command.cell, CELL)?;
            let stale = command.expected_case != case_revision
                || command.expected_cell.is_some_and(|r| r != cell_revision);
            let report_key = key("procedurerecord", &command.record.id);
            let duplicate = if let Some(row) = tx.get(&report_key)? {
                let old: StoredRecord = decode(&row, REPORT)?;
                if !same(&old.record, &command.record)? || !same(&old.assertions, &assertions)? {
                    latch(tx, &mut case, BlockReason::IntegrityConflict)?;
                    case.state = CaseState::Escalated;
                    let current = tx
                        .get(&key("case", &case.id))?
                        .ok_or(StoreError::Integrity("case missing".into()))?;
                    save(tx, "case", &case.id, Some(current.revision), CASE, &case)?;
                    event(
                        tx,
                        "rx.event.procedure-report-conflict.v1",
                        &(&old, &command.record),
                    )?;
                    return Ok(Err(StoreError::KeyConflict));
                }
                Some(old)
            } else {
                None
            };
            if let Some(source_event) = &assertions.source_event {
                let k = key("proceduresourceevent", (&assertions.source, source_event));
                if let Some(row) = tx.get(&k)? {
                    let original: Id = decode(&row, "rx.internal.procedure-source-event.v1")?;
                    if original != command.record.id {
                        return reject(Reject::InvalidInput);
                    }
                } else {
                    tx.put(
                        &k,
                        None,
                        &doc("rx.internal.procedure-source-event.v1", &command.record.id)?,
                    )?;
                }
            }
            if let Some(old) = duplicate {
                // An immutable report is one fact, regardless of transport request key.
                // Return current context; do not replay its former workflow transition.
                let result = Receipt {
                    case: super::intervention::snapshot(tx, &case.id)?,
                    record: old,
                    transition_error: Some(Reject::StaleRevision),
                    facts_recorded: true,
                };
                remember(tx, &scope, fingerprint, RECEIPT, &result)?;
                return Ok(Ok(result));
            }
            let record = {
                let external = command.record.action.changes_external_state();
                let record = StoredRecord {
                    record: command.record.clone(),
                    recorded_at: now.clone(),
                    assertions: assertions.clone(),
                    external_change: external,
                };
                tx.put(&report_key, None, &doc(REPORT, &record)?)?;
                let artifact = key("procedureassertions", command.record.assertions.sha256);
                let value = doc("rx.procedure-assertions.v1", &assertions)?;
                if let Some(old) = tx.get(&artifact)? {
                    if old.document != value {
                        return Err(StoreError::Integrity("procedure assertion conflict".into()));
                    }
                } else {
                    tx.put(&artifact, None, &value)?;
                }
                case.record_ids.push(record.record.id.clone());
                if external {
                    // A later physical report invalidates earlier personnel/handover conclusions.
                    progress.personnel_record = None;
                    progress.handover_record = None;
                    for person in progress.people.values_mut() {
                        person.accounted = false;
                        person.handover = false;
                    }
                    progress.last_external_change = Some(now.clone());
                    latch(tx, &mut case, BlockReason::ProcedureReported)?;
                    if was_closed {
                        case.state = CaseState::ContainmentPending;
                        progress.entry_record = None;
                    }
                    if case.kind == CaseType::DiagnosticOnly {
                        case.kind = CaseType::FaultRecovery;
                        case.state = CaseState::ContainmentPending;
                    }
                    if command.record.action == Action::WorkStarted {
                        for person in &assertions.people {
                            progress.people.insert(
                                person.clone(),
                                Participant {
                                    active: true,
                                    finished: false,
                                    accounted: false,
                                    handover: false,
                                },
                            );
                            if !case.participants.contains(person) {
                                case.participants.push(person.clone());
                            }
                        }
                    }
                    if command.record.action == Action::WorkFinished
                        && assertions.certainty == Some(Certainty::Reported)
                        && let Some(person) = progress.people.get_mut(&actor.id)
                    {
                        person.active = false;
                        person.finished = true;
                    }
                    if matches!(
                        command.record.action,
                        Action::IsolationStateReported
                            | Action::ResetObserved
                            | Action::ConfigurationReported
                    ) {
                        progress.entry_record = None;
                        case.state = CaseState::ContainmentPending;
                    }
                    if command.record.action == Action::ConfigurationReported {
                        progress.configuration_changed = true;
                        for id in &case.effective_cells {
                            let (rev, mut cell): (_, Cell) = load(tx, "cell", id, CELL)?;
                            if let Some(q) = cell.qualification.take() {
                                save(
                                    tx,
                                    "qualificationhistory",
                                    &q.id,
                                    None,
                                    "rx.internal.qualification-history.v1",
                                    &q,
                                )?;
                            }
                            save(tx, "cell", id, Some(rev), CELL, &cell)?;
                        }
                    }
                }
                event(tx, "rx.event.procedure-fact-recorded.v1", &record)?;
                record
            };
            let mut error = if was_closed {
                Some(Reject::BlockedByCase)
            } else if stale {
                Some(Reject::StaleRevision)
            } else {
                None
            };
            if error.is_none() {
                match advance(
                    tx,
                    &mut case,
                    &mut progress,
                    &record,
                    &now,
                    authority,
                    &before_context,
                ) {
                    Ok(()) => {}
                    Err(StoreError::Rejected(reason)) => error = Some(reason),
                    Err(e) => return Err(e),
                }
            }
            if stale && record.external_change {
                case.state = CaseState::ContainmentPending;
                progress.entry_record = None;
            }
            if error.is_some()
                && record.external_change
                && command.record.action == Action::WorkStarted
                && case.state != CaseState::ProcedureActive
            {
                case.state = CaseState::Escalated;
            }
            let old = tx.get(&key("procedureprogress", &case.id))?;
            save(
                tx,
                "procedureprogress",
                &case.id,
                old.map(|r| r.revision),
                PROGRESS,
                &progress,
            )?;
            // Trusted facts survive a stale workflow CAS. Only the desired promotion is refused.
            let row = tx
                .get(&key("case", &case.id))?
                .ok_or(StoreError::Integrity("case missing".into()))?;
            save(tx, "case", &case.id, Some(row.revision), CASE, &case)?;
            let result = Receipt {
                case: super::intervention::snapshot(tx, &case.id)?,
                record,
                transition_error: error,
                facts_recorded: true,
            };
            remember(tx, &scope, fingerprint, RECEIPT, &result)?;
            Ok(Ok(result))
        })?
    }
}
fn validate_report(
    command: &Submission,
    a: &Assertions,
    case: &Case,
    actor: &Principal,
) -> Result<()> {
    let r = &command.record;
    utc(&r.occurred_at)?;
    if command.expected_case.0 == 0
        || command.expected_cell == Some(Counter(0))
        || r.case_revision.0 == 0
        || a.schema.as_str() != "rx.procedure-assertions.v1"
        || a.case_id != case.id
        || a.case_revision != r.case_revision
        || a.procedure_digest != case.procedure.sha256
        || a.actor != actor.id
        || r.actor != actor.id
        || a.action != r.action
        || a.occurred_at != r.occurred_at
        || a.scope_ids != r.scope_ids
        || a.evidence_ids != r.evidence_ids
        || r.scope_ids.is_empty()
        || !unique(&r.scope_ids)
        || !unique(&a.people)
        || !unique(&r.evidence_ids)
        || a.physical_claims.len() > 128
        || a.people.len() > 64
        || r.evidence_ids.len() > 128
    {
        return reject(Reject::InvalidInput);
    }
    if r.action != Action::Acknowledge
        && (a.source != actor.id || a.source_event.is_none() || a.step.is_none())
    {
        return reject(Reject::InvalidInput);
    }
    if r.action.changes_external_state() && a.physical_claims.is_empty() {
        return reject(Reject::InvalidInput);
    }
    if matches!(r.action, Action::WorkStarted | Action::WorkFinished)
        && a.people != [actor.id.clone()]
    {
        return reject(Reject::InvalidInput);
    }
    Ok(())
}
fn load_progress(tx: &mut dyn Transaction, case: &Id) -> Result<Progress> {
    tx.get(&key("procedureprogress", case))?
        .map(|r| decode(&r, PROGRESS))
        .transpose()
        .map(|v| v.unwrap_or_default())
}
fn latch(tx: &mut dyn Transaction, case: &mut Case, reason: BlockReason) -> Result<()> {
    let initial = affected_cells(tx, &case.cell)?;
    let mut previous = BTreeSet::new();
    for id in &initial {
        let (_, cell): (_, Cell) = load(tx, "cell", id, CELL)?;
        previous.extend(cell.blocks.iter().map(|b| b.id.clone()));
    }
    let affected = invalidate_closure(tx, &case.cell, reason)?;
    case.effective_cells = affected.into_iter().collect();
    for id in &case.effective_cells {
        let (rev, mut cell): (_, Cell) = load(tx, "cell", id, CELL)?;
        cell.mode = Some(if case.kind == CaseType::Maintenance {
            OperatingMode::Maintenance
        } else {
            OperatingMode::Recovery
        });
        for block in &mut cell.blocks {
            if !previous.contains(&block.id) {
                block.case_id = Some(case.id.clone());
            }
        }
        case.block_ids.extend(
            cell.blocks
                .iter()
                .filter(|b| !previous.contains(&b.id))
                .map(|b| b.id.clone()),
        );
        if !cell.open_cases.contains(&case.id) {
            cell.open_cases.push(case.id.clone());
            save(tx, "cell", id, Some(rev), CELL, &cell)?;
        }
    }
    case.block_ids.sort();
    case.block_ids.dedup();
    Ok(())
}
fn advance<A: QualificationAuthority>(
    tx: &mut dyn Transaction,
    case: &mut Case,
    progress: &mut Progress,
    report: &StoredRecord,
    now: &TimePoint,
    authority: &A,
    before_context: &[crate::intervention::CaseRef],
) -> Result<()> {
    let action = report.record.action;
    if action == Action::Acknowledge {
        return Ok(());
    }
    let Some(row) = tx.get(&key("procedurepolicy", case.procedure.sha256))? else {
        case.state = CaseState::Escalated;
        progress.entry_record = None;
        return reject(Reject::UnsupportedSchema);
    };
    let admitted: Admission = decode(&row, POLICY)?;
    let policy = &admitted.policy;
    let (_, cell): (_, Cell) = load(tx, "cell", &case.cell, CELL)?;
    if admitted.reference != case.procedure
        || policy.definition != cell.configuration.definition.sha256
        || policy.envelope != cell.configuration.envelope.sha256
        || !policy.case_types.contains(&case.kind)
    {
        return reject(Reject::QualificationRequired);
    }
    let mut required_scopes = BTreeSet::new();
    for id in &case.effective_cells {
        let (_, cell): (_, Cell) = load(tx, "cell", id, CELL)?;
        required_scopes.extend(cell.scope_epochs.keys().cloned());
    }
    if report
        .record
        .scope_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        != required_scopes
    {
        return reject(Reject::ConditionUnknown);
    }
    if !authority.verify_procedure(&cell.configuration, &admitted.reference, policy) {
        case.state = CaseState::Escalated;
        progress.entry_record = None;
        return reject(Reject::QualificationRequired);
    }
    if matches!(action, Action::WorkStarted | Action::WorkFinished)
        && !same(&progress.entry_context, &before_context.to_vec())?
    {
        progress.entry_record = None;
        case.state = CaseState::ContainmentPending;
        return reject(Reject::ConditionUnknown);
    }
    let a = &report.assertions;
    let step = policy
        .steps
        .iter()
        .find(|s| Some(&s.id) == a.step.as_ref() && s.action == action)
        .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
    if !step.actors.contains(&report.record.actor) {
        return reject(Reject::Forbidden);
    }
    if matches!(
        action,
        Action::EntryConditionsReported | Action::PersonnelAccounted | Action::HandoverAccepted
    ) {
        let (_, actor): (_, Principal) = load(tx, "principal", &report.record.actor, PRINCIPAL)?;
        if !actor.active || !actor.roles.contains(&Role::RecoveryLead) || actor.id != case.lead {
            return reject(Reject::Forbidden);
        }
    }
    if a.certainty != Some(Certainty::Reported) {
        return reject(Reject::ConditionUnknown);
    }
    let (observed, until) = a
        .observed_at
        .as_ref()
        .zip(a.valid_until.as_ref())
        .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
    if now
        .age_ns(observed)
        .is_none_or(|age| age > policy.maximum_report_age_ns.0)
        || until.clock_id != now.clock_id
        || now.ticks_ns >= until.ticks_ns
    {
        return reject(Reject::ConditionUnknown);
    }
    for fact in &step.required_evidence {
        let (_, value): (_, FactRecord) = load(tx, "fact", (&case.cell, fact), FACT)?;
        let evaluation = evaluate_raw(
            tx,
            &cell,
            &[rx_domain::condition::Condition::Eq {
                fact: fact.clone(),
                schema: value.schema.clone(),
                unit: value.unit.clone(),
                expected: value.value.clone(),
            }],
            now,
        )?;
        if evaluation.verdict != Verdict::Pass
            || !report.record.evidence_ids.contains(&value.evidence_id)
        {
            return reject(Reject::ConditionUnknown);
        }
    }
    if !step.conditions.is_empty() {
        let proof = evaluate(tx, &cell, &step.conditions, now)?;
        if proof
            .evidence_ids
            .iter()
            .any(|id| !report.record.evidence_ids.contains(id))
        {
            return reject(Reject::ConditionUnknown);
        }
    }
    if matches!(
        action,
        Action::EntryConditionsReported | Action::PersonnelAccounted | Action::HandoverAccepted
    ) {
        let since = progress
            .last_external_change
            .as_ref()
            .unwrap_or(&case.opened_at);
        for id in &report.record.evidence_ids {
            let (_, evidence): (_, ObservationEvidence) = load(
                tx,
                "factevidence",
                id,
                "rx.internal.observation-evidence.v1",
            )?;
            if evidence.acquired_at.clock_id != since.clock_id
                || evidence.acquired_at.ticks_ns < since.ticks_ns
            {
                return reject(Reject::ConditionUnknown);
            }
        }
    }
    match action {
        Action::EntryConditionsReported => {
            if progress.configuration_changed {
                return reject(Reject::QualificationRequired);
            }
            if report.record.actor != case.lead
                || case.kind == CaseType::DiagnosticOnly
                || !matches!(
                    case.state,
                    CaseState::ContainmentPending | CaseState::Revalidating
                )
            {
                return reject(Reject::Forbidden);
            }
            for id in &case.effective_cells {
                require_fences(tx, id)?;
            }
            let proof = evaluate(tx, &cell, &policy.entry_conditions, now)?;
            if proof.evidence_ids.is_empty()
                || proof
                    .evidence_ids
                    .iter()
                    .any(|id| !report.record.evidence_ids.contains(id))
            {
                return reject(Reject::ConditionUnknown);
            }
            progress.entry_record = Some(report.record.id.clone());
            progress.entry_context = current_context(tx, case)?;
            progress.policy = Some(admitted.reference);
            case.state = CaseState::ProcedureActive;
        }
        Action::WorkStarted => {
            if case.state != CaseState::ProcedureActive || progress.entry_record.is_none() {
                return reject(Reject::ConditionUnknown);
            }
        }
        Action::WorkFinished => {
            if case.state != CaseState::ProcedureActive {
                return reject(Reject::ConditionUnknown);
            }
            let person = progress
                .people
                .get_mut(&report.record.actor)
                .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
            if !person.active && !person.finished {
                return reject(Reject::ConditionUnknown);
            }
            person.active = false;
            person.finished = true;
        }
        Action::PersonnelAccounted | Action::HandoverAccepted => {
            if report.record.actor != case.lead
                || progress.people.is_empty()
                || a.people.iter().collect::<BTreeSet<_>>()
                    != progress.people.keys().collect::<BTreeSet<_>>()
                || progress.people.values().any(|p| p.active || !p.finished)
            {
                return reject(Reject::ConditionUnknown);
            }
            if action == Action::PersonnelAccounted {
                progress.personnel_record = Some(report.record.id.clone());
            } else {
                progress.handover_record = Some(report.record.id.clone());
            }
            for person in progress.people.values_mut() {
                if action == Action::PersonnelAccounted {
                    person.accounted = true;
                } else {
                    person.handover = true;
                }
            }
        }
        Action::IsolationStateReported | Action::ResetObserved | Action::ConfigurationReported => {
            if case.state != CaseState::ProcedureActive {
                case.state = CaseState::ContainmentPending;
            }
        }
        Action::Acknowledge => {}
    }
    if matches!(action, Action::WorkStarted | Action::WorkFinished)
        && case.state == CaseState::ProcedureActive
    {
        progress.entry_context = current_context(tx, case)?;
    }
    if !progress.people.is_empty()
        && progress
            .people
            .values()
            .all(|p| p.finished && !p.active && p.accounted && p.handover)
    {
        case.state = CaseState::Revalidating;
    }
    Ok(())
}
pub(super) fn require_fences(tx: &mut dyn Transaction, cell_id: &Name) -> Result<()> {
    let (_, cell): (_, Cell) = load(tx, "cell", cell_id, CELL)?;
    let acknowledgments = tx
        .scan("fenceack/")?
        .into_iter()
        .map(|r| decode::<FenceAcknowledgment>(&r, "rx.internal.fence-ack.v1"))
        .collect::<Result<Vec<_>>>()?;
    for host in &cell.configuration.hosts {
        let (_, registered): (_, HostRegistration) = load(tx, "host", (cell_id, host), HOST)?;
        if !acknowledgments.iter().any(|a| {
            a.cell == *cell_id
                && a.host_boot == registered.boot_id
                && a.journal == registered.delivery_journal
                && a.epoch == cell.epoch
                && a.scopes == cell.scope_epochs
        }) {
            return reject(Reject::ConditionUnknown);
        }
    }
    Ok(())
}

fn current_context(
    tx: &mut dyn Transaction,
    case: &Case,
) -> Result<Vec<crate::intervention::CaseRef>> {
    case.effective_cells
        .iter()
        .map(|id| {
            let (_, cell): (_, Cell) = load(tx, "cell", id, CELL)?;
            Ok(super::intervention::reference(&cell))
        })
        .collect()
}
