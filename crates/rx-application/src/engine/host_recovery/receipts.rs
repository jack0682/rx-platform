use super::*;
use rx_domain::operation::Phase as OperationPhase;
use rx_ports::OutboxState;

fn original_work(
    tx: &mut dyn Transaction,
    b: &r::Binding,
    operation: &Id,
) -> Result<(Counter, Work, r::OperationCut)> {
    let allowed = b
        .context
        .operations
        .get(operation)
        .ok_or(StoreError::Rejected(Reject::Forbidden))?;
    let (revision, work): (_, Work) = load(tx, "work", operation, WORK)?;
    let (_, permit): (_, Permit) = load(tx, "permit", &work.permit, PERMIT)?;
    if work.host != b.context.host
        || work.cell != allowed.cell
        || work.permit != allowed.permit
        || work.host_journal != allowed.host_journal
        || work.intent.digest().map_err(domain_error)? != allowed.intent_digest
        || work.intent.profile_digest != allowed.profile_digest
        || allowed
            .invocation
            .as_ref()
            .is_some_and(|id| work.invocation.as_ref() != Some(id))
        || permit.operation != *operation
        || permit.cell != work.cell
        || permit.state == PermitState::Issued
    {
        return reject(Reject::ContinuityUnproven);
    }
    Ok((revision, work, allowed.clone()))
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// A bounded, read-only plan for an original entered operation. No generic dispatch payload.
    pub fn host_recovery_query(&mut self, binding: &Id, operation: &Id) -> Result<r::QueryPlan> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let b = read(tx, binding)?;
            if b.phase != r::Phase::RecoveryOnly {
                return reject(Reject::StaleRevision);
            }
            approved_current(tx, meta, &now, &b)?;
            let last = b
                .last_read
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
            reads::validate(tx, &now, &b.context, last, true)?;
            let (_, work, allowed) = original_work(tx, &b, operation)?;
            let mut original_receipt = None;
            for row in tx.scan("hostreceipt/")? {
                let receipt: HostReceipt = decode(&row, "rx.internal.host-receipt.v1")?;
                if receipt.operation == *operation
                    && receipt.journal == allowed.host_journal
                    && original_receipt
                        .as_ref()
                        .is_none_or(|old: &HostReceipt| old.sequence < receipt.sequence)
                {
                    original_receipt = Some(receipt);
                }
            }
            Ok(r::QueryPlan {
                binding: b.id,
                host: b.context.host,
                producer_session: b.context.producer.session,
                platform_session: b.proposal_read.platform_session,
                operation: allowed,
                original_receipt,
                lookup_allowed: work.invocation.is_some()
                    && work.operation.outcome() == Outcome::None,
            })
        })
    }
    /// Record a response to the original receipt query, including one arriving after CAS loss.
    /// Unlike normal Prepare receipt handling this path never enqueues Authorize.
    pub fn record_host_recovery_receipt(
        &mut self,
        binding: &Id,
        message: &Id,
        receipt: HostReceipt,
    ) -> Result<Work> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let mut b = read(tx, binding)?;
            if b.approved_by.is_none() || !matches!(b.phase, r::Phase::RecoveryOnly | r::Phase::Attention) { return reject(Reject::Forbidden); }
            let (revision, mut work, allowed) = original_work(tx, &b, &receipt.operation)?;
            if !allowed.messages.contains(message) || receipt.digest != allowed.intent_digest
                || receipt.journal != allowed.host_journal || receipt.sequence.0 == 0
                || (receipt.invocation.is_none() && receipt.state != ReceiptState::VoidedBeforeSend)
                || receipt.invocation.as_ref().is_some_and(|received| work.invocation.as_ref().is_some_and(|known| known != received)) {
                return reject(Reject::ContinuityUnproven);
            }
            let outbox = tx.outbox(message)?.ok_or(StoreError::Rejected(Reject::NotFound))?;
            let payload: Delivery = canonical::decode_json(&canonical::bytes(&outbox.document.value).map_err(domain_error)?).map_err(domain_error)?;
            let prepare = matches!(&payload, Delivery::Prepare { operation, host, permit } if operation == &receipt.operation && host == &b.context.host && permit == &allowed.permit);
            let authorize = matches!(&payload, Delivery::Authorize { operation, host, permit, .. } if operation == &receipt.operation && host == &b.context.host && permit == &allowed.permit);
            if (!prepare && !authorize) || !matches!(outbox.state, OutboxState::EmitEntered | OutboxState::Delivered) {
                return reject(Reject::ContinuityUnproven);
            }
            let source = key("hostreceipt", (&b.context.host, &receipt.journal, receipt.sequence));
            let document = doc("rx.internal.host-receipt.v1", &receipt)?;
            if let Some(old) = tx.get(&source)? {
                if old.document != document { return Err(StoreError::Integrity("recovery receipt sequence conflict".into())); }
            } else {
                tx.put(&source, None, &document)?;
                let evidence = super::super::delivery::key_id(&receipt.journal, &receipt.sequence.0.to_string());
                let evidence_key = key("receiptevidence", &evidence);
                if let Some(old) = tx.get(&evidence_key)? {
                    if old.document != document { return Err(StoreError::Integrity("recovery receipt evidence differs".into())); }
                } else { tx.put(&evidence_key, None, &document)?; }
                let newly_correlated = work.invocation.is_none() && receipt.invocation.is_some();
                if work.invocation.is_none() { work.invocation = receipt.invocation.clone(); }
                match receipt.state {
                    ReceiptState::VoidedBeforeSend if work.operation.outcome() == Outcome::None => {
                        work.operation.conclude(rx_domain::operation::Conclusion { outcome: Outcome::NotExecuted, evidence_ids: vec![evidence] }).map_err(domain_error)?;
                    }
                    ReceiptState::SendEntered | ReceiptState::NativeAccepted | ReceiptState::NativeRejected | ReceiptState::ResultCaptured
                        if work.operation.phase() == OperationPhase::Admitted => {
                        work.operation.sent().map_err(domain_error)?;
                    }
                    _ => {}
                }
                save(tx, "work", work.operation.id(), Some(revision), WORK, &work)?;
                if newly_correlated {
                    let (_, principal): (_, Principal) = load(tx, "principal", &b.context.host, PRINCIPAL)?;
                    super::super::evidence::reapply_correlated_native(tx, &principal, work.operation.id(), &now)?;
                }
                event(tx, "rx.event.host-receipt-recorded.v1", &receipt)?;
            }
            if (prepare || receipt.state != ReceiptState::Prepared) && outbox.state == OutboxState::EmitEntered {
                tx.transition_outbox(message, OutboxState::EmitEntered, OutboxState::Delivered)?;
            }
            let proof_key = key("host-recovery-receipt-proof", (binding, &receipt.journal, receipt.sequence));
            let proof = doc("rx.host-recovery-receipt-proof.v1", &(message, &receipt))?;
            if let Some(old) = tx.get(&proof_key)? {
                if old.document != proof { return Err(StoreError::KeyConflict); }
            } else { tx.put(&proof_key, None, &proof)?; }
            let previous = b.revision;
            let before = digest(&b)?;
            let current = approved_current(tx, meta, &now, &b);
            retain_attention(&mut b, current)?;
            if digest(&b)? != before { save_binding(tx, &mut b, Some(previous))?; }
            Ok(load(tx, "work", &receipt.operation, WORK)?.1)
        })
    }
}
