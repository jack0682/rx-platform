use super::*;
use crate::device_binding::*;
const PLAN: &str = "rx.device-binding-plan.v1";
fn fingerprint(v: &impl Serialize) -> Result<Digest> {
    canonical::digest("RX-DEVICE-BINDING-CONTEXT-v1", v).map_err(domain_error)
}
fn seeds(c: &BTreeMap<Name, Candidate>) -> (BTreeSet<Name>, BTreeSet<Name>) {
    (
        c.values().map(|v| v.step.host.clone()).collect(),
        c.values()
            .flat_map(|v| v.step.intent.resource_set.iter().cloned())
            .collect(),
    )
}
pub(super) fn approved(
    tx: &mut dyn Transaction,
    meta: &Installation,
    cell: &Name,
    r: &ReviewRef,
) -> Result<(
    crate::device_review::Job,
    crate::device_review::Version,
    crate::device_review::Decision,
)> {
    let j = device_review::job(tx, &r.id, cell)?;
    let v = device_review::latest(tx, &r.id)?.ok_or(StoreError::Rejected(Reject::NotFound))?;
    let (revision, d): (_, crate::device_review::Decision) = load(
        tx,
        "devicereviewdecision",
        &r.id,
        "rx.device-review-decision.v1",
    )?;
    if !device_review::context(tx, meta, &j)?
        || v.checker_digest != crate::device_review::checker_digest()
        || !v.ready_for_software_approval
        || !v.report.passed()
        || v.revision != r.revision
        || v.review_digest != r.review_digest
        || revision != r.decision_revision
        || d.revision != revision
        || d.review != r.id
        || d.cell != *cell
        || d.choice != crate::device_review::Choice::Approve
        || d.scope.as_str() != "DEVICE_PACKAGE_SOFTWARE"
        || d.report_revision != v.revision
        || d.report_digest != v.report_digest
        || d.review_digest != v.review_digest
    {
        return reject(Reject::QualificationRequired);
    }
    Ok((j, v, d))
}
pub(super) fn read(tx: &mut dyn Transaction, id: &Id, cell: &Name) -> Result<Plan> {
    let (revision, p): (_, Plan) = load(tx, "devicebindingplan", id, PLAN)?;
    if &p.cell != cell {
        return reject(Reject::Forbidden);
    }
    if p.id != *id
        || p.definition.input.id != *id
        || p.definition.input.cell != *cell
        || p.revision != revision
        || p.plan_digest != p.digest().map_err(StoreError::Integrity)?
        || (p.state == State::ImpactReviewed) != p.impact_review.is_some()
        || !p.definition.requires_process_review
        || !p.definition.requires_host_binding
        || !p.definition.requires_operating_envelope_review
    {
        return Err(StoreError::Integrity(
            "device binding plan identity/digest differs".into(),
        ));
    }
    Ok(p)
}
pub(super) fn current(tx: &mut dyn Transaction, p: &Plan) -> Result<bool> {
    let (_, cell): (_, Cell) = load(tx, "cell", &p.cell, CELL)?;
    let (hosts, resources) = seeds(&p.definition.candidates);
    Ok(p.definition.builder_digest == builder_digest()
        && package_intake::configuration_digest(&cell)? == p.definition.base_configuration_digest
        && process_change::config_ref(&cell.configuration)? == p.definition.before
        && fingerprint(&process_change::prospective_impact(
            tx, &p.cell, &hosts, &resources,
        )?)? == fingerprint(&p.definition.impact)?)
}
fn verify_prepared(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    p: &Prepared,
) -> Result<Impact> {
    let t = &p.ticket;
    if t.boot != meta.runtime_boot
        || now.age_ns(&t.issued).is_none_or(|a| a >= 30_000_000_000)
        || package_intake::current(tx, meta)?.as_ref() != Some(&t.registration)
        || p.verified.stored.owner() != &t.registration.store_owner
        || p.verified.stored.policy_fingerprint() != t.registration.policy_fingerprint
        || p.verified.report.digest().map_err(StoreError::Invalid)? != t.version.report_digest
        || !p.verified.report.passed()
    {
        return reject(Reject::StaleRevision);
    }
    let (_, cell): (_, Cell) = load(tx, "cell", &t.input.cell, CELL)?;
    if fingerprint(&cell.configuration)? != fingerprint(&t.before)? {
        return reject(Reject::StaleRevision);
    }
    let (_, v, d) = approved(tx, meta, &t.input.cell, &t.input.review)?;
    if v.review_digest != t.version.review_digest || d.revision != t.decision.revision {
        return reject(Reject::StaleRevision);
    }
    let (hosts, resources) = seeds(&p.candidates);
    process_change::prospective_impact(tx, &t.input.cell, &hosts, &resources)
}
fn ticket(
    meta: &Installation,
    now: TimePoint,
    caller: (&Identity, &Id),
    input: Propose,
    before: CellConfiguration,
    reviewed: (
        crate::device_review::Job,
        crate::device_review::Version,
        crate::device_review::Decision,
    ),
    action: Action,
) -> Ticket {
    let (job, version, decision) = reviewed;
    Ticket {
        action,
        input,
        before,
        registration: job.registration.clone(),
        job,
        version,
        decision,
        identity: caller.0.clone(),
        key: caller.1.clone(),
        boot: meta.runtime_boot.clone(),
        issued: now,
    }
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn prepare_device_binding(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: Propose,
    ) -> Result<Preflight> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let actor = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&input.cell),
                Role::Engineer,
                false,
            )?;
            if input.reason.trim().is_empty()
                || input.reason.chars().count() > 1000
                || input.bindings.is_empty()
                || input.bindings.len() > 64
            {
                return reject(Reject::InvalidInput);
            }
            let (scope, fp) =
                request(meta, &actor, "DeviceBinding.Propose", key_.as_str(), &input)?;
            if let Some(p) = prior::<Plan>(tx, &scope, fp, PLAN)? {
                process_change::access(&actor, &p.definition.impact)?;
                return Ok(Preflight::Recorded(Box::new(p)));
            }
            let (j, v, d) = approved(tx, meta, &input.cell, &input.review)?;
            let (_, cell): (_, Cell) = load(tx, "cell", &input.cell, CELL)?;
            Ok(Preflight::Verify(Box::new(ticket(
                meta,
                now,
                (identity, key_),
                input,
                cell.configuration,
                (j, v, d),
                Action::Propose,
            ))))
        })
    }
    pub fn commit_device_binding(&mut self, p: Prepared) -> Result<Plan> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let t = &p.ticket;
            if !matches!(t.action, Action::Propose) {
                return reject(Reject::InvalidInput);
            }
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let actor = authorize(
                tx,
                &t.identity,
                meta,
                &now,
                Some(&t.input.cell),
                Role::Engineer,
                false,
            )?;
            let (scope, fp) = request(
                meta,
                &actor,
                "DeviceBinding.Propose",
                t.key.as_str(),
                &t.input,
            )?;
            if let Some(old) = prior::<Plan>(tx, &scope, fp, PLAN)? {
                process_change::access(&actor, &old.definition.impact)?;
                return Ok(old);
            }
            let impact = verify_prepared(tx, meta, &now, &p)?;
            process_change::access(&actor, &impact)?;
            let before = process_change::store_config(tx, &t.before)?;
            let (_, cell): (_, Cell) = load(tx, "cell", &t.input.cell, CELL)?;
            let definition = Definition {
                input: t.input.clone(),
                before,
                base_configuration_digest: package_intake::configuration_digest(&cell)?,
                object: p.verified.stored.object().clone(),
                catalog: t.job.request.catalog.clone(),
                builder_digest: builder_digest(),
                candidates: p.candidates,
                issues: p.issues,
                impact,
                requires_process_review: true,
                requires_host_binding: true,
                requires_operating_envelope_review: true,
            };
            let mut plan = Plan {
                id: t.input.id.clone(),
                cell: t.input.cell.clone(),
                revision: Counter(1),
                state: State::Proposed,
                plan_digest: Digest::from_bytes([0; 32]),
                definition,
                proposed_by: actor.id,
                proposed_at: now,
                impact_review: None,
            };
            plan.plan_digest = plan.digest().map_err(StoreError::Invalid)?;
            if canonical::bytes(&plan).map_err(domain_error)?.len()
                + canonical::bytes(&t.before).map_err(domain_error)?.len()
                > 786_432
            {
                return reject(Reject::InvalidInput);
            }
            save(tx, "devicebindingplan", &plan.id, None, PLAN, &plan)?;
            save(
                tx,
                "devicebindinghistory",
                (&plan.id, plan.revision),
                None,
                PLAN,
                &plan,
            )?;
            event(tx, "rx.event.device-binding-proposed.v1", &plan)?;
            remember(tx, &scope, fp, PLAN, &plan)?;
            Ok(plan)
        })
    }
    pub fn prepare_device_binding_review(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: ReviewImpact,
    ) -> Result<Preflight> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let actor = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&input.cell),
                Role::Verifier,
                false,
            )?;
            let p = read(tx, &input.plan, &input.cell)?;
            process_change::access(&actor, &p.definition.impact)?;
            let (scope, fp) = request(
                meta,
                &actor,
                "DeviceBinding.ReviewImpact",
                key_.as_str(),
                &input,
            )?;
            if let Some(old) = prior::<Plan>(tx, &scope, fp, PLAN)? {
                return Ok(Preflight::Recorded(Box::new(old)));
            }
            if input.note.trim().is_empty() || input.note.chars().count() > 1000 {
                return reject(Reject::InvalidInput);
            }
            if actor.id == p.proposed_by {
                return reject(Reject::Forbidden);
            }
            if p.revision != input.expected
                || p.plan_digest != input.plan_digest
                || p.state != State::Proposed
                || !current(tx, &p)?
            {
                return reject(Reject::StaleRevision);
            }
            let (j, v, d) = approved(tx, meta, &input.cell, &p.definition.input.review)?;
            let before = process_change::read_config(tx, &p.definition.before)?;
            Ok(Preflight::Verify(Box::new(ticket(
                meta,
                now,
                (identity, key_),
                p.definition.input,
                before,
                (j, v, d),
                Action::Review(input),
            ))))
        })
    }
    pub fn commit_device_binding_review(&mut self, p: Prepared) -> Result<Plan> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let t = &p.ticket;
            let Action::Review(input) = &t.action else {
                return reject(Reject::InvalidInput);
            };
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let actor = authorize(
                tx,
                &t.identity,
                meta,
                &now,
                Some(&input.cell),
                Role::Verifier,
                false,
            )?;
            let mut old = read(tx, &input.plan, &input.cell)?;
            process_change::access(&actor, &old.definition.impact)?;
            let (scope, fp) = request(
                meta,
                &actor,
                "DeviceBinding.ReviewImpact",
                t.key.as_str(),
                input,
            )?;
            if let Some(result) = prior(tx, &scope, fp, PLAN)? {
                return Ok(result);
            }
            if actor.id == old.proposed_by {
                return reject(Reject::Forbidden);
            }
            if old.revision != input.expected
                || old.plan_digest != input.plan_digest
                || old.state != State::Proposed
                || !current(tx, &old)?
            {
                return reject(Reject::StaleRevision);
            }
            let impact = verify_prepared(tx, meta, &now, &p)?;
            process_change::access(&actor, &impact)?;
            if fingerprint(&impact)? != fingerprint(&old.definition.impact)?
                || fingerprint(&p.candidates)? != fingerprint(&old.definition.candidates)?
                || fingerprint(&p.issues)? != fingerprint(&old.definition.issues)?
            {
                return reject(Reject::StaleRevision);
            }
            old.revision = old.revision.increment().map_err(domain_error)?;
            old.state = State::ImpactReviewed;
            old.impact_review = Some(crate::process_change::ImpactDecision {
                actor: actor.id,
                note: input.note.clone(),
                at: now,
            });
            save(
                tx,
                "devicebindingplan",
                &old.id,
                Some(input.expected),
                PLAN,
                &old,
            )?;
            save(
                tx,
                "devicebindinghistory",
                (&old.id, old.revision),
                None,
                PLAN,
                &old,
            )?;
            event(tx, "rx.event.device-binding-impact-reviewed.v1", &old)?;
            remember(tx, &scope, fp, PLAN, &old)?;
            Ok(old)
        })
    }
    pub fn device_binding_plan(
        &mut self,
        identity: &Identity,
        cell: &Name,
        id: &Id,
    ) -> Result<Detail> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            process_draft::read_access(tx, identity, meta, &clock.now(), cell)?;
            let actor = authorize_identity(tx, identity, meta, &clock.now())?;
            let p = read(tx, id, cell)?;
            process_change::access(&actor, &p.definition.impact)?;
            let before = process_change::read_config(tx, &p.definition.before)?;
            let current = current(tx, &p)?;
            let approval = match approved(tx, meta, cell, &p.definition.input.review) {
                Ok(_) => true,
                Err(StoreError::Rejected(_) | StoreError::Unavailable(_)) => false,
                Err(e) => return Err(e),
            };
            Ok(Detail {
                plan: p,
                before,
                context_current: current,
                device_approval_current: approval,
                activation_authorized: false,
                configuration_changed: false,
                application_supported: false,
            })
        })
    }
    pub fn device_binding_plans(
        &mut self,
        identity: &Identity,
        cell: &Name,
        after: Option<&Id>,
    ) -> Result<Page> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            process_draft::read_access(tx, identity, meta, &clock.now(), cell)?;
            let actor = authorize_identity(tx, identity, meta, &clock.now())?;
            let mut plans = tx
                .scan("devicebindingplan/")?
                .iter()
                .map(|r| decode::<Plan>(r, PLAN))
                .collect::<Result<Vec<_>>>()?;
            plans.retain(|p| {
                &p.cell == cell
                    && after.is_none_or(|id| &p.id > id)
                    && p.definition
                        .impact
                        .cells
                        .iter()
                        .all(|c| actor.cells.contains(&c.id))
            });
            plans.sort_by(|a, b| a.id.cmp(&b.id));
            let more = plans.len() > 50;
            plans.truncate(50);
            let next = if more {
                plans.last().map(|p| p.id.clone())
            } else {
                None
            };
            let plans = plans
                .into_iter()
                .map(|p| Summary {
                    id: p.id,
                    revision: p.revision,
                    state: p.state,
                    plan_digest: p.plan_digest,
                    proposed_by: p.proposed_by,
                    proposed_at: p.proposed_at,
                    affected_cells: p
                        .definition
                        .impact
                        .cells
                        .iter()
                        .map(|c| c.id.clone())
                        .collect(),
                    issue_count: Counter(p.definition.issues.len() as u64),
                })
                .collect();
            Ok(Page {
                cell: cell.clone(),
                plans,
                next,
            })
        })
    }
}
