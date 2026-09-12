use super::*;
mod runtime_restart;
const PRODUCER: &str = "rx.internal.evidence-producer.v1";

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Internal composition lookup after the remote Host has authenticated; never a user RPC.
    pub fn current_evidence_producer(&mut self, principal: &Name) -> Result<EvidenceProducer> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let (_, producer): (_, EvidenceProducer) = load(tx, "producer", principal, PRODUCER)?;
            authorize(
                tx,
                &Identity {
                    principal: principal.clone(),
                    session: producer.session.clone(),
                    terminal: None,
                },
                meta,
                &clock.now(),
                None,
                Role::Host,
                false,
            )?;
            Ok(producer)
        })
    }
    /// Called only after the transport verifies a registered Host certificate and exact contract.
    /// Identical reconnects recover the same durable session even if the prior reply was lost.
    pub fn open_evidence_producer(
        &mut self,
        principal: &Name,
        peer_boot: Id,
        journal: Id,
        authentication_binding: Digest,
    ) -> Result<Session> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (_, p): (_, Principal) = load(tx, "principal", principal, PRINCIPAL)?;
            if !p.active
                || !p.roles.contains(&Role::Host)
                || p.roles.contains(&Role::OperatorApi)
                || p.cells.is_empty()
            {
                return reject(Reject::Unauthenticated);
            }
            let previous = tx.get(&key("producer", principal))?;
            let mut previous_runtime_boot = None;
            if let Some(row) = &previous {
                let old: EvidenceProducer = decode(row, PRODUCER)?;
                let (_, mut session): (_, Session) = load(tx, "session", &old.session, SESSION)?;
                if old.peer_boot == peer_boot
                    && old.journal == journal
                    && old.authentication_binding == authentication_binding
                    && session.active
                    && session.runtime_boot == meta.runtime_boot
                    && session.expires_at.clock_id == now.clock_id
                    && session.expires_at.ticks_ns > now.ticks_ns
                {
                    return Ok(session);
                }
                if old.principal == *principal
                    && old.peer_boot == peer_boot
                    && old.journal == journal
                    && old.authentication_binding == authentication_binding
                    && session.id == old.session
                    && session.principal == *principal
                    && session.terminal.is_none()
                    && session.active
                    && session.runtime_boot != meta.runtime_boot
                    && session.expires_at.clock_id == now.clock_id
                    && meta.clock_id == now.clock_id
                    && session.expires_at.ticks_ns > now.ticks_ns
                {
                    previous_runtime_boot = Some(session.runtime_boot.clone());
                }
                let (rev, _): (_, Session) = load(tx, "session", &old.session, SESSION)?;
                session.active = false;
                save(tx, "session", &session.id, Some(rev), SESSION, &session)?;
            }
            let registrations = tx
                .scan("host/")?
                .iter()
                .map(|r| decode::<HostRegistration>(r, HOST))
                .collect::<Result<Vec<_>>>()?;
            let runtime_only = if let Some(previous_boot) = previous_runtime_boot {
                runtime_restart::covers_registrations(
                    tx,
                    meta,
                    principal,
                    &previous_boot,
                    &registrations,
                )?
            } else {
                false
            };
            // A proven P-only session replacement already has exact durable RuntimeRestart
            // restrictions. Preserve them; this classification restores no Host registration,
            // grant, qualification, permit, mandate or Run. Unknown changes still revoke.
            if !runtime_only {
                let mut touched = BTreeSet::new();
                for host in registrations {
                    if &host.id == principal && !touched.contains(&host.cell) {
                        touched.extend(invalidate_closure(
                            tx,
                            &host.cell,
                            BlockReason::DeviceRestart,
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
            // Service identity lifetime is tied to Runtime/peer incarnation and current role,
            // not a renewable browser login timeout. This grants no native operating permission.
            save(tx, "session", &session.id, None, SESSION, &session)?;
            tx.put(
                &key("producer", principal),
                previous.map(|r| r.revision),
                &doc(
                    PRODUCER,
                    &EvidenceProducer {
                        principal: principal.clone(),
                        session: session.id.clone(),
                        peer_boot,
                        journal,
                        authentication_binding,
                        cells: BTreeMap::new(),
                    },
                )?,
            )?;
            event(tx, "rx.event.evidence-producer-opened.v1", &session.id)?;
            Ok(session)
        })
    }
    pub fn negotiate_evidence_cell(
        &mut self,
        identity: &Identity,
        definition: Digest,
    ) -> Result<Name> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let p = authorize(tx, identity, meta, &clock.now(), None, Role::Host, false)?;
            let (revision, mut producer): (_, EvidenceProducer) =
                load(tx, "producer", &p.id, PRODUCER)?;
            if producer.session != identity.session {
                return reject(Reject::Unauthenticated);
            }
            let cells = tx
                .scan("cell/")?
                .iter()
                .map(|r| decode::<Cell>(r, CELL))
                .collect::<Result<Vec<_>>>()?;
            let matching: Vec<_> = cells
                .iter()
                .filter(|c| {
                    c.configuration.definition.sha256 == definition
                        && c.configuration.hosts.contains(&p.id)
                        && p.cells.contains(&c.configuration.id)
                })
                .collect();
            if matching.len() != 1 {
                return reject(Reject::CapabilityMissing);
            }
            let cell = matching[0].configuration.id.clone();
            producer.cells.insert(cell.clone(), definition);
            save(tx, "producer", &p.id, Some(revision), PRODUCER, &producer)?;
            Ok(cell)
        })
    }
    /// Read without granting session scopes beyond those that were explicitly negotiated.
    pub fn inspect_evidence_producer(&mut self, identity: &Identity) -> Result<EvidenceProducer> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let p = authorize(tx, identity, meta, &clock.now(), None, Role::Host, false)?;
            let (_, producer): (_, EvidenceProducer) = load(tx, "producer", &p.id, PRODUCER)?;
            if producer.session != identity.session {
                return reject(Reject::Unauthenticated);
            }
            Ok(producer)
        })
    }
}
