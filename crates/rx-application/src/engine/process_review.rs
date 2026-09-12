use super::*;
use crate::process_review::*;
use rx_process_contract::{
    model::{ProcessSource, ResolvedProcess},
    package_review::Request,
};
const JOB: &str = "rx.internal.process-review-job.v1";
const VERSION: &str = "rx.process-review-version.v1";
const DECISION: &str = "rx.process-review-decision.v1";
const AUTHORITY: &str = "rx.internal.process-review-authority.v1";
#[derive(Serialize, serde::Deserialize)]
struct AuthorityState {
    boot: Id,
    digest: Option<Digest>,
}
fn access(
    tx: &mut dyn Transaction,
    identity: &Identity,
    meta: &Installation,
    now: &TimePoint,
    cell: &Name,
) -> Result<Principal> {
    let p = authorize_identity(tx, identity, meta, now)?;
    if !p.cells.contains(cell)
        || !(p.roles.contains(&Role::Engineer) || p.roles.contains(&Role::Verifier))
    {
        return reject(Reject::Forbidden);
    }
    Ok(p)
}
pub(super) fn authority(tx: &mut dyn Transaction, meta: &Installation) -> Result<Option<Digest>> {
    let Some(row) = tx.get(&name("processreview/authority"))? else {
        return Ok(None);
    };
    let a: AuthorityState = decode(&row, AUTHORITY)?;
    Ok(if a.boot == meta.runtime_boot {
        a.digest
    } else {
        None
    })
}
fn device_scope(actor: &Principal, job: &Job) -> Result<()> {
    if job.device_context.as_ref().is_some_and(|c| {
        c.required_cells()
            .iter()
            .any(|id| !actor.cells.contains(id))
    }) {
        return reject(Reject::Forbidden);
    }
    Ok(())
}
fn device_context_current(
    tx: &mut dyn Transaction,
    meta: &Installation,
    job: &Job,
) -> Result<bool> {
    crate::process_review::device_configuration(job, None).map_err(StoreError::Integrity)?;
    if let Some(context) = &job.device_context {
        for d in &context.dependencies {
            let plan = device_binding::read(tx, &d.plan.id, &job.request.cell)?;
            if canonical::bytes(&plan).map_err(domain_error)?
                != canonical::bytes(&d.plan).map_err(domain_error)?
                || !device_binding::current(tx, &plan)?
            {
                return Ok(false);
            }
            let actual = match device_binding::approved(
                tx,
                meta,
                &job.request.cell,
                &plan.definition.input.review,
            ) {
                Ok(value) => value,
                Err(StoreError::Rejected(_)) => return Ok(false),
                Err(e) => return Err(e),
            };
            if canonical::bytes(&actual).map_err(domain_error)?
                != canonical::bytes(&(&d.job, &d.version, &d.decision)).map_err(domain_error)?
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
pub(super) fn context_matches(
    tx: &mut dyn Transaction,
    meta: &Installation,
    job: &Job,
) -> Result<bool> {
    let (_, cell): (_, Cell) = load(tx, "cell", &job.request.cell, CELL)?;
    Ok(device_context_current(tx, meta, job)?
        && package_intake::configuration_digest(&cell)? == job.request.configuration_digest
        && canonical::digest("RX-PACKAGE-INTAKE-CELL-CONTEXT-v1", &job.configuration)
            .map_err(domain_error)?
            == job.request.configuration_digest
        && authority(tx, meta)? == Some(job.request.verification_authority_digest)
        && package_intake::current(tx, meta)?.is_some_and(|p| {
            p.policy_fingerprint == job.request.package_policy_fingerprint
                && p.policy_file_digest == job.request.package_policy_file_digest
        }))
}
pub(super) fn load_job(tx: &mut dyn Transaction, id: &Id, cell: &Name) -> Result<Job> {
    let (_, job): (_, Job) = load(tx, "processreviewjob", id, JOB)?;
    if &job.request.cell != cell {
        return reject(Reject::Forbidden);
    }
    if &job.request.id != id || &job.configuration.id != cell || job.request.validate().is_err() {
        return Err(StoreError::Integrity("review job identity differs".into()));
    }
    crate::process_review::device_configuration(&job, None).map_err(StoreError::Integrity)?;
    Ok(job)
}
fn persist_artifact<T: Serialize>(
    tx: &mut dyn Transaction,
    kind: &str,
    schema: &str,
    value: &T,
) -> Result<ArtifactRef> {
    let bytes = canonical::bytes(value).map_err(domain_error)?;
    let reference = ArtifactRef {
        sha256: rx_package::content_digest(&bytes),
        schema_id: name(schema),
        size_bytes: Counter(bytes.len() as u64),
    };
    let k = key(kind, reference.sha256);
    let document = doc(schema, value)?;
    if let Some(old) = tx.get(&k)? {
        if old.document != document {
            return Err(StoreError::Integrity("review artifact differs".into()));
        }
    } else {
        tx.put(&k, None, &document)?;
    }
    Ok(reference)
}
pub(super) fn artifact<T: Serialize + DeserializeOwned>(
    tx: &mut dyn Transaction,
    kind: &str,
    reference: &Option<ArtifactRef>,
) -> Result<Option<T>> {
    let Some(reference) = reference else {
        return Ok(None);
    };
    let (_, value): (_, T) = load(tx, kind, reference.sha256, reference.schema_id.as_str())?;
    let bytes = canonical::bytes(&value).map_err(domain_error)?;
    if rx_package::content_digest(&bytes) != reference.sha256
        || bytes.len() as u64 != reference.size_bytes.0
    {
        return Err(StoreError::Integrity(
            "review artifact integrity differs".into(),
        ));
    }
    Ok(Some(value))
}
fn version_digest(v: &Version) -> Result<Digest> {
    canonical::digest(
        "RX-PROCESS-REVIEW-VERSION-v1",
        &(
            &v.review,
            &v.cell,
            v.revision,
            v.report_digest,
            &v.signature,
            v.checker_digest,
            &v.platform_issues,
            &v.source,
            &v.resolved,
            v.ready_for_software_approval,
        ),
    )
    .map_err(domain_error)
}
pub(super) fn latest(tx: &mut dyn Transaction, id: &Id) -> Result<Option<(Counter, Version)>> {
    tx.get(&key("processreviewversion", id))?
        .map(|r| {
            let v: Version = decode(&r, VERSION)?;
            if version_digest(&v)? != v.review_digest
                || r.revision != v.revision
                || &v.review != id
                || v.report.digest().map_err(StoreError::Integrity)? != v.report_digest
                || v.report.request.id != v.review
                || v.report.request.cell != v.cell
                || v.report.resolved != v.resolved
            {
                return Err(StoreError::Integrity(
                    "review version identity differs".into(),
                ));
            }
            Ok((r.revision, v))
        })
        .transpose()
}
fn check_ticket(meta: &Installation, now: &TimePoint, boot: &Id, issued: &TimePoint) -> Result<()> {
    if &meta.runtime_boot != boot || now.age_ns(issued).is_none_or(|age| age >= 30_000_000_000) {
        return reject(Reject::Expired);
    }
    Ok(())
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Trusted composition, not a user-supplied approval policy endpoint.
    pub fn configure_process_review(&mut self, digest: Option<Digest>) -> Result<()> {
        let meta = &self.installation;
        self.repository.transact(|tx| {
            lifecycle::require_serving(tx)?;
            let k = name("processreview/authority");
            let old = tx.get(&k)?;
            let a = AuthorityState {
                boot: meta.runtime_boot.clone(),
                digest,
            };
            tx.put(&k, old.map(|r| r.revision), &doc(AUTHORITY, &a)?)?;
            event(tx, "rx.event.process-review-authority-configured.v1", &a)
        })
    }
    pub fn create_process_review(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: Create,
    ) -> Result<Job> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let p = access(tx, identity, meta, &now, &input.cell)?;
            let (scope, fp) = request(meta, &p, "ProcessReview.Create", key_.as_str(), &input)?;
            if let Some(old) = prior::<Job>(tx, &scope, fp, JOB)? {
                device_scope(&p, &old)?;
                return Ok(old);
            }
            let (_, receipt): (_, crate::package_intake::Receipt) = load(
                tx,
                "packageintakereceipt",
                &input.intake,
                "rx.package-intake-receipt.v1",
            )?;
            if receipt.cell != input.cell {
                return reject(Reject::Forbidden);
            }
            if receipt.manifest.entry.kind() != rx_package::PackageKind::Process {
                return reject(Reject::UnsupportedSchema);
            }
            let (_, cell): (_, Cell) = load(tx, "cell", &input.cell, CELL)?;
            let registration = package_intake::current(tx, meta)?.ok_or(
                StoreError::Unavailable("package intake service missing".into()),
            )?;
            if registration.generation != input.policy_generation
                || package_intake::configuration_digest(&cell)? != input.configuration_digest
            {
                return reject(Reject::StaleRevision);
            }
            let authority = authority(tx, meta)?.ok_or(StoreError::Unavailable(
                "process review authority not configured".into(),
            ))?;
            let selected = draft_bindings::selected_catalog(
                tx,
                meta,
                &cell.configuration,
                &p,
                &input.device_plans,
            )?;
            if input.binding_selections.len() > 128
                || input
                    .binding_selections
                    .values()
                    .any(|id| !selected.steps.contains_key(id))
            {
                return reject(Reject::InvalidInput);
            }
            let device_context = if selected.view.device_plans.is_empty() {
                None
            } else {
                let mut dependencies = vec![];
                for reference in &selected.view.device_plans {
                    let plan = device_binding::read(tx, &reference.id, &input.cell)?;
                    let (job, version, decision) = device_binding::approved(
                        tx,
                        meta,
                        &input.cell,
                        &plan.definition.input.review,
                    )?;
                    dependencies.push(DeviceDependency {
                        plan,
                        job,
                        version,
                        decision,
                    });
                }
                Some(DeviceContext {
                    catalog_digest: selected.view.catalog_digest,
                    dependencies,
                })
            };
            let device_context_digest = device_context
                .as_ref()
                .map(DeviceContext::digest)
                .transpose()
                .map_err(StoreError::Invalid)?;
            let request_ = Request {
                device_context_digest,
                schema: name(if device_context.is_some() {
                    "rx.process-review-request.v2"
                } else {
                    "rx.process-review-request.v1"
                }),
                id: input.id.clone(),
                intake: input.intake,
                cell: input.cell,
                package_manifest: receipt.object.manifest,
                package_signature: receipt.object.signature,
                configuration_digest: input.configuration_digest,
                package_policy_fingerprint: registration.policy_fingerprint,
                package_policy_file_digest: registration.policy_file_digest,
                verification_authority_digest: authority,
                binding_selections: input.binding_selections,
            };
            request_.validate().map_err(StoreError::Invalid)?;
            let job = Job {
                device_context,
                request: request_,
                configuration: cell.configuration,
                submitted_by: receipt.submitted_by,
                requested_by: p.id,
                created_at: now,
            };
            crate::process_review::device_configuration(&job, None).map_err(StoreError::Invalid)?;
            save(tx, "processreviewjob", &input.id, None, JOB, &job)?;
            event(tx, "rx.event.process-review-created.v1", &job)?;
            remember(tx, &scope, fp, JOB, &job)?;
            Ok(job)
        })
    }
    pub fn prepare_review_report(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: Submit,
    ) -> Result<Preflight> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let p = access(tx, identity, meta, &now, &input.cell)?;
            let job = load_job(tx, &input.review, &input.cell)?;
            device_scope(&p, &job)?;
            let (scope, fp) = request(meta, &p, "ProcessReview.Report", key_.as_str(), &input)?;
            if let Some(old) = prior(tx, &scope, fp, VERSION)? {
                return Ok(Preflight::Recorded(Box::new(old)));
            }
            if !context_matches(tx, meta, &job)?
                || latest(tx, &input.review)?.map(|v| v.0) != input.expected
            {
                return reject(Reject::StaleRevision);
            }
            let registration = package_intake::current(tx, meta)?
                .ok_or(StoreError::Unavailable("package service missing".into()))?;
            Ok(Preflight::Verify(Box::new(Ticket {
                job,
                identity: identity.clone(),
                key: key_.clone(),
                input,
                boot: meta.runtime_boot.clone(),
                registration,
                issued: now,
            })))
        })
    }
    pub fn commit_review_report(&mut self, prepared: Prepared) -> Result<Version> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let t = &prepared.ticket;
            let v = prepared.validated;
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let p = access(tx, &t.identity, meta, &now, &t.input.cell)?;
            device_scope(&p, &t.job)?;
            let (scope, fp) = request(meta, &p, "ProcessReview.Report", t.key.as_str(), &t.input)?;
            if let Some(old) = prior(tx, &scope, fp, VERSION)? {
                return Ok(old);
            }
            check_ticket(meta, &now, &t.boot, &t.issued)?;
            if !context_matches(tx, meta, &t.job)?
                || package_intake::current(tx, meta)?.as_ref() != Some(&t.registration)
                || v.stored.owner() != &t.registration.store_owner
                || v.stored.policy_fingerprint() != t.registration.policy_fingerprint
                || v.report.request.digest().map_err(StoreError::Invalid)?
                    != t.job.request.digest().map_err(StoreError::Invalid)?
            {
                return reject(Reject::StaleRevision);
            }
            let old = latest(tx, &t.input.review)?;
            if old.as_ref().map(|r| r.0) != t.input.expected {
                return reject(Reject::StaleRevision);
            }
            let revision = old.map_or(Ok(Counter(1)), |r| r.0.increment().map_err(domain_error))?;
            let source = v
                .source
                .as_ref()
                .map(|s| persist_artifact(tx, "reviewsource", "rx.process-source.v1", s))
                .transpose()?;
            let resolved = v
                .resolved
                .as_ref()
                .map(|s| persist_artifact(tx, "reviewresolved", "rx.resolved-process.v1", s))
                .transpose()?;
            if resolved != v.report.resolved {
                return Err(StoreError::Integrity(
                    "resolved review artifact differs from signed report".into(),
                ));
            }
            let ready = v.issues.is_empty()
                && v.report.issues.is_empty()
                && source.is_some()
                && resolved.is_some();
            let mut version = Version {
                review_digest: Digest::from_bytes([0; 32]),
                checker_digest: checker_digest(),
                review: t.input.review.clone(),
                cell: t.input.cell.clone(),
                revision,
                report_digest: v.report.digest().map_err(StoreError::Invalid)?,
                report: v.report,
                signature: v.signature,
                platform_issues: v.issues,
                source,
                resolved,
                ready_for_software_approval: ready,
                recorded_by: p.id,
                recorded_at: now,
            };
            version.review_digest = version_digest(&version)?;
            save(
                tx,
                "processreviewversion",
                &version.review,
                t.input.expected,
                VERSION,
                &version,
            )?;
            save(
                tx,
                "processreviewhistory",
                (&version.review, revision),
                None,
                VERSION,
                &version,
            )?;
            event(tx, "rx.event.process-review-report-recorded.v1", &version)?;
            remember(tx, &scope, fp, VERSION, &version)?;
            Ok(version)
        })
    }
    pub fn process_review(&mut self, identity: &Identity, cell: &Name, id: &Id) -> Result<Detail> {
        self.process_review_revision(identity, cell, id, None)
    }
    pub fn process_review_revision(
        &mut self,
        identity: &Identity,
        cell: &Name,
        id: &Id,
        revision: Option<Counter>,
    ) -> Result<Detail> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let actor = access(tx, identity, meta, &clock.now(), cell)?;
            let job = load_job(tx, id, cell)?;
            device_scope(&actor, &job)?;
            let current = latest(tx, id)?.map(|v| v.1);
            let latest_report_revision = current.as_ref().map(|v| v.revision);
            let is_latest = revision.is_none() || revision == latest_report_revision;
            let verification = if let Some(revision) = revision {
                let (_, v): (_, Version) =
                    load(tx, "processreviewhistory", (id, revision), VERSION)?;
                if v.revision != revision
                    || &v.review != id
                    || v.cell != *cell
                    || version_digest(&v)? != v.review_digest
                    || v.report.digest().map_err(StoreError::Integrity)? != v.report_digest
                {
                    return Err(StoreError::Integrity(
                        "historical review identity differs".into(),
                    ));
                }
                Some(v)
            } else {
                current
            };
            let decision = tx
                .get(&key("processreviewdecision", id))?
                .map(|r| decode::<Decision>(&r, DECISION))
                .transpose()?;
            let context_current = context_matches(tx, meta, &job)?
                && verification
                    .as_ref()
                    .is_none_or(|v| v.checker_digest == checker_digest());
            let source = verification
                .as_ref()
                .map(|v| artifact::<ProcessSource>(tx, "reviewsource", &v.source))
                .transpose()?
                .flatten();
            let resolved = verification
                .as_ref()
                .map(|v| artifact::<ResolvedProcess>(tx, "reviewresolved", &v.resolved))
                .transpose()?
                .flatten();
            let approval_matches_current_review = is_latest
                && context_current
                && verification
                    .as_ref()
                    .zip(decision.as_ref())
                    .is_some_and(|(v, d)| {
                        v.ready_for_software_approval
                            && d.choice == Choice::Approve
                            && d.report_revision == v.revision
                            && d.review_digest == v.review_digest
                    });
            Ok(Detail {
                latest_report_revision,
                is_latest,
                job,
                verification,
                decision,
                source,
                resolved,
                context_current,
                approval_matches_current_review,
                activation_authorized: false,
            })
        })
    }
    pub fn prepare_review_decision(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: Decide,
    ) -> Result<DecisionPreflight> {
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
                Role::Verifier,
                false,
            )?;
            if input.note.trim().is_empty() || input.note.chars().count() > 1000 {
                return reject(Reject::InvalidInput);
            }
            let job = load_job(tx, &input.review, &input.cell)?;
            device_scope(&p, &job)?;
            let (scope, fp) = request(meta, &p, "ProcessReview.Decide", key_.as_str(), &input)?;
            if let Some(old) = prior(tx, &scope, fp, DECISION)? {
                return Ok(DecisionPreflight::Recorded(Box::new(old)));
            }
            let (_, version) =
                latest(tx, &input.review)?.ok_or(StoreError::Rejected(Reject::NotFound))?;
            if version.revision != input.report_revision
                || version.review_digest != input.review_digest
                || tx
                    .get(&key("processreviewdecision", &input.review))?
                    .map(|r| r.revision)
                    != input.expected
            {
                return reject(Reject::StaleRevision);
            }
            if input.choice == Choice::Approve {
                if p.id == job.submitted_by {
                    return reject(Reject::Forbidden);
                }
                if !context_matches(tx, meta, &job)?
                    || version.checker_digest != checker_digest()
                    || !version.ready_for_software_approval
                {
                    return reject(Reject::QualificationRequired);
                }
            }
            let source = artifact(tx, "reviewsource", &version.source)?;
            let resolved = artifact(tx, "reviewresolved", &version.resolved)?;
            Ok(DecisionPreflight::Verify(Box::new(DecisionTicket {
                job,
                version,
                source,
                resolved,
                identity: identity.clone(),
                key: key_.clone(),
                input,
                boot: meta.runtime_boot.clone(),
                registration: package_intake::current(tx, meta)?,
                issued: now,
            })))
        })
    }
    pub fn commit_review_decision(&mut self, prepared: PreparedDecision) -> Result<Decision> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let t = prepared.ticket;
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let p = authorize(
                tx,
                &t.identity,
                meta,
                &now,
                Some(&t.input.cell),
                Role::Verifier,
                false,
            )?;
            device_scope(&p, &t.job)?;
            let (scope, fp) = request(meta, &p, "ProcessReview.Decide", t.key.as_str(), &t.input)?;
            if let Some(old) = prior(tx, &scope, fp, DECISION)? {
                return Ok(old);
            }
            check_ticket(meta, &now, &t.boot, &t.issued)?;
            let (_, latest) =
                latest(tx, &t.input.review)?.ok_or(StoreError::Rejected(Reject::NotFound))?;
            if latest.revision != t.input.report_revision
                || latest.review_digest != t.input.review_digest
            {
                return reject(Reject::StaleRevision);
            }
            if t.input.choice == Choice::Approve {
                let v = prepared
                    .validated
                    .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
                if p.id == t.job.submitted_by {
                    return reject(Reject::Forbidden);
                }
                if !context_matches(tx, meta, &t.job)?
                    || latest.checker_digest != checker_digest()
                    || !latest.ready_for_software_approval
                    || package_intake::current(tx, meta)? != t.registration
                    || t.registration.as_ref().is_none_or(|r| {
                        v.stored.owner() != &r.store_owner
                            || v.stored.policy_fingerprint() != r.policy_fingerprint
                    })
                    || v.report.digest().map_err(StoreError::Invalid)? != latest.report_digest
                    || !v.issues.is_empty()
                    || !v.report.issues.is_empty()
                    || v.source.is_none()
                    || v.resolved.is_none()
                {
                    return reject(Reject::QualificationRequired);
                }
            }
            let old = tx.get(&key("processreviewdecision", &t.input.review))?;
            if old.as_ref().map(|r| r.revision) != t.input.expected {
                return reject(Reject::StaleRevision);
            }
            let revision = old.map_or(Ok(Counter(1)), |r| {
                r.revision.increment().map_err(domain_error)
            })?;
            let decision = Decision {
                review_digest: t.input.review_digest,
                review: t.input.review,
                cell: t.input.cell,
                revision,
                report_revision: t.input.report_revision,
                report_digest: latest.report_digest,
                choice: t.input.choice,
                note: t.input.note,
                decided_by: p.id,
                decided_at: now,
                scope: name("PROCESS_PACKAGE_SOFTWARE"),
            };
            save(
                tx,
                "processreviewdecision",
                &decision.review,
                t.input.expected,
                DECISION,
                &decision,
            )?;
            save(
                tx,
                "processreviewdecisionhistory",
                (&decision.review, revision),
                None,
                DECISION,
                &decision,
            )?;
            event(tx, "rx.event.process-review-decided.v1", &decision)?;
            remember(tx, &scope, fp, DECISION, &decision)?;
            Ok(decision)
        })
    }
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn process_reviews(
        &mut self,
        identity: &Identity,
        cell: &Name,
        intake: &Id,
        after: Option<&Id>,
    ) -> Result<Page> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let actor = access(tx, identity, meta, &clock.now(), cell)?;
            let (_, receipt): (_, crate::package_intake::Receipt) = load(
                tx,
                "packageintakereceipt",
                intake,
                "rx.package-intake-receipt.v1",
            )?;
            if &receipt.cell != cell {
                return reject(Reject::Forbidden);
            }
            let mut jobs = tx
                .scan("processreviewjob/")?
                .iter()
                .map(|r| decode::<Job>(r, JOB))
                .collect::<Result<Vec<_>>>()?;
            jobs.retain(|j| {
                device_scope(&actor, j).is_ok()
                    && &j.request.cell == cell
                    && &j.request.intake == intake
                    && after.is_none_or(|id| &j.request.id > id)
            });
            jobs.sort_by(|a, b| a.request.id.cmp(&b.request.id));
            let more = jobs.len() > 50;
            jobs.truncate(50);
            let next = if more {
                jobs.last().map(|j| j.request.id.clone())
            } else {
                None
            };
            let mut reviews = Vec::new();
            for j in jobs {
                let v = latest(tx, &j.request.id)?.map(|v| v.1);
                let d = tx
                    .get(&key("processreviewdecision", &j.request.id))?
                    .map(|r| decode::<Decision>(&r, DECISION))
                    .transpose()?;
                reviews.push(Summary {
                    id: j.request.id,
                    intake: j.request.intake,
                    requested_by: j.requested_by,
                    created_at: j.created_at,
                    report_revision: v.as_ref().map(|v| v.revision),
                    ready_for_software_approval: v.is_some_and(|v| v.ready_for_software_approval),
                    decision: d,
                });
            }
            Ok(Page {
                cell: cell.clone(),
                intake: intake.clone(),
                reviews,
                next,
            })
        })
    }
}
