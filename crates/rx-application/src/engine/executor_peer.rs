use super::*;
const EXECUTOR_PEER: &str = "rx.internal.executor-peer.v1";
const PRODUCER: &str = "rx.internal.evidence-producer.v1";

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Trusted registered-certificate adapter only; opening a peer never starts a run.
    pub fn open_executor_peer(
        &mut self,
        principal: &Name,
        peer_boot: Id,
        authentication_binding: Digest,
    ) -> Result<Session> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (_, p): (_, Principal) = load(tx, "principal", principal, PRINCIPAL)?;
            if !p.active
                || !p.roles.contains(&Role::Executor)
                || p.roles.contains(&Role::Host)
                || p.roles.contains(&Role::OperatorApi)
                || p.cells.is_empty()
            {
                return reject(Reject::Unauthenticated);
            }
            if tx
                .get(&key("retiredexecutorboot", (principal, &peer_boot)))?
                .is_some()
            {
                return reject(Reject::Unauthenticated);
            }
            let previous = tx.get(&key("executorpeer", principal))?;
            if let Some(row) = &previous {
                let old: ExecutorPeer = decode(row, EXECUTOR_PEER)?;
                let (_, session): (_, Session) = load(tx, "session", &old.session, SESSION)?;
                if old.peer_boot == peer_boot
                    && old.authentication_binding == authentication_binding
                    && session.active
                    && session.runtime_boot == meta.runtime_boot
                    && session.expires_at.clock_id == now.clock_id
                    && session.expires_at.ticks_ns > now.ticks_ns
                {
                    return Ok(session);
                }
                if old.peer_boot != peer_boot {
                    tx.put(
                        &key("retiredexecutorboot", (principal, &old.peer_boot)),
                        None,
                        &doc("rx.internal.retired-executor-boot.v1", &old.peer_boot)?,
                    )?;
                }
            }
            let mut old_sessions = false;
            // Retire legacy/local sessions as well: current_session must have exactly one owner.
            for row in tx.scan("session/")? {
                let mut session: Session = decode(&row, SESSION)?;
                if &session.principal == principal && session.active {
                    old_sessions = true;
                    session.active = false;
                    save(
                        tx,
                        "session",
                        &session.id,
                        Some(row.revision),
                        SESSION,
                        &session,
                    )?;
                }
            }
            if previous.is_some() || old_sessions {
                let mut touched = BTreeSet::new();
                for row in tx.scan("cell/")? {
                    let cell: Cell = decode(&row, CELL)?;
                    if &cell.configuration.executor == principal
                        && !touched.contains(&cell.configuration.id)
                    {
                        touched.extend(invalidate_closure(
                            tx,
                            &cell.configuration.id,
                            BlockReason::AuthorityRevoked,
                        )?);
                    }
                }
            }
            let session = Session {
                terminal: None,
                id: id(),
                principal: principal.clone(),
                runtime_boot: meta.runtime_boot.clone(),
                expires_at: TimePoint {
                    clock_id: now.clock_id,
                    ticks_ns: Counter(u64::MAX),
                },
                active: true,
            };
            save(tx, "session", &session.id, None, SESSION, &session)?;
            save(
                tx,
                "executorpeer",
                principal,
                previous.map(|r| r.revision),
                EXECUTOR_PEER,
                &ExecutorPeer {
                    principal: principal.clone(),
                    session: session.id.clone(),
                    peer_boot,
                    authentication_binding,
                    cells: BTreeMap::new(),
                },
            )?;
            event(tx, "rx.event.executor-peer-opened.v1", &session.id)?;
            Ok(session)
        })
    }
    pub fn inspect_service_peer(&mut self, identity: &Identity) -> Result<ServicePeer> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let p = authorize_identity(tx, identity, meta, &clock.now())?;
            service_peer(tx, identity, &p)
        })
    }
    pub fn inspect_peer_cell(
        &mut self,
        identity: &Identity,
        cell_id: &Name,
    ) -> Result<(Counter, Cell)> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = authorize_read(tx, identity, meta, &clock.now(), cell_id)?;
            let cells = match service_peer(tx, identity, &p)? {
                ServicePeer::Host(h) => h.cells,
                ServicePeer::Executor(e) => e.cells,
                ServicePeer::OperatorApi(p) => p.cells,
            };
            let (revision, cell): (_, Cell) = load(tx, "cell", cell_id, CELL)?;
            if cells.get(cell_id) != Some(&cell.configuration.definition.sha256) {
                return reject(Reject::UnsupportedSchema);
            }
            Ok((revision, cell))
        })
    }
    pub fn negotiate_executor_cell(
        &mut self,
        identity: &Identity,
        definition: Digest,
    ) -> Result<Name> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let p = authorize(
                tx,
                identity,
                meta,
                &clock.now(),
                None,
                Role::Executor,
                false,
            )?;
            let (revision, mut peer): (_, ExecutorPeer) =
                load(tx, "executorpeer", &p.id, EXECUTOR_PEER)?;
            if peer.session != identity.session {
                return reject(Reject::Unauthenticated);
            }
            let matching = tx
                .scan("cell/")?
                .iter()
                .map(|r| decode::<Cell>(r, CELL))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .filter(|c| {
                    c.configuration.definition.sha256 == definition
                        && c.configuration.executor == p.id
                        && p.cells.contains(&c.configuration.id)
                })
                .collect::<Vec<_>>();
            if matching.len() != 1 {
                return reject(Reject::CapabilityMissing);
            }
            let cell = matching[0].configuration.id.clone();
            if peer.cells.get(&cell) != Some(&definition) {
                peer.cells.insert(cell.clone(), definition);
                save(
                    tx,
                    "executorpeer",
                    &p.id,
                    Some(revision),
                    EXECUTOR_PEER,
                    &peer,
                )?;
            }
            Ok(cell)
        })
    }
    pub fn executor_run_checkpoint(
        &mut self,
        identity: &Identity,
        run_id: &Id,
    ) -> Result<crate::checkpoint_artifact::RunSnapshot> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (_, run): (_, Run) = load(tx, "run", run_id, RUN)?;
            executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                &run.cell,
            )?;
            super::queries::read_run_checkpoint(tx, identity, meta, &now, run_id)
        })
    }
}

pub(super) fn executor_scope(
    tx: &mut dyn Transaction,
    context: ProcessingContext<'_>,
    cell_id: &Name,
) -> Result<(Principal, Counter, Cell)> {
    let ProcessingContext {
        identity,
        meta,
        now,
    } = context;
    let p = authorize(
        tx,
        identity,
        meta,
        now,
        Some(cell_id),
        Role::Executor,
        false,
    )?;
    let (_, peer): (_, ExecutorPeer) = load(tx, "executorpeer", &p.id, EXECUTOR_PEER)?;
    if peer.session != identity.session {
        return reject(Reject::Unauthenticated);
    }
    let (revision, cell): (_, Cell) = load(tx, "cell", cell_id, CELL)?;
    if cell.configuration.executor != p.id
        || peer.cells.get(cell_id) != Some(&cell.configuration.definition.sha256)
    {
        return reject(Reject::CapabilityMissing);
    }
    Ok((p, revision, cell))
}

pub(super) fn service_peer(
    tx: &mut dyn Transaction,
    identity: &Identity,
    p: &Principal,
) -> Result<ServicePeer> {
    if p.roles.contains(&Role::Executor)
        && let Some(row) = tx.get(&key("executorpeer", &p.id))?
    {
        let peer: ExecutorPeer = decode(&row, EXECUTOR_PEER)?;
        if peer.session == identity.session {
            return Ok(ServicePeer::Executor(peer));
        }
    }
    if p.roles.contains(&Role::Host)
        && let Some(row) = tx.get(&key("producer", &p.id))?
    {
        let peer: EvidenceProducer = decode(&row, PRODUCER)?;
        if peer.session == identity.session {
            return Ok(ServicePeer::Host(peer));
        }
    }
    if super::operator_peer::service_account(p)
        && let Some(row) = tx.get(&key("operatorpeer", &p.id))?
    {
        let peer: OperatorPeer = decode(&row, "rx.internal.operator-peer.v1")?;
        if peer.session == identity.session {
            return Ok(ServicePeer::OperatorApi(peer));
        }
    }
    reject(Reject::Unauthenticated)
}
