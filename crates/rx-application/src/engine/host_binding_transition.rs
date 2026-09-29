//! Durable, P-issued metadata comparison intents. Existing deployment barriers remain in force.
use super::*;
use crate::{
    host_binding_transition as binding,
    process_change::{State, Transition},
};
const RECORD: &str = "rx.internal.host-binding-intent.v1";
const REF: &str = "rx.internal.host-binding-intent-ref.v1";
const BATCH: &str = "rx.host-binding-intent-batch.v1";
pub(super) fn read(tx: &mut dyn Transaction, id: &Id) -> Result<(Counter, binding::Record)> {
    let (revision, value): (_, binding::Record) = load(tx, "hostbindingintent", id, RECORD)?;
    if value.intent.request != *id || value.activation_authorized {
        return Err(StoreError::Integrity(
            "Host binding intent identity differs".into(),
        ));
    }
    Ok((revision, value))
}
/// Binding standing of one plan Host, derived from its durable intent and the Host
/// generation currently registered for every intent cell. It never grants application,
/// configuration, qualification or execution authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Standing {
    /// No intent was issued in this P runtime, or no baseline has been captured.
    BaselineRequired,
    /// The baseline belongs to the Host generation that is registered now.
    BaselineCurrent,
    /// A baseline exists but the registered generation no longer matches it and no
    /// commit is confirmed for the current generation.
    CommitUnconfirmed,
    /// The recorded commit match was read from the Host generation registered now.
    CommitCurrent,
}
fn registered(tx: &mut dyn Transaction, record: &binding::Record) -> Result<Option<(Id, Id, Id)>> {
    // A registration is current only while its session is the Host's active producer
    // session; a later boot of the Host makes every earlier registration stale.
    let Some(row) = tx.get(&key("producer", &record.intent.host))? else {
        return Ok(None);
    };
    let producer: EvidenceProducer = decode(&row, "rx.internal.evidence-producer.v1")?;
    let mut current = None;
    for cell in record.intent.before_cells.keys() {
        let Some(row) = tx.get(&key("host", (cell, &record.intent.host)))? else {
            return Ok(None);
        };
        let h: HostRegistration = decode(&row, HOST)?;
        if h.session != producer.session || h.boot_id != producer.peer_boot {
            return Ok(None);
        }
        let tuple = (h.session, h.boot_id, h.delivery_journal);
        if current.as_ref().is_some_and(|old| old != &tuple) {
            return Ok(None);
        }
        current = Some(tuple);
    }
    Ok(current)
}
/// Pure standing decision; `current` is the (session, boot, delivery journal) registered
/// for every intent cell, or None when the cells disagree or a registration is missing.
fn standing(
    record: Option<&binding::Record>,
    runtime_boot: &Id,
    plan_digest: &Digest,
    current: Option<&(Id, Id, Id)>,
) -> Standing {
    let Some(r) = record.filter(|r| {
        &r.intent.runtime_boot == runtime_boot
            && &r.intent.host_plan_digest == plan_digest
            && r.baseline.is_some()
    }) else {
        return Standing::BaselineRequired;
    };
    let Some((session, boot, journal)) = current else {
        return Standing::CommitUnconfirmed;
    };
    let committed = (r.phase == binding::Phase::MetadataMatched)
        .then_some(())
        .and(r.observation.as_ref())
        .map(|o| &o.snapshot);
    if let Some(s) = committed {
        // A later read that contradicts the match (anything but a lost transport) withdraws it.
        let contradicted = r
            .issue
            .is_some_and(|i| i != binding::Rejection::TransportUnavailable);
        return if !contradicted
            && &s.host_boot == boot
            && &s.delivery_journal == journal
            && r.observed_session.as_ref() == Some(session)
        {
            Standing::CommitCurrent
        } else {
            Standing::CommitUnconfirmed
        };
    }
    match r.baseline.as_ref().map(|b| &b.snapshot) {
        Some(b) if &b.host_boot == boot && &b.delivery_journal == journal => {
            Standing::BaselineCurrent
        }
        _ => Standing::CommitUnconfirmed,
    }
}
pub(super) fn standings(
    tx: &mut dyn Transaction,
    meta: &Installation,
    change: &crate::process_change::Change,
) -> Result<BTreeMap<Name, Standing>> {
    let mut result = BTreeMap::new();
    let Some(plan) = &change.host_binding_plan else {
        return Ok(result);
    };
    let plan_digest = plan.digest().map_err(StoreError::Invalid)?;
    for host in plan.hosts.keys() {
        let slot = key("hostbindingintentslot", (&change.id, host));
        let record = match tx.get(&slot)? {
            Some(row) => {
                let id: Id = decode(&row, REF)?;
                Some(read(tx, &id)?.1)
            }
            None => None,
        };
        let current = match &record {
            Some(r) => registered(tx, r)?,
            None => None,
        };
        let standing = standing(
            record.as_ref(),
            &meta.runtime_boot,
            &plan_digest,
            current.as_ref(),
        );
        result.insert(host.clone(), standing);
    }
    Ok(result)
}
fn unchanged_except_read_time(old: &binding::Record, new: &binding::Record) -> Result<bool> {
    let value = |r: &binding::Record| {
        let mut v = r.clone();
        v.read_started = None;
        serde_json::to_value(v).map_err(|e| StoreError::Integrity(e.to_string()))
    };
    Ok(value(old)? == value(new)?)
}
fn identity(c: &CellConfiguration) -> binding::CellIdentity {
    binding::CellIdentity {
        definition: c.definition.sha256,
        envelope: c.envelope.sha256,
        environment: name(match c.environment {
            Environment::Simulation => "SIMULATION",
            Environment::Physical => "PHYSICAL",
        }),
    }
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn host_binding_intents(
        &mut self,
        identity_: &Identity,
        after: Option<&Id>,
    ) -> Result<Vec<binding::Record>> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let actor = authorize(tx, identity_, meta, &clock.now(), None, Role::Host, false)?;
            let mut result = vec![];
            for row in tx.scan("hostbindingintent/")? {
                let record: binding::Record = decode(&row, RECORD)?;
                if record.intent.host != actor.id
                    || after.is_some_and(|id| record.intent.request <= *id)
                    || record
                        .intent
                        .before_cells
                        .keys()
                        .any(|c| !actor.cells.contains(c))
                {
                    continue;
                }
                let change =
                    process_change::change(tx, &record.intent.change, &record.intent.cell)?;
                if change.state != State::Staged
                    || change
                        .host_binding_plan
                        .as_ref()
                        .and_then(|p| p.digest().ok())
                        != Some(record.intent.host_plan_digest)
                {
                    continue;
                }
                result.push(record);
            }
            result.sort_by(|a, b| a.intent.request.cmp(&b.intent.request));
            result.truncate(16);
            Ok(result)
        })
    }
    pub fn host_binding_read_issue(
        &mut self,
        identity_: &Identity,
        id: &Id,
        issue: binding::Rejection,
    ) -> Result<binding::Record> {
        if !matches!(
            issue,
            binding::Rejection::TransportUnavailable | binding::Rejection::AuthorizationChanged
        ) {
            return reject(Reject::InvalidInput);
        }
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let actor = authorize(tx, identity_, meta, &clock.now(), None, Role::Host, false)?;
            let (revision, mut record) = read(tx, id)?;
            if record.intent.host != actor.id
                || record
                    .intent
                    .before_cells
                    .keys()
                    .any(|c| !actor.cells.contains(c))
            {
                return reject(Reject::Forbidden);
            }
            for cell in record.intent.before_cells.keys() {
                let (_, host): (_, HostRegistration) = load(tx, "host", (cell, &actor.id), HOST)?;
                if host.session != identity_.session {
                    return reject(Reject::ContinuityUnproven);
                }
            }
            if record.issue == Some(issue) {
                return Ok(record);
            }
            // A failed read proves nothing about the Host: keep the recorded phase and
            // observation (standing decides currency) and only record why the read failed.
            record.issue = Some(issue);
            save(tx, "hostbindingintent", id, Some(revision), RECORD, &record)?;
            event(tx, "rx.event.host-binding-read-unavailable.v1", &record)?;
            Ok(record)
        })
    }
    pub fn issue_host_binding_intents(
        &mut self,
        identity_: &Identity,
        key_: &Id,
        input: Transition,
    ) -> Result<Vec<binding::Record>> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let actor = authorize(
                tx,
                identity_,
                meta,
                &now,
                Some(&input.cell),
                Role::ReleaseManager,
                true,
            )?;
            let change = process_change::change(tx, &input.change, &input.cell)?;
            process_change::access(&actor, &change.impact)?;
            let (scope, fingerprint) = request(
                meta,
                &actor,
                "ProcessChange.IssueHostBindingIntents",
                key_.as_str(),
                &input,
            )?;
            if let Some(ids) = prior::<Vec<Id>>(tx, &scope, fingerprint, BATCH)? {
                return ids.iter().map(|id| read(tx, id).map(|(_, r)| r)).collect();
            }
            if change.state != State::Staged
                || change.revision != input.expected
                || change.plan_digest != input.plan_digest
            {
                return reject(Reject::StaleRevision);
            }
            process_change::current(tx, meta, &change)?;
            let plan = change
                .host_binding_plan
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            plan.validate().map_err(StoreError::Invalid)?;
            if plan.environment.as_str() != "SIMULATION" {
                return reject(Reject::CapabilityMissing);
            }
            let plan_digest = plan.digest().map_err(StoreError::Invalid)?;
            let after = process_change::read_config(tx, &change.after)?;
            let mut result = vec![];
            for host in plan.hosts.keys() {
                let slot = key("hostbindingintentslot", (&change.id, host));
                if let Some(row) = tx.get(&slot)? {
                    let id: Id = decode(&row, REF)?;
                    let (_, old) = read(tx, &id)?;
                    if old.intent.host_plan_digest != plan_digest
                        || old.intent.runtime_boot != meta.runtime_boot
                    {
                        return reject(Reject::StaleRevision);
                    }
                    result.push(old);
                    continue;
                }
                let mut before_cells = BTreeMap::new();
                let mut after_cells = BTreeMap::new();
                for cell in change
                    .impact
                    .cells
                    .iter()
                    .filter(|c| c.hosts.contains(host))
                {
                    let (_, current): (_, Cell) = load(tx, "cell", &cell.id, CELL)?;
                    before_cells.insert(cell.id.clone(), identity(&current.configuration));
                    after_cells.insert(
                        cell.id.clone(),
                        identity(if cell.id == change.cell {
                            &after
                        } else {
                            &current.configuration
                        }),
                    );
                }
                if before_cells.is_empty()
                    || before_cells.len() > 64
                    || !before_cells.contains_key(&change.cell)
                    || before_cells
                        .values()
                        .chain(after_cells.values())
                        .any(|c| c.environment.as_str() != "SIMULATION")
                {
                    return reject(Reject::CapabilityMissing);
                }
                let record = binding::Record {
                    intent: binding::Intent {
                        request: id(),
                        change: change.id.clone(),
                        host: host.clone(),
                        cell: change.cell.clone(),
                        host_plan_digest: plan_digest,
                        before_configuration: change.before.sha256,
                        after_configuration: change.after.sha256,
                        before_cells,
                        after_cells,
                        runtime_boot: meta.runtime_boot.clone(),
                        created_at: now.clone(),
                    },
                    phase: binding::Phase::AwaitingBaseline,
                    baseline: None,
                    observation: None,
                    read_started: None,
                    observed_session: None,
                    issue: None,
                    created_by: actor.id.clone(),
                    activation_authorized: false,
                    adopted_from: vec![],
                };
                save(
                    tx,
                    "hostbindingintent",
                    &record.intent.request,
                    None,
                    RECORD,
                    &record,
                )?;
                tx.put(&slot, None, &doc(REF, &record.intent.request)?)?;
                event(tx, "rx.event.host-binding-intent-issued.v1", &record)?;
                result.push(record);
            }
            remember(
                tx,
                &scope,
                fingerprint,
                BATCH,
                &result
                    .iter()
                    .map(|r| r.intent.request.clone())
                    .collect::<Vec<_>>(),
            )?;
            Ok(result)
        })
    }
    /// Explicitly carry the original P-issued requests of a staged change into this P
    /// runtime. The request IDs, baselines and any confirmed commit are kept; the confirmation
    /// session is cleared, so the Host must be read again in this runtime before it counts.
    pub fn adopt_host_binding_intents(
        &mut self,
        identity_: &Identity,
        key_: &Id,
        input: Transition,
    ) -> Result<Vec<binding::Record>> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let actor = authorize(
                tx,
                identity_,
                meta,
                &now,
                Some(&input.cell),
                Role::ReleaseManager,
                true,
            )?;
            let change = process_change::change(tx, &input.change, &input.cell)?;
            process_change::access(&actor, &change.impact)?;
            let (scope, fingerprint) = request(
                meta,
                &actor,
                "ProcessChange.AdoptHostBindingIntents",
                key_.as_str(),
                &input,
            )?;
            if let Some(ids) = prior::<Vec<Id>>(tx, &scope, fingerprint, BATCH)? {
                return ids.iter().map(|id| read(tx, id).map(|(_, r)| r)).collect();
            }
            if change.state != State::Staged
                || change.revision != input.expected
                || change.plan_digest != input.plan_digest
            {
                return reject(Reject::StaleRevision);
            }
            process_change::current(tx, meta, &change)?;
            let plan = change
                .host_binding_plan
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            let plan_digest = plan.digest().map_err(StoreError::Invalid)?;
            let mut result = vec![];
            for host in plan.hosts.keys() {
                let row = tx
                    .get(&key("hostbindingintentslot", (&change.id, host)))?
                    .ok_or(StoreError::Rejected(Reject::NotFound))?;
                let id: Id = decode(&row, REF)?;
                let (revision, mut record) = read(tx, &id)?;
                if record.intent.host_plan_digest != plan_digest {
                    return reject(Reject::StaleRevision);
                }
                if record.intent.runtime_boot != meta.runtime_boot {
                    // Time comparisons against the original request need the same clock.
                    if record.intent.created_at.clock_id != now.clock_id
                        || now.age_ns(&record.intent.created_at).is_none()
                    {
                        return reject(Reject::CapabilityMissing);
                    }
                    save(
                        tx,
                        "hostbindingintenthistory",
                        (&id, &record.intent.runtime_boot),
                        None,
                        RECORD,
                        &record,
                    )?;
                    let previous = std::mem::replace(
                        &mut record.intent.runtime_boot,
                        meta.runtime_boot.clone(),
                    );
                    record.adopted_from.push(previous);
                    record.observed_session = None;
                    record.issue = None;
                    save(
                        tx,
                        "hostbindingintent",
                        &id,
                        Some(revision),
                        RECORD,
                        &record,
                    )?;
                    event(tx, "rx.event.host-binding-intent-adopted.v1", &record)?;
                }
                result.push(record);
            }
            remember(
                tx,
                &scope,
                fingerprint,
                BATCH,
                &result
                    .iter()
                    .map(|r| r.intent.request.clone())
                    .collect::<Vec<_>>(),
            )?;
            Ok(result)
        })
    }
    /// Internal registered-Host transport result, never a caller-uploaded HTTP confirmation.
    pub fn observe_host_binding_intent(
        &mut self,
        identity_: &Identity,
        id: &Id,
        observation: rx_domain::host_configuration::Observation,
        read_started: TimePoint,
    ) -> Result<binding::Record> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let actor = authorize(tx, identity_, meta, &now, None, Role::Host, false)?;
            let (revision, mut record) = read(tx, id)?;
            let before = record.clone();
            if actor.id != record.intent.host
                || record
                    .intent
                    .before_cells
                    .keys()
                    .any(|c| !actor.cells.contains(c))
            {
                return reject(Reject::Forbidden);
            }
            let change = process_change::change(tx, &record.intent.change, &record.intent.cell)?;
            if change.state != State::Staged
                || change
                    .host_binding_plan
                    .as_ref()
                    .and_then(|p| p.digest().ok())
                    != Some(record.intent.host_plan_digest)
            {
                return reject(Reject::StaleRevision);
            }
            process_change::current(tx, meta, &change)?;
            let mut current = None;
            for cell in record.intent.before_cells.keys() {
                let (_, host): (_, HostRegistration) = load(tx, "host", (cell, &actor.id), HOST)?;
                let tuple = (host.session, host.boot_id, host.delivery_journal);
                if tuple.0 != identity_.session || current.as_ref().is_some_and(|old| old != &tuple)
                {
                    return reject(Reject::ContinuityUnproven);
                }
                current = Some(tuple);
            }
            let (session, boot, journal) =
                current.ok_or(StoreError::Rejected(Reject::CapabilityMissing))?;
            let context = binding::ReadContext {
                runtime_boot: meta.runtime_boot.clone(),
                producer_session: session,
                host_boot: boot,
                delivery_journal: journal,
                started_at: read_started.clone(),
                now,
            };
            let result = if let Some(baseline) = &record.baseline {
                binding::confirm(&record.intent, baseline, &observation, &context)
            } else {
                binding::capture_baseline(&record.intent, &observation, &context).map(|baseline| {
                    record.baseline = Some(baseline);
                })
            };
            match result {
                Ok(()) => {
                    record.phase = if observation
                        .snapshot
                        .binding_commit
                        .as_ref()
                        .is_some_and(|c| c.request == record.intent.request)
                    {
                        binding::Phase::MetadataMatched
                    } else {
                        binding::Phase::BaselineRecorded
                    };
                    record.observation = Some(observation);
                    record.read_started = Some(read_started);
                    record.observed_session = Some(context.producer_session.clone());
                    record.issue = None;
                }
                Err(issue) => {
                    // A read that does not confirm keeps an earlier confirmed commit on record;
                    // standing compares that observation with the current registration.
                    record.issue = Some(issue);
                    if record.phase != binding::Phase::MetadataMatched {
                        record.phase = if record.baseline.is_some() {
                            binding::Phase::BaselineRecorded
                        } else {
                            binding::Phase::AwaitingBaseline
                        };
                    }
                }
            }
            // The worker re-reads every step. A read that changes nothing but its start
            // time is not new evidence and must not grow the record history or event log.
            if unchanged_except_read_time(&before, &record)? {
                return Ok(before);
            }
            save(tx, "hostbindingintent", id, Some(revision), RECORD, &record)?;
            event(tx, "rx.event.host-binding-intent-observed.v1", &record)?;
            Ok(record)
        })
    }
}

#[cfg(test)]
mod standing_tests {
    use super::{Standing, standing, unchanged_except_read_time};
    use crate::host_binding_transition as binding;
    use rx_domain::{host_configuration as data, types::*};
    use std::collections::BTreeMap;
    fn n(s: &str) -> Name {
        Name::new(s).unwrap()
    }
    fn id(v: u8) -> Id {
        Id::new(format!("00000000-0000-4000-8000-{v:012}")).unwrap()
    }
    fn at(t: u64) -> TimePoint {
        TimePoint {
            clock_id: "test/clock".into(),
            ticks_ns: Counter(t),
        }
    }
    const RUNTIME: u8 = 21;
    const PLAN: [u8; 32] = [5; 32];
    fn snapshot(boot: u8) -> data::Snapshot {
        data::Snapshot {
            schema: n("rx.host-process-configuration-snapshot.v1"),
            host: n("host/test"),
            host_boot: id(boot),
            delivery_journal: id(2),
            evidence_journal: Some(id(3)),
            installation_identity: Some(Digest::from_bytes([1; 32])),
            binding_commit: None,
            binding_digest: Digest::from_bytes([2; 32]),
            cells: vec![],
        }
    }
    fn observation(boot: u8) -> data::Observation {
        data::Observation {
            schema: n("rx.host-process-configuration-observation.v1"),
            snapshot: snapshot(boot),
            receipt: None,
            context_matches_current_host: false,
            activation_authorized: false,
        }
    }
    /// Baseline from Host boot 1 (session 31), P runtime RUNTIME, plan PLAN.
    fn baselined() -> binding::Record {
        let cells = BTreeMap::from([(
            n("cell/a"),
            binding::CellIdentity {
                definition: Digest::from_bytes([3; 32]),
                envelope: Digest::from_bytes([4; 32]),
                environment: n("SIMULATION"),
            },
        )]);
        binding::Record {
            intent: binding::Intent {
                request: id(9),
                change: id(20),
                host: n("host/test"),
                cell: n("cell/a"),
                host_plan_digest: Digest::from_bytes(PLAN),
                before_configuration: Digest::from_bytes([6; 32]),
                after_configuration: Digest::from_bytes([7; 32]),
                before_cells: cells.clone(),
                after_cells: cells,
                runtime_boot: id(RUNTIME),
                created_at: at(1),
            },
            phase: binding::Phase::BaselineRecorded,
            baseline: Some(binding::Baseline {
                snapshot: snapshot(1),
                observed_at: at(2),
                producer_session: id(31),
            }),
            observation: Some(observation(1)),
            read_started: Some(at(2)),
            observed_session: Some(id(31)),
            issue: None,
            created_by: n("release"),
            activation_authorized: false,
            adopted_from: vec![],
        }
    }
    /// Commit confirmed from Host boot 4 (session 34).
    fn matched() -> binding::Record {
        let mut r = baselined();
        r.phase = binding::Phase::MetadataMatched;
        r.observation = Some(observation(4));
        r.observed_session = Some(id(34));
        r
    }
    fn of(r: Option<&binding::Record>, current: Option<(u8, u8, u8)>) -> Standing {
        let current = current.map(|(s, b, j)| (id(s), id(b), id(j)));
        standing(r, &id(RUNTIME), &Digest::from_bytes(PLAN), current.as_ref())
    }
    #[test]
    fn missing_intent_other_runtime_other_plan_or_no_baseline_require_a_baseline() {
        assert_eq!(Standing::BaselineRequired, of(None, Some((31, 1, 2))));
        let mut other_runtime = baselined();
        other_runtime.intent.runtime_boot = id(22);
        assert_eq!(
            Standing::BaselineRequired,
            of(Some(&other_runtime), Some((31, 1, 2)))
        );
        let mut other_plan = baselined();
        other_plan.intent.host_plan_digest = Digest::from_bytes([8; 32]);
        assert_eq!(
            Standing::BaselineRequired,
            of(Some(&other_plan), Some((31, 1, 2)))
        );
        let mut awaiting = baselined();
        awaiting.baseline = None;
        awaiting.phase = binding::Phase::AwaitingBaseline;
        assert_eq!(
            Standing::BaselineRequired,
            of(Some(&awaiting), Some((31, 1, 2)))
        );
    }
    #[test]
    fn baseline_is_current_only_for_the_same_host_boot_and_journal() {
        let r = baselined();
        assert_eq!(Standing::BaselineCurrent, of(Some(&r), Some((31, 1, 2))));
        // A reconnect within the same boot keeps the baseline current.
        assert_eq!(Standing::BaselineCurrent, of(Some(&r), Some((32, 1, 2))));
        // Restart without a confirmed commit, a changed journal or no registration.
        assert_eq!(Standing::CommitUnconfirmed, of(Some(&r), Some((33, 3, 2))));
        assert_eq!(Standing::CommitUnconfirmed, of(Some(&r), Some((31, 1, 5))));
        assert_eq!(Standing::CommitUnconfirmed, of(Some(&r), None));
    }
    #[test]
    fn a_commit_match_is_current_only_for_the_observed_session_boot_and_journal() {
        let r = matched();
        assert_eq!(Standing::CommitCurrent, of(Some(&r), Some((34, 4, 2))));
        // A later restart or reconnect demotes the old match until it is read again.
        assert_eq!(Standing::CommitUnconfirmed, of(Some(&r), Some((35, 5, 2))));
        assert_eq!(Standing::CommitUnconfirmed, of(Some(&r), Some((35, 4, 2))));
        // A match never falls back to the pre-commit baseline generation.
        assert_eq!(Standing::CommitUnconfirmed, of(Some(&r), Some((31, 1, 2))));
        assert_eq!(Standing::CommitUnconfirmed, of(Some(&r), None));
        // A lost transport keeps the match; a contradicting read withdraws it.
        let mut lost = matched();
        lost.issue = Some(binding::Rejection::TransportUnavailable);
        assert_eq!(Standing::CommitCurrent, of(Some(&lost), Some((34, 4, 2))));
        let mut contradicted = matched();
        contradicted.issue = Some(binding::Rejection::MissingCommit);
        assert_eq!(
            Standing::CommitUnconfirmed,
            of(Some(&contradicted), Some((34, 4, 2)))
        );
    }
    #[test]
    fn only_the_read_start_time_is_ignored_when_deciding_a_reread_is_new_evidence() {
        let old = matched();
        let mut reread = old.clone();
        reread.read_started = Some(at(99));
        assert!(unchanged_except_read_time(&old, &reread).unwrap());
        let mut issue = reread.clone();
        issue.issue = Some(binding::Rejection::TransportUnavailable);
        assert!(!unchanged_except_read_time(&old, &issue).unwrap());
        let mut session = reread.clone();
        session.observed_session = Some(id(40));
        assert!(!unchanged_except_read_time(&old, &session).unwrap());
        let mut snapshot = reread;
        snapshot.observation = Some(observation(6));
        assert!(!unchanged_except_read_time(&old, &snapshot).unwrap());
    }
}
