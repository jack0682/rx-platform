use super::*;
const PREFIX: &str = "host-rejoin-proposal";
const TTL: u64 = 30_000_000_000;
pub(super) fn access(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: &TimePoint,
    identity: &Identity,
    c: &r::Context,
) -> Result<Principal> {
    let actor = authorize(
        tx,
        identity,
        meta,
        now,
        Some(&c.origin),
        Role::ReleaseManager,
        true,
    )?;
    if c.cells.keys().any(|id| !actor.cells.contains(id)) {
        return reject(Reject::Forbidden);
    }
    Ok(actor)
}
pub(super) fn view(
    tx: &mut dyn Transaction,
    meta: &Installation,
    now: TimePoint,
    identity: &Identity,
    p: r::Proposal,
) -> Result<r::ProposalView> {
    access(tx, meta, &now, identity, &p.context)?;
    let current = match build(
        tx,
        meta,
        now.clone(),
        identity,
        &p.context.host,
        &p.context.origin,
    ) {
        Ok(c) => {
            c.local_prerequisites_current
                && c.digest().map_err(StoreError::Integrity)? == p.context_digest
                && now.clock_id == p.valid_until.clock_id
                && now.ticks_ns < p.valid_until.ticks_ns
        }
        Err(StoreError::Rejected(_)) => false,
        Err(e) => return Err(e),
    };
    let read_current = if current {
        match reads::validate(tx, &now, &p.context, &p.read) {
            Ok(()) => true,
            Err(StoreError::Rejected(_)) => false,
            Err(e) => return Err(e),
        }
    } else {
        false
    };
    Ok(r::ProposalView {
        proposal_digest: p.digest().map_err(StoreError::Integrity)?,
        proposal: p,
        context_current: current,
        read_current,
        operation_authorized: false,
    })
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn lookup_host_rejoin_proposal(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: r::Prepare,
    ) -> Result<Option<r::ProposalView>> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let actor = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&input.origin),
                Role::ReleaseManager,
                true,
            )?;
            let (scope, fp) = request(meta, &actor, "HostRejoin.Propose", key_.as_str(), &input)?;
            let old: Option<r::Proposal> = prior(tx, &scope, fp, r::PROPOSAL_SCHEMA)?;
            old.map(|p| view(tx, meta, now, identity, p)).transpose()
        })
    }
    pub fn propose_host_rejoin(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: r::Prepare,
        read: crate::host_recovery::VerifiedRead,
    ) -> Result<r::ProposalView> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let actor = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&input.origin),
                Role::ReleaseManager,
                true,
            )?;
            let (scope, fp) = request(meta, &actor, "HostRejoin.Propose", key_.as_str(), &input)?;
            if let Some(old) = prior::<r::Proposal>(tx, &scope, fp, r::PROPOSAL_SCHEMA)? {
                return view(tx, meta, now, identity, old);
            }
            let c = build(tx, meta, now.clone(), identity, &input.host, &input.origin)?;
            if !c.local_prerequisites_current {
                return reject(Reject::ContinuityUnproven);
            }
            if c.digest().map_err(StoreError::Integrity)? != input.expected_context
                || c.expected_cells() != input.expected_cells
            {
                return reject(Reject::StaleRevision);
            }
            reads::validate(tx, &now, &c, &read.0)?;
            let cells = c
                .cells
                .iter()
                .filter(|(_, v)| v.cell.configuration.hosts.contains(&c.host))
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            let operations = host_recovery::original_operations(tx, &c.host, &cells)?;
            if operations.len() > crate::host_recovery::MAX_OPERATIONS {
                return reject(Reject::InvalidInput);
            }
            let value = r::Proposal {
                schema: name(r::PROPOSAL_SCHEMA),
                id: id(),
                revision: Counter(1),
                context: c,
                context_digest: input.expected_context,
                read: read.0,
                recovery_scope: Some(operations),
                proposed_by: identity.into(),
                proposed_at: now.clone(),
                valid_until: TimePoint {
                    clock_id: now.clock_id.clone(),
                    ticks_ns: Counter(
                        now.ticks_ns
                            .0
                            .checked_add(TTL)
                            .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
                    ),
                },
            };
            save(tx, PREFIX, &value.id, None, r::PROPOSAL_SCHEMA, &value)?;
            remember(tx, &scope, fp, r::PROPOSAL_SCHEMA, &value)?;
            event(tx, "rx.event.host-rejoin-proposed.v1", &value)?;
            view(tx, meta, now, identity, value)
        })
    }
    pub fn host_rejoin_proposal(
        &mut self,
        identity: &Identity,
        id: &Id,
    ) -> Result<r::ProposalView> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (revision, p): (_, r::Proposal) = load(tx, PREFIX, id, r::PROPOSAL_SCHEMA)?;
            if p.id != *id
                || p.revision != revision
                || p.schema.as_str() != r::PROPOSAL_SCHEMA
                || p.context_digest != p.context.digest().map_err(StoreError::Integrity)?
            {
                return Err(StoreError::Integrity(
                    "rejoin proposal identity/context differs".into(),
                ));
            }
            view(tx, meta, clock.now(), identity, p)
        })
    }
}

pub(super) fn load_proposal(tx: &mut dyn Transaction, id: &Id) -> Result<r::Proposal> {
    let (revision, p): (_, r::Proposal) = load(tx, PREFIX, id, r::PROPOSAL_SCHEMA)?;
    if p.id != *id
        || p.revision != revision
        || p.schema.as_str() != r::PROPOSAL_SCHEMA
        || p.context_digest != p.context.digest().map_err(StoreError::Integrity)?
    {
        return Err(StoreError::Integrity(
            "rejoin proposal identity/context differs".into(),
        ));
    }
    Ok(p)
}
