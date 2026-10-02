//! Explicit inventory statements and atomic Run-owned reservations; no native effects.
use super::*;
use crate::execution_inventory as data;
use rx_domain::definition::Reference;
use rx_process_contract::execution_v2 as v2;
const POOL: &str = "rx.execution-slot-pool.v2";
const BINDING: &str = "rx.execution-run-binding.v2";
fn pool_key(resource: &Reference) -> Name {
    key("executionslotpool", (&resource.catalog, &resource.id))
}
fn decode_pool(row: &rx_ports::Record, resource: &Reference) -> Result<data::Pool> {
    let pool: data::Pool = decode(row, POOL)?;
    if pool.layout.resource.catalog != resource.catalog
        || pool.layout.resource.id != resource.id
        || pool.generation.0 == 0
        || pool.layout.count.0 == 0
        || pool.layout.count.0 > v2::MAX_SLOTS as u64
        || pool.holds.len() as u64 > pool.layout.count.0
        || pool.holds.iter().any(|(slot, hold)| {
            u64::from(*slot) >= pool.layout.count.0
                || hold.ordinal.0 == 0
                || hold.slot_ordinal.0 < hold.ordinal.0
                || hold.slot_ordinal.0 > pool.layout.count.0
                || (hold.consumed && hold.part.is_none())
        })
    {
        return Err(StoreError::Integrity(
            "slot pool identity or bounds differ".into(),
        ));
    }
    Ok(pool)
}
fn load_pool(tx: &mut dyn Transaction, resource: &Reference) -> Result<(Counter, data::Pool)> {
    let row = tx.get(&pool_key(resource))?.ok_or_else(|| {
        StoreError::Invalid(
            "SLOT_POOL_UNINITIALIZED explicit simulation inventory declaration required".into(),
        )
    })?;
    Ok((row.revision, decode_pool(&row, resource)?))
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn initialize_execution_slots(
        &mut self,
        identity: &Identity,
        request_key: &Id,
        input: data::Initialize,
    ) -> Result<data::Pool> {
        if input.reason.trim().is_empty() || input.reason.chars().count() > 2048 {
            return reject(Reject::InvalidInput);
        }
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let actor = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&input.cell),
                Role::Operator,
                true,
            )?;
            let (scope, digest) = request(
                meta,
                &actor,
                "Execution.InitializeSlots",
                request_key.as_str(),
                &input,
            )?;
            if let Some(saved) = prior(tx, &scope, digest, POOL)? {
                return Ok(saved);
            }
            lifecycle::require_serving(tx)?;
            let (_, cell): (_, Cell) = load(tx, "cell", &input.cell, CELL)?;
            if cell.configuration.environment != Environment::Simulation {
                return reject(Reject::UnsupportedSchema);
            }
            let domain = execution_configuration::domain(tx, &cell.configuration)?
                .ok_or(StoreError::Rejected(Reject::UnsupportedSchema))?;
            let layout = domain
                .inputs
                .slot_resources(&domain.policy)
                .map_err(StoreError::Invalid)?
                .into_iter()
                .find(|p| p.resource == input.resource && p.rule == input.rule)
                .ok_or(StoreError::Rejected(Reject::StaleRevision))?;
            admission::quiet_cells(tx, &[input.cell.clone()].into())?;
            let key = pool_key(&input.resource);
            let old = tx.get(&key)?;
            let generation = match &old {
                Some(row) => {
                    let previous = decode_pool(row, &input.resource)?;
                    if previous.cell != input.cell {
                        return reject(Reject::Forbidden);
                    }
                    if input.expected_generation != Some(previous.generation) {
                        return reject(Reject::StaleRevision);
                    }
                    save(
                        tx,
                        "executionslotpoolhistory",
                        (
                            &input.resource.catalog,
                            &input.resource.id,
                            previous.generation,
                        ),
                        None,
                        POOL,
                        &previous,
                    )?;
                    previous.generation.increment().map_err(domain_error)?
                }
                None if input.expected_generation.is_none() => Counter(1),
                None => return reject(Reject::StaleRevision),
            };
            let pool = data::Pool {
                cell: input.cell,
                layout,
                generation,
                holds: BTreeMap::new(),
                actor: actor.id,
                recorded_at: now,
                reason: input.reason,
            };
            tx.require_workflow_execution_reader()?;
            tx.put(&key, old.map(|r| r.revision), &doc(POOL, &pool)?)?;
            event(tx, "rx.event.execution-slots-initialized.v2", &pool)?;
            remember(tx, &scope, digest, POOL, &pool)?;
            Ok(pool)
        })
    }
    pub fn execution_slot_pool(
        &mut self,
        identity: &Identity,
        resource: &Reference,
    ) -> Result<data::Pool> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (_, pool) = load_pool(tx, resource)?;
            authorize_read(tx, identity, meta, &clock.now(), &pool.cell)?;
            Ok(pool)
        })
    }
    pub fn create_execution_run(
        &mut self,
        identity: &Identity,
        request_key: &Id,
        input: data::CreateRun,
    ) -> Result<data::RunBinding> {
        if input.count.0 == 0 || input.count.0 > v2::MAX_SLOTS as u64 {
            return reject(Reject::InvalidInput);
        }
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let actor = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&input.cell),
                Role::Operator,
                true,
            )?;
            let (scope, digest) = request(
                meta,
                &actor,
                "Execution.CreateRun",
                request_key.as_str(),
                &input,
            )?;
            if let Some(saved) = prior(tx, &scope, digest, BINDING)? {
                return Ok(saved);
            }
            let (revision, cell): (_, Cell) = load(tx, "cell", &input.cell, CELL)?;
            check_revision(revision, input.expected_cell)?;
            ready(tx, &cell, &now)?;
            let source = cell
                .configuration
                .execution
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::UnsupportedSchema))?;
            if source.publication != input.publication {
                return reject(Reject::StaleRevision);
            }
            let domain = execution_configuration::domain(tx, &cell.configuration)?
                .ok_or(StoreError::Rejected(Reject::UnsupportedSchema))?;
            if input.count.0 > cell.configuration.maximum_budget.0
                || input.count.0 > domain.policy.slot_order.len() as u64
            {
                return reject(Reject::BudgetExhausted);
            }
            let resources = domain
                .inputs
                .slot_resources(&domain.policy)
                .map_err(StoreError::Invalid)?;
            let mut pools = Vec::new();
            for resource in resources {
                let (revision, pool) = load_pool(tx, &resource.resource)?;
                if pool.cell != input.cell {
                    return reject(Reject::Forbidden);
                }
                if pool.layout.resource != resource.resource
                    || pool.layout.rule != resource.rule
                    || pool.layout.layout_digest != resource.layout_digest
                    || pool.layout.count != resource.count
                {
                    return Err(StoreError::Invalid(
                        "SLOT_LAYOUT_CHANGED explicit quiet reinitialization required".into(),
                    ));
                }
                pools.push((revision, pool));
            }
            let chosen: Vec<_> = domain
                .policy
                .slot_order
                .iter()
                .enumerate()
                .filter(|(_, slot)| pools.iter().all(|(_, pool)| !pool.holds.contains_key(slot)))
                .take(input.count.0 as usize)
                .map(|(rank, slot)| data::Slot {
                    ordinal: Counter(0),
                    slot_ordinal: Counter(rank as u64 + 1),
                    index: *slot,
                })
                .collect();
            if chosen.len() as u64 != input.count.0 {
                return Err(StoreError::Invalid(format!(
                    "SLOT_POOL_EXHAUSTED requested {} available {}",
                    input.count.0,
                    chosen.len()
                )));
            }
            let run = Run {
                id: id(),
                cell: input.cell.clone(),
                recipe_digest: cell.configuration.recipe.sha256,
                envelope_digest: cell.configuration.envelope.sha256,
                purpose: None,
                state: RunState::Prepared,
                budget: None,
                executor_session: None,
                mandate: None,
                part_ids: vec![],
                pending_attempt: None,
            };
            let slots: Vec<_> = chosen
                .into_iter()
                .enumerate()
                .map(|(i, mut s)| {
                    s.ordinal = Counter(i as u64 + 1);
                    s
                })
                .collect();
            for (revision, pool) in &mut pools {
                for slot in &slots {
                    if pool
                        .holds
                        .insert(
                            slot.index,
                            data::Hold {
                                run: run.id.clone(),
                                ordinal: slot.ordinal,
                                slot_ordinal: slot.slot_ordinal,
                                part: None,
                                consumed: false,
                            },
                        )
                        .is_some()
                    {
                        return Err(StoreError::Integrity(
                            "slot already reserved at control cut".into(),
                        ));
                    }
                }
                tx.put(
                    &pool_key(&pool.layout.resource),
                    Some(*revision),
                    &doc(POOL, pool)?,
                )?;
            }
            let binding = data::RunBinding {
                run: run.id.clone(),
                cell: input.cell,
                configuration: cell
                    .configuration
                    .reference()
                    .map_err(StoreError::Invalid)?,
                publication: input.publication,
                policy: source.policy.clone(),
                pools: pools
                    .iter()
                    .map(|(_, p)| data::PoolBinding {
                        resource: p.layout.resource.clone(),
                        rule: p.layout.rule.clone(),
                        generation: p.generation,
                        layout_digest: p.layout.layout_digest,
                    })
                    .collect(),
                slots,
                actor: actor.id,
                request: request_key.clone(),
                created_at: now,
            };
            run_configuration::create(tx, &run, &cell.configuration)?;
            save(tx, "run", &run.id, None, RUN, &run)?;
            save(tx, "executionrun", &run.id, None, BINDING, &binding)?;
            event(tx, "rx.event.execution-run-created.v2", &binding)?;
            remember(tx, &scope, digest, BINDING, &binding)?;
            Ok(binding)
        })
    }
    pub fn execution_run(&mut self, identity: &Identity, run: &Id) -> Result<data::RunBinding> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (_, binding): (_, data::RunBinding) = load(tx, "executionrun", run, BINDING)?;
            if binding.run != *run {
                return Err(StoreError::Integrity(
                    "execution Run identity differs".into(),
                ));
            }
            authorize_read(tx, identity, meta, &clock.now(), &binding.cell)?;
            Ok(binding)
        })
    }
}

const OBJECT: &str = "rx.execution-object-binding.v2";
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn bind_execution_object(
        &mut self,
        identity: &Identity,
        request_key: &Id,
        input: data::BindObject,
    ) -> Result<data::ObjectBinding> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (_, run): (_, Run) = load(tx, "run", &input.run, RUN)?;
            let actor = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&run.cell),
                Role::Operator,
                true,
            )?;
            let (scope, digest) = request(
                meta,
                &actor,
                "Execution.BindObject",
                request_key.as_str(),
                &input,
            )?;
            if let Some(saved) = prior(tx, &scope, digest, OBJECT)? {
                return Ok(saved);
            }
            let (_, binding): (_, data::RunBinding) = load(tx, "executionrun", &run.id, BINDING)?;
            let (_, cell): (_, Cell) = load(tx, "cell", &run.cell, CELL)?;
            ready(tx, &cell, &now)?;
            run_configuration::require_current(tx, &run, &cell)?;
            if !matches!(run.state, RunState::Prepared | RunState::Executing)
                || input.ordinal.0 != run.part_ids.len() as u64 + 1
                || input.ordinal.0 > binding.slots.len() as u64
            {
                return reject(Reject::StaleRevision);
            }
            if let Some(last) = run.part_ids.last() {
                let (_, part): (_, PartAttempt) = load(tx, "part", last, PART)?;
                if part.disposition != PartDisposition::ConfirmedCompleted {
                    return reject(Reject::Busy);
                }
            }
            let domain = execution_configuration::domain(tx, &cell.configuration)?
                .ok_or(StoreError::Rejected(Reject::UnsupportedSchema))?;
            if input.object.catalog != domain.policy.workflow.catalog {
                return reject(Reject::Forbidden);
            }
            let object =
                definition_catalog::version(tx, &input.object.catalog, &input.object.id, None)?;
            if object.archived || object.definition.reference != input.object {
                return reject(Reject::StaleRevision);
            }
            let checked = domain
                .inputs
                .object_projection(&domain.policy, &object.definition)
                .map_err(StoreError::Invalid)?;
            let slot = &binding.slots[input.ordinal.0 as usize - 1];
            for pin in &binding.pools {
                let (_, pool) = load_pool(tx, &pin.resource)?;
                if pool.cell != run.cell
                    || pool.generation != pin.generation
                    || pool.layout.layout_digest != pin.layout_digest
                    || pool.holds.get(&slot.index).is_none_or(|h| {
                        h.run != run.id
                            || h.ordinal != input.ordinal
                            || h.part.is_some()
                            || h.consumed
                    })
                {
                    return reject(Reject::ContinuityUnproven);
                }
            }
            let owner = key(
                "executionobjectowner",
                (&input.object.catalog, &input.object.id),
            );
            if tx.get(&owner)?.is_some()
                || tx
                    .get(&key("executionobject", (&run.id, input.ordinal)))?
                    .is_some()
            {
                return reject(Reject::Busy);
            }
            let result = data::ObjectBinding {
                run: run.id.clone(),
                cell: run.cell,
                ordinal: input.ordinal,
                slot_ordinal: slot.slot_ordinal,
                slot: slot.index,
                object: checked.instance,
                model: checked.model,
                context: checked.context,
                candidate: checked.candidate,
                values_digest: checked.values_digest,
                sources: checked
                    .effective
                    .values
                    .into_iter()
                    .map(|(key, v)| (key, v.declared_by))
                    .collect(),
                actor: actor.id,
                request: request_key.clone(),
                created_at: now,
            };
            save(
                tx,
                "executionobject",
                (&run.id, input.ordinal),
                None,
                OBJECT,
                &result,
            )?;
            tx.put(&owner, None, &doc(OBJECT, &result)?)?;
            event(tx, "rx.event.execution-object-bound.v2", &result)?;
            remember(tx, &scope, digest, OBJECT, &result)?;
            Ok(result)
        })
    }
    pub fn execution_object(
        &mut self,
        identity: &Identity,
        run: &Id,
        ordinal: Counter,
    ) -> Result<data::ObjectBinding> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (_, object): (_, data::ObjectBinding) =
                load(tx, "executionobject", (run, ordinal), OBJECT)?;
            if object.run != *run || object.ordinal != ordinal {
                return Err(StoreError::Integrity(
                    "execution object binding identity differs".into(),
                ));
            }
            authorize_read(tx, identity, meta, &clock.now(), &object.cell)?;
            Ok(object)
        })
    }
}
