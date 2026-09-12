use super::*;
use crate::{
    configuration_dispatch::*,
    process_change::{Change, State, Transition},
};
use rx_domain::host_configuration as wire;
const TASK: &str = "rx.internal.host-configuration-task.v1";
const BATCH: &str = "rx.host-configuration-batch.v1";
pub(super) fn key_for(c: &Id, attempt: Counter, host: &Name) -> Name {
    key("hostconfigtaskslot", (c, attempt, host))
}
pub(super) fn read(tx: &mut dyn Transaction, id: &Id) -> Result<(Counter, Task)> {
    let (revision, t): (_, Task) = load(tx, "hostconfigtask", id, TASK)?;
    if &t.id != id
        || t.request.is_some() != t.request_digest.is_some()
        || t.request.as_ref().is_some_and(|r| {
            r.id != t.id
                || r.change != t.change
                || r.preparation != t.preparation
                || r.host != t.host
                || r.plan_digest != t.plan_digest
                || r.digest().ok() != t.request_digest
        })
    {
        return Err(StoreError::Integrity(
            "Host configuration task identity differs".into(),
        ));
    }
    if let Some(r) = &t.receipt {
        r.validate().map_err(StoreError::Integrity)?;
        if Some(r.request_digest) != t.request_digest {
            return Err(StoreError::Integrity(
                "Host configuration receipt differs".into(),
            ));
        }
    }
    Ok((revision, t))
}
fn save_task(tx: &mut dyn Transaction, t: &Task, expected: Option<Counter>) -> Result<()> {
    save(tx, "hostconfigtask", &t.id, expected, TASK, t)?;
    event(tx, "rx.event.host-configuration-task.v1", t)
}
fn host_access(
    tx: &mut dyn Transaction,
    identity: &Identity,
    meta: &Installation,
    now: &TimePoint,
    t: &Task,
) -> Result<()> {
    let p = authorize(tx, identity, meta, now, None, Role::Host, false)?;
    if p.id != t.host || t.cells.iter().any(|c| !p.cells.contains(&c.cell)) {
        return reject(Reject::Forbidden);
    }
    Ok(())
}
pub(super) fn generation_matches(tx: &mut dyn Transaction, t: &Task) -> Result<bool> {
    for c in &t.cells {
        let Some(row) = tx.get(&key("host", (&c.cell, &t.host)))? else {
            return Ok(false);
        };
        let h: HostRegistration = decode(&row, HOST)?;
        if h.boot_id != t.host_boot
            || h.delivery_journal != t.delivery_journal
            || h.session != t.producer_session
        {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(super) fn current_change(
    tx: &mut dyn Transaction,
    meta: &Installation,
    t: &Task,
) -> Result<Change> {
    let c = process_change::change(tx, &t.change, &t.origin)?;
    let p = c
        .preparation
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::StaleRevision))?;
    if c.state != State::Staged
        || c.plan_digest != t.plan_digest
        || p.attempt != t.preparation
        || p.runtime_boot != meta.runtime_boot
        || t.runtime_boot != meta.runtime_boot
    {
        return reject(Reject::StaleRevision);
    }
    process_change::current(tx, meta, &c)?;
    for b in &p.cells {
        let (_, cell): (_, Cell) = load(tx, "cell", &b.cell, CELL)?;
        if cell.epoch != b.epoch
            || cell.scope_epochs != b.scopes
            || b.change_blocks
                .iter()
                .any(|id| !cell.blocks.iter().any(|x| &x.id == id && x.latched))
        {
            return reject(Reject::StaleEpoch);
        }
    }
    Ok(c)
}
pub(super) fn barrier(tx: &mut dyn Transaction, c: &Change) -> Result<()> {
    if c.host_binding_plan.is_some() {
        return reject(Reject::CapabilityMissing);
    }
    let ids: BTreeSet<_> = c.impact.cells.iter().map(|c| c.id.clone()).collect();
    let p = c
        .preparation
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::StaleRevision))?;
    for row in tx.scan("run/")? {
        let r: Run = decode(&row, RUN)?;
        if ids.contains(&r.cell) && !matches!(r.state, RunState::Completed | RunState::Abandoned) {
            return reject(Reject::Busy);
        }
    }
    for row in tx.scan("work/")? {
        let w: Work = decode(&row, WORK)?;
        if ids.contains(&w.cell)
            && (matches!(w.operation.outcome(), Outcome::None | Outcome::Unresolved)
                || w.operation.integrity() == rx_domain::operation::Integrity::Disputed)
        {
            return reject(Reject::ContinuityUnproven);
        }
    }
    let resources: BTreeSet<_> = c
        .impact
        .cells
        .iter()
        .flat_map(|c| c.resources.iter().cloned())
        .collect();
    for resource in resources {
        if let Some(row) = tx.get(&key("resource", &resource))? {
            let r: Resource = decode(&row, RESOURCE)?;
            if r.holder.is_some() || r.quarantined {
                return reject(Reject::Busy);
            }
        }
    }
    for row in tx.scan("case/")? {
        let case: crate::intervention::Case = decode(&row, "rx.internal.intervention-case.v1")?;
        if case.state != crate::intervention::CaseState::Closed
            && (ids.contains(&case.cell) || case.effective_cells.iter().any(|c| ids.contains(c)))
        {
            return reject(Reject::BlockedByCase);
        }
    }
    let acks = tx.scan("fenceack/")?;
    for f in &p.fences {
        let (_, h): (_, HostRegistration) = load(tx, "host", (&f.cell, &f.host), HOST)?;
        let mut found = false;
        for row in &acks {
            let a: FenceAcknowledgment = decode(row, "rx.internal.fence-ack.v1")?;
            if a.invalidation == f.message
                && a.cell == f.cell
                && a.epoch == f.epoch
                && a.scopes == f.scopes
                && a.host_boot == h.boot_id
                && a.journal == h.delivery_journal
                && row.key == key("fenceack", (&f.host, &a.journal, a.sequence))
            {
                found = true;
                break;
            }
        }
        if !found {
            return reject(Reject::HostNotPrepared);
        }
    }
    Ok(())
}
fn send_authorized(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    t: &Task,
) -> Result<()> {
    let c = current_change(tx, meta, t)?;
    let sender = t.sender.identity();
    let p = authorize(
        tx,
        &sender,
        meta,
        now,
        Some(&t.origin),
        Role::ReleaseManager,
        true,
    )?;
    process_change::access(&p, &c.impact)?;
    if !generation_matches(tx, t)? {
        return reject(Reject::ContinuityUnproven);
    }
    barrier(tx, &c)
}
pub(super) fn summary(
    tx: &mut dyn Transaction,
    meta: &Installation,
    c: &Change,
) -> Result<Summary> {
    let expected: BTreeSet<_> = c
        .impact
        .cells
        .iter()
        .flat_map(|c| c.hosts.iter().cloned())
        .collect();
    let history = tx
        .scan("hostconfigtask/")?
        .iter()
        .map(|r| decode::<Task>(r, TASK))
        .collect::<Result<Vec<_>>>()?;
    let mut hosts = Vec::new();
    let mut applied = 0;
    let mut unknown = false;
    for host in expected {
        let t = if let Some(p) = &c.preparation {
            tx.get(&key_for(&c.id, p.attempt, &host))?
                .map(|r| decode::<Id>(&r, "rx.internal.host-config-task-ref.v1"))
                .transpose()?
                .map(|id| read(tx, &id).map(|v| v.1))
                .transpose()?
        } else {
            None
        };
        // Refresh must not hide an earlier Host effect or unknown send.
        let t = t.or_else(|| {
            history
                .iter()
                .filter(|t| t.change == c.id && t.host == host)
                .max_by_key(|t| t.preparation)
                .cloned()
        });
        let mut acknowledged = false;
        let mut outcome = None;
        if let Some(t) = &t {
            outcome = t.receipt.as_ref().map(|r| r.status);
            if outcome == Some(wire::Status::AppliedUnqualified) {
                applied += 1;
            }
            unknown |= t.phase == Phase::SendEntered && t.receipt.is_none();
            acknowledged = outcome == Some(wire::Status::AppliedUnqualified)
                && !t.integrity_disputed
                && t.issue.is_none()
                && generation_matches(tx, t)?
                && t.observation
                    .as_ref()
                    .is_some_and(|o| o.context_matches_current_host)
                && match current_change(tx, meta, t) {
                    Ok(_) => true,
                    Err(StoreError::Rejected(_)) => false,
                    Err(e) => return Err(e),
                };
        }
        hosts.push(HostStatus {
            host,
            task: t.as_ref().map(|t| t.id.clone()),
            preparation: t.as_ref().map(|t| t.preparation),
            phase: t.as_ref().map(|t| t.phase),
            outcome,
            issue: t.as_ref().and_then(|t| t.issue),
            acknowledged_for_preparation: acknowledged,
            integrity_disputed: t.as_ref().is_some_and(|t| t.integrity_disputed),
        });
    }
    Ok(Summary {
        mixed_configuration: applied > 0 && applied < hosts.len(),
        outcome_unknown: unknown,
        all_hosts_acknowledged: !hosts.is_empty()
            && hosts.iter().all(|h| h.acknowledged_for_preparation),
        hosts,
        revalidation_required_before_platform_apply: true,
        platform_configuration_applied: false,
    })
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// A current-process observation proof, never restored from a saved Task timestamp.
    pub fn record_host_configuration_read(
        &mut self,
        identity: &Identity,
        id: &Id,
        observation: wire::Observation,
        read_started: TimePoint,
    ) -> Result<Task> {
        let now = self.clock.now();
        if now
            .age_ns(&read_started)
            .is_none_or(|age| age > 3_000_000_000)
        {
            return reject(Reject::Expired);
        }
        let digest = canonical::digest("RX-HOST-CONFIGURATION-OBSERVATION-v1", &observation)
            .map_err(domain_error)?;
        let task = self.record_host_configuration_observation(identity, id, observation)?;
        self.configuration_reads
            .retain(|_, (at, _)| now.age_ns(at).is_some_and(|age| age <= 3_000_000_000));
        self.configuration_reads
            .insert(id.clone(), (read_started, digest));
        Ok(task)
    }
    pub fn authorize_host_configuration(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: Transition,
    ) -> Result<Batch> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let p = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&input.cell),
                Role::ReleaseManager,
                true,
            )?;
            let c = process_change::change(tx, &input.change, &input.cell)?;
            process_change::access(&p, &c.impact)?;
            let (scope, fp) = request(
                meta,
                &p,
                "ProcessChange.ConfigureHosts",
                key_.as_str(),
                &input,
            )?;
            if let Some(v) = prior(tx, &scope, fp, BATCH)? {
                return Ok(v);
            }
            if c.revision != input.expected
                || c.plan_digest != input.plan_digest
                || c.state != State::Staged
            {
                return reject(Reject::StaleRevision);
            }
            process_change::current(tx, meta, &c)?;
            let prep = c
                .preparation
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::StaleRevision))?;
            if prep.runtime_boot != meta.runtime_boot {
                return reject(Reject::StaleRevision);
            }
            barrier(tx, &c)?;
            for row in tx.scan("hostconfigtask/")? {
                let t: Task = decode(&row, TASK)?;
                if t.change == c.id
                    && t.preparation != prep.attempt
                    && t.phase == Phase::SendEntered
                    && (t.receipt.is_none() || t.integrity_disputed)
                {
                    return reject(Reject::ContinuityUnproven);
                }
            }
            let (terminal, certificate) = identity
                .terminal
                .clone()
                .ok_or(StoreError::Rejected(Reject::Forbidden))?;
            let sender = Sender {
                principal: p.id,
                session: identity.session.clone(),
                terminal,
                certificate,
            };
            let after = process_change::read_config(tx, &c.after)?;
            let hosts: BTreeSet<_> = c
                .impact
                .cells
                .iter()
                .flat_map(|c| c.hosts.iter().cloned())
                .collect();
            let mut tasks = Vec::new();
            for host in hosts {
                let slot = key_for(&c.id, prep.attempt, &host);
                if let Some(row) = tx.get(&slot)? {
                    let id = decode::<Id>(&row, "rx.internal.host-config-task-ref.v1")?;
                    let (rev, mut task) = read(tx, &id)?;
                    if task.receipt.is_none() && !task.integrity_disputed {
                        task.sender = sender.clone();
                        save_task(tx, &task, Some(rev))?;
                    }
                    tasks.push(id);
                    continue;
                }
                let mut cells = Vec::new();
                let mut incarnation = None;
                for impact in c.impact.cells.iter().filter(|c| c.hosts.contains(&host)) {
                    let (_, current): (_, Cell) = load(tx, "cell", &impact.id, CELL)?;
                    let (_, registration): (_, HostRegistration) =
                        load(tx, "host", (&impact.id, &host), HOST)?;
                    let fields = (
                        registration.boot_id.clone(),
                        registration.delivery_journal.clone(),
                        registration.session.clone(),
                    );
                    if incarnation.as_ref().is_some_and(|old| old != &fields) {
                        return reject(Reject::ContinuityUnproven);
                    }
                    incarnation = Some(fields);
                    let bound = prep
                        .cells
                        .iter()
                        .find(|b| b.cell == impact.id)
                        .ok_or(StoreError::Rejected(Reject::StaleRevision))?;
                    if current.epoch != bound.epoch || current.scope_epochs != bound.scopes {
                        return reject(Reject::StaleEpoch);
                    }
                    let fence = prep
                        .fences
                        .iter()
                        .find(|f| f.cell == impact.id && f.host == host)
                        .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
                    let target = if impact.id == c.cell {
                        &after
                    } else {
                        &current.configuration
                    };
                    cells.push(CellProjection {
                        cell: impact.id.clone(),
                        definition: target.definition.sha256,
                        envelope: target.envelope.sha256,
                        environment: name(match target.environment {
                            Environment::Simulation => "SIMULATION",
                            Environment::Physical => "PHYSICAL",
                        }),
                        before_configuration: process_change::config_ref(&current.configuration)?
                            .sha256,
                        after_configuration: process_change::config_ref(target)?.sha256,
                        recipe: target.recipe.clone(),
                        required_intents: target
                            .steps
                            .iter()
                            .filter(|s| s.host == host)
                            .map(|s| s.intent.digest().map_err(domain_error))
                            .collect::<Result<BTreeSet<_>>>()?
                            .into_iter()
                            .collect(),
                        required_conditions: target
                            .steps
                            .iter()
                            .filter(|s| s.host == host)
                            .flat_map(|s| s.condition_ids.iter().cloned())
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .collect(),
                        epoch: bound.epoch,
                        scopes: bound.scopes.clone(),
                        fence_request: fence.message.clone(),
                        change_blocks: bound.change_blocks.clone(),
                    });
                }
                if cells
                    .iter()
                    .map(|c| c.definition)
                    .collect::<BTreeSet<_>>()
                    .len()
                    != cells.len()
                {
                    return reject(Reject::InvalidInput);
                }
                let (host_boot, delivery_journal, producer_session) =
                    incarnation.ok_or(StoreError::Rejected(Reject::InvalidInput))?;
                let id = id();
                let t = Task {
                    id: id.clone(),
                    change: c.id.clone(),
                    origin: c.cell.clone(),
                    preparation: prep.attempt,
                    plan_digest: c.plan_digest,
                    host,
                    host_boot,
                    delivery_journal,
                    producer_session,
                    runtime_boot: meta.runtime_boot.clone(),
                    cells,
                    phase: Phase::AwaitingSnapshot,
                    request: None,
                    request_digest: None,
                    receipt: None,
                    observation: None,
                    issue: None,
                    integrity_disputed: false,
                    sender: sender.clone(),
                    created_at: now.clone(),
                };
                save_task(tx, &t, None)?;
                tx.put(
                    &slot,
                    None,
                    &doc("rx.internal.host-config-task-ref.v1", &id)?,
                )?;
                tasks.push(id);
            }
            let batch = Batch {
                change: c.id,
                preparation: prep.attempt,
                tasks,
            };
            remember(tx, &scope, fp, BATCH, &batch)?;
            Ok(batch)
        })
    }
    pub fn host_configuration_tasks(
        &mut self,
        identity: &Identity,
        after: Option<&Id>,
    ) -> Result<Vec<Task>> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = authorize(tx, identity, meta, &clock.now(), None, Role::Host, false)?;
            let mut tasks = tx
                .scan("hostconfigtask/")?
                .iter()
                .map(|r| decode::<Task>(r, TASK))
                .collect::<Result<Vec<_>>>()?;
            tasks.retain(|t| {
                t.host == p.id
                    && t.phase != Phase::Retired
                    && t.cells.iter().all(|c| p.cells.contains(&c.cell))
                    && after.is_none_or(|id| &t.id > id)
            });
            let mut selected = Vec::new();
            for t in tasks {
                let c = process_change::change(tx, &t.change, &t.origin)?;
                let current = c.state == State::Staged
                    && c.preparation
                        .as_ref()
                        .is_some_and(|p| p.attempt == t.preparation);
                // Keep unresolved older sends observable; settled history is read through the change.
                if !t.integrity_disputed
                    && (current || (t.phase == Phase::SendEntered && t.receipt.is_none()))
                    && !t
                        .receipt
                        .as_ref()
                        .is_some_and(|r| r.status == wire::Status::NotApplied)
                {
                    selected.push(t);
                }
            }
            let mut tasks = selected;
            tasks.sort_by(|a, b| a.id.cmp(&b.id));
            tasks.truncate(16);
            Ok(tasks)
        })
    }
    pub fn bind_host_configuration(
        &mut self,
        identity: &Identity,
        id: &Id,
        observation: wire::Observation,
        read_started: TimePoint,
    ) -> Result<Task> {
        observation.validate().map_err(StoreError::Invalid)?;
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (rev, mut t) = read(tx, id)?;
            host_access(tx, identity, meta, &now, &t)?;
            lifecycle::require_serving(tx)?;
            send_authorized(tx, meta, &now, &t)?;
            if t.request.is_some() {
                return Ok(t);
            }
            if now.age_ns(&read_started).is_none_or(|v| v > 3_000_000_000) {
                return reject(Reject::Expired);
            }
            let s = &observation.snapshot;
            if s.host != t.host
                || s.host_boot != t.host_boot
                || s.delivery_journal != t.delivery_journal
                || s.cells.iter().map(|c| &c.cell).collect::<BTreeSet<_>>()
                    != t.cells.iter().map(|c| &c.cell).collect()
            {
                return reject(Reject::ContinuityUnproven);
            }
            let mut cells = Vec::new();
            for target in &t.cells {
                let c = s
                    .cells
                    .iter()
                    .find(|c| c.cell == target.cell)
                    .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
                if c.definition != target.definition
                    || c.envelope != target.envelope
                    || c.environment != target.environment
                    || c.epoch != target.epoch
                    || c.scopes != target.scopes
                    || target
                        .change_blocks
                        .iter()
                        .any(|id| !c.blocked.contains(id))
                    || c.applied.as_ref().is_some_and(|a| {
                        a.configuration != target.before_configuration
                            && a.configuration != target.after_configuration
                    })
                {
                    return reject(Reject::ContinuityUnproven);
                }
                cells.push(wire::CellTarget {
                    cell: target.cell.clone(),
                    expected_context: c.applied.as_ref().map(|a| a.configuration),
                    before_configuration: target.before_configuration,
                    after_configuration: target.after_configuration,
                    recipe: target.recipe.clone(),
                    definition: target.definition,
                    envelope: target.envelope,
                    environment: target.environment.clone(),
                    required_intents: target.required_intents.clone(),
                    required_conditions: target.required_conditions.clone(),
                    epoch: target.epoch,
                    scopes: target.scopes.clone(),
                    fence_request: target.fence_request.clone(),
                });
            }
            let request = wire::Request {
                schema: name("rx.host-process-configuration-request.v1"),
                id: t.id.clone(),
                change: t.change.clone(),
                preparation: t.preparation,
                plan_digest: t.plan_digest,
                host: t.host.clone(),
                expected_host_boot: t.host_boot.clone(),
                expected_delivery_journal: t.delivery_journal.clone(),
                binding_digest: s.binding_digest,
                cells,
            };
            t.request_digest = Some(request.digest().map_err(StoreError::Invalid)?);
            t.request = Some(request);
            t.phase = Phase::Prepared;
            t.issue = None;
            save_task(tx, &t, Some(rev))?;
            Ok(t)
        })
    }
    pub fn enter_host_configuration_send(
        &mut self,
        identity: &Identity,
        id: &Id,
        retry_missing: bool,
    ) -> Result<Emission> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (rev, mut t) = read(tx, id)?;
            let now = clock.now();
            host_access(tx, identity, meta, &now, &t)?;
            if t.phase == Phase::SendEntered && !retry_missing {
                return Ok(Emission::Lookup { request: t.id });
            }
            lifecycle::require_serving(tx)?;
            send_authorized(tx, meta, &now, &t)?;
            if t.integrity_disputed || t.receipt.is_some() || t.phase == Phase::Retired {
                return reject(Reject::StaleRevision);
            }
            if retry_missing
                && !(t.phase == Phase::SendEntered
                    && t.issue == Some(Issue::ReceiptMissing)
                    && t.observation.as_ref().is_some_and(|o| {
                        o.receipt.is_none()
                            && o.snapshot.host_boot == t.host_boot
                            && o.snapshot.delivery_journal == t.delivery_journal
                    }))
            {
                return reject(Reject::ContinuityUnproven);
            }
            let request = t
                .request
                .clone()
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            t.phase = Phase::SendEntered;
            t.issue = None;
            save_task(tx, &t, Some(rev))?;
            Ok(Emission::Send {
                request: Box::new(request),
            })
        })
    }
    pub fn note_host_configuration_issue(
        &mut self,
        identity: &Identity,
        id: &Id,
        issue: Issue,
    ) -> Result<()> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (rev, mut t) = read(tx, id)?;
            host_access(tx, identity, meta, &clock.now(), &t)?;
            if t.issue == Some(issue) {
                return Ok(());
            }
            t.issue = Some(issue);
            save_task(tx, &t, Some(rev))
        })
    }
    pub fn record_host_configuration_observation(
        &mut self,
        identity: &Identity,
        id: &Id,
        observation: wire::Observation,
    ) -> Result<Task> {
        observation.validate().map_err(StoreError::Invalid)?;
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (rev, mut t) = read(tx, id)?;
            host_access(tx, identity, meta, &clock.now(), &t)?;
            if t.phase != Phase::SendEntered || observation.snapshot.host != t.host {
                return reject(Reject::InvalidInput);
            }
            let before = canonical::bytes(&t).map_err(domain_error)?;
            if let Some(receipt) = &observation.receipt {
                if Some(receipt.request_digest) != t.request_digest {
                    return reject(Reject::InvalidInput);
                }
                if let Some(old) = &t.receipt {
                    if canonical::bytes(old).map_err(domain_error)?
                        != canonical::bytes(receipt).map_err(domain_error)?
                    {
                        t.integrity_disputed = true;
                        t.issue = Some(Issue::ObservationMismatch);
                    }
                } else {
                    t.receipt = Some(receipt.clone());
                }
            } else if t.receipt.is_some() {
                t.integrity_disputed = true;
                t.issue = Some(Issue::ObservationMismatch);
            }
            let expected_snapshot =
                t.request.as_ref().is_some_and(|r| {
                    observation.snapshot.binding_digest == r.binding_digest
                        && observation.snapshot.cells.len() == r.cells.len()
                        && r.cells.iter().all(|target| {
                            observation.snapshot.cells.iter().any(|c| {
                                c.cell == target.cell
                                    && c.definition == target.definition
                                    && c.envelope == target.envelope
                                    && c.environment == target.environment
                                    && t.cells.iter().find(|p| p.cell == target.cell).is_some_and(
                                        |p| p.change_blocks.iter().all(|b| c.blocked.contains(b)),
                                    )
                                    && c.epoch == target.epoch
                                    && c.scopes == target.scopes
                                    && c.applied.as_ref().map(|a| a.configuration)
                                        == target.expected_context
                            })
                        })
                });
            if observation.receipt.is_none()
                && observation
                    .snapshot
                    .cells
                    .iter()
                    .any(|c| c.applied.as_ref().is_some_and(|a| a.request == t.id))
            {
                t.integrity_disputed = true;
                t.issue = Some(Issue::ObservationMismatch);
            }

            if !t.integrity_disputed {
                t.issue = if observation.snapshot.host_boot != t.host_boot
                    || observation.snapshot.delivery_journal != t.delivery_journal
                {
                    Some(Issue::HostGenerationChanged)
                } else if observation.receipt.is_none() {
                    Some(if expected_snapshot {
                        Issue::ReceiptMissing
                    } else {
                        Issue::ObservationMismatch
                    })
                } else if t
                    .receipt
                    .as_ref()
                    .is_some_and(|r| r.status == wire::Status::AppliedUnqualified)
                    && !observation.context_matches_current_host
                {
                    Some(Issue::ObservationMismatch)
                } else {
                    None
                };
            }
            t.observation = Some(observation);
            if canonical::bytes(&t).map_err(domain_error)? != before {
                save_task(tx, &t, Some(rev))?;
            }
            Ok(t)
        })
    }
}
