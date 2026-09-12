use super::*;

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn qualification_tasks(
        &mut self,
        identity: &Identity,
        after: Option<&Id>,
    ) -> Result<Vec<a::Task>> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let p = authorize(tx, identity, meta, &clock.now(), None, Role::Host, false)?;
            let mut tasks = Vec::new();
            for row in tx.scan("qualificationtask/")? {
                let t: a::Task = decode(&row, TASK)?;
                if t.host != p.id
                    || t.cells.iter().any(|c| !p.cells.contains(c))
                    || after.is_some_and(|id| &t.id <= id)
                    || t.disputed
                {
                    continue;
                }
                let b = batch(tx, &t.batch)?;
                if b.state != a::State::Suspended
                    || (t.phase == Phase::SendEntered && t.receipt.is_none())
                {
                    tasks.push(t);
                }
            }
            tasks.sort_by(|a, b| a.id.cmp(&b.id));
            tasks.truncate(16);
            Ok(tasks)
        })
    }
    pub fn bind_qualification_request(
        &mut self,
        identity: &Identity,
        id: &Id,
        observation: host::Observation,
        started: TimePoint,
    ) -> Result<a::Task> {
        observation.validate().map_err(StoreError::Invalid)?;
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (rev, mut t) = task(tx, id)?;
            host_access(tx, meta, &now, identity, &t)?;
            let b = batch(tx, &t.batch)?;
            sender(tx, meta, &now, &b, &t)?;
            if t.request.is_some() {
                return Ok(t);
            }
            let s = &observation.snapshot;
            if now.age_ns(&started).is_none_or(|age| age > 3_000_000_000)
                || s.host != t.host
                || s.host_boot != t.host_boot
                || s.delivery_journal != t.journal
                || s.cells.iter().map(|c| &c.cell).collect::<BTreeSet<_>>()
                    != t.cells.iter().collect()
            {
                return reject(Reject::ContinuityUnproven);
            }
            let change = process_change::change(tx, &b.change, &b.origin)?;
            let application = change
                .application
                .as_ref()
                .ok_or(StoreError::Integrity("application absent".into()))?;
            let source = application
                .host_proofs
                .iter()
                .find(|p| p.host == t.host)
                .ok_or(StoreError::Rejected(Reject::HostNotPrepared))?;
            let (_, source_task) = configuration_dispatch::read(tx, &source.task)?;
            let source_receipt = source_task
                .receipt
                .as_ref()
                .ok_or(StoreError::Rejected(Reject::HostNotPrepared))?;
            let mut targets = Vec::new();
            for id in &t.cells {
                let target = b
                    .cells
                    .iter()
                    .find(|c| &c.cell == id)
                    .ok_or(StoreError::Integrity("missing issued cell".into()))?;
                let observed = s.cells.iter().find(|c| &c.cell == id).unwrap();
                let (_, cell): (_, Cell) = load(tx, "cell", id, CELL)?;
                let context = observed
                    .applied
                    .as_ref()
                    .ok_or(StoreError::Rejected(Reject::HostNotPrepared))?;
                if observed.definition != cell.configuration.definition.sha256
                    || observed.envelope != cell.configuration.envelope.sha256
                    || observed.environment.as_str()
                        != match cell.configuration.environment {
                            Environment::Simulation => "SIMULATION",
                            Environment::Physical => "PHYSICAL",
                        }
                    || observed.epoch != target.epoch
                    || observed.scopes != target.scopes
                    || observed.blocked.iter().collect::<BTreeSet<_>>()
                        != cell.blocks.iter().map(|b| &b.id).collect()
                    || context.change != b.change
                    || context.request != source.task
                    || context.receipt_sequence != source_receipt.sequence
                    || context.configuration != target.configuration.sha256
                    || context.binding_digest != s.binding_digest
                {
                    return reject(Reject::ContinuityUnproven);
                }
                let fence = b
                    .job
                    .request
                    .fences
                    .iter()
                    .find(|f| f.cell == *id && f.host == t.host)
                    .ok_or(StoreError::Rejected(Reject::HostNotPrepared))?;
                targets.push(host::CellTarget {
                    cell: id.clone(),
                    configuration: target.configuration.sha256,
                    context_request: context.request.clone(),
                    context_sequence: context.receipt_sequence,
                    definition: observed.definition,
                    envelope: observed.envelope,
                    environment: observed.environment.clone(),
                    qualification: target.qualification.id.clone(),
                    qualification_revision: target.qualification.revision,
                    dependencies: target.qualification.dependencies.clone(),
                    limitations: target.limitations.clone(),
                    allowed_intents: cell
                        .configuration
                        .steps
                        .iter()
                        .filter(|s| s.host == t.host)
                        .map(|s| s.intent.digest().map_err(domain_error))
                        .collect::<Result<BTreeSet<_>>>()?
                        .into_iter()
                        .collect(),
                    purposes: target.purposes.iter().cloned().collect(),
                    epoch: target.epoch,
                    scopes: target.scopes.clone(),
                    fence_request: fence.message.clone(),
                    required_blocks: observed.blocked.clone(),
                });
            }
            if targets
                .iter()
                .map(|t| t.definition)
                .collect::<BTreeSet<_>>()
                .len()
                != targets.len()
            {
                return reject(Reject::InvalidInput);
            }
            let request = host::Request {
                schema: name("rx.host-qualification-request.v1"),
                id: t.id.clone(),
                host: t.host.clone(),
                expected_host_boot: t.host_boot.clone(),
                delivery_journal: t.journal.clone(),
                binding_digest: s.binding_digest,
                change: b.change,
                review: b.job.request.id,
                review_revision: b.report_revision,
                review_digest: b.report_digest,
                decision_revision: b.decision_revision,
                policy_digest: b.job.request.policy_digest,
                application_digest: b.job.request.application_digest,
                cells: targets,
            };
            t.digest = Some(request.digest().map_err(StoreError::Invalid)?);
            t.request = Some(request);
            t.phase = Phase::Prepared;
            t.issue = None;
            record_task(tx, &t, Some(rev))?;
            Ok(t)
        })
    }
    pub fn enter_qualification_send(
        &mut self,
        identity: &Identity,
        id: &Id,
        retry: bool,
    ) -> Result<a::Emission> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (rev, mut t) = task(tx, id)?;
            host_access(tx, meta, &now, identity, &t)?;
            if t.phase == Phase::SendEntered && !retry {
                return Ok(a::Emission::Lookup { request: t.id });
            }
            let b = batch(tx, &t.batch)?;
            sender(tx, meta, &now, &b, &t)?;
            if t.disputed || t.receipt.is_some() || t.phase == Phase::Retired {
                return reject(Reject::StaleRevision);
            }
            if retry && !(t.phase == Phase::SendEntered && t.issue == Some(Issue::ReceiptMissing)) {
                return reject(Reject::ContinuityUnproven);
            }
            let request = t
                .request
                .clone()
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            t.phase = Phase::SendEntered;
            t.issue = None;
            record_task(tx, &t, Some(rev))?;
            Ok(a::Emission::Send {
                request: Box::new(request),
            })
        })
    }
    pub fn qualification_issue(
        &mut self,
        identity: &Identity,
        id: &Id,
        issue: Issue,
    ) -> Result<()> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (rev, mut t) = task(tx, id)?;
            host_access(tx, meta, &clock.now(), identity, &t)?;
            if t.issue != Some(issue) {
                t.issue = Some(issue);
                record_task(tx, &t, Some(rev))?;
            }
            Ok(())
        })
    }
    pub fn record_qualification_observation(
        &mut self,
        identity: &Identity,
        id: &Id,
        observation: host::Observation,
        started: TimePoint,
    ) -> Result<a::Task> {
        observation.validate().map_err(StoreError::Invalid)?;
        let meta = &self.installation;
        let clock = &self.clock;
        let result = self.repository.transact(|tx| {
            let now = clock.now();
            let (rev, mut t) = task(tx, id)?;
            host_access(tx, meta, &now, identity, &t)?;
            if t.phase != Phase::SendEntered || observation.snapshot.host != t.host {
                return reject(Reject::InvalidInput);
            }
            let before = canonical::bytes(&t).map_err(domain_error)?;
            if let Some(r) = &observation.receipt {
                if Some(r.request_digest) != t.digest {
                    return reject(Reject::InvalidInput);
                }
                if let Some(old) = &t.receipt {
                    if canonical::bytes(old).map_err(domain_error)?
                        != canonical::bytes(r).map_err(domain_error)?
                    {
                        t.disputed = true;
                    }
                } else {
                    t.receipt = Some(r.clone());
                }
            } else if t.receipt.is_some() || observation.accepted.iter().any(|a| a.request == t.id)
            {
                t.disputed = true;
            }
            let b = batch(tx, &t.batch)?;
            let expected = t.request.as_ref().is_some_and(|r| {
                observation.snapshot.binding_digest == r.binding_digest
                    && observation.snapshot.cells.len() == r.cells.len()
                    && r.cells.iter().all(|c| {
                        observation.snapshot.cells.iter().any(|s| {
                            s.cell == c.cell
                                && s.epoch == c.epoch
                                && s.scopes == c.scopes
                                && s.definition == c.definition
                                && s.envelope == c.envelope
                                && s.environment == c.environment
                                && s.applied.as_ref().is_some_and(|a| {
                                    a.configuration == c.configuration
                                        && a.request == c.context_request
                                        && a.receipt_sequence == c.context_sequence
                                })
                                && (b.state == a::State::Active
                                    || s.blocked.iter().collect::<BTreeSet<_>>()
                                        == c.required_blocks.iter().collect())
                        })
                    })
            });
            t.issue = if t.disputed {
                Some(Issue::ObservationMismatch)
            } else if observation.snapshot.host_boot != t.host_boot
                || observation.snapshot.delivery_journal != t.journal
            {
                Some(Issue::HostGenerationChanged)
            } else if !expected {
                Some(Issue::ObservationMismatch)
            } else if observation.receipt.is_none() {
                Some(Issue::ReceiptMissing)
            } else if observation
                .receipt
                .as_ref()
                .is_some_and(|r| r.status == host::Status::Accepted)
                && !observation.receipt_matches_current_host
            {
                Some(Issue::ObservationMismatch)
            } else {
                None
            };
            t.observation = Some(observation.clone());
            if canonical::bytes(&t).map_err(domain_error)? != before {
                record_task(tx, &t, Some(rev))?;
            }
            if b.state == a::State::Active
                && (t.disputed
                    || t.issue == Some(Issue::HostGenerationChanged)
                    || t.issue == Some(Issue::ObservationMismatch))
            {
                suspend(
                    tx,
                    &b,
                    Some(meta),
                    name("HOST_QUALIFICATION_CONTINUITY_LOST"),
                )?;
            }
            Ok(t)
        })?;
        let now = self.clock.now();
        self.qualification_reads
            .retain(|_, (at, _)| now.age_ns(at).is_some_and(|a| a <= 3_000_000_000));
        if now.age_ns(&started).is_some_and(|age| age <= 3_000_000_000) {
            self.qualification_reads.insert(
                id.clone(),
                (
                    started,
                    canonical::digest("RX-QUALIFICATION-OBSERVATION-v1", &observation)
                        .map_err(domain_error)?,
                ),
            );
        }
        Ok(result)
    }
    pub fn qualification_batch(
        &mut self,
        identity: &Identity,
        cell: &Name,
        id: &Id,
    ) -> Result<a::View> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let b = batch(tx, id)?;
            if b.origin != *cell {
                return reject(Reject::Forbidden);
            }
            access(tx, meta, &clock.now(), identity, &b, false)?;
            view(tx, meta, b)
        })
    }
}
