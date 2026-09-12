use super::*;
use rx_ports::OutboxState;

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn finish_fence_delivery(
        &mut self,
        identity: &Identity,
        message: &Id,
        ack: FenceAcknowledgment,
    ) -> Result<()> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let p=authorize(tx,identity,meta,&clock.now(),Some(&ack.cell),Role::Host,false)?;
            let row=tx.outbox(message)?.ok_or(StoreError::Rejected(Reject::NotFound))?;
            let payload:Delivery=canonical::decode_json(&canonical::bytes(&row.document.value).map_err(domain_error)?).map_err(domain_error)?;
            if !matches!(payload,Delivery::Fence {cell,host,epoch,scopes,..} if cell==ack.cell && host==p.id && epoch==ack.epoch && scopes==ack.scopes)
                || ack.invalidation!=*message || ack.sequence.0==0 {return reject(Reject::InvalidInput);}
            let (_,host):(_,HostRegistration)=load(tx,"host",(&ack.cell,&p.id),HOST)?;
            if host.boot_id!=ack.host_boot || host.delivery_journal!=ack.journal || host.session!=identity.session {return reject(Reject::ContinuityUnproven);}
            let key_=key("fenceack",(&p.id,&ack.journal,ack.sequence));let incoming=doc("rx.internal.fence-ack.v1",&ack)?;
            if let Some(old)=tx.get(&key_)? {if old.document!=incoming {return Err(StoreError::Integrity("fence receipt conflict".into()));}}
            else {tx.put(&key_,None,&incoming)?;}
            if row.state==OutboxState::EmitEntered {tx.transition_outbox(message,OutboxState::EmitEntered,OutboxState::Delivered)?;}
            else if row.state!=OutboxState::Delivered {return reject(Reject::InvalidInput);}
            let(_,cell):(_,Cell)=load(tx,"cell",&ack.cell,CELL)?;
            if cell.epoch==ack.epoch && cell.scope_epochs==ack.scopes {
                let(revision,mut host):(_,HostRegistration)=load(tx,"host",(&ack.cell,&p.id),HOST)?;
                if host.epoch!=ack.epoch || host.scopes!=ack.scopes {host.epoch=ack.epoch;host.scopes=ack.scopes;save(tx,"host",(&ack.cell,&p.id),Some(revision),HOST,&host)?;}
            }
            Ok(())
        })
    }
    pub fn reconciliation_work(
        &mut self,
        identity: &Identity,
        after: Option<&Id>,
        limit: usize,
    ) -> Result<Vec<Work>> {
        if limit == 0 || limit > 128 {
            return reject(Reject::InvalidInput);
        }
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let p = authorize(tx, identity, meta, &clock.now(), None, Role::Host, false)?;
            let mut work = vec![];
            for row in tx.scan("work/")? {
                let item: Work = decode(&row, WORK)?;
                if item.host == p.id
                    && p.cells.contains(&item.cell)
                    && item.invocation.is_some()
                    && item.operation.outcome() == Outcome::None
                    && after.is_none_or(|after| item.operation.id() > after)
                    && tx
                        .outbox(&key_id(item.operation.id(), "authorize"))?
                        .is_some_and(|row| {
                            matches!(row.state, OutboxState::EmitEntered | OutboxState::Delivered)
                        })
                {
                    work.push(item);
                }
            }
            work.sort_by(|a, b| a.operation.id().cmp(b.operation.id()));
            work.truncate(limit);
            Ok(work)
        })
    }
    pub fn plan_delivery(&mut self, identity: &Identity, message: &Id) -> Result<DeliveryPlan> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let row = tx
                .outbox(message)?
                .ok_or(StoreError::Rejected(Reject::NotFound))?;
            if !matches!(row.state, OutboxState::New | OutboxState::EmitEntered) {
                return reject(Reject::StaleRevision);
            }
            let payload: Delivery = canonical::decode_json(
                &canonical::bytes(&row.document.value).map_err(domain_error)?,
            )
            .map_err(domain_error)?;
            let (cell_id, host_id, work, permit) = match &payload {
                Delivery::Prepare {
                    operation,
                    host,
                    permit,
                }
                | Delivery::Authorize {
                    operation,
                    host,
                    permit,
                    ..
                } => {
                    let (_, work): (_, Work) = load(tx, "work", operation, WORK)?;
                    let (_, p): (_, Permit) = load(tx, "permit", permit, PERMIT)?;
                    if &work.host != host || &work.permit != permit || p.operation != *operation {
                        return reject(Reject::InvalidInput);
                    }
                    if let Delivery::Authorize { invocation, .. } = &payload
                        && work.invocation.as_ref() != Some(invocation)
                    {
                        return reject(Reject::InvalidInput);
                    }
                    (
                        work.cell.clone(),
                        host.clone(),
                        Some(Box::new(work)),
                        Some(Box::new(p)),
                    )
                }
                Delivery::Arm { attempt, host, .. } => {
                    let (_, a): (_, StartAttempt) = load(tx, "attempt", attempt, ATTEMPT)?;
                    (a.cell, host.clone(), None, None)
                }
                Delivery::Fence { cell, host, .. } => (cell.clone(), host.clone(), None, None),
            };
            let principal = authorize(tx, identity, meta, &now, Some(&cell_id), Role::Host, false)?;
            if principal.id != host_id {
                return reject(Reject::Forbidden);
            }
            let (_, registration): (_, HostRegistration) =
                load(tx, "host", (&cell_id, &host_id), HOST)?;
            if registration.session != identity.session {
                return reject(Reject::Unauthenticated);
            }
            let first = row.state == OutboxState::New;
            if first || matches!(&payload, Delivery::Arm { .. }) {
                validate_emission(tx, &payload, meta, &now)?;
            }
            if first {
                tx.transition_outbox(message, OutboxState::New, OutboxState::EmitEntered)?;
            }
            Ok(DeliveryPlan {
                message: message.clone(),
                first_emission: first,
                cell: cell_id,
                registration,
                payload,
                work,
                permit,
            })
        })
    }
    /// Internal sender fact, not a remote command. An entered send cannot be proven absent by timeout.
    pub fn note_delivery_attention(&mut self, message: &Id, issue: DeliveryIssue) -> Result<()> {
        self.repository.transact(|tx| {
            let row = tx
                .outbox(message)?
                .ok_or(StoreError::Rejected(Reject::NotFound))?;
            let payload: Delivery = canonical::decode_json(
                &canonical::bytes(&row.document.value).map_err(domain_error)?,
            )
            .map_err(domain_error)?;
            let host = match &payload {
                Delivery::Arm { host, .. }
                | Delivery::Prepare { host, .. }
                | Delivery::Authorize { host, .. }
                | Delivery::Fence { host, .. } => host.clone(),
            };
            let attention = DeliveryAttention {
                message: message.clone(),
                host,
                issue,
            };
            let key_ = key("deliveryattention", message);
            let previous = tx.get(&key_)?;
            let incoming = doc("rx.internal.delivery-attention.v1", &attention)?;
            if previous
                .as_ref()
                .is_some_and(|old| old.document == incoming)
            {
                return Ok(());
            }
            tx.put(&key_, previous.map(|r| r.revision), &incoming)?;
            if row.state == OutboxState::EmitEntered
                && matches!(
                    issue,
                    DeliveryIssue::ResponseUnknown
                        | DeliveryIssue::RemoteNotFound
                        | DeliveryIssue::PeerChanged
                )
                && let Delivery::Prepare { operation, .. } | Delivery::Authorize { operation, .. } =
                    payload
            {
                let (revision, mut work): (_, Work) = load(tx, "work", &operation, WORK)?;
                if work.operation.outcome() == Outcome::None {
                    work.operation.lose_continuity().map_err(domain_error)?;
                    save(tx, "work", &operation, Some(revision), WORK, &work)?;
                }
            }
            event(tx, "rx.event.delivery-attention.v1", &attention)
        })
    }
    /// Internal outbox consumer API. A queued record is not a new execution permission.
    pub fn pending_deliveries(&mut self, limit: usize) -> Result<Vec<PendingDelivery>> {
        self.pending_deliveries_after(None, limit)
    }
    pub fn pending_deliveries_after(
        &mut self,
        after: Option<&Id>,
        limit: usize,
    ) -> Result<Vec<PendingDelivery>> {
        self.repository
            .pending_outbox_after(after, limit)?
            .into_iter()
            .map(|record| {
                if record.document.schema.as_str() != DELIVERY {
                    return Err(StoreError::Integrity("outbox schema".into()));
                }
                let payload = canonical::decode_json(
                    &canonical::bytes(&record.document.value).map_err(domain_error)?,
                )
                .map_err(domain_error)?;
                Ok(PendingDelivery {
                    id: record.id,
                    state: record.state,
                    payload,
                })
            })
            .collect()
    }
    /// Returns false for an already-emitted record: inspect its remote receipt before any replay.
    pub fn begin_delivery(&mut self, message: &Id) -> Result<bool> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let outbox = tx
                .outbox(message)?
                .ok_or(StoreError::Rejected(Reject::NotFound))?;
            if outbox.state == OutboxState::EmitEntered {
                return Ok(false);
            }
            if outbox.state != OutboxState::New {
                return reject(Reject::InvalidInput);
            }
            let payload: Delivery = canonical::decode_json(
                &canonical::bytes(&outbox.document.value).map_err(domain_error)?,
            )
            .map_err(domain_error)?;
            validate_emission(tx, &payload, meta, &now)?;
            tx.transition_outbox(message, OutboxState::New, OutboxState::EmitEntered)?;
            Ok(true)
        })
    }
    pub fn delivery_succeeded(&mut self, message: &Id) -> Result<()> {
        self.repository.transact(|tx| {
            let record = tx
                .outbox(message)?
                .ok_or(StoreError::Rejected(Reject::NotFound))?;
            if record.state == OutboxState::Delivered {
                return Ok(());
            }
            tx.transition_outbox(message, OutboxState::EmitEntered, OutboxState::Delivered)
        })
    }
    pub fn record_host_receipt(
        &mut self,
        identity: &Identity,
        message: &Id,
        receipt: HostReceipt,
    ) -> Result<Work> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now=clock.now();
            let (revision,mut work):(_,Work)=load(tx,"work",&receipt.operation,WORK)?;
            let principal=authorize(tx,identity,meta,&now,Some(&work.cell),Role::Host,false)?;
            if work.host!=principal.id || work.intent.digest().map_err(domain_error)?!=receipt.digest
                || receipt.journal!=work.host_journal || receipt.sequence.0==0 {return reject(Reject::InvalidInput);}
            let outbox=tx.outbox(message)?.ok_or(StoreError::Rejected(Reject::NotFound))?;
            let delivery:Delivery=canonical::decode_json(&canonical::bytes(&outbox.document.value).map_err(domain_error)?).map_err(domain_error)?;
            let prepare=matches!(&delivery,Delivery::Prepare {operation,host,..} if operation==&receipt.operation && host==&principal.id);
            let authorize_message=matches!(&delivery,Delivery::Authorize {operation,host,..} if operation==&receipt.operation && host==&principal.id);
            if (!prepare && !authorize_message) || !matches!(outbox.state,OutboxState::EmitEntered|OutboxState::Delivered) {return reject(Reject::InvalidInput);}
            let completes=prepare || receipt.state!=ReceiptState::Prepared;
            let source=key("hostreceipt",(&principal.id,&receipt.journal,receipt.sequence));
            let document=doc("rx.internal.host-receipt.v1",&receipt)?;
            if let Some(old)=tx.get(&source)? {
                if old.document!=document {
                    work.operation.dispute().map_err(domain_error)?;
                    save(tx,"work",work.operation.id(),Some(revision),WORK,&work)?;
                    invalidate_closure(tx,&work.cell,BlockReason::IntegrityConflict)?;
                    return Ok(Err(StoreError::Integrity("conflicting Host receipt sequence".into())));
                }
                if completes && outbox.state==OutboxState::EmitEntered {
                    tx.transition_outbox(message,OutboxState::EmitEntered,OutboxState::Delivered)?;
                }
                return Ok(Ok(work));
            }
            let newly_correlated=work.invocation.is_none() && receipt.invocation.is_some();
            if let Some(invocation)=&receipt.invocation {
                if work.invocation.as_ref().is_some_and(|old|old!=invocation) {return reject(Reject::InvalidInput);}
                work.invocation=Some(invocation.clone());
            } else if receipt.state!=ReceiptState::VoidedBeforeSend {return reject(Reject::InvalidInput);}
            tx.put(&source,None,&document)?;
            let evidence=key_id(&receipt.journal,&receipt.sequence.0.to_string());
            tx.put(&key("receiptevidence",&evidence),None,&document)?;
            match receipt.state {
                ReceiptState::Prepared=>{
                    if prepare && work.operation.outcome()==Outcome::None {
                        let (_,permit):(_,Permit)=load(tx,"permit",&work.permit,PERMIT)?;
                        if permit.state==PermitState::Issued {
                            tx.enqueue(&key_id(work.operation.id(),"authorize"),&doc(DELIVERY,&Delivery::Authorize {
                                operation:work.operation.id().clone(),host:work.host.clone(),permit:work.permit.clone(),
                                invocation:work.invocation.clone().ok_or(StoreError::Rejected(Reject::InvalidInput))?})?)?;
                        }
                    }
                },
                ReceiptState::VoidedBeforeSend=>{
                    if work.operation.outcome()==Outcome::None {
                        work.operation.conclude(rx_domain::operation::Conclusion {outcome:Outcome::NotExecuted,evidence_ids:vec![evidence]}).map_err(domain_error)?;
                    }
                },
                ReceiptState::SendEntered|ReceiptState::NativeAccepted|ReceiptState::NativeRejected|ReceiptState::ResultCaptured=>{
                    if work.operation.phase()==rx_domain::operation::Phase::Admitted {work.operation.sent().map_err(domain_error)?;}
                    let (pr,mut permit):(_,Permit)=load(tx,"permit",&work.permit,PERMIT)?;
                    if permit.state==PermitState::Issued {permit.state=PermitState::Consumed;save(tx,"permit",&permit.id,Some(pr),PERMIT,&permit)?;}
                },
            }
            save(tx,"work",work.operation.id(),Some(revision),WORK,&work)?;
            if newly_correlated {super::evidence::reapply_correlated_native(tx,&principal,work.operation.id(),&now)?;}
            if completes && outbox.state==OutboxState::EmitEntered {tx.transition_outbox(message,OutboxState::EmitEntered,OutboxState::Delivered)?;}
            event(tx,"rx.event.host-receipt-recorded.v1",&receipt)?;
            Ok(Ok(load(tx,"work",work.operation.id(),WORK)?.1))
        })?
    }
    pub fn inspect_permit(&mut self, identity: &Identity, permit_id: &Id) -> Result<Permit> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let (_, p): (_, Permit) = load(tx, "permit", permit_id, PERMIT)?;
            authorize_read(tx, identity, meta, &clock.now(), &p.cell)?;
            Ok(p)
        })
    }
}
/// Deterministic UUID for a child delivery/evidence reference; never interpreted as time.
pub(super) fn key_id(parent: &Id, slot: &str) -> Id {
    let digest =
        canonical::digest("RX-DELIVERY-ID-v1", &(parent, slot)).expect("delivery identity");
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    bytes[6] = (bytes[6] & 15) | 0x80;
    bytes[8] = (bytes[8] & 63) | 0x80;
    Id::new(uuid::Uuid::from_bytes(bytes).to_string()).expect("UUID")
}

fn validate_emission(
    tx: &mut dyn Transaction,
    payload: &Delivery,
    meta: &Installation,
    now: &TimePoint,
) -> Result<()> {
    match payload {
        Delivery::Prepare { operation, .. } | Delivery::Authorize { operation, .. } => {
            let (_, work): (_, Work) = load(tx, "work", operation, WORK)?;
            let (_, run): (_, Run) = load(tx, "run", &work.run, RUN)?;
            let (_, cell): (_, Cell) = load(tx, "cell", &work.cell, CELL)?;
            let identity = Identity {
                principal: cell.configuration.executor.clone(),
                session: run
                    .executor_session
                    .clone()
                    .ok_or(StoreError::Rejected(Reject::MandateRevoked))?,
                terminal: None,
            };
            active_run(tx, &cell, &run, &identity, meta, now)?;
            if work.operation.outcome() != Outcome::None
                || work.operation.integrity() != Integrity::Valid
            {
                return reject(Reject::ContinuityUnproven);
            }
            let (_, permit): (_, Permit) = load(tx, "permit", &work.permit, PERMIT)?;
            if permit.state != PermitState::Issued
                || permit.expires_at.clock_id != now.clock_id
                || permit.expires_at.ticks_ns <= now.ticks_ns
            {
                return reject(Reject::StaleEpoch);
            }
            let step = cell
                .configuration
                .steps
                .iter()
                .find(|s| s.intent.digest().ok() == work.intent.digest().ok())
                .ok_or(StoreError::Rejected(Reject::CapabilityMissing))?;
            evaluate(tx, &cell, &step.conditions, now)?;
        }
        Delivery::Arm {
            attempt,
            clear_blocks,
            ..
        } => {
            let (_, a): (_, StartAttempt) = load(tx, "attempt", attempt, ATTEMPT)?;
            let (_, cell): (_, Cell) = load(tx, "cell", &a.cell, CELL)?;
            ready(tx, &cell, now)?;
            if clear_blocks.iter().collect::<BTreeSet<_>>()
                != qualification_activation::arm_clear(tx, &cell)?
                    .iter()
                    .collect()
            {
                return reject(Reject::Forbidden);
            }
            if a.status != StartStatus::Arming
                || a.valid_until.clock_id != now.clock_id
                || a.valid_until.ticks_ns <= now.ticks_ns
            {
                return reject(Reject::StaleRevision);
            }
        }
        Delivery::Fence { .. } => {}
    }
    Ok(())
}
