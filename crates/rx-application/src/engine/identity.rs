use super::*;

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    /// Trusted local credential adapter only. The duration is applied when the writer runs.
    pub fn authenticated_user_session(
        &mut self,
        principal: &Name,
        session_id: Id,
        ttl_ns: Counter,
    ) -> Result<Session> {
        self.issue_user_session(principal, session_id, ttl_ns, None)
    }
    /// Only a credential adapter with a verified TLS client certificate may call this entry point.
    pub fn authenticated_terminal_user_session(
        &mut self,
        principal: &Name,
        session_id: Id,
        ttl_ns: Counter,
        certificate: Digest,
    ) -> Result<Session> {
        self.issue_user_session(principal, session_id, ttl_ns, Some(certificate))
    }
    fn issue_user_session(
        &mut self,
        principal: &Name,
        session_id: Id,
        ttl_ns: Counter,
        certificate: Option<Digest>,
    ) -> Result<Session> {
        let clock = &self.clock;
        let boot = self.installation.runtime_boot.clone();
        self.repository.transact(|tx| {
            let (_, p): (_, Principal) = load(tx, "principal", principal, PRINCIPAL)
                .map_err(|e| missing_as(e, Reject::Unauthenticated))?;
            if !p.active
                || p.roles.is_empty()
                || p.roles.contains(&Role::Host)
                || p.roles.contains(&Role::Executor)
                || p.roles.contains(&Role::OperatorApi)
                || ttl_ns.0 == 0
                || ttl_ns.0 > 28_800_000_000_000
            {
                return reject(Reject::Unauthenticated);
            }
            let terminal = if let Some(certificate) = certificate {
                let mut found = None;
                for record in tx.scan("terminal/")? {
                    let terminal: Terminal = decode(&record, TERMINAL)?;
                    if terminal.active && terminal.certificate_digest == certificate {
                        if found.is_some() {
                            return reject(Reject::Forbidden);
                        }
                        if !terminal.cells.iter().any(|c| p.cells.contains(c)) {
                            return reject(Reject::Forbidden);
                        }
                        found = Some(TerminalBinding {
                            id: terminal.id,
                            certificate_digest: certificate,
                            revision: record.revision,
                        });
                    }
                }
                Some(found.ok_or(StoreError::Rejected(Reject::Forbidden))?)
            } else {
                None
            };
            let mut expires_at = clock.now();
            expires_at.ticks_ns = Counter(
                expires_at
                    .ticks_ns
                    .0
                    .checked_add(ttl_ns.0)
                    .ok_or(StoreError::Rejected(Reject::InvalidInput))?,
            );
            let session = Session {
                terminal,
                id: session_id,
                principal: principal.clone(),
                runtime_boot: boot,
                expires_at,
                active: true,
            };
            save(tx, "session", &session.id, None, SESSION, &session)?;
            event(tx, "rx.event.user-session-opened.v1", &session.id)?;
            Ok(session)
        })
    }

    /// Credentials are verified by the adapter; roles are loaded from the stored principal.
    pub fn authenticated_session(
        &mut self,
        principal: &Name,
        session_id: Id,
        expires_at: TimePoint,
    ) -> Result<Session> {
        let now = self.clock.now();
        let boot = self.installation.runtime_boot.clone();
        self.repository.transact(|tx| {
            let (_, p): (_, Principal) = load(tx, "principal", principal, PRINCIPAL)?;
            if !p.active
                || now.clock_id != expires_at.clock_id
                || now.ticks_ns >= expires_at.ticks_ns
            {
                return reject(Reject::Unauthenticated);
            }
            let session = Session {
                terminal: None,
                id: session_id,
                principal: principal.clone(),
                runtime_boot: boot,
                expires_at,
                active: true,
            };
            save(tx, "session", &session.id, None, SESSION, &session)?;
            Ok(session)
        })
    }
    pub fn put_principal(
        &mut self,
        identity: &Identity,
        value: Principal,
        expected: Option<Counter>,
    ) -> Result<Counter> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            authorize(tx, identity, meta, &now, None, Role::AccountAdmin, false)?;
            if let Some(old) = tx.get(&key("principal", &value.id))? {
                let previous: Principal = decode(&old, PRINCIPAL)?;
                if previous.client_namespace != value.client_namespace {
                    return reject(Reject::InvalidInput);
                }
                let lost = |role: Role| {
                    previous.roles.contains(&role)
                        && (!value.active || !value.roles.contains(&role))
                };
                let scopes_reduced = !previous.cells.is_subset(&value.cells);
                if lost(Role::Operator)
                    || lost(Role::Executor)
                    || lost(Role::Host)
                    || scopes_reduced
                {
                    for record in tx.scan("cell/")? {
                        let mut cell: Cell = decode(&record, CELL)?;
                        if previous.cells.contains(&cell.configuration.id) {
                            invalidate_cell(
                                tx,
                                &mut cell,
                                record.revision,
                                BlockReason::AuthorityRevoked,
                            )?;
                        }
                    }
                }
            }
            let revision = save(tx, "principal", &value.id, expected, PRINCIPAL, &value)?;
            event(tx, "rx.event.principal-changed.v1", &value)?;
            Ok(revision)
        })
    }
    pub fn put_terminal(
        &mut self,
        identity: &Identity,
        value: Terminal,
        expected: Option<Counter>,
    ) -> Result<Counter> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            authorize(tx, identity, meta, &now, None, Role::AccountAdmin, false)?;
            let previous = tx.get(&key("terminal", &value.id))?;
            let mut affected = BTreeSet::new();
            if let Some(row) = previous {
                check_revision(
                    row.revision,
                    expected.ok_or(StoreError::Rejected(Reject::StaleRevision))?,
                )?;
                let old: Terminal = decode(&row, TERMINAL)?;
                if canonical::bytes(&old).map_err(domain_error)?
                    == canonical::bytes(&value).map_err(domain_error)?
                {
                    return Ok(row.revision);
                }
                affected.extend(old.cells);
                affected.extend(value.cells.iter().cloned());
            }
            let revision = save(tx, "terminal", &value.id, expected, TERMINAL, &value)?;
            let mut touched = BTreeSet::new();
            for cell in affected {
                if !touched.contains(&cell) && tx.get(&key("cell", &cell))?.is_some() {
                    touched.extend(invalidate_closure(
                        tx,
                        &cell,
                        BlockReason::AuthorityRevoked,
                    )?);
                }
            }
            event(tx, "rx.event.terminal-changed.v1", &value)?;
            Ok(revision)
        })
    }
}
