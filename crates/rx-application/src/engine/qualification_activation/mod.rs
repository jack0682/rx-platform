use super::*;
use crate::{
    configuration_dispatch::{Issue, Phase, Sender},
    qualification_activation as a, requalification as q,
};
use rx_domain::host_qualification as host;
const BATCH: &str = "rx.qualification-activation-batch.v1";
const TASK: &str = "rx.qualification-host-task.v1";
#[derive(serde::Serialize, serde::Deserialize)]
struct Owner {
    block: Id,
    cell: Name,
    change: Id,
    reason: BlockReason,
}
pub(super) fn record_blocks(
    tx: &mut dyn Transaction,
    change: &Id,
    cell: &Cell,
    before: &BTreeSet<Id>,
) -> Result<()> {
    for b in cell.blocks.iter().filter(|b| !before.contains(&b.id)) {
        save(
            tx,
            "changeblockowner",
            &b.id,
            None,
            "rx.change-block-owner.v1",
            &Owner {
                block: b.id.clone(),
                cell: cell.configuration.id.clone(),
                change: change.clone(),
                reason: b.reason,
            },
        )?;
    }
    Ok(())
}
fn batch(tx: &mut dyn Transaction, id: &Id) -> Result<a::Batch> {
    let (rev, b): (_, a::Batch) = load(tx, "qualificationbatch", id, BATCH)?;
    if b.id != *id || b.revision != rev {
        return Err(StoreError::Integrity("qualification batch identity".into()));
    }
    Ok(b)
}
fn record_batch(tx: &mut dyn Transaction, b: &a::Batch, expected: Option<Counter>) -> Result<()> {
    save(tx, "qualificationbatch", &b.id, expected, BATCH, b)?;
    save(
        tx,
        "qualificationbatchhistory",
        (&b.id, b.revision),
        None,
        BATCH,
        b,
    )?;
    event(tx, "rx.event.qualification-batch.v1", b)
}
fn task(tx: &mut dyn Transaction, id: &Id) -> Result<(Counter, a::Task)> {
    let (rev, t): (_, a::Task) = load(tx, "qualificationtask", id, TASK)?;
    if t.id != *id
        || t.request.is_some() != t.digest.is_some()
        || t.request
            .as_ref()
            .is_some_and(|r| r.id != t.id || r.host != t.host || r.digest().ok() != t.digest)
        || t.receipt
            .as_ref()
            .is_some_and(|r| r.validate().is_err() || Some(r.request_digest) != t.digest)
    {
        return Err(StoreError::Integrity("qualification task identity".into()));
    }
    Ok((rev, t))
}
fn record_task(tx: &mut dyn Transaction, t: &a::Task, expected: Option<Counter>) -> Result<()> {
    save(tx, "qualificationtask", &t.id, expected, TASK, t)?;
    event(tx, "rx.event.qualification-host-task.v1", t)
}
fn access(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    identity: &Identity,
    b: &a::Batch,
    terminal: bool,
) -> Result<Principal> {
    let p = if terminal {
        authorize(
            tx,
            identity,
            meta,
            now,
            Some(&b.origin),
            Role::ReleaseManager,
            true,
        )?
    } else {
        authorize_identity(tx, identity, meta, now)?
    };
    if !terminal
        && !p
            .roles
            .iter()
            .any(|r| matches!(r, Role::Engineer | Role::Verifier | Role::ReleaseManager))
    {
        return reject(Reject::Forbidden);
    }
    if b.cells.iter().any(|c| !p.cells.contains(&c.cell)) {
        return reject(Reject::Forbidden);
    }
    Ok(p)
}
fn approved(
    tx: &mut dyn Transaction,
    meta: &Installation,
    j: &q::Job,
    revision: Counter,
    digest: Digest,
    decision_revision: Counter,
) -> Result<(q::Version, q::Decision)> {
    if requalification::policy(tx, meta)?
        .digest()
        .map_err(StoreError::Integrity)?
        != j.request.policy_digest
    {
        return reject(Reject::QualificationRequired);
    }
    let v = requalification::latest(tx, &j.request.id)?
        .ok_or(StoreError::Rejected(Reject::NotFound))?;
    let (r, d): (_, q::Decision) = load(
        tx,
        "requalificationdecision",
        &j.request.id,
        "rx.requalification-decision.v1",
    )?;
    if v.revision != revision
        || v.digest != digest
        || requalification::version_digest(&v)? != digest
        || !v.ready_for_review
        || r != decision_revision
        || d.revision != r
        || d.report_revision != v.revision
        || d.report_digest != v.digest
        || d.choice != q::Choice::Approve
        || d.scope.as_str() != "REQUALIFICATION_EVIDENCE_REVIEW"
    {
        return reject(Reject::QualificationRequired);
    }
    Ok((v, d))
}
fn quiet(tx: &mut dyn Transaction, j: &q::Job) -> Result<()> {
    let cells: BTreeSet<_> = j
        .request
        .cells
        .iter()
        .map(|c| c.profile.cell.clone())
        .collect();
    for row in tx.scan("run/")? {
        let r: Run = decode(&row, RUN)?;
        if cells.contains(&r.cell) && !matches!(r.state, RunState::Completed | RunState::Abandoned)
        {
            return reject(Reject::Busy);
        }
    }
    let mut resources = BTreeSet::new();
    for name in &cells {
        let (_, c): (_, Cell) = load(tx, "cell", name, CELL)?;
        resources.extend(
            c.configuration
                .steps
                .iter()
                .flat_map(|s| s.intent.resource_set.iter().cloned()),
        );
    }
    for row in tx.scan("work/")? {
        let w: Work = decode(&row, WORK)?;
        if cells.contains(&w.cell) {
            resources.extend(w.intent.resource_set.iter().cloned());
            if matches!(w.operation.outcome(), Outcome::None | Outcome::Unresolved)
                || w.operation.integrity() == Integrity::Disputed
            {
                return reject(Reject::ContinuityUnproven);
            }
        }
    }
    for r in resources {
        if let Some(row) = tx.get(&key("resource", &r))? {
            let r: Resource = decode(&row, RESOURCE)?;
            if r.holder.is_some() || r.quarantined {
                return reject(Reject::Busy);
            }
        }
    }
    for row in tx.scan("case/")? {
        let c: crate::intervention::Case = decode(&row, "rx.internal.intervention-case.v1")?;
        if c.state != crate::intervention::CaseState::Closed
            && (cells.contains(&c.cell) || c.effective_cells.iter().any(|c| cells.contains(c)))
        {
            return reject(Reject::BlockedByCase);
        }
    }
    Ok(())
}
fn pending_current(tx: &mut dyn Transaction, meta: &Installation, b: &a::Batch) -> Result<()> {
    if b.state != a::State::Pending
        || b.runtime_boot != meta.runtime_boot
        || package_intake::current(tx, meta)?.as_ref() != Some(&b.registration)
    {
        return reject(Reject::StaleRevision);
    }
    requalification::current(tx, meta, &b.job)?;
    approved(
        tx,
        meta,
        &b.job,
        b.report_revision,
        b.report_digest,
        b.decision_revision,
    )?;
    if !requalification::fences_confirmed(tx, &b.job)? {
        return reject(Reject::HostNotPrepared);
    }
    quiet(tx, &b.job)
}
fn owned_clear(tx: &mut dyn Transaction, change: &Id, cell: &Cell, ids: &[Id]) -> Result<()> {
    if ids.len() > 512 || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return reject(Reject::InvalidInput);
    }
    for id in ids {
        let b = cell
            .blocks
            .iter()
            .find(|b| &b.id == id)
            .ok_or(StoreError::Rejected(Reject::StaleRevision))?;
        let row = tx
            .get(&key("changeblockowner", id))?
            .ok_or(StoreError::Rejected(Reject::Forbidden))?;
        let owner: Owner = decode(&row, "rx.change-block-owner.v1")?;
        if owner.block != *id
            || owner.cell != cell.configuration.id
            || owner.change != *change
            || owner.reason != b.reason
        {
            return reject(Reject::Forbidden);
        }
    }
    Ok(())
}
fn ticket(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: TimePoint,
    identity: Identity,
    key_: Id,
    action: a::Action,
    reviewed: (q::Job, q::Version, q::Decision),
) -> Result<a::Ticket> {
    let (j, version, decision) = reviewed;
    let change = process_change::change(tx, &j.request.change, &j.request.origin)?;
    let source = process_review::load_job(tx, &change.review.id, &change.cell)?;
    let registration = package_intake::current(tx, meta)?
        .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?;
    let package = rx_package::store::ObjectId {
        manifest: source.request.package_manifest,
        signature: source.request.package_signature,
    };
    let mut blobs = BTreeMap::new();
    for r in version.report.references() {
        blobs.insert(r.sha256, requalification::read_blob(tx, &r)?);
    }
    Ok(a::Ticket {
        action,
        identity,
        key: key_,
        job: j,
        version,
        decision,
        blobs,
        registration,
        package,
        issued: now,
    })
}
fn host_access(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    identity: &Identity,
    t: &a::Task,
) -> Result<()> {
    let p = authorize(tx, identity, meta, now, None, Role::Host, false)?;
    if p.id != t.host || t.cells.iter().any(|c| !p.cells.contains(c)) {
        return reject(Reject::Forbidden);
    }
    Ok(())
}
fn generation(tx: &mut dyn Transaction, t: &a::Task) -> Result<bool> {
    for c in &t.cells {
        let (_, r): (_, HostRegistration) = load(tx, "host", (c, &t.host), HOST)?;
        if r.boot_id != t.host_boot || r.delivery_journal != t.journal || r.session != t.session {
            return Ok(false);
        }
    }
    Ok(true)
}
fn sender(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    b: &a::Batch,
    t: &a::Task,
) -> Result<()> {
    pending_current(tx, meta, b)?;
    access(tx, meta, now, &b.sender.identity(), b, true)?;
    if !generation(tx, t)? {
        return reject(Reject::ContinuityUnproven);
    }
    Ok(())
}
fn view(tx: &mut dyn Transaction, meta: &Installation, b: a::Batch) -> Result<a::View> {
    let hosts = b
        .tasks
        .iter()
        .map(|id| task(tx, id).map(|v| v.1))
        .collect::<Result<Vec<_>>>()?;
    let accepted = hosts
        .iter()
        .filter(|t| {
            t.receipt
                .as_ref()
                .is_some_and(|r| r.status == host::Status::Accepted)
        })
        .count();
    let unknown = hosts
        .iter()
        .any(|t| t.phase == Phase::SendEntered && t.receipt.is_none());
    let result = if b.state == a::State::Active {
        active_current(tx, meta, &b)
    } else {
        pending_current(tx, meta, &b)
    };
    let current = match result {
        Ok(()) => true,
        Err(StoreError::Rejected(_)) => false,
        Err(e) => return Err(e),
    };
    Ok(a::View {
        mixed: accepted > 0 && accepted < hosts.len(),
        outcome_unknown: unknown,
        current,
        operation_authorized: false,
        accepted_hosts: accepted,
        batch: b,
        hosts,
    })
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Certificate {
    batch: Id,
    cell: Name,
}
fn active_current(tx: &mut dyn Transaction, meta: &Installation, b: &a::Batch) -> Result<()> {
    if b.state != a::State::Active
        || b.runtime_boot != meta.runtime_boot
        || package_intake::current(tx, meta)?.as_ref() != Some(&b.registration)
    {
        return reject(Reject::QualificationRequired);
    }
    approved(
        tx,
        meta,
        &b.job,
        b.report_revision,
        b.report_digest,
        b.decision_revision,
    )?;
    for target in &b.cells {
        let (_, cell): (_, Cell) = load(tx, "cell", &target.cell, CELL)?;
        if process_change::config_ref(&cell.configuration)? != target.configuration
            || cell.epoch != target.epoch
            || cell.scope_epochs != target.scopes
            || cell.qualification.as_ref().is_none_or(|q| {
                q.id != target.qualification.id || q.revision != target.qualification.revision
            })
        {
            return reject(Reject::QualificationRequired);
        }
    }
    for id in &b.tasks {
        let (_, t) = task(tx, id)?;
        if t.disputed
            || t.issue.is_some()
            || !generation(tx, &t)?
            || !t
                .observation
                .as_ref()
                .is_some_and(|o| o.receipt_matches_current_host)
        {
            return reject(Reject::HostNotPrepared);
        }
    }
    Ok(())
}
pub(super) fn check_ready(tx: &mut dyn Transaction, meta_boot: &str, cell: &Cell) -> Result<()> {
    let Some(q) = &cell.qualification else {
        return reject(Reject::QualificationRequired);
    };
    if let Some(row) = tx.get(&key("qualificationcertificate", &q.id))? {
        let cert: Certificate = decode(&row, "rx.qualification-certificate.v1")?;
        if cert.cell != cell.configuration.id {
            return Err(StoreError::Integrity(
                "qualification certificate cell".into(),
            ));
        }
        let b = batch(tx, &cert.batch)?;
        let meta: Installation = decode(
            &tx.get(&name("installation/current"))?
                .ok_or(StoreError::Integrity("installation missing".into()))?,
            "rx.internal.installation.v1",
        )?;
        if meta.clock_id != meta_boot {
            return reject(Reject::QualificationRequired);
        }
        active_current(tx, &meta, &b)?;
    }
    Ok(())
}
pub(super) fn purpose(tx: &mut dyn Transaction, cell: &Cell, purpose: Purpose) -> Result<()> {
    let Some(q) = &cell.qualification else {
        return reject(Reject::QualificationRequired);
    };
    if let Some(row) = tx.get(&key("qualificationcertificate", &q.id))? {
        let cert: Certificate = decode(&row, "rx.qualification-certificate.v1")?;
        let b = batch(tx, &cert.batch)?;
        let c = b
            .cells
            .iter()
            .find(|c| c.cell == cell.configuration.id)
            .ok_or(StoreError::Integrity("qualification cell absent".into()))?;
        let purpose = match purpose {
            Purpose::Production => "PRODUCTION",
            Purpose::Setup => "SETUP",
        };
        if !c.purposes.iter().any(|p| p.as_str() == purpose) {
            return reject(Reject::Forbidden);
        }
    }
    Ok(())
}
fn fresh_hosts(
    tx: &mut dyn Transaction,
    b: &a::Batch,
    now: &TimePoint,
    reads: &BTreeMap<Id, (TimePoint, Digest)>,
) -> Result<()> {
    for id in &b.tasks {
        let (_, t) = task(tx, id)?;
        let o = t
            .observation
            .as_ref()
            .ok_or(StoreError::Rejected(Reject::HostNotPrepared))?;
        if t.disputed
            || t.issue.is_some()
            || t.receipt
                .as_ref()
                .is_none_or(|r| r.status != host::Status::Accepted)
            || !o.receipt_matches_current_host
            || !generation(tx, &t)?
        {
            return reject(Reject::HostNotPrepared);
        }
        let (at, hash) = reads.get(id).ok_or(StoreError::Rejected(Reject::Expired))?;
        if now.age_ns(at).is_none_or(|age| age > 3_000_000_000)
            || canonical::digest("RX-QUALIFICATION-OBSERVATION-v1", o).map_err(domain_error)?
                != *hash
        {
            return reject(Reject::Expired);
        }
    }
    Ok(())
}
pub(super) fn suspend(
    tx: &mut dyn Transaction,
    b: &a::Batch,
    meta: Option<&Installation>,
    reason: Name,
) -> Result<a::Batch> {
    if b.state == a::State::Suspended {
        return Ok(b.clone());
    }
    let mut changed = b.clone();
    changed.state = a::State::Suspended;
    changed.suspended_reason = Some(reason);
    changed.revision = changed.revision.increment().map_err(domain_error)?;
    record_batch(tx, &changed, Some(b.revision))?;
    for target in &b.cells {
        let (rev, mut cell): (_, Cell) = load(tx, "cell", &target.cell, CELL)?;
        let own = cell
            .qualification
            .as_ref()
            .is_some_and(|q| q.id == target.qualification.id);
        if own {
            let q = cell.qualification.take().unwrap();
            let k = key("qualificationhistory", &q.id);
            if tx.get(&k)?.is_none() {
                tx.put(&k, None, &doc("rx.internal.qualification-history.v1", &q)?)?;
            }
            cell.commissioning = Some(Commissioning::RevalidationRequired);
            let before = cell.blocks.iter().map(|b| b.id.clone()).collect();
            invalidate_cell(tx, &mut cell, rev, BlockReason::AuthorityRevoked)?;
            record_blocks(tx, &b.change, &cell, &before)?;
        } else if b.state == a::State::Pending
            && meta.is_some_and(|m| m.runtime_boot == b.runtime_boot)
            && cell.epoch == target.epoch
        {
            let before = cell.blocks.iter().map(|b| b.id.clone()).collect();
            invalidate_cell(tx, &mut cell, rev, BlockReason::AuthorityRevoked)?;
            record_blocks(tx, &b.change, &cell, &before)?;
        }
    }
    let mut c = process_change::change(tx, &b.change, &b.origin)?;
    if c.state == crate::process_change::State::QualifiedActive
        && c.qualification_activation.as_ref() == Some(&b.id)
    {
        let rev = c.revision;
        c.state = crate::process_change::State::AppliedUnqualified;
        c.revision = c.revision.increment().map_err(domain_error)?;
        process_change::record(tx, &c, Some(rev))?;
    }
    Ok(changed)
}
pub(super) fn suspend_changed_roots(tx: &mut dyn Transaction, meta: &Installation) -> Result<()> {
    let policy = match requalification::policy(tx, meta) {
        Ok(p) => Some(p.digest().map_err(StoreError::Integrity)?),
        Err(StoreError::Rejected(_)) => None,
        Err(e) => return Err(e),
    };
    for row in tx.scan("qualificationbatch/")? {
        let b: a::Batch = decode(&row, BATCH)?;
        if b.state == a::State::Active
            && (b.runtime_boot != meta.runtime_boot
                || package_intake::current(tx, meta)?.as_ref() != Some(&b.registration)
                || policy != Some(b.job.request.policy_digest))
        {
            suspend(
                tx,
                &b,
                Some(meta),
                name("QUALIFICATION_TRUST_OR_RUNTIME_CHANGED"),
            )?;
        }
    }
    Ok(())
}
pub(super) fn cell_change(tx: &mut dyn Transaction, cell: &Cell) -> Result<Option<Id>> {
    let Some(q) = &cell.qualification else {
        return Ok(None);
    };
    let Some(row) = tx.get(&key("qualificationcertificate", &q.id))? else {
        return Ok(None);
    };
    let c: Certificate = decode(&row, "rx.qualification-certificate.v1")?;
    Ok(Some(batch(tx, &c.batch)?.change))
}
pub(super) fn arm_clear(tx: &mut dyn Transaction, cell: &Cell) -> Result<Vec<Id>> {
    let Some(q) = &cell.qualification else {
        return Ok(vec![]);
    };
    let Some(row) = tx.get(&key("qualificationcertificate", &q.id))? else {
        return Ok(vec![]);
    };
    let cert: Certificate = decode(&row, "rx.qualification-certificate.v1")?;
    let b = batch(tx, &cert.batch)?;
    let target = b
        .cells
        .iter()
        .find(|c| c.cell == cell.configuration.id && c.qualification.id == q.id)
        .ok_or(StoreError::Integrity(
            "qualification clear plan absent".into(),
        ))?;
    Ok(target.clear_blocks.clone())
}

mod activation;
mod host_tasks;
mod issuance;
