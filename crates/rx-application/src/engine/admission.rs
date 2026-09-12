use super::*;

pub(super) fn ready(tx: &mut dyn Transaction, cell: &Cell, _now: &TimePoint) -> Result<()> {
    lifecycle::require_serving(tx)?;
    let q = cell
        .qualification
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::NotCommissioned))?;
    qualification_activation::check_ready(tx, &_now.clock_id, cell)?;
    if cell.commissioning != Some(Commissioning::Commissioned)
        || q.envelope_digest != cell.configuration.envelope.sha256
        || q.environment != cell.configuration.environment
    {
        return reject(Reject::QualificationRequired);
    }
    if !cell.blocks.is_empty() || !cell.open_cases.is_empty() {
        return reject(Reject::BlockedByCase);
    }
    Ok(())
}

pub(super) fn prepared_host(
    tx: &mut dyn Transaction,
    cell: &Cell,
    host_id: &Name,
    installation: &Installation,
    now: &TimePoint,
) -> Result<HostRegistration> {
    let (_, host): (_, HostRegistration) =
        load(tx, "host", (&cell.configuration.id, host_id), HOST)
            .map_err(|e| missing_as(e, Reject::HostNotPrepared))?;
    authorize(
        tx,
        &Identity {
            principal: host_id.clone(),
            session: host.session.clone(),
            terminal: None,
        },
        installation,
        now,
        Some(&cell.configuration.id),
        Role::Host,
        false,
    )?;
    if host.epoch != cell.epoch
        || host.scopes != cell.scope_epochs
        || host.grant.valid_until.clock_id != now.clock_id
        || host.grant.valid_until.ticks_ns <= now.ticks_ns
    {
        return reject(Reject::HostNotPrepared);
    }
    Ok(host)
}

pub(super) fn active_run(
    tx: &mut dyn Transaction,
    cell: &Cell,
    run: &Run,
    identity: &Identity,
    installation: &Installation,
    now: &TimePoint,
) -> Result<()> {
    ready(tx, cell, now)?;
    run_configuration::require_current(tx, run, cell)?;
    if let Some(p) = run.purpose {
        qualification_activation::purpose(tx, cell, p)?;
    }
    if run.state != RunState::Executing
        || run.executor_session.as_ref() != Some(&identity.session)
        || identity.principal != cell.configuration.executor
    {
        return reject(Reject::MandateRevoked);
    }
    let (_, m): (_, Mandate) = load(
        tx,
        "mandate",
        run.mandate
            .as_ref()
            .ok_or(StoreError::Rejected(Reject::MandateRevoked))?,
        MANDATE,
    )?;
    if m.state != MandateState::Active
        || m.epoch != cell.epoch
        || m.scopes != cell.scope_epochs
        || m.executor_session != identity.session
    {
        return reject(Reject::MandateRevoked);
    }
    authorize(
        tx,
        identity,
        installation,
        now,
        Some(&run.cell),
        Role::Executor,
        false,
    )?;
    if !cell.configuration.maintained_conditions.is_empty() {
        evaluate(tx, cell, &cell.configuration.maintained_conditions, now)?;
    }
    Ok(())
}

pub(super) fn evaluate(
    tx: &mut dyn Transaction,
    cell: &Cell,
    conditions: &[rx_domain::condition::Condition],
    now: &TimePoint,
) -> Result<rx_domain::condition::Evaluation> {
    let evaluation = evaluate_raw(tx, cell, conditions, now)?;
    match evaluation.verdict {
        Verdict::Pass => Ok(evaluation),
        Verdict::Fail => reject(Reject::ConditionFailed),
        Verdict::Unknown => reject(Reject::ConditionUnknown),
    }
}
pub(super) fn evaluate_raw(
    tx: &mut dyn Transaction,
    cell: &Cell,
    conditions: &[rx_domain::condition::Condition],
    now: &TimePoint,
) -> Result<rx_domain::condition::Evaluation> {
    if conditions.is_empty() {
        return reject(Reject::ConditionUnknown);
    }
    let mut facts = BTreeMap::new();
    let mut generations = BTreeMap::new();
    for spec in &cell.configuration.fact_specs {
        if let Some(record) = tx.get(&key("fact", (&cell.configuration.id, &spec.id)))? {
            let f: FactRecord = decode(&record, FACT)?;
            let (_, h): (_, HostRegistration) =
                load(tx, "host", (&cell.configuration.id, &spec.host), HOST)?;
            if let Some(g) = h.source_sessions.get(&spec.id) {
                generations.insert(spec.id.clone(), g.clone());
            }
            facts.insert(spec.id.clone(), domain_fact(spec, &f));
        }
    }
    let evaluation = rx_domain::condition::Condition::All {
        children: conditions.to_vec(),
    }
    .evaluate(&Context {
        now,
        facts: &facts,
        generations: &generations,
    })
    .map_err(domain_error)?;
    Ok(evaluation)
}

pub(super) fn validate_part(tx: &mut dyn Transaction, run: &Run, part: &Option<Id>) -> Result<()> {
    match (run.purpose, part) {
        (Some(Purpose::Production), Some(id_)) => {
            let (_, p): (_, PartAttempt) = load(tx, "part", id_, PART)?;
            if p.run != run.id || !run.part_ids.contains(id_) {
                return reject(Reject::InvalidInput);
            }
        }
        (Some(Purpose::Setup), None) => {}
        _ => return reject(Reject::InvalidInput),
    }
    Ok(())
}

pub(super) fn predecessors_done(
    tx: &mut dyn Transaction,
    run: &Run,
    part: &Option<Id>,
    step: &StepBinding,
) -> Result<()> {
    let visit = if let Some(id_) = part {
        load::<PartAttempt>(tx, "part", id_, PART)?.1.ordinal
    } else {
        Counter(1)
    };
    for predecessor in &step.predecessors {
        let (_, a): (_, Activation) =
            load(tx, "activation", (&run.id, predecessor, visit), ACTIVATION)
                .map_err(|e| missing_as(e, Reject::ConditionUnknown))?;
        let operation = a
            .slots
            .get(&name("main"))
            .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
        let (_, work): (_, Work) = load(tx, "work", operation, WORK)?;
        if work.operation.outcome() != Outcome::Succeeded
            || work.operation.integrity() != Integrity::Valid
        {
            return reject(Reject::ConditionUnknown);
        }
    }
    Ok(())
}

pub(super) fn validate_configuration(c: &CellConfiguration) -> Result<()> {
    if let Some(process) = &c.process {
        rx_process_contract::validation::validate(process).map_err(StoreError::Invalid)?;
        let encoded = canonical::bytes(process).map_err(domain_error)?;
        if c.recipe.schema_id != process.schema || c.recipe.size_bytes.0 != encoded.len() as u64 {
            return reject(Reject::InvalidInput);
        }
        if rx_process_contract::frontier::resolved_digest(process).map_err(StoreError::Invalid)?
            != c.recipe.sha256
        {
            return reject(Reject::InvalidInput);
        }
        let mut required = BTreeMap::new();
        for node in rx_process_contract::validation::nodes(process) {
            if let rx_process_contract::CompiledBody::Operation { binding } = &node.body {
                required.insert(node.id.clone(), &process.bindings[binding]);
            }
        }
        if required.len() != c.steps.len() {
            return reject(Reject::InvalidInput);
        }
        for step in &c.steps {
            let binding = required
                .get(&step.id)
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            if !step.predecessors.is_empty()
                || step.host != binding.host
                || step.intent.digest().map_err(domain_error)?
                    != binding.intent.digest().map_err(domain_error)?
            {
                return reject(Reject::InvalidInput);
            }
        }
    }
    if c.scopes.is_empty()
        || c.hosts.is_empty()
        || c.steps.is_empty()
        || c.start_conditions.is_empty()
        || c.maximum_budget.0 == 0
        || c.permit_ttl_ns.0 == 0
        || c.start_timeout_ns.0 == 0
    {
        return reject(Reject::InvalidInput);
    }
    if c.scopes.iter().collect::<BTreeSet<_>>().len() != c.scopes.len()
        || c.hosts.iter().collect::<BTreeSet<_>>().len() != c.hosts.len()
        || c.steps.iter().map(|s| &s.id).collect::<BTreeSet<_>>().len() != c.steps.len()
        || c.fact_specs
            .iter()
            .map(|f| &f.id)
            .collect::<BTreeSet<_>>()
            .len()
            != c.fact_specs.len()
    {
        return reject(Reject::InvalidInput);
    }
    for fact in &c.fact_specs {
        if !c.hosts.contains(&fact.host) || fact.maximum_age_ns.0 == 0 {
            return reject(Reject::InvalidInput);
        }
    }
    let mut resolved = BTreeSet::new();
    for _ in 0..c.steps.len() {
        for s in &c.steps {
            if !c.hosts.contains(&s.host)
                || s.intent.site_config_digest != c.site_config_digest
                || s.conditions.is_empty()
                || s.condition_ids.len() != s.conditions.len()
                || s.condition_revision.0 == 0
                || s.handover_max_age_ns.0 == 0
                || s.condition_ids.iter().collect::<BTreeSet<_>>().len() != s.condition_ids.len()
            {
                return reject(Reject::InvalidInput);
            }
            s.intent.normalized().map_err(domain_error)?;
            match &s.completion {
                CompletionRule::NativeOutcomes {
                    table,
                    postconditions,
                } => {
                    table.validate().map_err(domain_error)?;
                    if table.profile_digest != s.intent.profile_digest
                        || table.completion_rule != s.intent.completion_rule
                        || (s.intent.kind != rx_domain::intent::Kind::FiniteAction
                            && postconditions.is_empty())
                    {
                        return reject(Reject::InvalidInput);
                    }
                }
                CompletionRule::Native {
                    success,
                    failure,
                    postconditions,
                    ..
                } => {
                    if success.is_empty()
                        || success.iter().any(|x| failure.iter().any(|f| f.0 == x.0))
                        || (s.intent.kind != rx_domain::intent::Kind::FiniteAction
                            && postconditions.is_empty())
                    {
                        return reject(Reject::InvalidInput);
                    }
                }
                CompletionRule::Predicate { conditions } if conditions.is_empty() => {
                    return reject(Reject::InvalidInput);
                }
                _ => {}
            }
            if s.predecessors.iter().all(|p| resolved.contains(p)) {
                resolved.insert(s.id.clone());
            }
        }
    }
    if resolved.len() != c.steps.len() {
        return reject(Reject::InvalidInput);
    }
    Ok(())
}

/// Always use the current source contract, including identity and uncertainty limits.
pub(super) fn domain_fact(spec: &FactSpec, f: &FactRecord) -> Fact {
    Fact {
        schema: f.schema.clone(),
        unit: f.unit.clone(),
        source_generation: f.source_generation.clone(),
        acquired_at: f.acquired_at.clone(),
        maximum_age_ns: spec.maximum_age_ns,
        acquisition_uncertainty_ns: f.acquisition_uncertainty_ns,
        quality_good: f.quality_good
            && f.source_host == spec.host
            && f.acquisition_uncertainty_ns <= spec.maximum_uncertainty_ns,
        origin_age_bounded: f.origin_age_bounded,
        disputed: f.disputed,
        value: FactValue::Scalar(f.value.clone()),
        evidence_id: f.evidence_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_source_identity_and_uncertainty_contract_apply_to_stored_facts() {
        let n = |s: &str| Name::new(s).unwrap();
        let id = || Id::new(uuid::Uuid::new_v4().to_string()).unwrap();
        let spec = FactSpec {
            id: n("source"),
            host: n("host/current"),
            schema: n("boolean/v1"),
            unit: n("unitless"),
            maximum_age_ns: Counter(100),
            maximum_uncertainty_ns: Counter(2),
        };
        let mut fact = FactRecord {
            cell: n("cell/a"),
            id: spec.id.clone(),
            source_host: spec.host.clone(),
            source_generation: id(),
            schema: spec.schema.clone(),
            unit: spec.unit.clone(),
            acquired_at: TimePoint {
                clock_id: "test".into(),
                ticks_ns: Counter(10),
            },
            maximum_age_ns: Counter(9999),
            acquisition_uncertainty_ns: Counter(0),
            quality_good: true,
            origin_age_bounded: true,
            disputed: false,
            value: TypedValue::Boolean(true),
            evidence_id: id(),
        };
        assert!(domain_fact(&spec, &fact).quality_good);
        assert_eq!(
            domain_fact(&spec, &fact).maximum_age_ns,
            spec.maximum_age_ns
        );
        fact.source_host = n("host/previous");
        assert!(!domain_fact(&spec, &fact).quality_good);
        fact.source_host = spec.host.clone();
        fact.acquisition_uncertainty_ns = Counter(3);
        assert!(!domain_fact(&spec, &fact).quality_good);
    }
}
