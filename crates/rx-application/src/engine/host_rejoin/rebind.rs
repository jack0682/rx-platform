use super::super::host_link;
use super::*;
use crate::{host_link as link, host_recovery as recovery};
const PREFIX: &str = "host-rejoin-rebind";
const REF: &str = "rx.host-rejoin-rebind-reference.v1";
const PLAN: &str = "rx.internal.host-link-plan.v1";
const TTL: u64 = 30_000_000_000;
pub(super) fn load_rebind(tx: &mut dyn Transaction, id: &Id) -> Result<(r::Rebind, r::Proposal)> {
    let (revision, b): (_, r::Rebind) = load(tx, PREFIX, id, r::REBIND_SCHEMA)?;
    let (_, p) = binding::load_binding(tx, &b.binding)?;
    if b.id != *id
        || b.revision != revision
        || b.schema.as_str() != r::REBIND_SCHEMA
        || b.proposal_digest != p.digest().map_err(StoreError::Integrity)?
        || b.steps.iter().any(|(c, v)| {
            v.plan.cell != *c
                || v.plan.host != p.context.host
                || v.plan.producer_session != p.context.producer.session
                || v.plan.host_boot != p.context.producer.peer_boot
                || (v.phase == r::GrantPhase::Acknowledged) != v.commit.is_some()
        })
    {
        return Err(StoreError::Integrity("rebind record differs".into()));
    }
    Ok((b, p))
}
fn save_rebind(tx: &mut dyn Transaction, b: &mut r::Rebind, old: Option<Counter>) -> Result<()> {
    b.revision = old.map_or(Ok(Counter(1)), |r| r.increment().map_err(domain_error))?;
    save(tx, PREFIX, &b.id, old, r::REBIND_SCHEMA, b)?;
    event(tx, "rx.event.host-rejoin-rebind.v1", b)
}
fn closed(tx: &mut dyn Transaction, p: &r::Proposal) -> Result<()> {
    for row in tx.scan("work/")? {
        let w: Work = decode(&row, WORK)?;
        if p.context.cells.contains_key(&w.cell)
            && w.operation.disposition() != rx_domain::operation::Disposition::Released
        {
            return reject(Reject::Busy);
        }
    }
    for row in tx.scan("run/")? {
        let run: Run = decode(&row, RUN)?;
        if p.context.cells.contains_key(&run.cell)
            && matches!(run.state, RunState::Executing | RunState::RecoveryRequired)
        {
            return reject(Reject::Busy);
        }
    }
    let mut all = BTreeSet::new();
    for cut in p
        .context
        .cells
        .values()
        .filter(|v| v.cell.configuration.hosts.contains(&p.context.host))
    {
        let resources = cut
            .cell
            .configuration
            .steps
            .iter()
            .filter(|s| s.host == p.context.host)
            .flat_map(|s| s.intent.resource_set.iter())
            .collect::<BTreeSet<_>>();
        for id in resources {
            if !all.insert(id) {
                return reject(Reject::CapabilityMissing);
            }
            if let Some(row) = tx.get(&key("resource", id))? {
                let resource: Resource = decode(&row, RESOURCE)?;
                if resource.holder.is_some() || resource.quarantined {
                    return reject(Reject::Busy);
                }
            }
        }
    }
    Ok(())
}
fn current(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    b: &r::Rebind,
    p: &r::Proposal,
) -> Result<()> {
    proposal::access(tx, meta, now, &b.approved_by.identity(), &p.context)?;
    if now.clock_id != b.valid_until.clock_id || now.ticks_ns >= b.valid_until.ticks_ns {
        return reject(Reject::Expired);
    }
    let (parent, _) = binding::load_binding(tx, &b.binding)?;
    if parent.phase != recovery::Phase::RecoveryOnly {
        return reject(Reject::ContinuityUnproven);
    }
    binding::current(tx, meta, now, &parent, p)?;
    let (_, owner): (_, Id) = load(tx, "host-rejoin-rebind-owner", &p.context.host, REF)?;
    if owner != b.id {
        return reject(Reject::StaleRevision);
    }
    for step in b.steps.values() {
        if step.commit.as_ref().is_some_and(|c| {
            c.grant.valid_until.clock_id != now.clock_id
                || c.grant.valid_until.ticks_ns <= now.ticks_ns
        }) {
            return reject(Reject::Expired);
        }
        for resource in &step.plan.resources {
            let (_, maximum): (_, Counter) = load(
                tx,
                "host-link-fence",
                (&p.context.host, resource),
                "rx.internal.host-link-fence.v1",
            )?;
            if maximum != step.plan.fence {
                return reject(Reject::StaleEpoch);
            }
        }
    }
    closed(tx, p)
}
fn actual_read(
    tx: &mut dyn Transaction,
    now: &TimePoint,
    b: &r::Rebind,
    p: &r::Proposal,
    read: &recovery::ReadEvidence,
    committing: bool,
) -> Result<()> {
    let mut allowed = BTreeMap::new();
    for step in b.steps.values() {
        if read.platform_session != step.plan.platform_session {
            return reject(Reject::ContinuityUnproven);
        }
        for resource in &step.plan.resources {
            let mut values = BTreeSet::new();
            if !committing && step.phase != r::GrantPhase::Acknowledged {
                values.insert(b.previous_fences[resource]);
            }
            if step.phase != r::GrantPhase::Pending {
                values.insert(step.plan.fence);
            }
            allowed.insert(resource.clone(), values);
        }
    }
    reads::validate_fences(
        tx,
        now,
        &p.context,
        read,
        (!b.steps.is_empty()).then_some(&allowed),
    )?;
    for (id, actual) in &read.cells {
        let cell = &p.context.cells[id].cell;
        if actual.snapshot.epoch != cell.epoch
            || actual.snapshot.scopes != cell.scope_epochs
            || actual.snapshot.block_ids.iter().collect::<BTreeSet<_>>()
                != cell
                    .blocks
                    .iter()
                    .filter(|v| v.latched)
                    .map(|v| &v.id)
                    .collect()
            || !actual.snapshot.pending_operations.is_empty()
            || !actual.snapshot.pending_permits.is_empty()
        {
            return reject(Reject::ContinuityUnproven);
        }
    }
    Ok(())
}
pub(super) fn bound_current(
    tx: &mut dyn Transaction,
    meta: &Installation,
    b: &r::Rebind,
    p: &r::Proposal,
) -> Result<bool> {
    if b.phase != r::RebindPhase::Bound
        || p.context.runtime_boot != meta.runtime_boot
        || p.context.store_generation != meta.store_generation
    {
        return Ok(false);
    }
    let (_, producer): (_, EvidenceProducer) = load(
        tx,
        "producer",
        &p.context.host,
        "rx.internal.evidence-producer.v1",
    )?;
    if producer.session != p.context.producer.session
        || producer.peer_boot != p.context.producer.peer_boot
    {
        return Ok(false);
    }
    for (cell, step) in &b.steps {
        let (_, plan): (_, Id) = load(
            tx,
            "host-link-current",
            (&p.context.host, cell),
            "rx.internal.host-link-id.v1",
        )?;
        let (_, host): (_, HostRegistration) = load(tx, "host", (cell, &p.context.host), HOST)?;
        let commit = step
            .commit
            .as_ref()
            .ok_or(StoreError::Integrity("bound rebind has no grant".into()))?;
        if plan != step.plan.id
            || host.boot_id != step.plan.host_boot
            || host.session != step.plan.producer_session
            || host.grant.id != commit.grant.id
            || host.grant.fence != step.plan.fence
        {
            return Ok(false);
        }
    }
    Ok(true)
}
fn view(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    identity: &Identity,
    b: r::Rebind,
    p: r::Proposal,
) -> Result<r::RebindView> {
    proposal::access(tx, meta, now, identity, &p.context)?;
    let current = if b.phase == r::RebindPhase::Bound {
        bound_current(tx, meta, &b, &p)?
    } else {
        match current(tx, meta, now, &b, &p) {
            Ok(()) => true,
            Err(StoreError::Rejected(_)) => false,
            Err(e) => return Err(e),
        }
    };
    Ok(r::RebindView {
        rebind: b,
        proposal: p,
        current,
        production_authorized: false,
    })
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn approve_host_rebind(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: r::ApproveRebind,
    ) -> Result<r::RebindView> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (parent, p) = binding::load_binding(tx, &input.binding)?;
            let actor = proposal::access(tx, meta, &now, identity, &p.context)?;
            let (scope, fp) = request(
                meta,
                &actor,
                "HostRejoin.ApproveRebind",
                key_.as_str(),
                &input,
            )?;
            if let Some(old) = prior::<Id>(tx, &scope, fp, REF)? {
                let (b, p) = load_rebind(tx, &old)?;
                return view(tx, meta, &now, identity, b, p);
            }
            if input.expected_binding_revision != parent.revision
                || input.proposal_digest != p.digest().map_err(StoreError::Integrity)?
                || input.expected_cells != p.context.expected_cells()
                || parent.phase != recovery::Phase::RecoveryOnly
            {
                return reject(Reject::StaleRevision);
            }
            binding::current(tx, meta, &now, &parent, &p)?;
            closed(tx, &p)?;
            let k = key("host-rejoin-rebind-owner", &p.context.host);
            let old = tx.get(&k)?;
            if let Some(row) = &old {
                let existing: Id = decode(row, REF)?;
                let (previous, _) = load_rebind(tx, &existing)?;
                // An entered grant requires an explicit resolution, never a second request ID.
                if matches!(
                    previous.phase,
                    r::RebindPhase::Prepared | r::RebindPhase::Bound
                ) {
                    return reject(Reject::Busy);
                }
            }
            let mut b = r::Rebind {
                schema: name(r::REBIND_SCHEMA),
                id: id(),
                revision: Counter(1),
                binding: parent.id,
                proposal_digest: input.proposal_digest,
                approved_by: identity.into(),
                approved_at: now.clone(),
                valid_until: TimePoint {
                    clock_id: now.clock_id.clone(),
                    ticks_ns: Counter(
                        now.ticks_ns
                            .0
                            .checked_add(TTL)
                            .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
                    ),
                },
                phase: r::RebindPhase::Approved,
                previous_fences: BTreeMap::new(),
                steps: BTreeMap::new(),
                bound_at: None,
            };
            save_rebind(tx, &mut b, None)?;
            tx.put(&k, old.map(|r| r.revision), &doc(REF, &b.id)?)?;
            remember(tx, &scope, fp, REF, &b.id)?;
            view(tx, meta, &now, identity, b, p)
        })
    }
    pub fn host_rebind(&mut self, identity: &Identity, id: &Id) -> Result<r::RebindView> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (b, p) = load_rebind(tx, id)?;
            view(tx, meta, &now, identity, b, p)
        })
    }
    pub fn prepare_host_rebind(
        &mut self,
        id: &Id,
        verified: recovery::VerifiedRead,
        ttls: BTreeMap<Name, Counter>,
    ) -> Result<r::Rebind> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (mut b, p) = load_rebind(tx, id)?;
            if !matches!(b.phase, r::RebindPhase::Approved | r::RebindPhase::Prepared) {
                return reject(Reject::StaleRevision);
            }
            current(tx, meta, &now, &b, &p)?;
            actual_read(tx, &now, &b, &p, &verified.0, false)?;
            if b.phase == r::RebindPhase::Prepared {
                return Ok(b);
            }
            let host_cells = p
                .context
                .cells
                .iter()
                .filter(|(_, v)| v.cell.configuration.hosts.contains(&p.context.host))
                .map(|(c, _)| c)
                .collect::<BTreeSet<_>>();
            if ttls.keys().collect::<BTreeSet<_>>() != host_cells
                || ttls.values().any(|v| v.0 < 1000 || v.0 > 30000)
            {
                return reject(Reject::InvalidInput);
            }
            let (parent, _) = binding::load_binding(tx, &b.binding)?;
            for cell in host_cells {
                let cut = &p.context.cells[cell];
                let actual = &verified.0.cells[cell];
                let config = &cut.cell.configuration;
                let resources = config
                    .steps
                    .iter()
                    .filter(|s| s.host == p.context.host)
                    .flat_map(|s| s.intent.resource_set.iter().cloned())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let mut maximum = 0;
                for resource in &resources {
                    let observed = actual.snapshot.resource_fences[resource];
                    maximum = maximum.max(observed.0);
                    b.previous_fences.insert(resource.clone(), observed);
                }
                let fence = Counter(maximum).increment().map_err(domain_error)?;
                for resource in &resources {
                    let k = key("host-link-fence", (&p.context.host, resource));
                    let old = tx.get(&k)?;
                    tx.put(
                        &k,
                        old.map(|v| v.revision),
                        &doc("rx.internal.host-link-fence.v1", &fence)?,
                    )?;
                }
                let proof = parent.fences[cell]
                    .acknowledgment
                    .clone()
                    .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
                let plan = link::Plan {
                    id: id_for_plan(),
                    host: p.context.host.clone(),
                    cell: cell.clone(),
                    producer_session: p.context.producer.session.clone(),
                    host_boot: p.context.producer.peer_boot.clone(),
                    evidence_journal: actual.snapshot.evidence_journal.clone(),
                    delivery_journal: actual.snapshot.delivery_journal.clone(),
                    platform_session: verified.0.platform_session.clone(),
                    expected_cell: cut.revision,
                    definition: config.definition.sha256,
                    epoch: cut.cell.epoch,
                    scopes: cut.cell.scope_epochs.clone(),
                    source_sessions: actual
                        .snapshot
                        .observations
                        .iter()
                        .map(|o| (o.source.clone(), o.generation.clone()))
                        .collect(),
                    block_ids: actual.snapshot.block_ids.clone(),
                    resources,
                    fence,
                    fence_request: proof.invalidation.clone(),
                    grant_request: id_for_plan(),
                    ttl_ms: ttls[cell],
                    prepared_at: now.clone(),
                    valid_until: TimePoint {
                        clock_id: now.clock_id.clone(),
                        ticks_ns: Counter(
                            now.ticks_ns
                                .0
                                .checked_add(100_000_000)
                                .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
                        ),
                    },
                    bound: false,
                    provenance: Some(link::BootstrapProvenance {
                        transport: verified.0.transport.clone(),
                        configuration: verified.0.configuration.clone(),
                        configuration_read_started: verified.0.configuration_started.clone(),
                        configuration_read_finished: verified.0.configuration_finished.clone(),
                        host_read: actual.snapshot.clone(),
                    }),
                };
                host_link::baseline::validate_provenance(
                    plan.provenance.as_ref().unwrap(),
                    &actual.snapshot,
                    &actual.started,
                    config,
                    &now,
                )?;
                save(tx, "host-link-plan", &plan.id, None, PLAN, &plan)?;
                b.steps.insert(
                    cell.clone(),
                    r::RebindStep {
                        plan,
                        phase: r::GrantPhase::Pending,
                        fence: proof,
                        commit: None,
                    },
                );
            }
            let old = b.revision;
            b.phase = r::RebindPhase::Prepared;
            save_rebind(tx, &mut b, Some(old))?;
            Ok(b)
        })
    }
    pub fn plan_host_rebind_grant(
        &mut self,
        id: &Id,
        cell: &Name,
        verified: recovery::VerifiedRead,
    ) -> Result<link::Plan> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (mut b, p) = load_rebind(tx, id)?;
            if b.phase != r::RebindPhase::Prepared {
                return reject(Reject::StaleRevision);
            }
            current(tx, meta, &now, &b, &p)?;
            actual_read(tx, &now, &b, &p, &verified.0, false)?;
            let previous = b.revision;
            let step = b
                .steps
                .get_mut(cell)
                .ok_or(StoreError::Rejected(Reject::Forbidden))?;
            if step.phase == r::GrantPhase::Acknowledged {
                return reject(Reject::StaleRevision);
            }
            step.phase = r::GrantPhase::SendEntered;
            let plan = step.plan.clone();
            save_rebind(tx, &mut b, Some(previous))?;
            Ok(plan)
        })
    }
    pub fn record_host_rebind_grant(
        &mut self,
        id: &Id,
        cell: &Name,
        commit: link::Commit,
        host_boot: &Id,
    ) -> Result<r::Rebind> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (mut b, p) = load_rebind(tx, id)?;
            let step = b
                .steps
                .get(cell)
                .ok_or(StoreError::Rejected(Reject::Forbidden))?;
            if step.phase == r::GrantPhase::Pending
                || commit.plan != step.plan.id
                || host_boot != &step.plan.host_boot
                || !same(&commit.fence_receipt, &step.fence)?
            {
                return reject(Reject::ContinuityUnproven);
            }
            host_link::validate_response(meta, &step.plan.prepared_at, &step.plan, &commit)?;
            if let Some(old) = &step.commit {
                if !same(old, &commit)? {
                    return Err(StoreError::KeyConflict);
                }
                return Ok(b);
            }
            let old = b.revision;
            let step = b.steps.get_mut(cell).unwrap();
            step.commit = Some(commit);
            step.phase = r::GrantPhase::Acknowledged;
            match current(tx, meta, &now, &b, &p) {
                Ok(()) => {}
                Err(StoreError::Rejected(_)) => b.phase = r::RebindPhase::Attention,
                Err(e) => return Err(e),
            }
            save_rebind(tx, &mut b, Some(old))?;
            Ok(b)
        })
    }
    pub fn commit_host_rebind(
        &mut self,
        id: &Id,
        verified: recovery::VerifiedRead,
    ) -> Result<r::Rebind> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (mut b, p) = load_rebind(tx, id)?;
            if b.phase == r::RebindPhase::Bound {
                return Ok(b);
            }
            if b.phase != r::RebindPhase::Prepared
                || b.steps
                    .values()
                    .any(|s| s.phase != r::GrantPhase::Acknowledged)
            {
                return reject(Reject::HostNotPrepared);
            }
            current(tx, meta, &now, &b, &p)?;
            actual_read(tx, &now, &b, &p, &verified.0, true)?;
            for (cell, step) in &b.steps {
                let request = step.commit.as_ref().unwrap();
                host_link::validate_response(meta, &now, &step.plan, request)?;
                let registration = HostRegistration {
                    id: step.plan.host.clone(),
                    session: step.plan.producer_session.clone(),
                    boot_id: step.plan.host_boot.clone(),
                    delivery_journal: step.plan.delivery_journal.clone(),
                    cell: cell.clone(),
                    epoch: step.plan.epoch,
                    scopes: step.plan.scopes.clone(),
                    source_sessions: step.plan.source_sessions.clone(),
                    grant: request.grant.clone(),
                };
                let k = key("host", (cell, &p.context.host));
                let old = tx
                    .get(&k)?
                    .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
                tx.put(&k, Some(old.revision), &doc(HOST, &registration)?)?;
                save(
                    tx,
                    "host-link-receipt",
                    &step.plan.id,
                    None,
                    "rx.internal.host-link-receipt.v1",
                    &link::BoundReceipt {
                        request_digest: canonical::digest("RX-HOST-LINK-COMMIT-v1", request)
                            .map_err(domain_error)?,
                        registration: registration.clone(),
                    },
                )?;
                let (pr, _): (_, link::Plan) = load(tx, "host-link-plan", &step.plan.id, PLAN)?;
                let mut plan = step.plan.clone();
                plan.bound = true;
                plan.valid_until = request.grant.valid_until.clone();
                host_link::baseline::create(
                    tx,
                    meta,
                    &p.context.producer,
                    &p.context.cells[cell].cell.configuration,
                    &plan,
                    &registration,
                    &now,
                )?;
                save(tx, "host-link-plan", &plan.id, Some(pr), PLAN, &plan)?;
                let k = key("host-link-current", (&p.context.host, cell));
                let old = tx.get(&k)?;
                tx.put(
                    &k,
                    old.map(|r| r.revision),
                    &doc("rx.internal.host-link-id.v1", &plan.id)?,
                )?;
                let base = p.context.cells[cell].baseline.as_ref().unwrap();
                let origin = r::ConfigurationOrigin {
                    rebind: b.id.clone(),
                    new_plan: plan.id.clone(),
                    prior_plan: base.plan.clone(),
                    prior_baseline_digest: base.digest().map_err(StoreError::Integrity)?,
                    configuration: crate::runtime_invalidation::configuration_digest(
                        &p.context.cells[cell].cell.configuration,
                    )
                    .map_err(StoreError::Integrity)?,
                    applied_context: plan
                        .provenance
                        .as_ref()
                        .unwrap()
                        .configuration
                        .snapshot
                        .cells
                        .iter()
                        .find(|c| &c.cell == cell)
                        .unwrap()
                        .applied
                        .clone(),
                };
                save(
                    tx,
                    "host-rebind-configuration-origin",
                    &plan.id,
                    None,
                    "rx.host-rebind-configuration-origin.v1",
                    &origin,
                )?;
            }
            let old = b.revision;
            b.phase = r::RebindPhase::Bound;
            b.bound_at = Some(now);
            save_rebind(tx, &mut b, Some(old))?;
            Ok(b)
        })
    }
    pub fn ready_host_rebind(
        &mut self,
        host: &Name,
        cell: &Name,
        retired_session: &Id,
    ) -> Result<Option<Id>> {
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let Some(row) = tx.get(&key("host-rejoin-rebind-owner", host))? else {
                return Ok(None);
            };
            let id: Id = decode(&row, REF)?;
            let (b, p) = load_rebind(tx, &id)?;
            if p.context.host != *host
                || !b.steps.contains_key(cell)
                || p.context
                    .replacement
                    .as_ref()
                    .is_none_or(|o| &o.before.session != retired_session)
                || !bound_current(tx, meta, &b, &p)?
            {
                return Ok(None);
            }
            Ok(Some(b.id))
        })
    }
}
fn id_for_plan() -> Id {
    id()
}
