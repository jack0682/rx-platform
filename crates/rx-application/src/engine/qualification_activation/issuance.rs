use super::*;

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn prepare_qualification_issue(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: a::IssueRequest,
    ) -> Result<a::Preflight> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let j = requalification::job(tx, &input.review, &input.cell)?;
            let p = authorize(
                tx,
                identity,
                meta,
                &now,
                Some(&input.cell),
                Role::ReleaseManager,
                true,
            )?;
            if j.request
                .cells
                .iter()
                .any(|c| !p.cells.contains(&c.profile.cell))
            {
                return reject(Reject::Forbidden);
            }
            let (scope, fp) = request(meta, &p, "Qualification.Issue", key_.as_str(), &input)?;
            if let Some(b) = prior(tx, &scope, fp, BATCH)? {
                return Ok(a::Preflight::Recorded(Box::new(b)));
            }
            let slot = key(
                "qualificationbatchslot",
                (
                    &input.review,
                    input.report_revision,
                    input.decision_revision,
                ),
            );
            if let Some(row) = tx.get(&slot)? {
                let id: Id = decode(&row, "rx.qualification-batch-ref.v1")?;
                let b = batch(tx, &id)?;
                if canonical::bytes(&b.issuance).map_err(domain_error)?
                    != canonical::bytes(&input).map_err(domain_error)?
                {
                    return Err(StoreError::KeyConflict);
                }
                let sender = b.sender.identity();
                if b.state != a::State::Pending
                    || (sender.principal == identity.principal
                        && sender.session == identity.session
                        && sender.terminal == identity.terminal)
                {
                    remember(tx, &scope, fp, BATCH, &b)?;
                    return Ok(a::Preflight::Recorded(Box::new(b)));
                }
                pending_current(tx, meta, &b)?;
            }
            requalification::current(tx, meta, &j)?;
            let (v, d) = approved(
                tx,
                meta,
                &j,
                input.report_revision,
                input.report_digest,
                input.decision_revision,
            )?;
            let ids: BTreeSet<_> = j.request.cells.iter().map(|c| &c.profile.cell).collect();
            if input.expected_cells.keys().collect::<BTreeSet<_>>() != ids
                || input.clear_blocks.keys().collect::<BTreeSet<_>>() != ids
            {
                return reject(Reject::InvalidInput);
            }
            for c in &j.request.cells {
                let (rev, cell): (_, Cell) = load(tx, "cell", &c.profile.cell, CELL)?;
                check_revision(rev, input.expected_cells[&c.profile.cell])?;
                owned_clear(
                    tx,
                    &j.request.change,
                    &cell,
                    &input.clear_blocks[&c.profile.cell],
                )?;
            }
            if !requalification::fences_confirmed(tx, &j)? {
                return reject(Reject::HostNotPrepared);
            }
            quiet(tx, &j)?;
            Ok(a::Preflight::Verify(Box::new(ticket(
                tx,
                meta,
                now,
                identity.clone(),
                key_.clone(),
                a::Action::Issue(input),
                (j, v, d),
            )?)))
        })
    }
    pub fn commit_qualification_issue(&mut self, p: a::Prepared) -> Result<a::Batch> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let t = p.ticket;
            let a::Action::Issue(input) = &t.action else {
                return reject(Reject::InvalidInput);
            };
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let actor = authorize(
                tx,
                &t.identity,
                meta,
                &now,
                Some(&input.cell),
                Role::ReleaseManager,
                true,
            )?;
            if t.job
                .request
                .cells
                .iter()
                .any(|c| !actor.cells.contains(&c.profile.cell))
            {
                return reject(Reject::Forbidden);
            }
            let (scope, fp) = request(meta, &actor, "Qualification.Issue", t.key.as_str(), input)?;
            if let Some(b) = prior(tx, &scope, fp, BATCH)? {
                return Ok(b);
            }
            if now
                .age_ns(&t.issued)
                .is_none_or(|age| age >= 30_000_000_000)
                || package_intake::current(tx, meta)?.as_ref() != Some(&t.registration)
            {
                return reject(Reject::StaleRevision);
            }
            requalification::current(tx, meta, &t.job)?;
            approved(
                tx,
                meta,
                &t.job,
                input.report_revision,
                input.report_digest,
                input.decision_revision,
            )?;
            quiet(tx, &t.job)?;
            if !requalification::fences_confirmed(tx, &t.job)? {
                return reject(Reject::HostNotPrepared);
            }
            let slot = key(
                "qualificationbatchslot",
                (
                    &input.review,
                    input.report_revision,
                    input.decision_revision,
                ),
            );
            if let Some(row) = tx.get(&slot)? {
                let id: Id = decode(&row, "rx.qualification-batch-ref.v1")?;
                let mut b = batch(tx, &id)?;
                if canonical::bytes(&b.issuance).map_err(domain_error)?
                    != canonical::bytes(input).map_err(domain_error)?
                {
                    return Err(StoreError::KeyConflict);
                }
                pending_current(tx, meta, &b)?;
                let (terminal, certificate) = t
                    .identity
                    .terminal
                    .clone()
                    .ok_or(StoreError::Rejected(Reject::Forbidden))?;
                let previous = b.revision;
                b.revision = b.revision.increment().map_err(domain_error)?;
                b.sender = Sender {
                    principal: actor.id,
                    session: t.identity.session.clone(),
                    terminal,
                    certificate,
                };
                record_batch(tx, &b, Some(previous))?;
                remember(tx, &scope, fp, BATCH, &b)?;
                return Ok(b);
            }
            for row in tx.scan("qualificationtask/")? {
                let task: a::Task = decode(&row, TASK)?;
                let old = batch(tx, &task.batch)?;
                if old.change == t.job.request.change
                    && task.phase == Phase::SendEntered
                    && (task.receipt.is_none() || task.disputed)
                {
                    return reject(Reject::ContinuityUnproven);
                }
            }
            let bid = id();
            let mut cells = Vec::new();
            let mut hosts = BTreeMap::<Name, Vec<Name>>::new();
            for target in &t.job.request.cells {
                let (rev, cell): (_, Cell) = load(tx, "cell", &target.profile.cell, CELL)?;
                check_revision(rev, input.expected_cells[&target.profile.cell])?;
                owned_clear(
                    tx,
                    &t.job.request.change,
                    &cell,
                    &input.clear_blocks[&target.profile.cell],
                )?;
                let evidence = t
                    .version
                    .report
                    .checks
                    .iter()
                    .filter(|r| r.cell == target.profile.cell)
                    .flat_map(|r| r.evidence.iter().cloned())
                    .collect();
                let deps = target
                    .profile
                    .references()
                    .iter()
                    .map(|r| r.sha256)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                cells.push(a::IssuedCell {
                    cell: target.profile.cell.clone(),
                    configuration: target.profile.configuration.clone(),
                    epoch: target.epoch,
                    scopes: target.scopes.clone(),
                    expected_revision: rev,
                    clear_blocks: input.clear_blocks[&target.profile.cell].clone(),
                    purposes: target.profile.purposes.clone(),
                    qualification: Qualification {
                        id: id(),
                        revision: Counter(1),
                        envelope_digest: target.profile.envelope.sha256,
                        environment: target.profile.environment,
                        evidence,
                        dependencies: deps,
                        reviewed_by: t.decision.decided_by.clone(),
                    },
                    limitations: target.profile.limitations.clone(),
                });
                for h in cell.configuration.hosts {
                    hosts
                        .entry(h)
                        .or_default()
                        .push(target.profile.cell.clone());
                }
            }
            let mut tasks = Vec::new();
            for (host, names) in hosts {
                let mut generation = None;
                for cell in &names {
                    let (_, r): (_, HostRegistration) = load(tx, "host", (cell, &host), HOST)?;
                    let next = (r.boot_id, r.delivery_journal, r.session);
                    if generation.as_ref().is_some_and(|v| v != &next) {
                        return reject(Reject::ContinuityUnproven);
                    }
                    generation = Some(next);
                }
                let (boot, journal, session) =
                    generation.ok_or(StoreError::Rejected(Reject::InvalidInput))?;
                let task = a::Task {
                    id: id(),
                    batch: bid.clone(),
                    host,
                    host_boot: boot,
                    journal,
                    session,
                    cells: names,
                    phase: Phase::AwaitingSnapshot,
                    request: None,
                    digest: None,
                    receipt: None,
                    observation: None,
                    issue: None,
                    disputed: false,
                };
                record_task(tx, &task, None)?;
                tasks.push(task.id);
            }
            let (terminal, certificate) = t.identity.terminal.clone().unwrap();
            let b = a::Batch {
                issuance: input.clone(),
                id: bid,
                origin: input.cell.clone(),
                change: t.job.request.change.clone(),
                revision: Counter(1),
                state: a::State::Pending,
                job: t.job,
                report_revision: input.report_revision,
                report_digest: input.report_digest,
                decision_revision: input.decision_revision,
                registration: t.registration,
                cells,
                tasks,
                issued_by: actor.id.clone(),
                issued_at: now,
                runtime_boot: meta.runtime_boot.clone(),
                activated_at: None,
                suspended_reason: None,
                sender: Sender {
                    principal: actor.id,
                    session: t.identity.session,
                    terminal,
                    certificate,
                },
            };
            record_batch(tx, &b, None)?;
            tx.put(&slot, None, &doc("rx.qualification-batch-ref.v1", &b.id)?)?;
            remember(tx, &scope, fp, BATCH, &b)?;
            Ok(b)
        })
    }
}
