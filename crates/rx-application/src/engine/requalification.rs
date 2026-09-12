use super::*;
use crate::requalification as q;
mod runtime_restrictions;
const POLICY: &str = "rx.internal.requalification-policy.v1";
const JOB: &str = "rx.requalification-job.v1";
const VERSION: &str = "rx.requalification-version.v1";
const DECISION: &str = "rx.requalification-decision.v1";
#[derive(serde::Serialize, serde::Deserialize)]
struct Registration {
    boot: Id,
    policy: Option<q::Policy>,
}
pub(super) fn policy(tx: &mut dyn Transaction, meta: &Installation) -> Result<q::Policy> {
    let row = tx
        .get(&name("requalification/policy"))?
        .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?;
    let r: Registration = decode(&row, POLICY)?;
    if r.boot != meta.runtime_boot {
        return reject(Reject::CapabilityMissing);
    }
    r.policy
        .ok_or(StoreError::Rejected(Reject::CapabilityMissing))
}
pub(super) fn access(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    identity: &Identity,
    job: &q::Job,
    role: Option<Role>,
) -> Result<Principal> {
    let p = if let Some(r) = role {
        authorize(tx, identity, meta, now, Some(&job.request.origin), r, false)?
    } else {
        authorize_identity(tx, identity, meta, now)?
    };
    if !p
        .roles
        .iter()
        .any(|r| matches!(r, Role::Engineer | Role::Verifier | Role::ReleaseManager))
        || job
            .request
            .cells
            .iter()
            .any(|c| !p.cells.contains(&c.profile.cell))
    {
        return reject(Reject::Forbidden);
    }
    Ok(p)
}
pub(super) fn job(tx: &mut dyn Transaction, id: &Id, cell: &Name) -> Result<q::Job> {
    let (_, j): (_, q::Job) = load(tx, "requalificationjob", id, JOB)?;
    if j.request.id != *id || j.request.origin != *cell {
        return reject(Reject::Forbidden);
    }
    j.request.digest().map_err(StoreError::Integrity)?;
    Ok(j)
}
fn cohort_impact(
    tx: &mut dyn Transaction,
    origin: &Name,
    cells: &BTreeSet<Name>,
) -> Result<Digest> {
    let mut hosts = BTreeSet::new();
    let mut resources = BTreeSet::new();
    for id in cells {
        let (_, cell): (_, Cell) = load(tx, "cell", id, CELL)?;
        hosts.extend(cell.configuration.hosts.iter().cloned());
        resources.extend(
            cell.configuration
                .steps
                .iter()
                .flat_map(|s| s.intent.resource_set.iter().cloned()),
        );
    }
    let impact = process_change::prospective_impact(tx, origin, &hosts, &resources)?;
    if impact
        .cells
        .iter()
        .map(|c| c.id.clone())
        .collect::<BTreeSet<_>>()
        != *cells
    {
        return reject(Reject::QualificationRequired);
    }
    canonical::digest("RX-REQUALIFICATION-IMPACT-v1", &impact).map_err(domain_error)
}
pub(super) fn impact_current(tx: &mut dyn Transaction, job: &q::Job) -> Result<()> {
    let cells = job
        .request
        .cells
        .iter()
        .map(|c| c.profile.cell.clone())
        .collect();
    let actual = cohort_impact(tx, &job.request.origin, &cells)?;
    if job
        .request
        .impact_digest
        .is_some_and(|expected| expected != actual)
    {
        return reject(Reject::StaleRevision);
    }
    Ok(())
}
pub(super) fn current(tx: &mut dyn Transaction, meta: &Installation, j: &q::Job) -> Result<()> {
    if j.request.runtime_boot != meta.runtime_boot
        || policy(tx, meta)?.digest().map_err(StoreError::Integrity)? != j.request.policy_digest
    {
        return reject(Reject::StaleRevision);
    }
    let c = process_change::change(tx, &j.request.change, &j.request.origin)?;
    if c.state != crate::process_change::State::AppliedUnqualified
        || c.revision != j.request.change_revision
        || canonical::digest("RX-REQUALIFICATION-APPLICATION-v1", &c.application)
            .map_err(domain_error)?
            != j.request.application_digest
    {
        return reject(Reject::StaleRevision);
    }
    impact_current(tx, j)?;
    runtime_restrictions::current(tx, meta, j)?;
    for t in &j.request.cells {
        let (rev, cell): (_, Cell) = load(tx, "cell", &t.profile.cell, CELL)?;
        if rev != t.expected_revision
            || cell.epoch != t.epoch
            || cell.scope_epochs != t.scopes
            || process_change::config_ref(&cell.configuration)? != t.profile.configuration
            || cell.qualification.is_some()
            || t.blocks
                .iter()
                .any(|b| !cell.blocks.iter().any(|x| x.id == *b && x.latched))
        {
            return reject(Reject::StaleRevision);
        }
    }
    Ok(())
}
pub(super) fn fences_confirmed(tx: &mut dyn Transaction, j: &q::Job) -> Result<bool> {
    let acks = tx.scan("fenceack/")?;
    for f in &j.request.fences {
        let Some(reg) = tx.get(&key("host", (&f.cell, &f.host)))? else {
            return Ok(false);
        };
        let h: HostRegistration = decode(&reg, HOST)?;
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
            return Ok(false);
        }
    }
    Ok(true)
}
pub(super) fn latest(tx: &mut dyn Transaction, id: &Id) -> Result<Option<q::Version>> {
    tx.get(&key("requalificationversion", id))?
        .map(|r| decode(&r, VERSION))
        .transpose()
}
pub(super) fn version_digest(v: &q::Version) -> Result<Digest> {
    canonical::digest(
        "RX-REQUALIFICATION-VERSION-v1",
        &(
            &v.review,
            v.revision,
            &v.report,
            &v.signature,
            v.ready_for_review,
            &v.recorded_by,
            &v.recorded_at,
        ),
    )
    .map_err(domain_error)
}
const CHUNK: usize = 256 * 1024;
#[derive(serde::Serialize, serde::Deserialize)]
struct Blob {
    size: Counter,
    chunks: Vec<Digest>,
}
fn put_blob(tx: &mut dyn Transaction, hash: Digest, bytes: &[u8]) -> Result<()> {
    use base64::Engine as _;
    let mut chunks = Vec::new();
    for (index, part) in bytes.chunks(CHUNK).enumerate() {
        chunks.push(rx_package::content_digest(part));
        let k = key("qualificationchunk", (hash, Counter(index as u64)));
        let d = doc(
            "rx.internal.qualification-chunk.v1",
            &base64::engine::general_purpose::STANDARD.encode(part),
        )?;
        if let Some(old) = tx.get(&k)? {
            if old.document != d {
                return Err(StoreError::Integrity(
                    "qualification chunk collision".into(),
                ));
            }
        } else {
            tx.put(&k, None, &d)?;
        }
    }
    let k = key("qualificationblob", hash);
    let d = doc(
        "rx.internal.qualification-blob.v1",
        &Blob {
            size: Counter(bytes.len() as u64),
            chunks,
        },
    )?;
    if let Some(old) = tx.get(&k)? {
        if old.document != d {
            return Err(StoreError::Integrity("qualification blob collision".into()));
        }
    } else {
        tx.put(&k, None, &d)?;
    }
    Ok(())
}
pub(super) fn read_blob(tx: &mut dyn Transaction, r: &ArtifactRef) -> Result<Vec<u8>> {
    use base64::Engine as _;
    let (_, blob): (_, Blob) = load(
        tx,
        "qualificationblob",
        r.sha256,
        "rx.internal.qualification-blob.v1",
    )?;
    if blob.size != r.size_bytes
        || blob.size.0 == 0
        || blob.size.0 > q::MAX_ARTIFACT
        || blob.chunks.len() != blob.size.0.div_ceil(CHUNK as u64) as usize
    {
        return Err(StoreError::Integrity("qualification blob shape".into()));
    }
    let mut bytes = Vec::with_capacity(blob.size.0 as usize);
    for (index, hash) in blob.chunks.iter().enumerate() {
        let (_, s): (_, String) = load(
            tx,
            "qualificationchunk",
            (r.sha256, Counter(index as u64)),
            "rx.internal.qualification-chunk.v1",
        )?;
        let part = base64::engine::general_purpose::STANDARD
            .decode(s)
            .map_err(|_| StoreError::Integrity("qualification artifact encoding".into()))?;
        if part.len() > CHUNK || rx_package::content_digest(&part) != *hash {
            return Err(StoreError::Integrity("qualification chunk differs".into()));
        }
        bytes.extend(part);
    }
    if bytes.len() as u64 != r.size_bytes.0 || rx_package::content_digest(&bytes) != r.sha256 {
        return Err(StoreError::Integrity(
            "qualification artifact differs".into(),
        ));
    }
    Ok(bytes)
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn configure_requalification(&mut self, p: Option<q::Policy>) -> Result<()> {
        if let Some(p) = &p {
            p.digest().map_err(StoreError::Invalid)?;
        }
        let meta = &self.installation;
        let boot = meta.runtime_boot.clone();
        self.repository.transact(|tx| {
            let k = name("requalification/policy");
            let old = tx.get(&k)?;
            tx.put(
                &k,
                old.map(|r| r.revision),
                &doc(POLICY, &Registration { boot, policy: p })?,
            )?;
            qualification_activation::suspend_changed_roots(tx, meta)?;
            Ok(())
        })
    }
    pub fn begin_requalification(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: q::Begin,
    ) -> Result<q::Job> {
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
                Role::ReleaseManager,
                true,
            )?;
            let c = process_change::change(tx, &input.change, &input.cell)?;
            process_change::access(&actor, &c.impact)?;
            let (scope, fp) =
                request(meta, &actor, "Requalification.Begin", key_.as_str(), &input)?;
            if let Some(j) = prior(tx, &scope, fp, JOB)? {
                return Ok(j);
            }
            let p = policy(tx, meta)?;
            if p.digest().map_err(StoreError::Invalid)? != input.policy_digest
                || c.state != crate::process_change::State::AppliedUnqualified
                || c.revision != input.expected_change
                || input.expected_cells.keys().collect::<BTreeSet<_>>()
                    != c.impact.cells.iter().map(|c| &c.id).collect()
            {
                return reject(Reject::StaleRevision);
            }
            if tx.get(&key("requalificationjob", &input.id))?.is_some() {
                return reject(Reject::StaleRevision);
            }
            let a = c
                .application
                .as_ref()
                .ok_or(StoreError::Integrity("application absent".into()))?;
            let restrictions = runtime_restrictions::select(tx, meta, &input, &c)?;
            let impact_digest = cohort_impact(
                tx,
                &input.cell,
                &input.expected_cells.keys().cloned().collect(),
            )?;
            let mut cells = Vec::new();
            let mut fences = Vec::new();
            for target in &a.cells {
                let (rev, mut cell): (_, Cell) = load(tx, "cell", &target.cell, CELL)?;
                check_revision(rev, input.expected_cells[&target.cell])?;
                if process_change::config_ref(&cell.configuration)? != target.after
                    || cell.qualification.is_some()
                {
                    return reject(Reject::StaleRevision);
                }
                let profile = p
                    .profiles
                    .iter()
                    .find(|p| {
                        p.cell == target.cell
                            && p.configuration == target.after
                            && p.envelope == cell.configuration.envelope
                            && p.definition == cell.configuration.definition
                            && p.environment == cell.configuration.environment
                    })
                    .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?
                    .clone();
                let declared: BTreeSet<_> = profile.references().iter().map(|r| r.sha256).collect();
                if !q::required_dependencies(&cell.configuration).is_subset(&declared) {
                    return reject(Reject::CapabilityMissing);
                }
                let before: BTreeSet<_> = cell.blocks.iter().map(|b| b.id.clone()).collect();
                for (host, message) in invalidate_for_change(tx, &mut cell, rev)? {
                    fences.push(crate::process_change::FenceTarget {
                        cell: target.cell.clone(),
                        host,
                        message,
                        epoch: cell.epoch,
                        scopes: cell.scope_epochs.clone(),
                    });
                }
                qualification_activation::record_blocks(tx, &c.id, &cell, &before)?;
                cells.push(q::CellTarget {
                    profile,
                    expected_revision: rev.increment().map_err(domain_error)?,
                    epoch: cell.epoch,
                    scopes: cell.scope_epochs,
                    blocks: cell
                        .blocks
                        .into_iter()
                        .filter(|b| {
                            !before.contains(&b.id)
                                || input.runtime_restrictions.contains_key(&b.id)
                        })
                        .map(|b| b.id)
                        .collect(),
                });
            }
            let j = q::Job {
                request: q::Request {
                    schema: name("rx.requalification-request.v2"),
                    impact_digest: Some(impact_digest),
                    runtime_restrictions: restrictions,
                    id: input.id,
                    change: c.id,
                    change_revision: c.revision,
                    application_digest: canonical::digest(
                        "RX-REQUALIFICATION-APPLICATION-v1",
                        &c.application,
                    )
                    .map_err(domain_error)?,
                    origin: input.cell,
                    runtime_boot: meta.runtime_boot.clone(),
                    policy_digest: input.policy_digest,
                    cells,
                    fences,
                },
                requested_by: actor.id,
                terminal: identity.terminal.as_ref().unwrap().0.clone(),
                requested_at: now,
            };
            j.request.digest().map_err(StoreError::Invalid)?;
            qualification_activation::bind_runtime_restrictions(tx, meta, &j)?;
            save(tx, "requalificationjob", &j.request.id, None, JOB, &j)?;
            event(tx, "rx.event.requalification-requested.v1", &j)?;
            remember(tx, &scope, fp, JOB, &j)?;
            Ok(j)
        })
    }
    pub fn prepare_requalification_report(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: q::Submit,
    ) -> Result<q::Preflight> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let j = job(tx, &input.review, &input.cell)?;
            let p = access(tx, meta, &now, identity, &j, Some(Role::Engineer))?;
            let (scope, fp) = request(meta, &p, "Requalification.Report", key_.as_str(), &input)?;
            if let Some(v) = prior(tx, &scope, fp, VERSION)? {
                return Ok(q::Preflight::Recorded(Box::new(v)));
            }
            current(tx, meta, &j)?;
            if latest(tx, &input.review)?.as_ref().map(|v| v.revision) != input.expected {
                return reject(Reject::StaleRevision);
            }
            Ok(q::Preflight::Verify(Box::new(q::Ticket {
                policy: j.request.policy_digest,
                job: j,
                identity: identity.clone(),
                key: key_.clone(),
                input,
                issued: now,
            })))
        })
    }
    pub fn commit_requalification_report(&mut self, p: q::Prepared) -> Result<q::Version> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let t = &p.ticket;
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let actor = access(tx, meta, &now, &t.identity, &t.job, Some(Role::Engineer))?;
            let (scope, fp) = request(
                meta,
                &actor,
                "Requalification.Report",
                t.key.as_str(),
                &t.input,
            )?;
            if let Some(v) = prior(tx, &scope, fp, VERSION)? {
                return Ok(v);
            }
            current(tx, meta, &t.job)?;
            if now.age_ns(&t.issued).is_none_or(|n| n >= 30_000_000_000)
                || p.verified.policy != t.policy
                || latest(tx, &t.input.review)?.as_ref().map(|v| v.revision) != t.input.expected
            {
                return reject(Reject::StaleRevision);
            }
            for (hash, bytes) in &p.verified.blobs {
                put_blob(tx, *hash, bytes)?;
            }
            let rev = t
                .input
                .expected
                .map_or(Ok(Counter(1)), |r| r.increment().map_err(domain_error))?;
            let mut v = q::Version {
                review: t.input.review.clone(),
                revision: rev,
                report: p.verified.report,
                signature: p.verified.signature,
                digest: Digest::from_bytes([0; 32]),
                ready_for_review: p.verified.ready,
                recorded_by: actor.id,
                recorded_at: now,
            };
            v.digest = version_digest(&v)?;
            save(
                tx,
                "requalificationversion",
                &v.review,
                t.input.expected,
                VERSION,
                &v,
            )?;
            save(
                tx,
                "requalificationhistory",
                (&v.review, v.revision),
                None,
                VERSION,
                &v,
            )?;
            event(tx, "rx.event.requalification-report.v1", &v)?;
            remember(tx, &scope, fp, VERSION, &v)?;
            Ok(v)
        })
    }
    pub fn prepare_requalification_decision(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: q::Decide,
    ) -> Result<q::DecisionPreflight> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let j = job(tx, &input.review, &input.cell)?;
            let actor = access(tx, meta, &now, identity, &j, Some(Role::Verifier))?;
            let (scope, fp) = request(
                meta,
                &actor,
                "Requalification.Decide",
                key_.as_str(),
                &input,
            )?;
            if let Some(d) = prior(tx, &scope, fp, DECISION)? {
                return Ok(q::DecisionPreflight::Recorded(Box::new(d)));
            }
            current(tx, meta, &j)?;
            let v = latest(tx, &input.review)?.ok_or(StoreError::Rejected(Reject::NotFound))?;
            if v.digest != version_digest(&v)?
                || input.report_revision != v.revision
                || input.report_digest != v.digest
                || actor.id == j.requested_by
                || actor.id == v.recorded_by
                || input.note.trim().is_empty()
                || input.note.chars().count() > 4000
            {
                return reject(Reject::InvalidInput);
            }
            let previous = tx.get(&key("requalificationdecision", &input.review))?;
            if previous.as_ref().map(|r| r.revision) != input.expected {
                return reject(Reject::StaleRevision);
            }
            if input.choice == q::Choice::Reject {
                let d = make_decision(tx, &actor, &now, &input)?;
                remember(tx, &scope, fp, DECISION, &d)?;
                return Ok(q::DecisionPreflight::Recorded(Box::new(d)));
            }
            if !v.ready_for_review || !fences_confirmed(tx, &j)? {
                return reject(Reject::HostNotPrepared);
            }
            let mut blobs = BTreeMap::new();
            for r in v.report.references() {
                blobs.insert(r.sha256, read_blob(tx, &r)?);
            }
            Ok(q::DecisionPreflight::Verify(Box::new(q::DecisionTicket {
                policy: j.request.policy_digest,
                job: j,
                version: v,
                identity: identity.clone(),
                key: key_.clone(),
                input,
                issued: now,
                blobs,
            })))
        })
    }
    pub fn commit_requalification_decision(
        &mut self,
        p: q::PreparedDecision,
    ) -> Result<q::Decision> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let t = p.ticket;
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let actor = access(tx, meta, &now, &t.identity, &t.job, Some(Role::Verifier))?;
            let (scope, fp) = request(
                meta,
                &actor,
                "Requalification.Decide",
                t.key.as_str(),
                &t.input,
            )?;
            if let Some(d) = prior(tx, &scope, fp, DECISION)? {
                return Ok(d);
            }
            current(tx, meta, &t.job)?;
            let v = latest(tx, &t.input.review)?.ok_or(StoreError::Rejected(Reject::NotFound))?;
            if now.age_ns(&t.issued).is_none_or(|a| a >= 30_000_000_000)
                || v.digest != t.version.digest
                || v.revision != t.input.report_revision
                || !v.ready_for_review
                || actor.id == t.job.requested_by
                || actor.id == v.recorded_by
                || !fences_confirmed(tx, &t.job)?
            {
                return reject(Reject::StaleRevision);
            }
            let d = make_decision(tx, &actor, &now, &t.input)?;
            remember(tx, &scope, fp, DECISION, &d)?;
            Ok(d)
        })
    }
    pub fn requalification(
        &mut self,
        identity: &Identity,
        cell: &Name,
        id: &Id,
    ) -> Result<q::Detail> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let j = job(tx, id, cell)?;
            access(tx, meta, &now, identity, &j, None)?;
            let context_current = match current(tx, meta, &j) {
                Ok(()) => true,
                Err(StoreError::Rejected(_)) => false,
                Err(e) => return Err(e),
            };
            let fences = fences_confirmed(tx, &j)?;
            let v = latest(tx, id)?;
            if v.as_ref()
                .is_some_and(|v| version_digest(v).ok() != Some(v.digest))
            {
                return Err(StoreError::Integrity("qualification version digest".into()));
            }
            let decision = tx
                .get(&key("requalificationdecision", id))?
                .map(|r| decode::<q::Decision>(&r, DECISION))
                .transpose()?;
            let approval = context_current
                && fences
                && v.as_ref().zip(decision.as_ref()).is_some_and(|(v, d)| {
                    v.ready_for_review
                        && d.choice == q::Choice::Approve
                        && d.report_revision == v.revision
                        && d.report_digest == v.digest
                });
            Ok(q::Detail {
                job: j,
                version: v,
                decision,
                context_current,
                fences_confirmed: fences,
                approval_current: approval,
                activation_authorized: false,
            })
        })
    }
    pub fn requalification_artifact(
        &mut self,
        identity: &Identity,
        cell: &Name,
        id: &Id,
        reference: &ArtifactRef,
    ) -> Result<Vec<u8>> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let j = job(tx, id, cell)?;
            access(tx, meta, &clock.now(), identity, &j, None)?;
            let mut allowed = false;
            for row in tx.scan("requalificationhistory/")? {
                let v: q::Version = decode(&row, VERSION)?;
                if v.review == *id && v.report.references().iter().any(|r| r == reference) {
                    allowed = true;
                    break;
                }
            }
            if !allowed {
                return reject(Reject::Forbidden);
            }
            read_blob(tx, reference)
        })
    }
}
fn make_decision(
    tx: &mut dyn Transaction,
    actor: &Principal,
    now: &TimePoint,
    input: &q::Decide,
) -> Result<q::Decision> {
    let actual = tx.get(&key("requalificationdecision", &input.review))?;
    if actual.as_ref().map(|r| r.revision) != input.expected {
        return reject(Reject::StaleRevision);
    }
    let d = q::Decision {
        review: input.review.clone(),
        revision: input
            .expected
            .map_or(Ok(Counter(1)), |r| r.increment().map_err(domain_error))?,
        report_revision: input.report_revision,
        report_digest: input.report_digest,
        choice: input.choice,
        note: input.note.clone(),
        decided_by: actor.id.clone(),
        decided_at: now.clone(),
        scope: name("REQUALIFICATION_EVIDENCE_REVIEW"),
    };
    save(
        tx,
        "requalificationdecision",
        &d.review,
        input.expected,
        DECISION,
        &d,
    )?;
    save(
        tx,
        "requalificationdecisionhistory",
        (&d.review, d.revision),
        None,
        DECISION,
        &d,
    )?;
    event(tx, "rx.event.requalification-decision.v1", &d)?;
    Ok(d)
}
