use super::*;
use crate::host_link::*;
const PLAN: &str = "rx.internal.host-link-plan.v1";
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Registered, pinned Host transport adapter only; not an externally callable RPC.
    pub fn prepare_host_link(&mut self, request: Prepare) -> Result<Plan> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let snapshot = &request.snapshot;
            snapshot.validate().map_err(domain_error)?;
            if snapshot.host != request.host
                || now
                    .age_ns(&request.read_started)
                    .is_none_or(|age| age > 100_000_000)
                || now.age_ns(&snapshot.captured_at).is_none()
                || snapshot.captured_at.ticks_ns < request.read_started.ticks_ns
                || request.ttl_ms.0 == 0
                || request.ttl_ms.0 > 30_000
                || !snapshot.sources_available
            {
                return reject(Reject::ConditionUnknown);
            }
            let (_, producer): (_, EvidenceProducer) = load(
                tx,
                "producer",
                &request.host,
                "rx.internal.evidence-producer.v1",
            )?;
            let identity = Identity {
                principal: request.host.clone(),
                session: producer.session.clone(),
                terminal: None,
            };
            authorize(
                tx,
                &identity,
                meta,
                &now,
                Some(&snapshot.cell),
                Role::Host,
                false,
            )?;
            let (revision, cell): (_, Cell) = load(tx, "cell", &snapshot.cell, CELL)?;
            if producer.peer_boot != snapshot.host_boot
                || producer.journal != snapshot.evidence_journal
                || producer.cells.get(&snapshot.cell) != Some(&snapshot.definition)
                || snapshot.definition != cell.configuration.definition.sha256
                || snapshot.envelope != cell.configuration.envelope.sha256
                || snapshot.environment.as_str()
                    != match cell.configuration.environment {
                        Environment::Simulation => "SIMULATION",
                        Environment::Physical => "PHYSICAL",
                    }
                || !cell.configuration.hosts.contains(&request.host)
            {
                return reject(Reject::ContinuityUnproven);
            }
            if snapshot.epoch > cell.epoch
                || snapshot.scopes.keys().collect::<BTreeSet<_>>()
                    != cell.scope_epochs.keys().collect()
                || snapshot
                    .scopes
                    .iter()
                    .any(|(s, e)| cell.scope_epochs.get(s).is_none_or(|p| e > p))
            {
                return reject(Reject::StaleEpoch);
            }
            let mut source_sessions = BTreeMap::new();
            for spec in cell
                .configuration
                .fact_specs
                .iter()
                .filter(|s| s.host == request.host)
            {
                let observation = snapshot
                    .observations
                    .iter()
                    .find(|o| o.source == spec.id)
                    .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
                if observation.schema != spec.schema || observation.unit != spec.unit {
                    return reject(Reject::InvalidInput);
                }
                source_sessions.insert(spec.id.clone(), observation.generation.clone());
            }
            if source_sessions.len() != snapshot.observations.len() {
                return reject(Reject::InvalidInput);
            }
            let current_key = key("host-link-current", (&request.host, &snapshot.cell));
            if let Some(row) = tx.get(&current_key)? {
                let previous: Id = decode(&row, "rx.internal.host-link-id.v1")?;
                let (_, plan): (_, Plan) = load(tx, "host-link-plan", &previous, PLAN)?;
                if plan.host_boot == snapshot.host_boot
                    && (plan.delivery_journal != snapshot.delivery_journal
                        || plan.evidence_journal != snapshot.evidence_journal)
                {
                    return reject(Reject::ContinuityUnproven);
                }
                if plan.platform_session == request.platform_session
                    && plan.producer_session == producer.session
                    && plan.host_boot == snapshot.host_boot
                    && plan.epoch == cell.epoch
                    && plan.scopes == cell.scope_epochs
                    && plan.source_sessions == source_sessions
                    && plan.valid_until.clock_id == now.clock_id
                    && now.ticks_ns < plan.valid_until.ticks_ns
                {
                    return Ok(plan);
                }
            }
            if !snapshot.pending_operations.is_empty() || !snapshot.pending_permits.is_empty() {
                return reject(Reject::Busy);
            }
            if snapshot
                .block_ids
                .iter()
                .any(|id| !cell.blocks.iter().any(|b| &b.id == id))
            {
                return reject(Reject::ContinuityUnproven);
            }
            if let Some(row) = tx.get(&key("host", (&snapshot.cell, &request.host)))? {
                let previous: HostRegistration = decode(&row, HOST)?;
                if previous.boot_id != snapshot.host_boot
                    || previous.delivery_journal != snapshot.delivery_journal
                    || previous.session != producer.session
                    || previous.source_sessions != source_sessions
                {
                    return reject(Reject::ContinuityUnproven);
                }
                if previous.grant.valid_until.clock_id == now.clock_id
                    && previous.grant.valid_until.ticks_ns > now.ticks_ns
                {
                    return reject(Reject::Busy);
                }
            }
            let resources: Vec<_> = cell
                .configuration
                .steps
                .iter()
                .filter(|s| s.host == request.host)
                .flat_map(|s| s.intent.resource_set.iter().cloned())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            for row in tx.scan("host/")? {
                let other: HostRegistration = decode(&row, HOST)?;
                if other.id == request.host
                    && (other.boot_id != snapshot.host_boot
                        || other.delivery_journal != snapshot.delivery_journal
                        || other.session != producer.session
                        || other.source_sessions.iter().any(|(source, generation)| {
                            source_sessions
                                .get(source)
                                .is_some_and(|next| next != generation)
                        }))
                {
                    return reject(Reject::ContinuityUnproven);
                }
                if other.id == request.host
                    && other.cell != snapshot.cell
                    && other.grant.valid_until.clock_id == now.clock_id
                    && other.grant.valid_until.ticks_ns > now.ticks_ns
                    && other.grant.resources.iter().any(|r| resources.contains(r))
                {
                    return reject(Reject::Busy);
                }
            }
            for row in tx.scan("run/")? {
                let run: Run = decode(&row, RUN)?;
                if run.cell == snapshot.cell && run.state == RunState::Executing {
                    return reject(Reject::Busy);
                }
            }
            let mut maximum = 0;
            for resource in &resources {
                maximum = maximum.max(
                    snapshot
                        .resource_fences
                        .get(resource)
                        .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?
                        .0,
                );
                if let Some(row) = tx.get(&key("host-link-fence", (&request.host, resource)))? {
                    maximum =
                        maximum.max(decode::<Counter>(&row, "rx.internal.host-link-fence.v1")?.0);
                }
            }
            let fence = Counter(maximum).increment().map_err(domain_error)?;
            for resource in &resources {
                let k = key("host-link-fence", (&request.host, resource));
                let previous = tx.get(&k)?;
                tx.put(
                    &k,
                    previous.map(|r| r.revision),
                    &doc("rx.internal.host-link-fence.v1", &fence)?,
                )?;
            }
            let plan = Plan {
                id: id(),
                host: request.host,
                cell: snapshot.cell.clone(),
                producer_session: producer.session,
                host_boot: snapshot.host_boot.clone(),
                evidence_journal: snapshot.evidence_journal.clone(),
                delivery_journal: snapshot.delivery_journal.clone(),
                platform_session: request.platform_session,
                expected_cell: revision,
                definition: snapshot.definition,
                epoch: cell.epoch,
                scopes: cell.scope_epochs,
                source_sessions,
                block_ids: cell
                    .blocks
                    .iter()
                    .filter(|b| b.latched)
                    .map(|b| b.id.clone())
                    .collect(),
                resources,
                fence,
                fence_request: id(),
                grant_request: id(),
                ttl_ms: request.ttl_ms,
                prepared_at: now.clone(),
                valid_until: TimePoint {
                    clock_id: now.clock_id,
                    ticks_ns: Counter(
                        now.ticks_ns
                            .0
                            .checked_add(100_000_000)
                            .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
                    ),
                },
                bound: false,
            };
            save(tx, "host-link-plan", &plan.id, None, PLAN, &plan)?;
            let previous = tx.get(&current_key)?;
            tx.put(
                &current_key,
                previous.map(|r| r.revision),
                &doc("rx.internal.host-link-id.v1", &plan.id)?,
            )?;
            event(tx, "rx.event.host-link-prepared.v1", &plan)?;
            Ok(plan)
        })
    }
    pub fn commit_host_link(&mut self, request: Commit) -> Result<HostRegistration> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let (revision, mut plan): (_, Plan) = load(tx, "host-link-plan", &request.plan, PLAN)?;
            let (_, producer): (_, EvidenceProducer) = load(
                tx,
                "producer",
                &plan.host,
                "rx.internal.evidence-producer.v1",
            )?;
            let identity = Identity {
                principal: plan.host.clone(),
                session: producer.session.clone(),
                terminal: None,
            };
            authorize(
                tx,
                &identity,
                meta,
                &now,
                Some(&plan.cell),
                Role::Host,
                false,
            )?;
            if producer.session != plan.producer_session
                || producer.peer_boot != plan.host_boot
                || producer.journal != plan.evidence_journal
                || producer.cells.get(&plan.cell) != Some(&plan.definition)
            {
                return reject(Reject::ContinuityUnproven);
            }
            let request_digest =
                canonical::digest("RX-HOST-LINK-COMMIT-v1", &request).map_err(domain_error)?;
            if plan.bound {
                let (_, cached): (_, BoundReceipt) = load(
                    tx,
                    "host-link-receipt",
                    &plan.id,
                    "rx.internal.host-link-receipt.v1",
                )?;
                if cached.request_digest != request_digest {
                    return Err(StoreError::KeyConflict);
                }
                return Ok(cached.registration);
            }
            if now.age_ns(&plan.prepared_at).is_none()
                || plan.valid_until.clock_id != now.clock_id
                || plan.valid_until.ticks_ns <= now.ticks_ns
            {
                return reject(Reject::Expired);
            }
            let (cell_revision, cell): (_, Cell) = load(tx, "cell", &plan.cell, CELL)?;
            check_revision(cell_revision, plan.expected_cell)?;
            if cell.epoch != plan.epoch || cell.scope_epochs != plan.scopes {
                return reject(Reject::StaleEpoch);
            }
            for resource in &plan.resources {
                let (_, maximum): (_, Counter) = load(
                    tx,
                    "host-link-fence",
                    (&plan.host, resource),
                    "rx.internal.host-link-fence.v1",
                )?;
                if maximum != plan.fence {
                    return reject(Reject::StaleEpoch);
                }
            }
            let ack = &request.fence_receipt;
            if ack.cell != plan.cell
                || ack.invalidation != plan.fence_request
                || ack.host_boot != plan.host_boot
                || ack.journal != plan.delivery_journal
                || ack.epoch != plan.epoch
                || ack.scopes != plan.scopes
                || ack.sequence.0 == 0
            {
                return reject(Reject::ContinuityUnproven);
            }
            let expected_until = request
                .grant_sent_at
                .ticks_ns
                .0
                .checked_add(
                    plan.ttl_ms
                        .0
                        .checked_mul(1_000_000)
                        .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
                )
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            let grant = request.grant;
            if grant.owner.as_str() != meta.id.as_str()
                || grant.fence != plan.fence
                || grant.resources != plan.resources
                || grant.ttl_ms != plan.ttl_ms
                || request.grant_sent_at.clock_id != now.clock_id
                || request.grant_sent_at.ticks_ns != plan.prepared_at.ticks_ns
                || request.grant_sent_at.ticks_ns > now.ticks_ns
                || grant.valid_until.clock_id != now.clock_id
                || grant.valid_until.ticks_ns.0 != expected_until
                || grant.valid_until.ticks_ns <= now.ticks_ns
            {
                return reject(Reject::InvalidInput);
            }
            let registration = HostRegistration {
                id: plan.host.clone(),
                session: plan.producer_session.clone(),
                boot_id: plan.host_boot.clone(),
                delivery_journal: plan.delivery_journal.clone(),
                cell: plan.cell.clone(),
                epoch: plan.epoch,
                scopes: plan.scopes.clone(),
                source_sessions: plan.source_sessions.clone(),
                grant,
            };
            let k = key("host", (&plan.cell, &plan.host));
            let old = tx.get(&k)?;
            if let Some(row) = &old {
                let old: HostRegistration = decode(row, HOST)?;
                if old.boot_id != registration.boot_id
                    || old.delivery_journal != registration.delivery_journal
                    || old.session != registration.session
                    || old.source_sessions != registration.source_sessions
                {
                    return reject(Reject::ContinuityUnproven);
                }
            }
            tx.put(&k, old.map(|r| r.revision), &doc(HOST, &registration)?)?;
            let k = key("fenceack", (&plan.host, &ack.journal, ack.sequence));
            let doc_ = doc("rx.internal.fence-ack.v1", ack)?;
            if let Some(old) = tx.get(&k)? {
                if old.document != doc_ {
                    return Err(StoreError::Integrity("link fence receipt conflict".into()));
                }
            } else {
                tx.put(&k, None, &doc_)?;
            }
            save(
                tx,
                "host-link-receipt",
                &plan.id,
                None,
                "rx.internal.host-link-receipt.v1",
                &BoundReceipt {
                    request_digest,
                    registration: registration.clone(),
                },
            )?;
            plan.bound = true;
            plan.valid_until = registration.grant.valid_until.clone();
            save(tx, "host-link-plan", &plan.id, Some(revision), PLAN, &plan)?;
            event(tx, "rx.event.host-link-bound.v1", &registration)?;
            Ok(registration)
        })
    }
    pub fn bound_host_link(&mut self, plan_id: &Id) -> Result<HostRegistration> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (_, plan): (_, Plan) = load(tx, "host-link-plan", plan_id, PLAN)?;
            let (_, producer): (_, EvidenceProducer) = load(
                tx,
                "producer",
                &plan.host,
                "rx.internal.evidence-producer.v1",
            )?;
            let identity = Identity {
                principal: plan.host.clone(),
                session: producer.session.clone(),
                terminal: None,
            };
            authorize(
                tx,
                &identity,
                meta,
                &clock.now(),
                Some(&plan.cell),
                Role::Host,
                false,
            )?;
            if !plan.bound
                || producer.session != plan.producer_session
                || producer.peer_boot != plan.host_boot
                || producer.journal != plan.evidence_journal
            {
                return reject(Reject::ContinuityUnproven);
            }
            Ok(load::<HostRegistration>(tx, "host", (&plan.cell, &plan.host), HOST)?.1)
        })
    }
    pub fn current_time(&self) -> TimePoint {
        self.clock.now()
    }
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn prepare_host_renewal(&mut self, plan_id: &Id) -> Result<Renewal> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            lifecycle::require_serving(tx)?;
            let now = clock.now();
            let (_, plan): (_, Plan) = load(tx, "host-link-plan", plan_id, PLAN)?;
            validate_owner(tx, meta, &now, &plan)?;
            let (_, registration): (_, HostRegistration) =
                load(tx, "host", (&plan.cell, &plan.host), HOST)?;
            let (_, cell): (_, Cell) = load(tx, "cell", &plan.cell, CELL)?;
            renewal_context(tx, &plan, &registration, &cell)?;
            let k = key("host-link-renewal", plan_id);
            let old = tx.get(&k)?;
            let mut sequence = Counter(0);
            if let Some(row) = &old {
                let renewal: Renewal = decode(row, "rx.internal.host-link-renewal.v1")?;
                if !renewal.completed {
                    return Ok(renewal);
                }
                sequence = renewal.sequence;
            }
            if registration.grant.valid_until.clock_id != now.clock_id
                || registration.grant.valid_until.ticks_ns <= now.ticks_ns
            {
                return reject(Reject::Expired);
            }
            let renewal = Renewal {
                plan: plan.id,
                request: id(),
                sequence: sequence.increment().map_err(domain_error)?,
                sent_at: now,
                grant: registration.grant,
                completed: false,
                response_digest: None,
            };
            tx.put(
                &k,
                old.map(|r| r.revision),
                &doc("rx.internal.host-link-renewal.v1", &renewal)?,
            )?;
            Ok(renewal)
        })
    }
    pub fn commit_host_renewal(
        &mut self,
        renewal: Renewal,
        grant: Grant,
    ) -> Result<HostRegistration> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            lifecycle::require_serving(tx)?;
            let now = clock.now();
            let (plan_revision, mut plan): (_, Plan) =
                load(tx, "host-link-plan", &renewal.plan, PLAN)?;
            validate_owner(tx, meta, &now, &plan)?;
            let k = key("host-link-renewal", &plan.id);
            let row = tx.get(&k)?.ok_or(StoreError::Rejected(Reject::NotFound))?;
            let mut expected: Renewal = decode(&row, "rx.internal.host-link-renewal.v1")?;
            if expected.request != renewal.request
                || expected.sequence != renewal.sequence
                || expected.sent_at != renewal.sent_at
                || canonical::bytes(&expected.grant).map_err(domain_error)?
                    != canonical::bytes(&renewal.grant).map_err(domain_error)?
            {
                return Err(StoreError::KeyConflict);
            }
            let response = canonical::digest("RX-HOST-RENEWAL-v1", &grant).map_err(domain_error)?;
            if expected.completed {
                if expected.response_digest != Some(response) {
                    return Err(StoreError::KeyConflict);
                }
                return Ok(load::<HostRegistration>(tx, "host", (&plan.cell, &plan.host), HOST)?.1);
            }
            let (revision, mut registration): (_, HostRegistration) =
                load(tx, "host", (&plan.cell, &plan.host), HOST)?;
            let (_, cell): (_, Cell) = load(tx, "cell", &plan.cell, CELL)?;
            renewal_context(tx, &plan, &registration, &cell)?;
            if registration.grant.id != expected.grant.id {
                return reject(Reject::StaleEpoch);
            }
            let mut baseline = expected.grant.clone();
            baseline.valid_until = grant.valid_until.clone();
            let until = expected
                .sent_at
                .ticks_ns
                .0
                .checked_add(
                    expected
                        .grant
                        .ttl_ms
                        .0
                        .checked_mul(1_000_000)
                        .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
                )
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            if canonical::bytes(&baseline).map_err(domain_error)?
                != canonical::bytes(&grant).map_err(domain_error)?
                || grant.valid_until.clock_id != now.clock_id
                || grant.valid_until.ticks_ns.0 != until
                || grant.valid_until.ticks_ns <= now.ticks_ns
            {
                return reject(Reject::Expired);
            }
            registration.grant = grant;
            save(
                tx,
                "host",
                (&plan.cell, &plan.host),
                Some(revision),
                HOST,
                &registration,
            )?;
            plan.valid_until = registration.grant.valid_until.clone();
            save(
                tx,
                "host-link-plan",
                &plan.id,
                Some(plan_revision),
                PLAN,
                &plan,
            )?;
            expected.completed = true;
            expected.response_digest = Some(response);
            tx.put(
                &k,
                Some(row.revision),
                &doc("rx.internal.host-link-renewal.v1", &expected)?,
            )?;
            event(tx, "rx.event.host-grant-renewed.v1", &registration)?;
            Ok(registration)
        })
    }
}
fn validate_owner(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    plan: &Plan,
) -> Result<()> {
    let (_, producer): (_, EvidenceProducer) = load(
        tx,
        "producer",
        &plan.host,
        "rx.internal.evidence-producer.v1",
    )?;
    authorize(
        tx,
        &Identity {
            principal: plan.host.clone(),
            session: producer.session.clone(),
            terminal: None,
        },
        meta,
        now,
        Some(&plan.cell),
        Role::Host,
        false,
    )?;
    if producer.session != plan.producer_session
        || producer.peer_boot != plan.host_boot
        || producer.journal != plan.evidence_journal
    {
        return reject(Reject::ContinuityUnproven);
    }
    Ok(())
}

fn renewal_context(
    tx: &mut dyn Transaction,
    plan: &Plan,
    r: &HostRegistration,
    cell: &Cell,
) -> Result<()> {
    let (_, bound): (_, BoundReceipt) = load(
        tx,
        "host-link-receipt",
        &plan.id,
        "rx.internal.host-link-receipt.v1",
    )?;
    if !plan.bound
        || r.epoch != cell.epoch
        || r.scopes != cell.scope_epochs
        || r.id != plan.host
        || r.session != plan.producer_session
        || r.boot_id != plan.host_boot
        || r.delivery_journal != plan.delivery_journal
        || r.source_sessions != plan.source_sessions
        || r.grant.id != bound.registration.grant.id
        || r.grant.fence != plan.fence
        || r.grant.resources.iter().collect::<BTreeSet<_>>() != plan.resources.iter().collect()
        || cell.configuration.definition.sha256 != plan.definition
        || cell
            .configuration
            .steps
            .iter()
            .filter(|s| s.host == plan.host)
            .flat_map(|s| &s.intent.resource_set)
            .any(|id| !r.grant.resources.contains(id))
    {
        return reject(Reject::StaleEpoch);
    }
    Ok(())
}
