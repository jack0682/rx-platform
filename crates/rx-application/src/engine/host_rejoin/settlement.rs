use super::super::settlement as common;
use super::*;
use crate::settlement::Authorization;
use rx_domain::operation::Disposition;
const REQUEST: &str = "rx.host-rejoin-settlement-reference.v1";

/// Per-operation continuity anchors, not a claim of complete journal-prefix preservation.
fn context(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    reference: &r::SettlementReference,
    operation: &Id,
) -> Result<(r::Binding, r::Proposal, Work, Cell, Counter)> {
    let (b, p) = binding::load_binding(tx, &reference.binding)?;
    if b.phase != crate::host_recovery::Phase::RecoveryOnly {
        return reject(Reject::ContinuityUnproven);
    }
    binding::current(tx, meta, now, &b, &p)?;
    let (work, _) = binding::original_work(tx, &p, operation)?;
    common::known(&work)?;
    let (revision, cell): (_, Cell) = load(tx, "cell", &work.cell, CELL)?;
    let origin = p
        .context
        .replacement
        .as_ref()
        .and_then(|o| o.cells.get(&work.cell))
        .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
    if work.operation.disposition() != Disposition::Quarantined
        || cell.blocks.is_empty()
        || cell.blocks.iter().any(|v| {
            v.reason != BlockReason::AuthorityRevoked
                && !(v.reason == BlockReason::DeviceRestart && v.id == origin.block.id)
        })
    {
        return reject(Reject::ContinuityUnproven);
    }
    let old = p.context.cells[&work.cell]
        .registration
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
    let (_, permit): (_, Permit) = load(tx, "permit", &work.permit, PERMIT)?;
    let (_, run): (_, Run) = load(tx, "run", &work.run, RUN)?;
    let (_, mandate): (_, Mandate) = load(tx, "mandate", &permit.mandate, MANDATE)?;
    if permit.state != PermitState::Consumed
        || permit.host_boot != old.boot_id
        || permit.host != work.host
        || work.host_journal != old.delivery_journal
        || run.state != RunState::RecoveryRequired
        || mandate.state == MandateState::Active
        || !same(
            &run_configuration::read(tx, &run, &cell.configuration)?,
            &cell.configuration,
        )?
    {
        return reject(Reject::ContinuityUnproven);
    }
    let (_, query): (_, r::QueryObservation) = load(
        tx,
        "host-rejoin-query",
        &reference.query,
        "rx.host-rejoin-query.v1",
    )?;
    if query.id != reference.query
        || query.binding != reference.binding
        || query.operation != *operation
        || !query.context_current_at_record
        || query.result.binding != b.id
        || query.result.operation != *operation
        || query.result.operation_authorized
    {
        return reject(Reject::ContinuityUnproven);
    }
    let receipt = query
        .result
        .receipt
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
    if receipt.state != ReceiptState::ResultCaptured
        || receipt.journal != work.host_journal
        || receipt.operation != *operation
        || receipt.digest != work.intent.digest().map_err(domain_error)?
        || receipt.invocation != work.invocation
    {
        return reject(Reject::ContinuityUnproven);
    }
    let mut prior_receipts = 0;
    for row in tx.scan("hostreceipt/")? {
        let known: HostReceipt = decode(&row, "rx.internal.host-receipt.v1")?;
        if known.operation == *operation && known.journal == work.host_journal {
            prior_receipts += 1;
            if known.sequence > receipt.sequence
                || (known.sequence == receipt.sequence && !same(&known, receipt)?)
            {
                return reject(Reject::ContinuityUnproven);
            }
        }
    }
    if prior_receipts == 0 {
        return reject(Reject::ContinuityUnproven);
    }
    let mut originals = BTreeMap::new();
    for id in work.operation.evidence_ids() {
        if let Some(row) = tx.get(&key("evidence", id))?
            && row.document.schema.as_str() == "rx.internal.native-evidence.v1"
        {
            let value: NativeEvidence = decode(&row, "rx.internal.native-evidence.v1")?;
            if value.operation != *operation || Some(&value.invocation) != work.invocation.as_ref()
            {
                return reject(Reject::ContinuityUnproven);
            }
            originals.insert(value.id.clone(), value);
        }
    }
    let observed = query
        .result
        .evidence
        .iter()
        .map(|v| (v.id.clone(), v.clone()))
        .collect::<BTreeMap<_, _>>();
    if originals.is_empty()
        || observed.len() != query.result.evidence.len()
        || !same(&originals, &observed)?
    {
        return reject(Reject::ContinuityUnproven);
    }
    Ok((b, p, work, cell, revision))
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn approve_rejoin_settlement(
        &mut self,
        identity: &Identity,
        key_: &Id,
        command: r::ApproveSettlement,
    ) -> Result<Authorization> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (_, p) = binding::load_binding(tx, &command.reference.binding)?;
            let actor = proposal::access(tx, meta, &now, identity, &p.context)?;
            let (scope, fp) = request(
                meta,
                &actor,
                "HostRejoin.ApproveSettlement",
                key_.as_str(),
                &command,
            )?;
            if let Some(saved) = prior::<Id>(tx, &scope, fp, REQUEST)? {
                return Ok(load(tx, "settlement", &saved, common::AUTH)?.1);
            }
            let input = &command.settlement;
            if input.justification.trim().is_empty() || input.justification.len() > 2048 {
                return reject(Reject::InvalidInput);
            }
            let (_, p, work, cell, revision) =
                context(tx, meta, &now, &command.reference, &input.operation)?;
            check_revision(work.operation.revision(), input.expected_operation)?;
            check_revision(revision, input.expected_cell)?;
            let mut resources = BTreeMap::new();
            for id in &work.intent.resource_set {
                let (_, r): (_, Resource) = load(tx, "resource", id, RESOURCE)?;
                if r.holder.as_ref() != Some(work.operation.id()) || !r.quarantined {
                    return reject(Reject::Busy);
                }
                resources.insert(id.clone(), r.fence);
            }
            let auth = Authorization {
                id: id(),
                operation: input.operation.clone(),
                operation_revision: work.operation.revision(),
                intent_digest: work.intent.digest().map_err(domain_error)?,
                runtime_boot: meta.runtime_boot.clone(),
                cell: work.cell.clone(),
                cell_epoch: cell.epoch,
                scopes: cell.scope_epochs.clone(),
                configuration: common::configuration(&cell)?,
                host: work.host.clone(),
                host_boot: p.context.producer.peer_boot.clone(),
                host_session: p.context.producer.session.clone(),
                host_journal: work.host_journal.clone(),
                resource_fences: resources,
                approved_by: identity.into(),
                approved_at: now.clone(),
                valid_until: TimePoint {
                    clock_id: now.clock_id.clone(),
                    ticks_ns: Counter(
                        now.ticks_ns
                            .0
                            .checked_add(common::TTL)
                            .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
                    ),
                },
                justification: input.justification.clone(),
                applied_at: None,
                rejoin: Some(command.reference.clone()),
            };
            save(tx, "settlement", &auth.id, None, common::AUTH, &auth)?;
            let k = key("settlement-current", &auth.operation);
            let old = tx.get(&k)?;
            tx.put(
                &k,
                old.map(|r| r.revision),
                &doc(common::CURRENT, &auth.id)?,
            )?;
            remember(tx, &scope, fp, REQUEST, &auth.id)?;
            event(tx, "rx.event.rejoin-settlement-approved.v1", &auth)?;
            Ok(auth)
        })
    }
    pub fn apply_rejoin_settlement(
        &mut self,
        id: &Id,
        verified: r::VerifiedHandover,
    ) -> Result<Authorization> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (ar, mut auth): (_, Authorization) = load(tx, "settlement", id, common::AUTH)?;
            let reference = auth
                .rejoin
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::Forbidden))?;
            let (_, p) = binding::load_binding(tx, &reference.binding)?;
            proposal::access(tx, meta, &now, &auth.approved_by.identity(), &p.context)?;
            if auth.applied_at.is_some() {
                return Ok(auth);
            }
            let (_, p, mut work, cell, _) = context(tx, meta, &now, reference, &auth.operation)?;
            binding::settled_read(tx, &now, &p, &verified.read)?;
            if auth.id != *id
                || common::pending_authorization(tx, &auth.operation)?.as_ref() != Some(id)
                || auth.runtime_boot != meta.runtime_boot
                || now.clock_id != auth.valid_until.clock_id
                || now.ticks_ns >= auth.valid_until.ticks_ns
                || work.cell != auth.cell
                || work.host != auth.host
                || work.operation.revision() != auth.operation_revision
                || work.intent.digest().map_err(domain_error)? != auth.intent_digest
                || cell.epoch != auth.cell_epoch
                || cell.scope_epochs != auth.scopes
                || common::configuration(&cell)? != auth.configuration
                || auth.host_boot != p.context.producer.peer_boot
                || auth.host_session != p.context.producer.session
                || auth.host_journal != work.host_journal
                || auth.resource_fences.keys().collect::<BTreeSet<_>>()
                    != work.intent.resource_set.iter().collect()
            {
                return reject(Reject::ContinuityUnproven);
            }
            for (id, fence) in &auth.resource_fences {
                let (_, r): (_, Resource) = load(tx, "resource", id, RESOURCE)?;
                if r.holder.as_ref() != Some(&auth.operation) || !r.quarantined || r.fence != *fence
                {
                    return reject(Reject::Busy);
                }
            }
            let (wr, _): (_, Work) = load(tx, "work", &auth.operation, WORK)?;
            handover::release_confirmed(
                tx,
                wr,
                &mut work,
                &auth.host_boot,
                &now,
                &verified.observations,
            )?;
            if let Some(part_id) = &work.part {
                let (pr, mut part): (_, PartAttempt) = load(tx, "part", part_id, PART)?;
                let (rr, mut run): (_, Run) = load(tx, "run", &work.run, RUN)?;
                match handover::complete_part_transition(tx, &cell, &mut run, rr, &mut part, pr) {
                    Ok(_) | Err(StoreError::Rejected(Reject::ConditionUnknown)) => {}
                    Err(e) => return Err(e),
                }
            }
            let reconciliation_key = key("reconciliation", &auth.operation);
            if let Some(row) = tx.get(&reconciliation_key)? {
                let mut request: ReconciliationRequest =
                    decode(&row, "rx.internal.reconciliation-request.v1")?;
                if request.operation != auth.operation
                    || request.host != auth.host
                    || request.cell != auth.cell
                {
                    return Err(StoreError::Integrity(
                        "rejoin reconciliation identity differs".into(),
                    ));
                }
                request.state = ReconciliationState::Complete;
                request.issue = None;
                tx.put(
                    &reconciliation_key,
                    Some(row.revision),
                    &doc("rx.internal.reconciliation-request.v1", &request)?,
                )?;
            }
            auth.applied_at = Some(now);
            save(tx, "settlement", id, Some(ar), common::AUTH, &auth)?;
            event(tx, "rx.event.rejoin-settlement-applied.v1", &auth)?;
            Ok(auth)
        })
    }
}
