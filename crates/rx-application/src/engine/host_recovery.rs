use super::*;
use crate::{host_binding_baseline as baseline, host_recovery as r};
mod context;
mod fences;
mod reads;
mod receipts;
const PREFIX: &str = "host-recovery";
const TRANSPORT: &str = "rx.host-recovery-transport.v1";
const REFERENCE: &str = "rx.host-recovery-ref.v1";

#[derive(serde::Serialize, serde::Deserialize)]
struct TransportRegistration {
    runtime_boot: Id,
    pin: baseline::TransportPin,
}
fn digest<T: Serialize>(v: &T) -> Result<Digest> {
    canonical::digest("RX-HOST-RECOVERY-RECORD-v1", v).map_err(domain_error)
}
fn same<T: Serialize>(a: &T, b: &T) -> Result<bool> {
    Ok(digest(a)? == digest(b)?)
}
fn read(tx: &mut dyn Transaction, id: &Id) -> Result<r::Binding> {
    let (revision, b): (_, r::Binding) = load(tx, PREFIX, id, r::SCHEMA)?;
    if b.id != *id
        || b.schema.as_str() != r::SCHEMA
        || b.revision != revision
        || b.fences.keys().collect::<BTreeSet<_>>() != b.context.host_cells.iter().collect()
        || b.fences.iter().any(|(cell, f)| {
            f.task.binding != b.id
                || f.task.cell != *cell
                || f.task
                    .originating_message
                    .as_ref()
                    .is_some_and(|id| id != &f.task.request)
                || f.task.digest().ok() != Some(f.payload_digest)
                || (f.phase == r::FencePhase::Acknowledged) != f.acknowledgment.is_some()
        })
        || (b.phase == r::Phase::RecoveryOnly
            && (b.last_read.is_none()
                || b.fences
                    .values()
                    .any(|f| f.phase != r::FencePhase::Acknowledged)))
    {
        return Err(StoreError::Integrity(
            "Host recovery identity/phase differs".into(),
        ));
    }
    Ok(b)
}
fn save_binding(
    tx: &mut dyn Transaction,
    b: &mut r::Binding,
    previous: Option<Counter>,
) -> Result<()> {
    b.revision = previous.map_or(Ok(Counter(1)), |n| n.increment().map_err(domain_error))?;
    save(tx, PREFIX, &b.id, previous, r::SCHEMA, b)?;
    event(tx, "rx.event.host-recovery.v1", b)
}
fn access(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    identity: &Identity,
    c: &r::Context,
) -> Result<Principal> {
    let p = authorize(
        tx,
        identity,
        meta,
        now,
        Some(&c.origin),
        Role::ReleaseManager,
        true,
    )?;
    if c.cells.keys().any(|cell| !p.cells.contains(cell)) {
        return reject(Reject::Forbidden);
    }
    Ok(p)
}
fn approved_current(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    b: &r::Binding,
) -> Result<()> {
    let actor = b
        .approved_by
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::Forbidden))?;
    access(tx, meta, now, &actor.identity(), &b.context)?;
    context::current(tx, meta, now, &b.context)?;
    let (_, owner): (_, Id) = load(tx, "host-recovery-owner", &b.context.host, REFERENCE)?;
    if owner != b.id {
        return reject(Reject::StaleRevision);
    }
    Ok(())
}
fn retain_attention(b: &mut r::Binding, result: Result<()>) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(StoreError::Rejected(reason)) => {
            b.phase = r::Phase::Attention;
            b.detail = Some(format!("recovery context is no longer current: {reason:?}"));
            Ok(())
        }
        Err(error) => Err(error),
    }
}
fn view_current(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    b: &r::Binding,
) -> Result<bool> {
    let result = approved_current(tx, meta, now, b).and_then(|_| {
        if b.phase != r::Phase::RecoveryOnly {
            return reject(Reject::ContinuityUnproven);
        }
        let last = b
            .last_read
            .as_ref()
            .ok_or(StoreError::Rejected(Reject::ConditionUnknown))?;
        reads::validate(tx, now, &b.context, last, true)
    });
    match result {
        Ok(()) => Ok(true),
        Err(StoreError::Rejected(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Release startup declaration only. It is not evidence that any TLS read occurred.
    pub fn register_host_recovery_transport(
        &mut self,
        host: Name,
        pin: baseline::TransportPin,
    ) -> Result<()> {
        pin.validate_current_bindings()
            .map_err(StoreError::Invalid)?;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let k = key("host-recovery-transport", &host);
            let previous = tx.get(&k)?;
            if let Some(row) = &previous {
                let old: TransportRegistration = decode(row, TRANSPORT)?;
                if old.runtime_boot == meta.runtime_boot {
                    return if old.pin == pin {
                        Ok(())
                    } else {
                        Err(StoreError::KeyConflict)
                    };
                }
            }
            tx.put(
                &k,
                previous.map(|row| row.revision),
                &doc(
                    TRANSPORT,
                    &TransportRegistration {
                        runtime_boot: meta.runtime_boot.clone(),
                        pin,
                    },
                )?,
            )?;
            Ok(())
        })
    }
    pub fn host_recovery_context(
        &mut self,
        identity: &Identity,
        host: &Name,
        origin: &Name,
    ) -> Result<r::Context> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository
            .transact(|tx| context::build(tx, meta, &now, identity, host, origin))
    }
    /// Recover the durable proposal before any new network read. This is historical receipt
    /// recovery, not a claim that its original observed Host or authorization remains current.
    pub fn lookup_host_recovery_proposal(
        &mut self,
        identity: &Identity,
        request_key: &Id,
        input: &r::Prepare,
    ) -> Result<Option<r::Binding>> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let actor = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&input.origin),
                Role::ReleaseManager,
                true,
            )?;
            let (scope, fp) = request(
                meta,
                &actor,
                "HostRecovery.Propose",
                request_key.as_str(),
                input,
            )?;
            let Some(id) = prior::<Id>(tx, &scope, fp, REFERENCE)? else {
                return Ok(None);
            };
            let b = read(tx, &id)?;
            access(tx, meta, &now, identity, &b.context)?;
            Ok(Some(b))
        })
    }
    pub fn propose_host_recovery(
        &mut self,
        identity: &Identity,
        request_key: &Id,
        input: r::Prepare,
        verified: r::VerifiedRead,
    ) -> Result<r::Binding> {
        if let Some(record) = self.lookup_host_recovery_proposal(identity, request_key, &input)? {
            return Ok(record);
        }
        // This synchronous command runs on the existing single writer. Candidate enumeration
        // is outside the DB transaction, and every selected immutable row is rechecked inside it.
        let candidates = fences::candidates(&mut self.repository)?;
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let actor = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&input.origin),
                Role::ReleaseManager,
                true,
            )?;
            let (scope, fp) = request(
                meta,
                &actor,
                "HostRecovery.Propose",
                request_key.as_str(),
                &input,
            )?;
            if let Some(existing) = prior::<Id>(tx, &scope, fp, REFERENCE)? {
                let b = read(tx, &existing)?;
                access(tx, meta, &now, identity, &b.context)?;
                return Ok(Ok(b));
            }
            let mut c = context::build(tx, meta, &now, identity, &input.host, &input.origin)?;
            if !c.blockers.is_empty() {
                return reject(Reject::ContinuityUnproven);
            }
            if c.digest().map_err(StoreError::Integrity)? != input.expected_context
                || c.expected_cells() != input.expected_cells
            {
                return reject(Reject::StaleRevision);
            }
            if let Err(error) = reads::validate(tx, &now, &c, &verified.0, false) {
                if matches!(error, StoreError::Rejected(_)) {
                    // Retain an authenticated but ineligible read as raw audit evidence only.
                    event(
                        tx,
                        "rx.event.host-recovery-read-rejected.v1",
                        &(
                            r::StoredActor::from(identity),
                            request_key,
                            &input,
                            &verified.0,
                            error.to_string(),
                        ),
                    )?;
                    return Ok(Err(error));
                }
                return Err(error);
            }
            let id = id();
            let mut fences = BTreeMap::new();
            for cell in &c.host_cells {
                let cut = &c.cells[cell];
                let mut task = r::FenceTask {
                    binding: id.clone(),
                    cell: cell.clone(),
                    request: id_for_fence(),
                    originating_message: None,
                    epoch: cut.epoch,
                    scopes: cut.scopes.clone(),
                    block_ids: cut
                        .blocks
                        .iter()
                        .filter(|b| b.latched)
                        .map(|b| b.id.clone())
                        .collect(),
                };
                let matching = fences::matching(tx, &c.host, &task, &candidates)?;
                if matching.len() > 1 {
                    c.blockers
                        .push(r::Blocker::FenceCandidatesAmbiguous { cell: cell.clone() });
                } else if let Some(message) = matching.into_iter().next() {
                    task.request = message.clone();
                    task.originating_message = Some(message);
                }
                fences.insert(
                    cell.clone(),
                    r::FenceStep {
                        payload_digest: task.digest().map_err(StoreError::Invalid)?,
                        task,
                        phase: r::FencePhase::Pending,
                        acknowledgment: None,
                    },
                );
            }
            let ambiguous = !c.blockers.is_empty();
            let mut b = r::Binding {
                schema: name(r::SCHEMA),
                id,
                revision: Counter(1),
                phase: if ambiguous {
                    r::Phase::Attention
                } else {
                    r::Phase::Proposed
                },
                requested_context_digest: input.expected_context,
                context: c,
                proposed_by: r::StoredActor::from(identity),
                proposed_at: now,
                proposal_read: verified.0,
                approved_by: None,
                approved_at: None,
                fences,
                last_read: None,
                detail: ambiguous.then(|| "AMBIGUOUS_CURRENT_FENCE: no subset selected".into()),
            };
            save_binding(tx, &mut b, None)?;
            remember(tx, &scope, fp, REFERENCE, &b.id)?;
            Ok(Ok(b))
        })?
    }
    pub fn approve_host_recovery(
        &mut self,
        identity: &Identity,
        request_key: &Id,
        input: r::Approve,
    ) -> Result<r::Binding> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let mut b = read(tx, &input.id)?;
            let actor = access(tx, meta, &now, identity, &b.context)?;
            let (scope, fp) = request(
                meta,
                &actor,
                "HostRecovery.Approve",
                request_key.as_str(),
                &input,
            )?;
            if let Some(existing) = prior::<Id>(tx, &scope, fp, REFERENCE)? {
                return read(tx, &existing);
            }
            if b.revision != input.expected_revision
                || b.proposal_digest().map_err(StoreError::Integrity)? != input.proposal_digest
                || b.context.expected_cells() != input.expected_cells
            {
                return reject(Reject::StaleRevision);
            }
            context::current(tx, meta, &now, &b.context)?;
            let owner_key = key("host-recovery-owner", &b.context.host);
            let owner_row = tx.get(&owner_key)?;
            if let Some(row) = &owner_row {
                let previous: Id = decode(row, REFERENCE)?;
                let previous = read(tx, &previous)?;
                if previous.id != b.id
                    && previous.context.runtime_boot == meta.runtime_boot
                    && matches!(previous.phase, r::Phase::Fencing | r::Phase::RecoveryOnly)
                {
                    return reject(Reject::Busy);
                }
            }
            tx.put(
                &owner_key,
                owner_row.map(|row| row.revision),
                &doc(REFERENCE, &b.id)?,
            )?;
            let revision = b.revision;
            b.phase = r::Phase::Fencing;
            b.approved_by = Some(r::StoredActor::from(identity));
            b.approved_at = Some(now);
            b.detail = None;
            save_binding(tx, &mut b, Some(revision))?;
            remember(tx, &scope, fp, REFERENCE, &b.id)?;
            Ok(b)
        })
    }
    /// Persist the exact request before any network send; entered requests keep their original ID.
    pub fn plan_host_recovery_fence(
        &mut self,
        binding: &Id,
        cell: &Name,
        verified: r::VerifiedRead,
    ) -> Result<r::FenceTask> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let mut b = read(tx, binding)?;
            if b.phase != r::Phase::Fencing {
                return reject(Reject::StaleRevision);
            }
            let valid = approved_current(tx, meta, &now, &b).and_then(|_| {
                reads::validate(tx, &now, &b.context, &verified.0, false)?;
                if verified.0.platform_session != b.proposal_read.platform_session {
                    return reject(Reject::ContinuityUnproven);
                }
                Ok(())
            });
            if let Err(error) = valid {
                if matches!(error, StoreError::Rejected(_)) {
                    let revision = b.revision;
                    b.phase = r::Phase::Attention;
                    b.detail = Some(error.to_string());
                    b.last_read = Some(verified.0);
                    save_binding(tx, &mut b, Some(revision))?;
                    return Ok(Err(error));
                }
                return Err(error);
            }
            let revision = b.revision;
            let f = b
                .fences
                .get_mut(cell)
                .ok_or(StoreError::Rejected(Reject::Forbidden))?;
            let task = f.task.clone();
            fences::enter(tx, &b.context.host, &task)?;
            if f.phase == r::FencePhase::Pending {
                f.phase = r::FencePhase::SendEntered;
                save_binding(tx, &mut b, Some(revision))?;
            }
            Ok(Ok(task))
        })?
    }
    /// A correlated late ACK remains a fact after approval/CAS loss, but can never promote a binding.
    pub fn record_host_recovery_fence(
        &mut self,
        binding: &Id,
        cell: &Name,
        ack: FenceAcknowledgment,
    ) -> Result<r::Binding> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let mut b = read(tx, binding)?;
            let f = b
                .fences
                .get(cell)
                .ok_or(StoreError::Rejected(Reject::Forbidden))?;
            let baseline = baseline::load(tx, &b.context.registrations[cell].plan)?
                .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?;
            if f.phase == r::FencePhase::Pending
                || ack.cell != *cell
                || ack.invalidation != f.task.request
                || ack.epoch != f.task.epoch
                || ack.scopes != f.task.scopes
                || ack.sequence.0 == 0
                || ack.host_boot != baseline.host_boot
                || ack.journal != baseline.delivery_journal
            {
                return reject(Reject::ContinuityUnproven);
            }
            if let Some(old) = &f.acknowledgment
                && !same(old, &ack)?
            {
                return Err(StoreError::KeyConflict);
            }
            fences::complete(tx, &b.context.host, &f.task)?;
            let k = key("fenceack", (&b.context.host, &ack.journal, ack.sequence));
            let document = doc("rx.internal.fence-ack.v1", &ack)?;
            if let Some(old) = tx.get(&k)? {
                if old.document != document {
                    return Err(StoreError::Integrity(
                        "recovery fence receipt conflict".into(),
                    ));
                }
            } else {
                tx.put(&k, None, &document)?;
            }
            let fact = key("host-recovery-fence-proof", (binding, &f.task.request));
            let incoming = doc(
                "rx.host-recovery-fence-proof.v1",
                &(binding, f.payload_digest, &ack),
            )?;
            if let Some(old) = tx.get(&fact)? {
                if old.document != incoming {
                    return Err(StoreError::KeyConflict);
                }
            } else {
                tx.put(&fact, None, &incoming)?;
            }
            let revision = b.revision;
            let before = digest(&b)?;
            let step = b.fences.get_mut(cell).unwrap();
            step.phase = r::FencePhase::Acknowledged;
            step.acknowledgment = Some(ack);
            let current = approved_current(tx, meta, &now, &b);
            retain_attention(&mut b, current)?;
            if digest(&b)? != before {
                save_binding(tx, &mut b, Some(revision))?;
            }
            Ok(b)
        })
    }
    pub fn commit_host_recovery(
        &mut self,
        binding: &Id,
        expected_revision: Counter,
        verified: r::VerifiedRead,
    ) -> Result<r::Binding> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let mut b = read(tx, binding)?;
            let revision = b.revision;
            let current = approved_current(tx, meta, &now, &b);
            retain_attention(&mut b, current)?;
            if b.phase == r::Phase::Attention {
                b.last_read = Some(verified.0);
                save_binding(tx, &mut b, Some(revision))?;
                return Ok(b);
            }
            // A commit reply retry also carries an actual read. Never discard newly observed
            // continuity loss merely because the historical binding already reached RecoveryOnly.
            if (b.phase != r::Phase::RecoveryOnly
                && (b.phase != r::Phase::Fencing || b.revision != expected_revision))
                || b.fences
                    .values()
                    .any(|f| f.phase != r::FencePhase::Acknowledged)
            {
                return reject(Reject::HostNotPrepared);
            }
            let valid = if verified.0.platform_session != b.proposal_read.platform_session {
                reject(Reject::ContinuityUnproven)
            } else {
                reads::validate(tx, &now, &b.context, &verified.0, true)
            };
            retain_attention(&mut b, valid)?;
            if b.phase != r::Phase::Attention {
                reads::ingest(tx, &now, &b.context, &verified.0)?;
                let current = approved_current(tx, meta, &now, &b);
                retain_attention(&mut b, current)?;
            }
            if b.phase != r::Phase::Attention {
                b.phase = r::Phase::RecoveryOnly;
            }
            b.last_read = Some(verified.0);
            save_binding(tx, &mut b, Some(revision))?;
            if b.phase == r::Phase::RecoveryOnly {
                let k = key("host-recovery-last", &b.context.host);
                let previous = tx.get(&k)?;
                tx.put(&k, previous.map(|r| r.revision), &doc(REFERENCE, &b.id)?)?;
            }
            Ok(b)
        })
    }
    pub fn refresh_host_recovery(
        &mut self,
        binding: &Id,
        verified: r::VerifiedRead,
    ) -> Result<r::Binding> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let mut b = read(tx, binding)?;
            if b.phase != r::Phase::RecoveryOnly {
                return reject(Reject::StaleRevision);
            }
            let result = approved_current(tx, meta, &now, &b).and_then(|_| {
                if verified.0.platform_session != b.proposal_read.platform_session {
                    return reject(Reject::ContinuityUnproven);
                }
                reads::validate(tx, &now, &b.context, &verified.0, true)
            });
            let revision = b.revision;
            retain_attention(&mut b, result)?;
            if b.phase == r::Phase::RecoveryOnly {
                reads::ingest(tx, &now, &b.context, &verified.0)?;
                let current = approved_current(tx, meta, &now, &b);
                retain_attention(&mut b, current)?;
            }
            b.last_read = Some(verified.0);
            save_binding(tx, &mut b, Some(revision))?;
            Ok(b)
        })
    }
    pub fn host_recovery(&mut self, identity: &Identity, binding: &Id) -> Result<r::View> {
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let b = read(tx, binding)?;
            access(tx, meta, &now, identity, &b.context)?;
            let current = view_current(tx, meta, &now, &b)?;
            Ok(r::View {
                binding: b,
                current,
                operation_authorized: false,
            })
        })
    }
    /// Bounded, Host-filtered discovery; partial cell rights never expose a shared recovery record.
    pub fn list_host_recoveries(
        &mut self,
        identity: &Identity,
        host: &Name,
        after: Option<&Id>,
        limit: usize,
    ) -> Result<r::RecoveryPage> {
        if limit == 0 || limit > 50 {
            return reject(Reject::InvalidInput);
        }
        let meta = &self.installation;
        let now = self.clock.now();
        self.repository.transact(|tx| {
            let actor = authorize(tx, identity, meta, &now, None, Role::ReleaseManager, true)?;
            let mut bindings = Vec::new();
            for row in tx.scan(&format!("{PREFIX}/"))? {
                let b: r::Binding = decode(&row, r::SCHEMA)?;
                if b.context.host != *host
                    || after.is_some_and(|id| b.id <= *id)
                    || b.context
                        .cells
                        .keys()
                        .any(|cell| !actor.cells.contains(cell))
                {
                    continue;
                }
                let b = read(tx, &b.id)?;
                access(tx, meta, &now, identity, &b.context)?;
                bindings.push(b);
            }
            bindings.sort_by(|a, b| a.id.cmp(&b.id));
            let more = bindings.len() > limit;
            bindings.truncate(limit);
            let next = more.then(|| bindings.last().unwrap().id.clone());
            let mut items = Vec::new();
            for b in bindings {
                let current = view_current(tx, meta, &now, &b)?;
                items.push(r::View {
                    binding: b,
                    current,
                    operation_authorized: false,
                });
            }
            Ok(r::RecoveryPage { items, next })
        })
    }
}
fn id_for_fence() -> Id {
    id()
}
