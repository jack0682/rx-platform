use super::*;

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn install_cell(
        &mut self,
        identity: &Identity,
        configuration: CellConfiguration,
    ) -> Result<Counter> {
        self.install_cell_inner(identity, None, configuration)
    }

    pub fn install_cell_request(
        &mut self,
        identity: &Identity,
        request_key: &str,
        configuration: CellConfiguration,
    ) -> Result<Counter> {
        self.install_cell_inner(identity, Some(request_key), configuration)
    }

    fn install_cell_inner(
        &mut self,
        identity: &Identity,
        request_key: Option<&str>,
        configuration: CellConfiguration,
    ) -> Result<Counter> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let principal = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&configuration.id),
                Role::Engineer,
                false,
            )?;
            let cache = request_key
                .map(|key_| {
                    request(
                        meta,
                        &principal,
                        "Configuration.InstallCell",
                        key_,
                        &configuration,
                    )
                })
                .transpose()?;
            if let Some((scope, fingerprint)) = &cache
                && let Some(revision) =
                    prior(tx, scope, *fingerprint, "rx.internal.install-cell-reply.v1")?
            {
                return Ok(revision);
            }
            lifecycle::require_serving(tx)?;
            validate_configuration(&configuration)?;
            let cell = Cell {
                mode: Some(OperatingMode::Setup),
                commissioning: Some(Commissioning::NotCommissioned),
                epoch: Counter(1),
                scope_epochs: configuration
                    .scopes
                    .iter()
                    .cloned()
                    .map(|s| (s, Counter(1)))
                    .collect(),
                configuration,
                qualification: None,
                blocks: vec![],
                open_cases: vec![],
            };
            let revision = save(tx, "cell", &cell.configuration.id, None, CELL, &cell)?;
            event(tx, "rx.event.cell-installed.v1", &cell)?;
            if let Some((scope, fingerprint)) = cache {
                remember(
                    tx,
                    &scope,
                    fingerprint,
                    "rx.internal.install-cell-reply.v1",
                    &revision,
                )?;
            }
            Ok(revision)
        })
    }
    pub fn qualify(
        &mut self,
        identity: &Identity,
        cell_id: &Name,
        expected: Counter,
        evidence: Vec<ArtifactRef>,
        dependencies: Vec<Digest>,
    ) -> Result<Qualification> {
        let clock = &self.clock;
        let meta = &self.installation;
        let authority = &self.authority;
        self.repository.transact(|tx| {
            let now = clock.now();
            let principal = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(cell_id),
                Role::Verifier,
                false,
            )?;
            lifecycle::require_serving(tx)?;
            let (revision, mut cell): (_, Cell) = load(tx, "cell", cell_id, CELL)?;
            check_revision(revision, expected)?;
            if evidence.is_empty()
                || !authority.verify(&cell.configuration, &evidence, &dependencies)
            {
                return reject(Reject::QualificationRequired);
            }
            if cell.qualification.is_some() {
                return reject(Reject::InvalidInput);
            }
            let qualification = Qualification {
                id: id(),
                revision: Counter(1),
                envelope_digest: cell.configuration.envelope.sha256,
                environment: cell.configuration.environment,
                evidence,
                dependencies,
                reviewed_by: principal.id,
            };
            cell.qualification = Some(qualification.clone());
            cell.commissioning = Some(Commissioning::Commissioned);
            save(tx, "cell", cell_id, Some(revision), CELL, &cell)?;
            event(tx, "rx.event.qualification-registered.v1", &qualification)?;
            Ok(qualification)
        })
    }
    pub fn inspect_cell(&mut self, identity: &Identity, cell_id: &Name) -> Result<(Counter, Cell)> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            authorize_read(tx, identity, meta, &now, cell_id)?;
            load(tx, "cell", cell_id, CELL)
        })
    }
    pub fn register_host(
        &mut self,
        identity: &Identity,
        registration: HostRegistration,
    ) -> Result<()> {
        let clock = &self.clock;
        let meta = &self.installation;
        self.repository.transact(|tx| {
            let now = clock.now();
            let principal = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&registration.cell),
                Role::Host,
                false,
            )?;
            let (_, cell): (_, Cell) = load(tx, "cell", &registration.cell, CELL)?;
            if registration.id != principal.id
                || registration.session != identity.session
                || !cell.configuration.hosts.contains(&registration.id)
                || registration.epoch != cell.epoch
                || registration.scopes != cell.scope_epochs
                || registration.grant.fence.0 == 0
                || registration.grant.owner.as_str() != meta.id.as_str()
                || registration.grant.valid_until.clock_id != now.clock_id
                || registration.grant.valid_until.ticks_ns <= now.ticks_ns
            {
                return reject(Reject::StaleEpoch);
            }
            let resources: BTreeSet<_> = cell
                .configuration
                .steps
                .iter()
                .filter(|s| s.host == principal.id)
                .flat_map(|s| s.intent.resource_set.iter().cloned())
                .collect();
            if resources != registration.grant.resources.iter().cloned().collect() {
                return reject(Reject::CapabilityMissing);
            }
            for record in tx.scan("host/")? {
                let known: HostRegistration = decode(&record, HOST)?;
                if known.id == registration.id
                    && (known.boot_id != registration.boot_id
                        || known.session != registration.session
                        || known.source_sessions.iter().any(|(source, generation)| {
                            registration
                                .source_sessions
                                .get(source)
                                .is_some_and(|next| next != generation)
                        }))
                {
                    return reject(Reject::ContinuityUnproven);
                }
            }
            let key_ = key("host", (&registration.cell, &registration.id));
            let previous = tx.get(&key_)?;
            if let Some(record) = &previous {
                let old: HostRegistration = decode(record, HOST)?;
                if old.boot_id != registration.boot_id
                    || old.session != registration.session
                    || old.source_sessions != registration.source_sessions
                {
                    // Do not silently turn a restarted Host into a prepared one.
                    return reject(Reject::ContinuityUnproven);
                }
                if registration.grant.fence < old.grant.fence {
                    return reject(Reject::StaleEpoch);
                }
            }
            tx.put(
                &key_,
                previous.map(|r| r.revision),
                &doc(HOST, &registration)?,
            )?;
            event(tx, "rx.event.host-registered.v1", &registration)?;
            Ok(())
        })
    }
}
