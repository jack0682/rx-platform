use super::*;
use crate::diagnostics::*;
use rx_domain::condition::Condition;

pub(super) fn read(
    tx: &mut dyn Transaction,
    cell: &Cell,
    meta: &Installation,
    now: &TimePoint,
) -> Result<CellDiagnostics> {
    let config = &cell.configuration;
    let mut valid_for = 3_000_000_000u64;
    let mut hosts = vec![];
    let mut registrations = BTreeMap::new();
    for host in &config.hosts {
        let registration = tx
            .get(&key("host", (&config.id, host)))?
            .map(|r| decode::<HostRegistration>(&r, HOST))
            .transpose()?;
        let context = if let Some(h) = &registration {
            let identity = Identity {
                principal: host.clone(),
                session: h.session.clone(),
                terminal: None,
            };
            let authorized = match authorize(
                tx,
                &identity,
                meta,
                now,
                Some(&config.id),
                Role::Host,
                false,
            ) {
                Ok(_) => true,
                Err(StoreError::Rejected(_)) => false,
                Err(e) => return Err(e),
            };
            if !authorized {
                HostContext::IdentityUnavailable
            } else if h.id != *host
                || h.cell != config.id
                || h.epoch != cell.epoch
                || h.scopes != cell.scope_epochs
            {
                HostContext::ContextStale
            } else if h.grant.valid_until.clock_id != now.clock_id {
                HostContext::ClockMismatch
            } else if h.grant.valid_until.ticks_ns <= now.ticks_ns {
                HostContext::LeaseExpired
            } else {
                valid_for = valid_for.min(h.grant.valid_until.ticks_ns.0 - now.ticks_ns.0);
                HostContext::Current
            }
        } else {
            HostContext::Unregistered
        };
        hosts.push(HostDiagnostic {
            runtime: None,
            host: host.clone(),
            context,
            grant_valid_until: registration.as_ref().map(|h| h.grant.valid_until.clone()),
        });
        if let Some(h) = registration {
            registrations.insert(host.clone(), h);
        }
    }
    let mut samples = BTreeMap::new();
    let mut facts = BTreeMap::new();
    let mut generations = BTreeMap::new();
    for spec in &config.fact_specs {
        if let Some(h) = registrations.get(&spec.host)
            && let Some(g) = h.source_sessions.get(&spec.id)
        {
            generations.insert(spec.id.clone(), g.clone());
        }
        if let Some(record) = tx.get(&key("fact", (&config.id, &spec.id)))? {
            let f: FactRecord = decode(&record, FACT)?;
            if f.cell != config.id || f.id != spec.id {
                return Err(StoreError::Integrity("observation identity differs".into()));
            }
            facts.insert(spec.id.clone(), domain_fact(spec, &f));
            samples.insert(spec.id.clone(), f);
        }
    }
    let context = Context {
        now,
        facts: &facts,
        generations: &generations,
    };
    let mut sources = vec![];
    for spec in &config.fact_specs {
        let mut issues = vec![];
        let f = samples.get(&spec.id);
        let generation = generations.get(&spec.id);
        if !registrations.contains_key(&spec.host) {
            issues.push(SourceIssue::HostUnregistered);
        }
        if generation.is_none() {
            issues.push(SourceIssue::GenerationUnregistered);
        }
        let mut age = None;
        let mut usable = false;
        if let Some(f) = f {
            if generation.is_some_and(|g| g != &f.source_generation) {
                issues.push(SourceIssue::GenerationMismatch);
            }
            if f.source_host != spec.host {
                issues.push(SourceIssue::WrongSourceHost);
            }
            if f.schema != spec.schema {
                issues.push(SourceIssue::SchemaMismatch);
            }
            if f.unit != spec.unit {
                issues.push(SourceIssue::UnitMismatch);
            }
            if !f.quality_good {
                issues.push(SourceIssue::BadQuality);
            }
            if !f.origin_age_bounded {
                issues.push(SourceIssue::OriginAgeUnbounded);
            }
            if f.disputed {
                issues.push(SourceIssue::Disputed);
            }
            if f.acquisition_uncertainty_ns > spec.maximum_uncertainty_ns {
                issues.push(SourceIssue::UncertaintyExceeded);
            }
            if f.acquired_at.clock_id != now.clock_id {
                issues.push(SourceIssue::ClockMismatch);
            } else if let Some(measured) = now.age_ns(&f.acquired_at) {
                age = Some(Counter(measured));
                if let Some(bounded) = measured.checked_add(f.acquisition_uncertainty_ns.0) {
                    if bounded > spec.maximum_age_ns.0 {
                        issues.push(SourceIssue::AgeExceeded);
                    }
                } else {
                    issues.push(SourceIssue::AgeOverflow);
                }
            } else {
                issues.push(SourceIssue::FutureTimestamp);
            }
            let probe = Condition::Eq {
                fact: spec.id.clone(),
                schema: spec.schema.clone(),
                unit: spec.unit.clone(),
                expected: f.value.clone(),
            }
            .evaluate(&context)
            .map_err(domain_error)?;
            usable = probe.verdict == Verdict::Pass;
            if usable && let Some(until) = probe.valid_until {
                valid_for = valid_for.min(until.ticks_ns.0.saturating_sub(now.ticks_ns.0));
            }
        } else {
            issues.push(SourceIssue::MissingObservation);
        }
        sources.push(SourceDiagnostic {
            source: spec.id.clone(),
            host: spec.host.clone(),
            expected_generation: generation.cloned(),
            maximum_age_ns: spec.maximum_age_ns,
            maximum_uncertainty_ns: spec.maximum_uncertainty_ns,
            age_ns: age,
            usable,
            issues,
            observation: f.cloned(),
        });
    }
    let mut conditions = vec![];
    let mut group =
        |kind: ConditionGroup, step: Option<&Name>, items: &[Condition]| -> Result<()> {
            for (index, expression) in items.iter().enumerate() {
                let evaluation = expression.evaluate(&context).map_err(domain_error)?;
                conditions.push(ConditionDiagnostic {
                    path: format!("{:?}/{}/{}", kind, step.map_or("", Name::as_str), index + 1),
                    group: kind,
                    step: step.cloned(),
                    expression: expression.clone(),
                    verdict: evaluation.verdict,
                    reason: match evaluation.verdict {
                        Verdict::Pass => ConditionReason::Satisfied,
                        Verdict::Fail => ConditionReason::NotSatisfied,
                        Verdict::Unknown => {
                            let mut referenced = BTreeSet::new();
                            source_ids(expression, &mut referenced);
                            if referenced.iter().any(|id| {
                                sources
                                    .iter()
                                    .find(|s| &s.source == *id)
                                    .is_none_or(|s| !s.usable)
                            }) {
                                ConditionReason::SourceUnavailable
                            } else {
                                ConditionReason::ExpressionMismatch
                            }
                        }
                    },
                    evidence_ids: evaluation.evidence_ids,
                    valid_until: evaluation.valid_until,
                });
            }
            Ok(())
        };
    group(ConditionGroup::Start, None, &config.start_conditions)?;
    group(
        ConditionGroup::Maintained,
        None,
        &config.maintained_conditions,
    )?;
    for step in &config.steps {
        group(ConditionGroup::Operation, Some(&step.id), &step.conditions)?;
    }
    Ok(CellDiagnostics {
        schema: "rx.cell-diagnostics.v1",
        display_valid_for_ns: Counter(valid_for),
        sources,
        hosts,
        conditions,
    })
}

fn source_ids<'a>(expression: &'a Condition, result: &mut BTreeSet<&'a Name>) {
    match expression {
        Condition::All { children } | Condition::Any { children } => {
            for child in children {
                source_ids(child, result);
            }
        }
        Condition::Eq { fact, .. }
        | Condition::Range { fact, .. }
        | Condition::SetContains { fact, .. } => {
            result.insert(fact);
        }
    }
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Trusted local supervisor binding only. It creates no stored execution authority.
    pub fn service_owner(
        &mut self,
        target: crate::service_health::Target,
    ) -> Result<crate::service_health::Owner> {
        let boot = self.installation.runtime_boot.clone();
        self.repository.transact(|tx| {
            let (_, cell): (_, Cell) = load(tx, "cell", &target.cell, CELL)?;
            if !cell.configuration.hosts.contains(&target.host) {
                return reject(Reject::InvalidInput);
            }
            Ok(crate::service_health::Owner {
                target,
                id: id(),
                runtime_boot: boot,
                definition: cell.configuration.definition.sha256,
                envelope: cell.configuration.envelope.sha256,
            })
        })
    }
}
