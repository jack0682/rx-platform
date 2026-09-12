use super::*;
use crate::{
    process_change::*,
    process_review::{Choice, Decision, Job, Version},
};
const CHANGE: &str = "rx.process-change.v1";
const CONFIG: &str = "rx.cell-configuration.v1";
fn fingerprint<T: Serialize>(v: &T) -> Result<Digest> {
    canonical::digest("RX-PROCESS-CHANGE-CONTEXT-v1", v).map_err(domain_error)
}
pub(super) fn config_ref(c: &CellConfiguration) -> Result<ArtifactRef> {
    let b = canonical::bytes(c).map_err(domain_error)?;
    Ok(ArtifactRef {
        sha256: rx_package::content_digest(&b),
        schema_id: name(CONFIG),
        size_bytes: Counter(b.len() as u64),
    })
}
pub(super) fn store_config(tx: &mut dyn Transaction, c: &CellConfiguration) -> Result<ArtifactRef> {
    let r = config_ref(c)?;
    let k = key("changeconfiguration", r.sha256);
    let d = doc(CONFIG, c)?;
    if let Some(old) = tx.get(&k)? {
        if old.document != d {
            return Err(StoreError::Integrity(
                "change configuration collision".into(),
            ));
        }
    } else {
        tx.put(&k, None, &d)?;
    }
    Ok(r)
}
pub(super) fn read_config(tx: &mut dyn Transaction, r: &ArtifactRef) -> Result<CellConfiguration> {
    let (_, c): (_, CellConfiguration) = load(tx, "changeconfiguration", r.sha256, CONFIG)?;
    if &config_ref(&c)? != r {
        return Err(StoreError::Integrity(
            "change configuration integrity differs".into(),
        ));
    }
    Ok(c)
}
fn impact(tx: &mut dyn Transaction, origin: &Name) -> Result<Impact> {
    prospective_impact(tx, origin, &BTreeSet::new(), &BTreeSet::new())
}
pub(super) fn prospective_impact(
    tx: &mut dyn Transaction,
    origin: &Name,
    extra_hosts: &BTreeSet<Name>,
    extra_resources: &BTreeSet<Name>,
) -> Result<Impact> {
    let cells = tx
        .scan("cell/")?
        .iter()
        .map(|r| decode::<Cell>(r, CELL))
        .collect::<Result<Vec<_>>>()?;
    if !cells.iter().any(|c| &c.configuration.id == origin) {
        return reject(Reject::NotFound);
    }
    let mut selected = BTreeSet::from([origin.clone()]);
    loop {
        let mut scopes = BTreeSet::new();
        let mut resources = extra_resources.clone();
        let mut hosts = extra_hosts.clone();
        for c in cells
            .iter()
            .filter(|c| selected.contains(&c.configuration.id))
        {
            scopes.extend(c.configuration.scopes.iter().cloned());
            hosts.extend(c.configuration.hosts.iter().cloned());
            for step in &c.configuration.steps {
                resources.extend(step.intent.resource_set.iter().cloned());
            }
        }
        let before = selected.len();
        for c in &cells {
            if c.configuration.scopes.iter().any(|s| scopes.contains(s))
                || c.configuration.hosts.iter().any(|h| hosts.contains(h))
                || c.configuration
                    .steps
                    .iter()
                    .any(|s| s.intent.resource_set.iter().any(|r| resources.contains(r)))
            {
                selected.insert(c.configuration.id.clone());
            }
        }
        if selected.len() > 64 {
            return reject(Reject::InvalidInput);
        }
        if selected.len() == before {
            break;
        }
    }
    let mut affected = Vec::new();
    for c in cells
        .iter()
        .filter(|c| selected.contains(&c.configuration.id))
    {
        let config = &c.configuration;
        affected.push(AffectedCell {
            id: config.id.clone(),
            configuration_digest: package_intake::configuration_digest(c)?,
            definition: config.definition.clone(),
            envelope: config.envelope.clone(),
            recipe: config.recipe.clone(),
            hosts: config
                .hosts
                .iter()
                .chain(extra_hosts.iter().filter(|_| &config.id == origin))
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            scopes: config
                .scopes
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            resources: config
                .steps
                .iter()
                .flat_map(|s| s.intent.resource_set.iter().cloned())
                .chain(
                    extra_resources
                        .iter()
                        .filter(|_| &config.id == origin)
                        .cloned(),
                )
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        });
    }
    affected.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Impact {
        cells: affected,
        scope_policy: name("WHOLE_CELL_AND_SHARED_HOST_RESOURCE_SCOPE_CLOSURE"),
        qualification_review_required: true,
        recovery_review_required: true,
        host_configuration_ack_required: true,
    })
}
pub(super) fn access(p: &Principal, impact: &Impact) -> Result<()> {
    if impact.cells.iter().any(|c| !p.cells.contains(&c.id)) {
        return reject(Reject::Forbidden);
    }
    Ok(())
}
pub(super) fn review(
    tx: &mut dyn Transaction,
    meta: &Installation,
    cell: &Name,
    r: &ReviewRef,
) -> Result<(Job, Version, Decision, rx_process_contract::ResolvedProcess)> {
    let job = process_review::load_job(tx, &r.id, cell)?;
    let (_, v) =
        process_review::latest(tx, &r.id)?.ok_or(StoreError::Rejected(Reject::NotFound))?;
    let (revision, d): (_, Decision) = load(
        tx,
        "processreviewdecision",
        &r.id,
        "rx.process-review-decision.v1",
    )?;
    if job.device_context.is_some()
        || !process_review::context_matches(tx, meta, &job)?
        || v.checker_digest != crate::process_review::checker_digest()
        || !v.ready_for_software_approval
        || v.revision != r.revision
        || v.review_digest != r.review_digest
        || revision != r.decision_revision
        || d.revision != revision
        || d.choice != Choice::Approve
        || d.report_revision != v.revision
        || d.review_digest != v.review_digest
        || d.scope.as_str() != "PROCESS_PACKAGE_SOFTWARE"
    {
        return reject(Reject::QualificationRequired);
    }
    let resolved = process_review::artifact(tx, "reviewresolved", &v.resolved)?
        .ok_or(StoreError::Rejected(Reject::NotFound))?;
    Ok((job, v, d, resolved))
}
fn plan_digest(v: &Change) -> Result<Digest> {
    canonical::digest(
        "RX-PROCESS-CHANGE-PLAN-v1",
        &(
            &v.id,
            &v.cell,
            &v.review,
            &v.before,
            &v.after,
            &v.step_origins,
            &v.impact,
            &v.reason,
            &v.proposed_by,
            v.builder_digest,
        ),
    )
    .map_err(domain_error)
}
pub(super) fn change(tx: &mut dyn Transaction, id: &Id, cell: &Name) -> Result<Change> {
    let (revision, c): (_, Change) = load(tx, "processchange", id, CHANGE)?;
    if c.cell != *cell {
        return reject(Reject::Forbidden);
    }
    let state_valid = match c.state {
        State::Proposed => {
            c.impact_review.is_none() && c.staged_by.is_none() && c.preparation.is_none()
        }
        State::ImpactReviewed => {
            c.impact_review.is_some() && c.staged_by.is_none() && c.preparation.is_none()
        }
        State::Staged => {
            c.impact_review.is_some() && c.staged_by.is_some() && c.application.is_none()
        }
        State::QualifiedActive => c.application.is_some() && c.qualification_activation.is_some(),
        State::AppliedUnqualified => {
            c.impact_review.is_some()
                && c.staged_by.is_some()
                && c.preparation.is_some()
                && c.application.is_some()
        }
    };
    if !state_valid || c.id != *id || c.revision != revision || c.plan_digest != plan_digest(&c)? {
        return Err(StoreError::Integrity(
            "process change identity differs".into(),
        ));
    }
    Ok(c)
}
pub(super) fn record(
    tx: &mut dyn Transaction,
    c: &Change,
    expected: Option<Counter>,
) -> Result<()> {
    let revision = save(tx, "processchange", &c.id, expected, CHANGE, c)?;
    if revision != c.revision {
        return Err(StoreError::Integrity("change revision differs".into()));
    }
    save(
        tx,
        "processchangehistory",
        (&c.id, c.revision),
        None,
        CHANGE,
        c,
    )?;
    event(tx, "rx.event.process-change-recorded.v1", c)
}
pub(super) fn current(tx: &mut dyn Transaction, meta: &Installation, c: &Change) -> Result<()> {
    if c.builder_digest != builder_digest()
        || fingerprint(&impact(tx, &c.cell)?)? != fingerprint(&c.impact)?
    {
        return reject(Reject::StaleRevision);
    }
    review(tx, meta, &c.cell, &c.review)?;
    Ok(())
}
pub(super) fn check_prepared(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    p: &Prepared,
) -> Result<()> {
    let t = &p.ticket;
    if t.boot != meta.runtime_boot
        || now
            .age_ns(&t.issued)
            .is_none_or(|age| age >= 30_000_000_000)
        || package_intake::current(tx, meta)?.as_ref() != Some(&t.registration)
        || p.verified.stored.owner() != &t.registration.store_owner
        || p.verified.stored.policy_fingerprint() != t.registration.policy_fingerprint
        || !p.verified.issues.is_empty()
        || !p.verified.report.issues.is_empty()
    {
        return reject(Reject::StaleRevision);
    }
    let r = ReviewRef {
        id: t.job.request.id.clone(),
        revision: t.version.revision,
        review_digest: t.version.review_digest,
        decision_revision: t.decision.revision,
    };
    review(tx, meta, &t.job.request.cell, &r)?;
    if fingerprint(&impact(tx, &t.job.request.cell)?)? != fingerprint(&t.impact)? {
        return reject(Reject::StaleRevision);
    }
    validate_configuration(&p.target)
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn prepare_process_change(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: Create,
    ) -> Result<Preflight> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let principal = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&input.cell),
                Role::Engineer,
                false,
            )?;
            let (scope, fp) = request(
                meta,
                &principal,
                "ProcessChange.Propose",
                key_.as_str(),
                &input,
            )?;
            let affected = impact(tx, &input.cell)?;
            access(&principal, &affected)?;
            if let Some(c) = prior::<Change>(tx, &scope, fp, CHANGE)? {
                access(&principal, &c.impact)?;
                return Ok(Preflight::Recorded(Box::new(c)));
            }
            if input.reason.trim().is_empty() || input.reason.chars().count() > 2000 {
                return reject(Reject::InvalidInput);
            }
            let (job, version, decision, resolved) = review(tx, meta, &input.cell, &input.review)?;
            if tx.get(&key("processchange", &input.id))?.is_some() {
                return reject(Reject::StaleRevision);
            }
            let registration = package_intake::current(tx, meta)?.ok_or(
                StoreError::Unavailable("package verifier unavailable".into()),
            )?;
            Ok(Preflight::Verify(Box::new(Ticket {
                action: Action::Propose(input),
                identity: identity.clone(),
                key: key_.clone(),
                job,
                version,
                decision,
                registration,
                impact: affected,
                resolved,
                issued: now,
                boot: meta.runtime_boot.clone(),
            })))
        })
    }
    pub fn commit_process_change(&mut self, p: Prepared) -> Result<Change> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let t = &p.ticket;
            let Action::Propose(input) = &t.action else {
                return reject(Reject::InvalidInput);
            };
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let principal = authorize(
                tx,
                &t.identity,
                meta,
                &now,
                Some(&input.cell),
                Role::Engineer,
                false,
            )?;
            access(&principal, &t.impact)?;
            let (scope, fp) = request(
                meta,
                &principal,
                "ProcessChange.Propose",
                t.key.as_str(),
                input,
            )?;
            if let Some(c) = prior::<Change>(tx, &scope, fp, CHANGE)? {
                access(&principal, &c.impact)?;
                return Ok(c);
            }
            check_prepared(tx, meta, &now, &p)?;
            let before = store_config(tx, &t.job.configuration)?;
            let after = store_config(tx, &p.target)?;
            let mut c = Change {
                qualification_activation: None,
                application: None,
                id: input.id.clone(),
                cell: input.cell.clone(),
                revision: Counter(1),
                state: State::Proposed,
                plan_digest: Digest::from_bytes([0; 32]),
                builder_digest: builder_digest(),
                review: input.review.clone(),
                before,
                after,
                step_origins: p.origins,
                impact: t.impact.clone(),
                reason: input.reason.clone(),
                proposed_by: principal.id,
                proposed_at: now,
                impact_review: None,
                staged_by: None,
                preparation: None,
            };
            c.plan_digest = plan_digest(&c)?;
            record(tx, &c, None)?;
            remember(tx, &scope, fp, CHANGE, &c)?;
            Ok(c)
        })
    }
    pub fn review_process_change_impact(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: ReviewImpact,
    ) -> Result<Change> {
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
                Some(&input.target.cell),
                Role::Verifier,
                false,
            )?;
            let mut c = change(tx, &input.target.change, &input.target.cell)?;
            access(&p, &c.impact)?;
            let (scope, fp) = request(
                meta,
                &p,
                "ProcessChange.ReviewImpact",
                key_.as_str(),
                &input,
            )?;
            if let Some(c) = prior(tx, &scope, fp, CHANGE)? {
                return Ok(c);
            }
            if p.id == c.proposed_by {
                return reject(Reject::Forbidden);
            }
            if input.note.trim().is_empty() || input.note.chars().count() > 2000 {
                return reject(Reject::InvalidInput);
            }
            if c.revision != input.target.expected
                || c.plan_digest != input.target.plan_digest
                || !matches!(c.state, State::Proposed | State::ImpactReviewed)
            {
                return reject(Reject::StaleRevision);
            }
            current(tx, meta, &c)?;
            let expected = c.revision;
            c.revision = c.revision.increment().map_err(domain_error)?;
            c.state = State::ImpactReviewed;
            c.impact_review = Some(ImpactDecision {
                actor: p.id,
                note: input.note,
                at: now,
            });
            record(tx, &c, Some(expected))?;
            remember(tx, &scope, fp, CHANGE, &c)?;
            Ok(c)
        })
    }
    pub fn prepare_process_change_stage(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: Transition,
    ) -> Result<Preflight> {
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
                false,
            )?;
            let c = change(tx, &input.change, &input.cell)?;
            access(&p, &c.impact)?;
            let (scope, fp) = request(meta, &p, "ProcessChange.Stage", key_.as_str(), &input)?;
            if let Some(c) = prior(tx, &scope, fp, CHANGE)? {
                return Ok(Preflight::Recorded(Box::new(c)));
            }
            if c.state != State::ImpactReviewed
                || c.revision != input.expected
                || c.plan_digest != input.plan_digest
            {
                return reject(Reject::StaleRevision);
            }
            current(tx, meta, &c)?;
            let (job, version, decision, resolved) = review(tx, meta, &input.cell, &c.review)?;
            let registration = package_intake::current(tx, meta)?.ok_or(
                StoreError::Unavailable("package verifier unavailable".into()),
            )?;
            Ok(Preflight::Verify(Box::new(Ticket {
                action: Action::Stage(input),
                identity: identity.clone(),
                key: key_.clone(),
                job,
                version,
                decision,
                registration,
                impact: c.impact,
                resolved,
                issued: now,
                boot: meta.runtime_boot.clone(),
            })))
        })
    }
    pub fn commit_process_change_stage(&mut self, p: Prepared) -> Result<Change> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let t = &p.ticket;
            let Action::Stage(input) = &t.action else {
                return reject(Reject::InvalidInput);
            };
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let principal = authorize(
                tx,
                &t.identity,
                meta,
                &now,
                Some(&input.cell),
                Role::ReleaseManager,
                false,
            )?;
            let mut c = change(tx, &input.change, &input.cell)?;
            access(&principal, &c.impact)?;
            let (scope, fp) = request(
                meta,
                &principal,
                "ProcessChange.Stage",
                t.key.as_str(),
                input,
            )?;
            if let Some(c) = prior(tx, &scope, fp, CHANGE)? {
                return Ok(c);
            }
            if c.state != State::ImpactReviewed
                || c.revision != input.expected
                || c.plan_digest != input.plan_digest
            {
                return reject(Reject::StaleRevision);
            }
            check_prepared(tx, meta, &now, &p)?;
            if config_ref(&p.target)? != c.after || p.origins != c.step_origins {
                return Err(StoreError::Integrity(
                    "staged target differs from reviewed plan".into(),
                ));
            }
            let expected = c.revision;
            c.revision = c.revision.increment().map_err(domain_error)?;
            c.state = State::Staged;
            c.staged_by = Some(principal.id);
            record(tx, &c, Some(expected))?;
            remember(tx, &scope, fp, CHANGE, &c)?;
            Ok(c)
        })
    }
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn begin_process_change_preparation(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: BeginPreparation,
    ) -> Result<Change> {
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
                Some(&input.target.cell),
                Role::ReleaseManager,
                true,
            )?;
            let mut c = change(tx, &input.target.change, &input.target.cell)?;
            access(&p, &c.impact)?;
            let (scope, fp) = request(
                meta,
                &p,
                "ProcessChange.BeginPreparation",
                key_.as_str(),
                &input,
            )?;
            if let Some(c) = prior(tx, &scope, fp, CHANGE)? {
                return Ok(c);
            }
            if c.state != State::Staged
                || c.revision != input.target.expected
                || c.plan_digest != input.target.plan_digest
                || (c.preparation.is_some() && !input.refresh)
                || (c.preparation.is_none() && input.refresh)
            {
                return reject(Reject::StaleRevision);
            }
            current(tx, meta, &c)?;
            let affected: BTreeSet<_> = c.impact.cells.iter().map(|v| v.id.clone()).collect();
            for row in tx.scan("processchange/")? {
                let other: Change = decode(&row, CHANGE)?;
                if other.id != c.id
                    && other.preparation.is_some()
                    && other.state != State::QualifiedActive
                    && other.impact.cells.iter().any(|v| affected.contains(&v.id))
                {
                    return reject(Reject::Busy);
                }
            }
            let mut cells = Vec::new();
            let mut fences = Vec::new();
            for target in &c.impact.cells {
                let (revision, mut cell): (_, Cell) = load(tx, "cell", &target.id, CELL)?;
                let old_blocks: BTreeSet<_> = cell.blocks.iter().map(|b| b.id.clone()).collect();
                let sent = invalidate_for_change(tx, &mut cell, revision)?;
                qualification_activation::record_blocks(tx, &c.id, &cell, &old_blocks)?;
                cells.push(BoundCell {
                    cell: target.id.clone(),
                    epoch: cell.epoch,
                    scopes: cell.scope_epochs.clone(),
                    change_blocks: cell
                        .blocks
                        .iter()
                        .filter(|b| !old_blocks.contains(&b.id))
                        .map(|b| b.id.clone())
                        .collect(),
                });
                for (host, message) in sent {
                    fences.push(FenceTarget {
                        cell: target.id.clone(),
                        host,
                        message,
                        epoch: cell.epoch,
                        scopes: cell.scope_epochs.clone(),
                    });
                }
            }
            let attempt = c.preparation.as_ref().map_or(Ok(Counter(1)), |p| {
                p.attempt.increment().map_err(domain_error)
            })?;
            let preparation = Preparation {
                attempt,
                runtime_boot: meta.runtime_boot.clone(),
                cells,
                fences,
                started_by: p.id,
                started_at: now,
            };
            save(
                tx,
                "change_preparation_history",
                (&c.id, attempt),
                None,
                "rx.process-change-preparation.v1",
                &preparation,
            )?;
            let expected = c.revision;
            c.revision = c.revision.increment().map_err(domain_error)?;
            c.preparation = Some(preparation);
            record(tx, &c, Some(expected))?;
            remember(tx, &scope, fp, CHANGE, &c)?;
            Ok(c)
        })
    }
    pub fn process_change(&mut self, identity: &Identity, cell: &Name, id: &Id) -> Result<Detail> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = authorize_identity(tx, identity, meta, &clock.now())?;
            if !p
                .roles
                .iter()
                .any(|r| matches!(r, Role::Engineer | Role::Verifier | Role::ReleaseManager))
            {
                return reject(Reject::Forbidden);
            }
            let c = change(tx, id, cell)?;
            access(&p, &c.impact)?;
            let before = read_config(tx, &c.before)?;
            let after = read_config(tx, &c.after)?;
            let mut blockers = Vec::new();
            let mut count = 0u64;
            let mut add = |b| {
                count += 1;
                if blockers.len() < 256 {
                    blockers.push(b);
                }
            };
            if matches!(c.state, State::AppliedUnqualified | State::QualifiedActive) {
                return process_apply::detail(tx, meta, c, before, after);
            }
            if fingerprint(&impact(tx, cell)?)? != fingerprint(&c.impact)?
                || c.builder_digest != builder_digest()
            {
                add(Blocker::ContextChanged);
            }
            match review(tx, meta, cell, &c.review) {
                Ok(_) => {}
                Err(StoreError::Rejected(_) | StoreError::Unavailable(_)) => {
                    add(Blocker::ReviewNoLongerApproved)
                }
                Err(e) => return Err(e),
            }
            let ids: BTreeSet<_> = c.impact.cells.iter().map(|c| c.id.clone()).collect();
            if let Some(prep) = &c.preparation {
                let mut stale = prep.runtime_boot != meta.runtime_boot;
                for bound in &prep.cells {
                    let (_, live): (_, Cell) = load(tx, "cell", &bound.cell, CELL)?;
                    stale |= live.epoch != bound.epoch
                        || live.scope_epochs != bound.scopes
                        || bound
                            .change_blocks
                            .iter()
                            .any(|b| !live.blocks.iter().any(|v| &v.id == b && v.latched));
                }
                if stale {
                    add(Blocker::PreparationStale);
                }
                let acks = tx.scan("fenceack/")?;
                for fence in &prep.fences {
                    let host = tx
                        .get(&key("host", (&fence.cell, &fence.host)))?
                        .map(|r| decode::<HostRegistration>(&r, HOST))
                        .transpose()?;
                    let mut found = false;
                    if let Some(host) = host {
                        for row in &acks {
                            let a: FenceAcknowledgment = decode(row, "rx.internal.fence-ack.v1")?;
                            if a.invalidation == fence.message
                                && a.cell == fence.cell
                                && a.epoch == fence.epoch
                                && a.scopes == fence.scopes
                                && a.host_boot == host.boot_id
                                && a.journal == host.delivery_journal
                                && row.key == key("fenceack", (&fence.host, &a.journal, a.sequence))
                            {
                                found = true;
                                break;
                            }
                        }
                    }
                    if !found {
                        add(Blocker::HostFenceUnconfirmed {
                            cell: fence.cell.clone(),
                            host: fence.host.clone(),
                        });
                    }
                }
            } else {
                add(Blocker::PreparationRequired);
            }
            for row in tx.scan("run/")? {
                let r: Run = decode(&row, RUN)?;
                if ids.contains(&r.cell)
                    && !matches!(r.state, RunState::Completed | RunState::Abandoned)
                {
                    add(Blocker::RunNeedsDisposition { run: r.id });
                }
            }
            for row in tx.scan("work/")? {
                let w: Work = decode(&row, WORK)?;
                if ids.contains(&w.cell)
                    && (matches!(w.operation.outcome(), Outcome::None | Outcome::Unresolved)
                        || w.operation.integrity() == rx_domain::operation::Integrity::Disputed)
                {
                    add(Blocker::OperationUnresolved {
                        operation: w.operation.id().clone(),
                    });
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
                        add(Blocker::ResourceRetained {
                            resource,
                            holder: r.holder,
                        });
                    }
                }
            }
            for row in tx.scan("case/")? {
                let case: crate::intervention::Case =
                    decode(&row, "rx.internal.intervention-case.v1")?;
                if case.state != crate::intervention::CaseState::Closed
                    && (ids.contains(&case.cell)
                        || case.effective_cells.iter().any(|v| ids.contains(v)))
                {
                    add(Blocker::OpenCase { case: case.id });
                }
            }
            let host_configuration = configuration_dispatch::summary(tx, meta, &c)?;
            if host_configuration.mixed_configuration {
                add(Blocker::MixedHostConfiguration);
            }
            if host_configuration.outcome_unknown {
                add(Blocker::HostConfigurationOutcomeUnknown);
            }
            for target in &c.impact.cells {
                for host in &target.hosts {
                    if !host_configuration
                        .hosts
                        .iter()
                        .any(|h| &h.host == host && h.acknowledged_for_preparation)
                    {
                        add(Blocker::HostConfigurationAcknowledgementRequired {
                            cell: target.id.clone(),
                            host: host.clone(),
                        });
                    }
                }
            }
            Ok(Detail {
                host_configuration,
                change: c,
                before,
                after,
                blockers,
                blocker_count: Counter(count),
                blockers_truncated: count > 256,
                applied: false,
                activation_authorized: false,
            })
        })
    }
}
