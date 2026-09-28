//! Durable, P-issued metadata comparison intents. Existing deployment barriers remain in force.
use super::*;
use crate::{
    host_binding_transition as binding,
    process_change::{State, Transition},
};
const RECORD: &str = "rx.internal.host-binding-intent.v1";
const REF: &str = "rx.internal.host-binding-intent-ref.v1";
const BATCH: &str = "rx.host-binding-intent-batch.v1";
fn read(tx: &mut dyn Transaction, id: &Id) -> Result<(Counter, binding::Record)> {
    let (revision, value): (_, binding::Record) = load(tx, "hostbindingintent", id, RECORD)?;
    if value.intent.request != *id || value.activation_authorized {
        return Err(StoreError::Integrity(
            "Host binding intent identity differs".into(),
        ));
    }
    Ok((revision, value))
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
            if record.issue == Some(issue) && record.phase != binding::Phase::MetadataMatched {
                return Ok(record);
            }
            record.issue = Some(issue);
            record.phase = if record.baseline.is_some() {
                binding::Phase::BaselineRecorded
            } else {
                binding::Phase::AwaitingBaseline
            };
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
                    record.issue = Some(issue);
                    record.phase = if record.baseline.is_some() {
                        binding::Phase::BaselineRecorded
                    } else {
                        binding::Phase::AwaitingBaseline
                    };
                }
            }
            save(tx, "hostbindingintent", id, Some(revision), RECORD, &record)?;
            event(tx, "rx.event.host-binding-intent-observed.v1", &record)?;
            Ok(record)
        })
    }
}
