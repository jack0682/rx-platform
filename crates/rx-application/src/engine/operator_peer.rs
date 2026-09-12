use super::*;
const PEER: &str = "rx.internal.operator-peer.v1";
pub(super) fn service_account(p: &Principal) -> bool {
    p.active
        && p.roles.contains(&Role::OperatorApi)
        && !p.cells.is_empty()
        && p.roles
            .iter()
            .all(|r| matches!(r, Role::OperatorApi | Role::Observer))
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Registered operator API transport, not an authenticated human or execution authority.
    pub fn open_operator_peer(
        &mut self,
        principal: &Name,
        peer_boot: Id,
        authentication_binding: Digest,
    ) -> Result<Session> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (_, p): (_, Principal) = load(tx, "principal", principal, PRINCIPAL)?;
            if !service_account(&p)
                || tx
                    .get(&key("retiredoperatorboot", (principal, &peer_boot)))?
                    .is_some()
            {
                return reject(Reject::Unauthenticated);
            }
            let previous = tx.get(&key("operatorpeer", principal))?;
            if let Some(row) = &previous {
                let old: OperatorPeer = decode(row, PEER)?;
                let (revision, mut session): (_, Session) =
                    load(tx, "session", &old.session, SESSION)?;
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
                        &key("retiredoperatorboot", (principal, &old.peer_boot)),
                        None,
                        &doc("rx.internal.retired-operator-boot.v1", &old.peer_boot)?,
                    )?;
                }
                session.active = false;
                save(
                    tx,
                    "session",
                    &session.id,
                    Some(revision),
                    SESSION,
                    &session,
                )?;
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
                "operatorpeer",
                principal,
                previous.map(|r| r.revision),
                PEER,
                &OperatorPeer {
                    principal: principal.clone(),
                    session: session.id.clone(),
                    peer_boot,
                    authentication_binding,
                    cells: BTreeMap::new(),
                },
            )?;
            event(tx, "rx.event.operator-peer-opened.v1", &session.id)?;
            Ok(session)
        })
    }
    pub fn negotiate_operator_cell(
        &mut self,
        identity: &Identity,
        definition: Digest,
    ) -> Result<Name> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let principal = authorize(
                tx,
                identity,
                meta,
                &clock.now(),
                None,
                Role::OperatorApi,
                false,
            )?;
            if !service_account(&principal) {
                return reject(Reject::Forbidden);
            }
            let (revision, mut peer): (_, OperatorPeer) =
                load(tx, "operatorpeer", &principal.id, PEER)?;
            if peer.session != identity.session {
                return reject(Reject::Unauthenticated);
            }
            let cells = tx
                .scan("cell/")?
                .into_iter()
                .map(|r| decode::<Cell>(&r, CELL))
                .collect::<Result<Vec<_>>>()?;
            let matches: Vec<_> = cells
                .iter()
                .filter(|c| {
                    c.configuration.definition.sha256 == definition
                        && principal.cells.contains(&c.configuration.id)
                })
                .collect();
            if matches.len() != 1 {
                return reject(Reject::InvalidInput);
            }
            let cell = matches[0].configuration.id.clone();
            if peer.cells.get(&cell) != Some(&definition) {
                peer.cells.insert(cell.clone(), definition);
                save(
                    tx,
                    "operatorpeer",
                    &principal.id,
                    Some(revision),
                    PEER,
                    &peer,
                )?;
            }
            Ok(cell)
        })
    }
}
