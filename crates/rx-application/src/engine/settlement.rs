use super::*;
use crate::settlement::{Approve, Authorization};
use rx_domain::operation::{Disposition, Phase};
use rx_ports::OutboxState;
pub(super) const AUTH: &str = "rx.internal.settlement-authorization.v1";
pub(super) const CURRENT: &str = "rx.internal.settlement-current.v1";
pub(super) const TTL: u64 = 30_000_000_000;
pub(super) fn configuration(cell: &Cell) -> Result<Digest> {
    canonical::digest("RX-SETTLEMENT-CONFIGURATION-v1", &cell.configuration).map_err(domain_error)
}
pub(super) fn known(work: &Work) -> Result<()> {
    if work.intent.kind != rx_domain::intent::Kind::FiniteAction
        || work.operation.phase() != Phase::Settled
        || work.operation.outcome() != Outcome::Succeeded
        || work.operation.integrity() != Integrity::Valid
        || work.invocation.is_none()
    {
        return reject(Reject::ConditionUnknown);
    }
    Ok(())
}
fn fenced(tx: &mut dyn Transaction, cell: &Cell, host: &HostRegistration) -> Result<()> {
    if host.epoch != cell.epoch || host.scopes != cell.scope_epochs {
        return reject(Reject::ContinuityUnproven);
    }
    let blocks = cell
        .blocks
        .iter()
        .map(|b| b.id.clone())
        .collect::<BTreeSet<_>>();
    for row in tx.scan("fenceack/")? {
        let ack: FenceAcknowledgment = decode(&row, "rx.internal.fence-ack.v1")?;
        if ack.cell != cell.configuration.id
            || ack.host_boot != host.boot_id
            || ack.journal != host.delivery_journal
            || ack.epoch != cell.epoch
            || ack.scopes != cell.scope_epochs
        {
            continue;
        }
        if let Some(message) = tx.outbox(&ack.invalidation)?
            && message.state == OutboxState::Delivered
        {
            let payload: Delivery = canonical::decode_json(
                &canonical::bytes(&message.document.value).map_err(domain_error)?,
            )
            .map_err(domain_error)?;
            if matches!(payload,Delivery::Fence{cell:c,host:h,epoch,scopes,block_ids} if c==cell.configuration.id && h==host.id && epoch==cell.epoch && scopes==cell.scope_epochs && block_ids.iter().cloned().collect::<BTreeSet<_>>()==blocks)
            {
                return Ok(());
            }
        }
    }
    reject(Reject::ContinuityUnproven)
}
fn context(
    tx: &mut dyn Transaction,
    work: &Work,
    cell: &Cell,
    host: &HostRegistration,
) -> Result<()> {
    known(work)?;
    if work.operation.disposition() != Disposition::Quarantined
        || cell.blocks.is_empty()
        || cell.blocks.iter().any(|b| {
            !matches!(
                b.reason,
                BlockReason::AuthorityRevoked | BlockReason::RuntimeRestart
            )
        })
    {
        return reject(Reject::ContinuityUnproven);
    }
    let (_, run): (_, Run) = load(tx, "run", &work.run, RUN)?;
    let (_, permit): (_, Permit) = load(tx, "permit", &work.permit, PERMIT)?;
    let (_, mandate): (_, Mandate) = load(tx, "mandate", &permit.mandate, MANDATE)?;
    if run.state != RunState::RecoveryRequired
        || mandate.state == MandateState::Active
        || permit.state != PermitState::Consumed
        || permit.host_boot != host.boot_id
        || work.host_journal != host.delivery_journal
        || work.host != host.id
    {
        return reject(Reject::ContinuityUnproven);
    }
    let original = run_configuration::read(tx, &run, &cell.configuration)?;
    if canonical::bytes(&original).map_err(domain_error)?
        != canonical::bytes(&cell.configuration).map_err(domain_error)?
    {
        return reject(Reject::ContinuityUnproven);
    }
    fenced(tx, cell, host)
}
pub(super) fn pending_authorization(
    tx: &mut dyn Transaction,
    operation: &Id,
) -> Result<Option<Id>> {
    let Some(row) = tx.get(&key("settlement-current", operation))? else {
        return Ok(None);
    };
    let id: Id = decode(&row, CURRENT)?;
    let (_, auth): (_, Authorization) = load(tx, "settlement", &id, AUTH)?;
    Ok(auth.applied_at.is_none().then_some(id))
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn approve_settlement(
        &mut self,
        identity: &Identity,
        key_: &str,
        command: Approve,
    ) -> Result<Authorization> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (_, work): (_, Work) = load(tx, "work", &command.operation, WORK)?;
            let actor = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&work.cell),
                Role::ReleaseManager,
                true,
            )?;
            let (scope, fingerprint) =
                request(meta, &actor, "Recovery.ApproveSettlement", key_, &command)?;
            if let Some(saved) = prior(tx, &scope, fingerprint, AUTH)? {
                return Ok(saved);
            }
            if command.justification.trim().is_empty() || command.justification.len() > 2048 {
                return reject(Reject::InvalidInput);
            }
            check_revision(work.operation.revision(), command.expected_operation)?;
            let (revision, cell): (_, Cell) = load(tx, "cell", &work.cell, CELL)?;
            check_revision(revision, command.expected_cell)?;
            let (_, host): (_, HostRegistration) =
                load(tx, "host", (&work.cell, &work.host), HOST)?;
            context(tx, &work, &cell, &host)?;
            let mut resources = BTreeMap::new();
            for resource in &work.intent.resource_set {
                let (_, r): (_, Resource) = load(tx, "resource", resource, RESOURCE)?;
                if r.holder.as_ref() != Some(work.operation.id()) || !r.quarantined {
                    return reject(Reject::Busy);
                }
                resources.insert(resource.clone(), r.fence);
            }
            let auth = Authorization {
                id: id(),
                operation: command.operation.clone(),
                operation_revision: work.operation.revision(),
                intent_digest: work.intent.digest().map_err(domain_error)?,
                runtime_boot: meta.runtime_boot.clone(),
                cell: work.cell.clone(),
                cell_epoch: cell.epoch,
                scopes: cell.scope_epochs.clone(),
                configuration: configuration(&cell)?,
                host: host.id,
                host_boot: host.boot_id,
                host_session: host.session,
                host_journal: host.delivery_journal,
                resource_fences: resources,
                approved_by: identity.into(),
                approved_at: now.clone(),
                valid_until: TimePoint {
                    clock_id: now.clock_id.clone(),
                    ticks_ns: Counter(
                        now.ticks_ns
                            .0
                            .checked_add(TTL)
                            .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
                    ),
                },
                justification: command.justification,
                applied_at: None,
            };
            save(tx, "settlement", &auth.id, None, AUTH, &auth)?;
            let k = key("settlement-current", &auth.operation);
            let old = tx.get(&k)?;
            tx.put(&k, old.map(|r| r.revision), &doc(CURRENT, &auth.id)?)?;
            let rk = key("reconciliation", &auth.operation);
            let old = tx.get(&rk)?;
            let generation = if let Some(row) = &old {
                decode::<ReconciliationRequest>(row, "rx.internal.reconciliation-request.v1")?
                    .generation
                    .increment()
                    .map_err(domain_error)?
            } else {
                Counter(1)
            };
            let plan = ReconciliationRequest {
                id: id(),
                operation: auth.operation.clone(),
                cell: auth.cell.clone(),
                host: auth.host.clone(),
                generation,
                requested_by: actor.id,
                state: ReconciliationState::Pending,
                issue: None,
            };
            tx.put(
                &rk,
                old.map(|r| r.revision),
                &doc("rx.internal.reconciliation-request.v1", &plan)?,
            )?;
            remember(tx, &scope, fingerprint, AUTH, &auth)?;
            event(tx, "rx.event.settlement-approved.v1", &auth)?;
            Ok(auth)
        })
    }
    pub fn settle_resources(
        &mut self,
        identity: &Identity,
        authorization: &Id,
        command: ReleaseResources,
    ) -> Result<Work> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (ar, mut auth): (_, Authorization) = load(tx, "settlement", authorization, AUTH)?;
            let actor = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&auth.cell),
                Role::Host,
                false,
            )?;
            if actor.id != auth.host || command.operation != auth.operation {
                return reject(Reject::Forbidden);
            }
            let (wr, mut work): (_, Work) = load(tx, "work", &auth.operation, WORK)?;
            if auth.applied_at.is_some() {
                return Ok(work);
            }
            authorize(
                tx,
                &auth.approved_by.identity(),
                meta,
                &now,
                Some(&auth.cell),
                Role::ReleaseManager,
                true,
            )?;
            let (cell_revision, cell): (_, Cell) = load(tx, "cell", &auth.cell, CELL)?;
            check_revision(cell_revision, command.expected_cell)?;
            check_revision(work.operation.revision(), command.expected_operation)?;
            let (_, host): (_, HostRegistration) =
                load(tx, "host", (&auth.cell, &auth.host), HOST)?;
            if auth.id != *authorization
                || work.cell != auth.cell
                || work.host != auth.host
                || auth
                    .resource_fences
                    .keys()
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    != work.intent.resource_set.iter().cloned().collect()
                || pending_authorization(tx, &auth.operation)?.as_ref() != Some(authorization)
                || auth.runtime_boot != meta.runtime_boot
                || now.clock_id != auth.valid_until.clock_id
                || now.ticks_ns >= auth.valid_until.ticks_ns
                || auth.cell_epoch != cell.epoch
                || auth.scopes != cell.scope_epochs
                || auth.configuration != configuration(&cell)?
                || auth.operation_revision != work.operation.revision()
                || auth.intent_digest != work.intent.digest().map_err(domain_error)?
                || host.session != identity.session
                || host.session != auth.host_session
                || host.boot_id != auth.host_boot
                || host.delivery_journal != auth.host_journal
            {
                return reject(Reject::ContinuityUnproven);
            }
            context(tx, &work, &cell, &host)?;
            for (resource, fence) in &auth.resource_fences {
                let (_, r): (_, Resource) = load(tx, "resource", resource, RESOURCE)?;
                if r.holder.as_ref() != Some(&auth.operation) || r.fence != *fence || !r.quarantined
                {
                    return reject(Reject::Busy);
                }
            }
            handover::release_confirmed(
                tx,
                wr,
                &mut work,
                &host.boot_id,
                &now,
                &command.observations,
            )?;
            if let Some(part_id) = &work.part {
                let (pr, mut part): (_, PartAttempt) = load(tx, "part", part_id, PART)?;
                let (rr, mut run): (_, Run) = load(tx, "run", &work.run, RUN)?;
                match handover::complete_part_transition(tx, &cell, &mut run, rr, &mut part, pr) {
                    Ok(_) => {}
                    Err(StoreError::Rejected(Reject::ConditionUnknown)) => {}
                    Err(error) => return Err(error),
                }
            }
            auth.applied_at = Some(now);
            save(tx, "settlement", authorization, Some(ar), AUTH, &auth)?;
            event(tx, "rx.event.settlement-applied.v1", &auth)?;
            Ok(work)
        })
    }
}
