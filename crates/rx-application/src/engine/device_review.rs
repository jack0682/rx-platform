use super::*;
use crate::device_review::*;
const JOB: &str = "rx.internal.device-review-job.v1";
const VERSION: &str = "rx.device-review-version.v1";
const DECISION: &str = "rx.device-review-decision.v1";
#[derive(Serialize, serde::Deserialize)]
struct AuthorityState {
    boot: Id,
    digest: Option<Digest>,
}
fn access(
    tx: &mut dyn Transaction,
    who: &Identity,
    meta: &Installation,
    now: &TimePoint,
    cell: &Name,
) -> Result<Principal> {
    let p = authorize_identity(tx, who, meta, now)?;
    if !p.cells.contains(cell)
        || !(p.roles.contains(&Role::Engineer) || p.roles.contains(&Role::Verifier))
    {
        return reject(Reject::Forbidden);
    }
    Ok(p)
}
pub(super) fn authority(tx: &mut dyn Transaction, meta: &Installation) -> Result<Option<Digest>> {
    let Some(row) = tx.get(&name("devicereview/authority"))? else {
        return Ok(None);
    };
    let a: AuthorityState = decode(&row, "rx.internal.device-review-authority.v1")?;
    Ok(if a.boot == meta.runtime_boot {
        a.digest
    } else {
        None
    })
}
pub(super) fn context(tx: &mut dyn Transaction, meta: &Installation, job: &Job) -> Result<bool> {
    let (_, cell): (_, Cell) = load(tx, "cell", &job.request.cell, CELL)?;
    Ok(job.request.installation == meta.id
        && package_intake::configuration_digest(&cell)? == job.request.configuration_digest
        && package_intake::current(tx, meta)?.as_ref() == Some(&job.registration)
        && authority(tx, meta)? == Some(job.request.verification_authority_digest))
}
pub(super) fn job(tx: &mut dyn Transaction, id: &Id, cell: &Name) -> Result<Job> {
    let (_, j): (_, Job) = load(tx, "devicereviewjob", id, JOB)?;
    if &j.request.cell != cell {
        return reject(Reject::Forbidden);
    }
    if &j.request.id != id {
        return Err(StoreError::Integrity("device review job identity".into()));
    }
    j.request.validate().map_err(StoreError::Integrity)?;
    Ok(j)
}
pub(super) fn latest(tx: &mut dyn Transaction, id: &Id) -> Result<Option<Version>> {
    let Some(row) = tx.get(&key("devicereviewversion", id))? else {
        return Ok(None);
    };
    let v: Version = decode(&row, VERSION)?;
    if v.revision != row.revision
        || &v.review != id
        || v.digest().map_err(StoreError::Integrity)? != v.review_digest
        || v.report.digest().map_err(StoreError::Integrity)? != v.report_digest
        || v.ready_for_software_approval != v.report.passed()
    {
        return Err(StoreError::Integrity(
            "device review version differs".into(),
        ));
    }
    Ok(Some(v))
}
fn check_ticket(meta: &Installation, now: &TimePoint, boot: &Id, issued: &TimePoint) -> Result<()> {
    if &meta.runtime_boot != boot || now.age_ns(issued).is_none_or(|v| v >= 30_000_000_000) {
        return reject(Reject::Expired);
    }
    Ok(())
}
fn read_detail(
    tx: &mut dyn Transaction,
    meta: &Installation,
    j: Job,
    revision: Option<Counter>,
) -> Result<Detail> {
    let current = latest(tx, &j.request.id)?;
    let latest_revision = current.as_ref().map(|v| v.revision);
    let selected = if let Some(r) = revision {
        let (_, v): (_, Version) = load(tx, "devicereviewhistory", (&j.request.id, r), VERSION)?;
        if v.review != j.request.id
            || v.cell != j.request.cell
            || v.revision != r
            || v.digest().map_err(StoreError::Integrity)? != v.review_digest
        {
            return Err(StoreError::Integrity("device history differs".into()));
        }
        Some(v)
    } else {
        current
    };
    let decision = tx
        .get(&key("devicereviewdecision", &j.request.id))?
        .map(|r| decode::<Decision>(&r, DECISION))
        .transpose()?;
    if decision.as_ref().is_some_and(|d| {
        d.review != j.request.id
            || d.cell != j.request.cell
            || d.scope.as_str() != "DEVICE_PACKAGE_SOFTWARE"
    }) {
        return Err(StoreError::Integrity(
            "device decision identity/scope".into(),
        ));
    }
    let is_latest = selected.as_ref().map(|v| v.revision) == latest_revision;
    let current = context(tx, meta, &j)?;
    let matches = current
        && is_latest
        && selected.as_ref().is_some_and(|v| {
            v.checker_digest == checker_digest()
                && v.ready_for_software_approval
                && decision.as_ref().is_some_and(|d| {
                    d.choice == Choice::Approve
                        && d.review_digest == v.review_digest
                        && d.report_revision == v.revision
                        && d.report_digest == v.report_digest
                })
        });
    Ok(Detail {
        job: j,
        version: selected,
        decision,
        latest_report_revision: latest_revision,
        is_latest,
        context_current: current,
        approval_matches_current_review: matches,
        activation_authorized: false,
    })
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn configure_device_review(&mut self, digest: Option<Digest>) -> Result<()> {
        let meta = &self.installation;
        self.repository.transact(|tx| {
            lifecycle::require_serving(tx)?;
            let k = name("devicereview/authority");
            let old = tx.get(&k)?;
            let a = AuthorityState {
                boot: meta.runtime_boot.clone(),
                digest,
            };
            tx.put(
                &k,
                old.map(|r| r.revision),
                &doc("rx.internal.device-review-authority.v1", &a)?,
            )?;
            event(tx, "rx.event.device-review-authority-configured.v1", &a)
        })
    }
    pub fn create_device_review(
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
            let (scope, fp) = request(meta, &p, "DeviceReview.Create", key_.as_str(), &input)?;
            if let Some(old) = prior(tx, &scope, fp, JOB)? {
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
            let catalog = receipt
                .device_catalog
                .ok_or(StoreError::Rejected(Reject::UnsupportedSchema))?;
            let (_, cell): (_, Cell) = load(tx, "cell", &input.cell, CELL)?;
            let registration = package_intake::current(tx, meta)?.ok_or(
                StoreError::Unavailable("device package service missing".into()),
            )?;
            if registration.generation != input.policy_generation
                || package_intake::configuration_digest(&cell)? != input.configuration_digest
                || receipt.configuration_digest != input.configuration_digest
            {
                return reject(Reject::StaleRevision);
            }
            let authority = authority(tx, meta)?.ok_or(StoreError::Unavailable(
                "device review authority missing".into(),
            ))?;
            let j = Job {
                request: Request {
                    schema: name("rx.device-review-request.v1"),
                    id: input.id.clone(),
                    intake: input.intake,
                    installation: meta.id.clone(),
                    cell: input.cell,
                    package_manifest: receipt.object.manifest,
                    package_signature: receipt.object.signature,
                    catalog,
                    configuration_digest: input.configuration_digest,
                    package_policy_fingerprint: registration.policy_fingerprint,
                    package_policy_file_digest: registration.policy_file_digest,
                    verification_authority_digest: authority,
                },
                registration,
                submitted_by: receipt.submitted_by,
                requested_by: p.id,
                created_at: now,
            };
            j.request.validate().map_err(StoreError::Invalid)?;
            save(tx, "devicereviewjob", &input.id, None, JOB, &j)?;
            event(tx, "rx.event.device-review-created.v1", &j)?;
            remember(tx, &scope, fp, JOB, &j)?;
            Ok(j)
        })
    }
    pub fn prepare_device_report(
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
            let (scope, fp) = request(meta, &p, "DeviceReview.Report", key_.as_str(), &input)?;
            if let Some(old) = prior(tx, &scope, fp, VERSION)? {
                return Ok(Preflight::Recorded(Box::new(old)));
            }
            let j = job(tx, &input.review, &input.cell)?;
            if !context(tx, meta, &j)?
                || latest(tx, &input.review)?.map(|v| v.revision) != input.expected
            {
                return reject(Reject::StaleRevision);
            }
            let registration = package_intake::current(tx, meta)?
                .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?;
            Ok(Preflight::Verify(Box::new(Ticket {
                job: j,
                identity: identity.clone(),
                key: key_.clone(),
                input,
                registration,
                boot: meta.runtime_boot.clone(),
                issued: now,
            })))
        })
    }
    pub fn commit_device_report(&mut self, prepared: Prepared) -> Result<Version> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let t = prepared.ticket;
            let v = prepared.validated;
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let p = access(tx, &t.identity, meta, &now, &t.input.cell)?;
            let (scope, fp) = request(meta, &p, "DeviceReview.Report", t.key.as_str(), &t.input)?;
            if let Some(old) = prior(tx, &scope, fp, VERSION)? {
                return Ok(old);
            }
            check_ticket(meta, &now, &t.boot, &t.issued)?;
            if !context(tx, meta, &t.job)?
                || package_intake::current(tx, meta)?.as_ref() != Some(&t.registration)
                || v.stored.owner() != &t.registration.store_owner
                || v.stored.policy_fingerprint() != t.registration.policy_fingerprint
                || v.report.request.digest().map_err(StoreError::Invalid)?
                    != t.job.request.digest().map_err(StoreError::Invalid)?
                || v.report.digest().map_err(StoreError::Invalid)? != t.input.report_digest
                || latest(tx, &t.input.review)?.map(|v| v.revision) != t.input.expected
            {
                return reject(Reject::StaleRevision);
            }
            let revision = t
                .input
                .expected
                .map_or(Ok(Counter(1)), |r| r.increment().map_err(domain_error))?;
            let mut version = Version {
                review: t.input.review.clone(),
                cell: t.input.cell,
                revision,
                review_digest: Digest::from_bytes([0; 32]),
                checker_digest: checker_digest(),
                report_digest: t.input.report_digest,
                ready_for_software_approval: v.report.passed(),
                report: v.report,
                signature: v.signature,
                recorded_by: p.id,
                recorded_at: now,
            };
            version.review_digest = version.digest().map_err(StoreError::Invalid)?;
            save(
                tx,
                "devicereviewversion",
                &version.review,
                t.input.expected,
                VERSION,
                &version,
            )?;
            save(
                tx,
                "devicereviewhistory",
                (&version.review, revision),
                None,
                VERSION,
                &version,
            )?;
            event(tx, "rx.event.device-report-recorded.v1", &version)?;
            remember(tx, &scope, fp, VERSION, &version)?;
            Ok(version)
        })
    }
    pub fn prepare_device_decision(
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
            let (scope, fp) = request(meta, &p, "DeviceReview.Decide", key_.as_str(), &input)?;
            if let Some(old) = prior(tx, &scope, fp, DECISION)? {
                return Ok(DecisionPreflight::Recorded(Box::new(old)));
            }
            let j = job(tx, &input.review, &input.cell)?;
            let v = latest(tx, &input.review)?.ok_or(StoreError::Rejected(Reject::NotFound))?;
            if v.revision != input.report_revision
                || v.review_digest != input.review_digest
                || tx
                    .get(&key("devicereviewdecision", &input.review))?
                    .map(|r| r.revision)
                    != input.expected
            {
                return reject(Reject::StaleRevision);
            }
            if input.choice == Choice::Approve {
                if p.id == j.submitted_by {
                    return reject(Reject::Forbidden);
                }
                if !context(tx, meta, &j)?
                    || !v.ready_for_software_approval
                    || v.checker_digest != checker_digest()
                {
                    return reject(Reject::QualificationRequired);
                }
            }
            Ok(DecisionPreflight::Verify(Box::new(DecisionTicket {
                job: j,
                version: v,
                identity: identity.clone(),
                key: key_.clone(),
                input,
                registration: package_intake::current(tx, meta)?,
                boot: meta.runtime_boot.clone(),
                issued: now,
            })))
        })
    }
    pub fn commit_device_decision(&mut self, prepared: PreparedDecision) -> Result<Decision> {
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
            let (scope, fp) = request(meta, &p, "DeviceReview.Decide", t.key.as_str(), &t.input)?;
            if let Some(old) = prior(tx, &scope, fp, DECISION)? {
                return Ok(old);
            }
            check_ticket(meta, &now, &t.boot, &t.issued)?;
            let v = latest(tx, &t.input.review)?.ok_or(StoreError::Rejected(Reject::NotFound))?;
            if v.revision != t.input.report_revision || v.review_digest != t.input.review_digest {
                return reject(Reject::StaleRevision);
            }
            if t.input.choice == Choice::Approve {
                if p.id == t.job.submitted_by {
                    return reject(Reject::Forbidden);
                }
                let proof = prepared
                    .validated
                    .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
                if !context(tx, meta, &t.job)?
                    || !v.ready_for_software_approval
                    || v.checker_digest != checker_digest()
                    || package_intake::current(tx, meta)? != t.registration
                    || t.registration.as_ref().is_none_or(|r| {
                        proof.stored.owner() != &r.store_owner
                            || proof.stored.policy_fingerprint() != r.policy_fingerprint
                    })
                    || proof.report.digest().map_err(StoreError::Invalid)? != v.report_digest
                    || !proof.report.passed()
                {
                    return reject(Reject::QualificationRequired);
                }
            }
            let revision = t
                .input
                .expected
                .map_or(Ok(Counter(1)), |v| v.increment().map_err(domain_error))?;
            let d = Decision {
                review: t.input.review,
                cell: t.input.cell,
                revision,
                review_digest: v.review_digest,
                report_revision: v.revision,
                report_digest: v.report_digest,
                choice: t.input.choice,
                note: t.input.note,
                decided_by: p.id,
                decided_at: now,
                scope: name("DEVICE_PACKAGE_SOFTWARE"),
            };
            save(
                tx,
                "devicereviewdecision",
                &d.review,
                t.input.expected,
                DECISION,
                &d,
            )?;
            save(
                tx,
                "devicereviewdecisionhistory",
                (&d.review, revision),
                None,
                DECISION,
                &d,
            )?;
            event(tx, "rx.event.device-review-decided.v1", &d)?;
            remember(tx, &scope, fp, DECISION, &d)?;
            Ok(d)
        })
    }
    pub fn device_review(
        &mut self,
        identity: &Identity,
        cell: &Name,
        id: &Id,
        revision: Option<Counter>,
    ) -> Result<Detail> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            access(tx, identity, meta, &clock.now(), cell)?;
            let j = job(tx, id, cell)?;
            read_detail(tx, meta, j, revision)
        })
    }
    pub fn device_reviews(
        &mut self,
        identity: &Identity,
        cell: &Name,
        intake: &Id,
        after: Option<&Id>,
    ) -> Result<Page> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            access(tx, identity, meta, &clock.now(), cell)?;
            let (_, r): (_, crate::package_intake::Receipt) = load(
                tx,
                "packageintakereceipt",
                intake,
                "rx.package-intake-receipt.v1",
            )?;
            if &r.cell != cell {
                return reject(Reject::Forbidden);
            }
            let mut jobs = tx
                .scan("devicereviewjob/")?
                .iter()
                .map(|r| decode::<Job>(r, JOB))
                .collect::<Result<Vec<_>>>()?;
            jobs.retain(|j| {
                &j.request.cell == cell
                    && &j.request.intake == intake
                    && after.is_none_or(|v| &j.request.id > v)
            });
            jobs.sort_by(|a, b| a.request.id.cmp(&b.request.id));
            let more = jobs.len() > 50;
            jobs.truncate(50);
            let next = if more {
                jobs.last().map(|j| j.request.id.clone())
            } else {
                None
            };
            let reviews = jobs
                .into_iter()
                .map(|j| read_detail(tx, meta, j, None).map(Summary::from_detail))
                .collect::<Result<Vec<_>>>()?;
            Ok(Page {
                cell: cell.clone(),
                intake: intake.clone(),
                reviews,
                next,
            })
        })
    }
}
