use super::*;
use crate::host_recovery::{self as h, FencePhase, Phase};
const PREFIX: &str = "host-rejoin-binding";
const REFERENCE: &str = "rx.host-rejoin-reference.v1";
fn same<T: Serialize>(a: &T, b: &T) -> Result<bool> {
    Ok(canonical::bytes(a).map_err(domain_error)? == canonical::bytes(b).map_err(domain_error)?)
}
pub(super) fn load_binding(tx: &mut dyn Transaction, id: &Id) -> Result<(r::Binding, r::Proposal)> {
    let (revision, b): (_, r::Binding) = load(tx, PREFIX, id, r::BINDING_SCHEMA)?;
    let p = proposal::load_proposal(tx, id)?;
    let cells = p
        .context
        .cells
        .iter()
        .filter(|(_, v)| v.cell.configuration.hosts.contains(&p.context.host))
        .map(|(k, _)| k)
        .collect::<BTreeSet<_>>();
    if b.id != *id
        || b.revision != revision
        || b.schema.as_str() != r::BINDING_SCHEMA
        || b.proposal_digest != p.digest().map_err(StoreError::Integrity)?
        || p.recovery_scope.is_none()
        || b.fences.keys().collect::<BTreeSet<_>>() != cells
        || b.fences.values().any(|f| {
            f.task.binding != b.id
                || f.task.digest().ok() != Some(f.payload_digest)
                || (f.phase == FencePhase::Acknowledged) != f.acknowledgment.is_some()
        })
    {
        return Err(StoreError::Integrity(
            "rejoin binding/proposal differs".into(),
        ));
    }
    Ok((b, p))
}
fn save_binding(
    tx: &mut dyn Transaction,
    b: &mut r::Binding,
    previous: Option<Counter>,
) -> Result<()> {
    b.revision = previous.map_or(Ok(Counter(1)), |v| v.increment().map_err(domain_error))?;
    save(tx, PREFIX, &b.id, previous, r::BINDING_SCHEMA, b)?;
    event(tx, "rx.event.host-rejoin-binding.v1", b)
}
pub(super) fn current(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    b: &r::Binding,
    p: &r::Proposal,
) -> Result<()> {
    let identity = b.approved_by.identity();
    proposal::access(tx, meta, now, &identity, &p.context)?;
    let c = build(
        tx,
        meta,
        now.clone(),
        &identity,
        &p.context.host,
        &p.context.origin,
    )?;
    if !c.local_prerequisites_current
        || c.digest().map_err(StoreError::Integrity)? != p.context_digest
    {
        return reject(Reject::StaleRevision);
    }
    let (_, owner): (_, Id) = load(tx, "host-rejoin-owner", &c.host, REFERENCE)?;
    if owner != b.id {
        return reject(Reject::StaleRevision);
    }
    Ok(())
}
pub(super) fn settled_read(
    tx: &mut dyn Transaction,
    now: &TimePoint,
    p: &r::Proposal,
    evidence: &h::ReadEvidence,
) -> Result<()> {
    reads::validate(tx, now, &p.context, evidence)?;
    for (cell, actual) in &evidence.cells {
        let cut = &p.context.cells[cell].cell;
        if actual.snapshot.epoch != cut.epoch
            || actual.snapshot.scopes != cut.scope_epochs
            || actual.snapshot.block_ids.iter().collect::<BTreeSet<_>>()
                != cut
                    .blocks
                    .iter()
                    .filter(|b| b.latched)
                    .map(|b| &b.id)
                    .collect()
        {
            return reject(Reject::ContinuityUnproven);
        }
    }
    Ok(())
}
fn is_current(result: Result<()>) -> Result<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(StoreError::Rejected(_)) => Ok(false),
        Err(e) => Err(e),
    }
}
fn view(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    identity: &Identity,
    b: r::Binding,
    p: r::Proposal,
) -> Result<r::BindingView> {
    proposal::access(tx, meta, now, identity, &p.context)?;
    let context_current = is_current(current(tx, meta, now, &b, &p))?;
    let read_current = if context_current && b.phase == Phase::RecoveryOnly {
        match &b.last_read {
            Some(read) => is_current(settled_read(tx, now, &p, read))?,
            None => false,
        }
    } else {
        false
    };
    Ok(r::BindingView {
        binding: b,
        proposal: p,
        context_current,
        read_current,
        operation_authorized: false,
    })
}
fn attention(
    tx: &mut dyn Transaction,
    b: &mut r::Binding,
    error: &StoreError,
    read: Option<h::ReadEvidence>,
) -> Result<()> {
    let revision = b.revision;
    b.phase = Phase::Attention;
    b.detail = Some(error.to_string());
    if let Some(read) = read {
        b.last_read = Some(read);
    }
    save_binding(tx, b, Some(revision))
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn approve_host_rejoin(
        &mut self,
        identity: &Identity,
        request_key: &Id,
        input: r::Approve,
    ) -> Result<r::BindingView> {
        let candidates = host_recovery::fences::candidates(&mut self.repository)?;
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let p = proposal::load_proposal(tx, &input.id)?;
            let actor = proposal::access(tx, meta, &now, identity, &p.context)?;
            let (scope, fp) = request(
                meta,
                &actor,
                "HostRejoin.Approve",
                request_key.as_str(),
                &input,
            )?;
            if let Some(old) = prior::<Id>(tx, &scope, fp, REFERENCE)? {
                let (b, p) = load_binding(tx, &old)?;
                return view(tx, meta, &now, identity, b, p);
            }
            if p.revision != input.expected_revision
                || p.digest().map_err(StoreError::Integrity)? != input.proposal_digest
                || p.context.expected_cells() != input.expected_cells
            {
                return reject(Reject::StaleRevision);
            }
            let proposal_view = proposal::view(tx, meta, now.clone(), identity, p.clone())?;
            if !proposal_view.context_current {
                return reject(Reject::StaleRevision);
            }
            let operations = p
                .recovery_scope
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
            let cells = p
                .context
                .cells
                .iter()
                .filter(|(_, v)| v.cell.configuration.hosts.contains(&p.context.host))
                .map(|(k, _)| k.clone())
                .collect::<Vec<_>>();
            if !same(
                operations,
                &host_recovery::original_operations(tx, &p.context.host, &cells)?,
            )? {
                return reject(Reject::StaleRevision);
            }
            if tx.get(&key(PREFIX, &p.id))?.is_some() {
                return reject(Reject::Busy);
            }
            let owner_key = key("host-rejoin-owner", &p.context.host);
            let previous = tx.get(&owner_key)?;
            if let Some(row) = &previous {
                let old: Id = decode(row, REFERENCE)?;
                let (old, oldp) = load_binding(tx, &old)?;
                if matches!(old.phase, Phase::Fencing | Phase::RecoveryOnly)
                    && is_current(current(tx, meta, &now, &old, &oldp))?
                {
                    return reject(Reject::Busy);
                }
            }
            let mut fences = BTreeMap::new();
            for cell in cells {
                let cut = &p.context.cells[&cell].cell;
                let mut task = h::FenceTask {
                    binding: p.id.clone(),
                    cell: cell.clone(),
                    request: id(),
                    originating_message: None,
                    epoch: cut.epoch,
                    scopes: cut.scope_epochs.clone(),
                    block_ids: cut
                        .blocks
                        .iter()
                        .filter(|b| b.latched)
                        .map(|b| b.id.clone())
                        .collect(),
                };
                let matching =
                    host_recovery::fences::matching(tx, &p.context.host, &task, &candidates)?;
                if matching.len() > 1 {
                    return reject(Reject::ContinuityUnproven);
                }
                if let Some(message) = matching.into_iter().next() {
                    task.request = message.clone();
                    task.originating_message = Some(message);
                }
                fences.insert(
                    cell,
                    h::FenceStep {
                        payload_digest: task.digest().map_err(StoreError::Invalid)?,
                        task,
                        phase: FencePhase::Pending,
                        acknowledgment: None,
                    },
                );
            }
            let mut b = r::Binding {
                schema: name(r::BINDING_SCHEMA),
                id: p.id.clone(),
                revision: Counter(1),
                proposal_digest: input.proposal_digest,
                approved_by: identity.into(),
                approved_at: now.clone(),
                phase: Phase::Fencing,
                fences,
                last_read: None,
                detail: None,
            };
            save_binding(tx, &mut b, None)?;
            tx.put(
                &owner_key,
                previous.map(|v| v.revision),
                &doc(REFERENCE, &b.id)?,
            )?;
            remember(tx, &scope, fp, REFERENCE, &b.id)?;
            view(tx, meta, &now, identity, b, p)
        })
    }
    pub fn host_rejoin_binding(&mut self, identity: &Identity, id: &Id) -> Result<r::BindingView> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (b, p) = load_binding(tx, id)?;
            view(tx, meta, &now, identity, b, p)
        })
    }
    pub fn plan_host_rejoin_fence(
        &mut self,
        id: &Id,
        cell: &Name,
        verified: h::VerifiedRead,
    ) -> Result<h::FenceTask> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (mut b, p) = load_binding(tx, id)?;
            if b.phase != Phase::Fencing {
                return reject(Reject::StaleRevision);
            }
            let check = current(tx, meta, &now, &b, &p)
                .and_then(|_| reads::validate(tx, &now, &p.context, &verified.0));
            if let Err(e) = check {
                if matches!(e, StoreError::Rejected(_)) {
                    attention(tx, &mut b, &e, Some(verified.0))?;
                    return Ok(Err(e));
                }
                return Err(e);
            }
            let revision = b.revision;
            let step = b
                .fences
                .get_mut(cell)
                .ok_or(StoreError::Rejected(Reject::Forbidden))?;
            if step.phase == FencePhase::Acknowledged {
                return reject(Reject::StaleRevision);
            }
            host_recovery::fences::enter(tx, &p.context.host, &step.task)?;
            let task = step.task.clone();
            step.phase = FencePhase::SendEntered;
            b.last_read = Some(verified.0);
            save_binding(tx, &mut b, Some(revision))?;
            Ok(Ok(task))
        })?
    }
    pub fn record_host_rejoin_fence(
        &mut self,
        id: &Id,
        cell: &Name,
        ack: FenceAcknowledgment,
    ) -> Result<r::Binding> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (mut b, p) = load_binding(tx, id)?;
            let step = b
                .fences
                .get(cell)
                .ok_or(StoreError::Rejected(Reject::Forbidden))?;
            let base = p.context.cells[cell]
                .baseline
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
            if step.phase == FencePhase::Pending
                || ack.cell != *cell
                || ack.invalidation != step.task.request
                || ack.epoch != step.task.epoch
                || ack.scopes != step.task.scopes
                || ack.sequence.0 == 0
                || ack.host_boot != p.context.producer.peer_boot
                || ack.journal != base.delivery_journal
            {
                return reject(Reject::ContinuityUnproven);
            }
            if let Some(old) = &step.acknowledgment
                && !same(old, &ack)?
            {
                return Err(StoreError::KeyConflict);
            }
            host_recovery::fences::complete(tx, &p.context.host, &step.task)?;
            let source = key("fenceack", (&p.context.host, &ack.journal, ack.sequence));
            let document = doc("rx.internal.fence-ack.v1", &ack)?;
            if let Some(old) = tx.get(&source)? {
                if old.document != document {
                    return Err(StoreError::Integrity(
                        "rejoin fence sequence conflict".into(),
                    ));
                }
            } else {
                tx.put(&source, None, &document)?;
            }
            let previous = b.revision;
            let before = canonical::bytes(&b).map_err(domain_error)?;
            let step = b.fences.get_mut(cell).unwrap();
            step.phase = FencePhase::Acknowledged;
            step.acknowledgment = Some(ack);
            match current(tx, meta, &now, &b, &p) {
                Ok(()) => {}
                Err(e @ StoreError::Rejected(_)) => {
                    b.phase = Phase::Attention;
                    b.detail = Some(e.to_string());
                }
                Err(e) => return Err(e),
            }
            if before != canonical::bytes(&b).map_err(domain_error)? {
                save_binding(tx, &mut b, Some(previous))?;
            }
            Ok(b)
        })
    }
    /// Every completion/refresh uses a new actual read; no proposal timestamp refresh.
    pub fn refresh_host_rejoin(
        &mut self,
        id: &Id,
        verified: h::VerifiedRead,
    ) -> Result<r::Binding> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (mut b, p) = load_binding(tx, id)?;
            if !matches!(b.phase, Phase::Fencing | Phase::RecoveryOnly)
                || b.fences
                    .values()
                    .any(|f| f.phase != FencePhase::Acknowledged)
            {
                return reject(Reject::HostNotPrepared);
            }
            let check = current(tx, meta, &now, &b, &p)
                .and_then(|_| settled_read(tx, &now, &p, &verified.0));
            if let Err(e) = check {
                if matches!(e, StoreError::Rejected(_)) {
                    attention(tx, &mut b, &e, Some(verified.0))?;
                    return Ok(b);
                }
                return Err(e);
            }
            let previous = b.revision;
            b.phase = Phase::RecoveryOnly;
            b.last_read = Some(verified.0);
            save_binding(tx, &mut b, Some(previous))?;
            Ok(b)
        })
    }
}

pub(super) fn original_work(
    tx: &mut dyn Transaction,
    p: &r::Proposal,
    operation: &Id,
) -> Result<(Work, h::OperationCut)> {
    let allowed = p
        .recovery_scope
        .as_ref()
        .and_then(|s| s.get(operation))
        .ok_or(StoreError::Rejected(Reject::Forbidden))?;
    let (_, work): (_, Work) = load(tx, "work", operation, WORK)?;
    let (_, permit): (_, Permit) = load(tx, "permit", &work.permit, PERMIT)?;
    if work.host != p.context.host
        || work.cell != allowed.cell
        || work.permit != allowed.permit
        || work.host_journal != allowed.host_journal
        || work.intent.digest().map_err(domain_error)? != allowed.intent_digest
        || work.intent.profile_digest != allowed.profile_digest
        || allowed
            .invocation
            .as_ref()
            .is_some_and(|i| work.invocation.as_ref() != Some(i))
        || permit.operation != *operation
        || permit.cell != work.cell
        || permit.state == PermitState::Issued
    {
        return reject(Reject::ContinuityUnproven);
    }
    Ok((work, allowed.clone()))
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn host_rejoin_query_plan(&mut self, id: &Id, operation: &Id) -> Result<h::QueryPlan> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (b, p) = load_binding(tx, id)?;
            if b.phase != Phase::RecoveryOnly {
                return reject(Reject::Forbidden);
            }
            current(tx, meta, &now, &b, &p)?;
            let last = b
                .last_read
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
            settled_read(tx, &now, &p, last)?;
            let (work, allowed) = original_work(tx, &p, operation)?;
            let mut original_receipt = None;
            for row in tx.scan("hostreceipt/")? {
                let receipt: HostReceipt = decode(&row, "rx.internal.host-receipt.v1")?;
                if receipt.operation == *operation
                    && receipt.journal == allowed.host_journal
                    && original_receipt
                        .as_ref()
                        .is_none_or(|r: &HostReceipt| r.sequence < receipt.sequence)
                {
                    original_receipt = Some(receipt);
                }
            }
            Ok(h::QueryPlan {
                binding: b.id,
                host: p.context.host,
                producer_session: p.context.producer.session,
                platform_session: last.platform_session.clone(),
                operation: allowed,
                original_receipt,
                lookup_allowed: work.invocation.is_some(),
            })
        })
    }
    /// Keep correlated late facts, without updating Work, recording a terminal outcome or admitting a command.
    pub fn record_host_rejoin_query(
        &mut self,
        query: r::VerifiedQuery,
    ) -> Result<r::QueryObservation> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let (b, p) = load_binding(tx, &query.binding)?;
            if !matches!(b.phase, Phase::RecoveryOnly | Phase::Attention)
                || b.fences
                    .values()
                    .any(|f| f.phase != FencePhase::Acknowledged)
            {
                return reject(Reject::Forbidden);
            }
            let (work, allowed) = original_work(tx, &p, &query.operation)?;
            if let Some(receipt) = &query.receipt {
                if receipt.operation != query.operation
                    || receipt.digest != allowed.intent_digest
                    || receipt.journal != allowed.host_journal
                    || receipt.sequence.0 == 0
                    || receipt
                        .invocation
                        .as_ref()
                        .is_some_and(|i| work.invocation.as_ref().is_some_and(|w| w != i))
                    || (receipt.invocation.is_none()
                        && receipt.state != ReceiptState::VoidedBeforeSend)
                {
                    return reject(Reject::ContinuityUnproven);
                }
                let source = key(
                    "hostreceipt",
                    (&p.context.host, &receipt.journal, receipt.sequence),
                );
                if let Some(old) = tx.get(&source)?
                    && old.document != doc("rx.internal.host-receipt.v1", receipt)?
                {
                    return Err(StoreError::Integrity(
                        "rejoin query receipt conflicts with original".into(),
                    ));
                }
            }
            let invocation = work
                .invocation
                .as_ref()
                .or_else(|| query.receipt.as_ref().and_then(|r| r.invocation.as_ref()));
            let mut evidence = Vec::new();
            event(
                tx,
                "rx.event.host-rejoin-query-raw.v1",
                &(
                    &query.binding,
                    &query.operation,
                    &query.receipt,
                    &query.batch,
                    query.lookup,
                ),
            )?;
            if let Some(batch) = query.batch {
                if batch.journal != p.context.producer.journal
                    || batch.first.0 == 0
                    || batch.records.len() > 128
                {
                    return reject(Reject::ContinuityUnproven);
                }
                for item in batch
                    .records
                    .into_iter()
                    .filter(|r| r.operation == query.operation)
                {
                    if Some(&item.invocation) != invocation
                        || item.profile_digest != allowed.profile_digest
                        || now.age_ns(&item.captured_at).is_none()
                    {
                        return reject(Reject::ContinuityUnproven);
                    }
                    evidence.push(item);
                }
            }
            let context_current_at_record =
                is_current(current(tx, meta, &now, &b, &p).and_then(|_| {
                    settled_read(
                        tx,
                        &now,
                        &p,
                        b.last_read
                            .as_ref()
                            .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?,
                    )
                }))?;
            let result = h::QueryResult {
                binding: b.id.clone(),
                operation: query.operation.clone(),
                receipt: query.receipt,
                lookup: query.lookup,
                evidence,
                evidence_complete: false,
                publication_required: matches!(
                    query.lookup,
                    h::QueryLookup::PrefixObserved | h::QueryLookup::Unavailable
                ),
                operation_authorized: false,
            };
            let record = r::QueryObservation {
                id: id(),
                binding: b.id,
                operation: query.operation,
                observed_at: now,
                context_current_at_record,
                result,
            };
            save(
                tx,
                "host-rejoin-query",
                &record.id,
                None,
                "rx.host-rejoin-query.v1",
                &record,
            )?;
            event(tx, "rx.event.host-rejoin-query.v1", &record)?;
            Ok(record)
        })
    }
}
