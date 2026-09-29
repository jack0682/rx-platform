//! A review-bound historical basis; not qualification, clearance or native authority by itself.
use super::*;
use crate::{host_binding_baseline as baseline, host_rejoin as h};
const BINDING: &str = "rx.host-rebind-restriction-binding.v1";
#[derive(serde::Serialize, serde::Deserialize)]
struct Binding {
    review: Id,
    change: Id,
    cell: Name,
    block: Id,
    request_digest: Digest,
    restriction_digest: Digest,
}

fn candidates(
    tx: &mut dyn Transaction,
    meta: &Installation,
    cell_id: &Name,
    actor: Option<&Principal>,
) -> Result<Vec<h::Restriction>> {
    let (_, cell): (_, Cell) = load(tx, "cell", cell_id, CELL)?;
    let configuration = crate::runtime_invalidation::configuration_digest(&cell.configuration)
        .map_err(StoreError::Integrity)?;
    let mut found = BTreeMap::new();
    for host in &cell.configuration.hosts {
        let Some(row) = tx.get(&key("host-rejoin-rebind-owner", host))? else {
            continue;
        };
        let id: Id = decode(&row, "rx.host-rejoin-rebind-reference.v1")?;
        let (latest, proposal) = host_rejoin::rebind_record(tx, &id)?;
        if actor.is_some_and(|a| proposal.context.cells.keys().any(|c| !a.cells.contains(c))) {
            return reject(Reject::Forbidden);
        }
        if !host_rejoin::rebind_current(tx, meta, &latest, &proposal)? {
            continue;
        }
        let Some(step) = latest.steps.get(cell_id) else {
            continue;
        };
        let latest_base = baseline::load(tx, &step.plan.id)?
            .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
        let (_, registered): (_, HostRegistration) = load(tx, "host", (cell_id, host), HOST)?;
        let (_, producer): (_, EvidenceProducer) =
            load(tx, "producer", host, "rx.internal.evidence-producer.v1")?;
        if registered.delivery_journal != latest_base.delivery_journal
            || registered.source_sessions != latest_base.source_sessions
            || registered.boot_id != latest_base.host_boot
            || registered.session != latest_base.producer_session
            || producer.journal != latest_base.evidence_journal
            || producer.authentication_binding != latest_base.producer_authentication_binding
            || producer.session != latest_base.producer_session
            || producer.peer_boot != latest_base.host_boot
        {
            continue;
        }
        if host_recovery::registered_transport(tx, meta, host)? != latest_base.transport {
            return reject(Reject::ContinuityUnproven);
        }
        let mut plan = step.plan.id.clone();
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(plan.clone()) || visited.len() > 64 {
                return Err(StoreError::Integrity(
                    "Host restriction lineage cycle/overflow".into(),
                ));
            }
            let Some(row) = tx.get(&key("host-rebind-configuration-origin", &plan))? else {
                break;
            };
            let link: h::ConfigurationOrigin =
                decode(&row, "rx.host-rebind-configuration-origin.v1")?;
            let (rebind, p) = host_rejoin::rebind_record(tx, &link.rebind)?;
            let Some(target) = rebind.steps.get(cell_id) else {
                return reject(Reject::ContinuityUnproven);
            };
            let base = baseline::load(tx, &plan)?
                .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
            let prior = baseline::load(tx, &link.prior_plan)?
                .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
            if rebind.phase != h::RebindPhase::Bound
                || link.new_plan != plan
                || target.plan.id != plan
                || p.context.host != *host
                || p.context.installation != meta.id
                || p.context.store_generation != meta.store_generation
                || p.context.runtime_boot != meta.runtime_boot
                || prior.digest().map_err(StoreError::Integrity)? != link.prior_baseline_digest
                || p.context
                    .cells
                    .get(cell_id)
                    .and_then(|c| c.baseline.as_ref())
                    .is_none_or(|b| b.plan != prior.plan)
                || base.host != *host
                || base.cell != *cell_id
                || prior.host != *host
                || prior.cell != *cell_id
                || base.delivery_journal != latest_base.delivery_journal
                || base.evidence_journal != latest_base.evidence_journal
                || base.transport != latest_base.transport
                || base.source_sessions != latest_base.source_sessions
            {
                return reject(Reject::ContinuityUnproven);
            }
            // Changed configurations need their own explicit recovery basis; never adopt by reason.
            if crate::runtime_invalidation::configuration_digest(
                &p.context.cells[cell_id].cell.configuration,
            )
            .map_err(StoreError::Integrity)?
                != configuration
            {
                break;
            }
            let origin = crate::host_invalidation::load(tx, &p.context.producer.session)?
                .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
            let origin_digest = origin.digest().map_err(StoreError::Integrity)?;
            if p.context.replacement_digest != Some(origin_digest)
                || origin.after.principal != *host
            {
                return reject(Reject::ContinuityUnproven);
            }
            if let Some(cut) = origin.cells.get(cell_id)
                && let Some(actual) = cell.blocks.iter().find(|b| b.id == cut.block.id)
            {
                if canonical::bytes(actual).map_err(domain_error)?
                    != canonical::bytes(&cut.block).map_err(domain_error)?
                {
                    return reject(Reject::StaleRevision);
                }
                let restriction = h::Restriction {
                    cell: cell_id.clone(),
                    host: host.clone(),
                    block: cut.block.clone(),
                    origin_session: origin.after.session.clone(),
                    origin_digest,
                    rebind: rebind.id.clone(),
                    rebind_digest: rebind.digest().map_err(StoreError::Integrity)?,
                    current_rebind: latest.id.clone(),
                    current_rebind_digest: latest.digest().map_err(StoreError::Integrity)?,
                    normal_plan: step.plan.id.clone(),
                    baseline_digest: latest_base.digest().map_err(StoreError::Integrity)?,
                    configuration,
                };
                restriction.digest().map_err(StoreError::Integrity)?;
                if found
                    .insert(restriction.block.id.clone(), restriction)
                    .is_some()
                {
                    return Err(StoreError::Integrity(
                        "ambiguous Host restriction owner".into(),
                    ));
                }
            }
            plan = link.prior_plan;
        }
    }
    if found.len() > 128 {
        return reject(Reject::InvalidInput);
    }
    Ok(found.into_values().collect())
}
pub(super) fn select(
    tx: &mut dyn Transaction,
    meta: &Installation,
    input: &q::Begin,
    change: &crate::process_change::Change,
    actor: &Principal,
) -> Result<Vec<h::Restriction>> {
    if input.host_rebind_restrictions.len() > 128 {
        return reject(Reject::InvalidInput);
    }
    if input.host_rebind_restrictions.is_empty() {
        return Ok(vec![]);
    }
    let application = change
        .application
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
    let mut choices = BTreeMap::new();
    for cell in input.expected_cells.keys() {
        for r in candidates(tx, meta, cell, Some(actor))? {
            choices.insert(r.block.id.clone(), r);
        }
    }
    let mut selected = Vec::new();
    for (block, digest) in &input.host_rebind_restrictions {
        let actual = choices
            .remove(block)
            .ok_or(StoreError::Rejected(Reject::Forbidden))?;
        if actual.digest().map_err(StoreError::Integrity)? != *digest
            || !application
                .cells
                .iter()
                .any(|c| c.cell == actual.cell && c.after.sha256 == actual.configuration)
        {
            return reject(Reject::StaleRevision);
        }
        selected.push(actual);
    }
    Ok(selected)
}
pub(in crate::engine) fn current(
    tx: &mut dyn Transaction,
    meta: &Installation,
    job: &q::Job,
) -> Result<()> {
    for expected in &job.request.host_rebind_restrictions {
        let actual = candidates(tx, meta, &expected.cell, None)?
            .into_iter()
            .find(|r| r.block.id == expected.block.id)
            .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
        if actual.digest().map_err(StoreError::Integrity)?
            != expected.digest().map_err(StoreError::Integrity)?
        {
            return reject(Reject::StaleRevision);
        }
    }
    Ok(())
}
pub(super) fn bind(tx: &mut dyn Transaction, meta: &Installation, job: &q::Job) -> Result<()> {
    current(tx, meta, job)?;
    for r in &job.request.host_rebind_restrictions {
        let value = Binding {
            review: job.request.id.clone(),
            change: job.request.change.clone(),
            cell: r.cell.clone(),
            block: r.block.id.clone(),
            request_digest: job.request.digest().map_err(StoreError::Integrity)?,
            restriction_digest: r.digest().map_err(StoreError::Integrity)?,
        };
        save(
            tx,
            "host-rebind-restriction-binding",
            (&value.review, &value.block),
            None,
            BINDING,
            &value,
        )?;
    }
    Ok(())
}
pub(in crate::engine) fn owned_clear(
    tx: &mut dyn Transaction,
    job: &q::Job,
    cell: &Cell,
    block: &Block,
) -> Result<()> {
    let r = job
        .request
        .host_rebind_restrictions
        .iter()
        .find(|r| r.cell == cell.configuration.id && r.block.id == block.id)
        .ok_or(StoreError::Rejected(Reject::Forbidden))?;
    if canonical::bytes(&r.block).map_err(domain_error)?
        != canonical::bytes(block).map_err(domain_error)?
    {
        return reject(Reject::StaleRevision);
    }
    let (_, bound): (_, Binding) = load(
        tx,
        "host-rebind-restriction-binding",
        (&job.request.id, &block.id),
        BINDING,
    )?;
    if bound.review != job.request.id
        || bound.change != job.request.change
        || bound.cell != cell.configuration.id
        || bound.block != block.id
        || bound.request_digest != job.request.digest().map_err(StoreError::Integrity)?
        || bound.restriction_digest != r.digest().map_err(StoreError::Integrity)?
    {
        return reject(Reject::Forbidden);
    }
    Ok(())
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn host_rebind_restrictions(
        &mut self,
        identity: &Identity,
        cell: &Name,
    ) -> Result<h::Restrictions> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let actor = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(cell),
                Role::ReleaseManager,
                true,
            )?;
            let restrictions = candidates(tx, meta, cell, Some(&actor))?;
            Ok(h::Restrictions {
                cell: cell.clone(),
                restriction_digests: restrictions
                    .iter()
                    .map(|r| {
                        Ok((
                            r.block.id.clone(),
                            r.digest().map_err(StoreError::Integrity)?,
                        ))
                    })
                    .collect::<Result<_>>()?,
                restrictions,
                clearance_authorized: false,
            })
        })
    }
}
