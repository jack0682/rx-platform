use super::*;

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn prepare_qualification_activation(
        &mut self,
        identity: &Identity,
        key_: &Id,
        input: a::Finalize,
    ) -> Result<a::Preflight> {
        let meta = &self.installation;
        let clock = &self.clock;
        let reads = &self.qualification_reads;
        self.repository.transact(|tx| {
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let b = batch(tx, &input.batch)?;
            if b.origin != input.cell {
                return reject(Reject::Forbidden);
            }
            let p = access(tx, meta, &now, identity, &b, true)?;
            let (scope, fp) = request(meta, &p, "Qualification.Activate", key_.as_str(), &input)?;
            if let Some(saved) = prior(tx, &scope, fp, BATCH)? {
                return Ok(a::Preflight::Recorded(Box::new(saved)));
            }
            if b.revision != input.expected
                || input.expected_cells.keys().collect::<BTreeSet<_>>()
                    != b.cells.iter().map(|c| &c.cell).collect()
            {
                return reject(Reject::StaleRevision);
            }
            pending_current(tx, meta, &b)?;
            for target in &b.cells {
                let (rev, cell): (_, Cell) = load(tx, "cell", &target.cell, CELL)?;
                check_revision(rev, input.expected_cells[&target.cell])?;
                owned_clear(tx, &b.job, &cell, &target.clear_blocks)?;
            }
            fresh_hosts(tx, &b, &now, reads)?;
            let (v, d) = approved(
                tx,
                meta,
                &b.job,
                b.report_revision,
                b.report_digest,
                b.decision_revision,
            )?;
            Ok(a::Preflight::Verify(Box::new(ticket(
                tx,
                meta,
                now,
                identity.clone(),
                key_.clone(),
                a::Action::Activate(input),
                (b.job, v, d),
            )?)))
        })
    }
    pub fn commit_qualification_activation(&mut self, p: a::Prepared) -> Result<a::Batch> {
        let meta = &self.installation;
        let clock = &self.clock;
        let reads = &self.qualification_reads;
        self.repository.transact(|tx| {
            let t = p.ticket;
            let a::Action::Activate(input) = &t.action else {
                return reject(Reject::InvalidInput);
            };
            let now = clock.now();
            lifecycle::require_serving(tx)?;
            let mut b = batch(tx, &input.batch)?;
            let actor = access(tx, meta, &now, &t.identity, &b, true)?;
            let (scope, fp) = request(
                meta,
                &actor,
                "Qualification.Activate",
                t.key.as_str(),
                input,
            )?;
            if let Some(b) = prior(tx, &scope, fp, BATCH)? {
                return Ok(b);
            }
            if b.revision != input.expected
                || now
                    .age_ns(&t.issued)
                    .is_none_or(|age| age >= 30_000_000_000)
                || t.registration != b.registration
            {
                return reject(Reject::StaleRevision);
            }
            pending_current(tx, meta, &b)?;
            fresh_hosts(tx, &b, &now, reads)?;
            for target in &b.cells {
                let (rev, mut cell): (_, Cell) = load(tx, "cell", &target.cell, CELL)?;
                check_revision(rev, input.expected_cells[&target.cell])?;
                owned_clear(tx, &b.job, &cell, &target.clear_blocks)?;
                cell.qualification = Some(target.qualification.clone());
                cell.commissioning = Some(Commissioning::Commissioned);
                cell.mode = Some(OperatingMode::Setup);
                cell.blocks
                    .retain(|block| !target.clear_blocks.contains(&block.id));
                save(tx, "cell", &target.cell, Some(rev), CELL, &cell)?;
                save(
                    tx,
                    "qualificationcertificate",
                    &target.qualification.id,
                    None,
                    "rx.qualification-certificate.v1",
                    &Certificate {
                        batch: b.id.clone(),
                        cell: target.cell.clone(),
                    },
                )?;
                event(
                    tx,
                    "rx.event.qualification-registered.v1",
                    &target.qualification,
                )?;
            }
            let old = b.revision;
            b.state = a::State::Active;
            b.revision = b.revision.increment().map_err(domain_error)?;
            b.activated_at = Some(now);
            record_batch(tx, &b, Some(old))?;
            let mut change = process_change::change(tx, &b.change, &b.origin)?;
            let old = change.revision;
            change.revision = change.revision.increment().map_err(domain_error)?;
            change.state = crate::process_change::State::QualifiedActive;
            change.qualification_activation = Some(b.id.clone());
            process_change::record(tx, &change, Some(old))?;
            remember(tx, &scope, fp, BATCH, &b)?;
            Ok(b)
        })
    }
    pub fn suspend_qualification(
        &mut self,
        identity: &Identity,
        key_: &Id,
        id: &Id,
        reason: Name,
    ) -> Result<a::Batch> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let b = batch(tx, id)?;
            let actor = access(tx, meta, &clock.now(), identity, &b, true)?;
            let (scope, fp) = request(
                meta,
                &actor,
                "Qualification.Suspend",
                key_.as_str(),
                &(id, &reason),
            )?;
            if let Some(b) = prior(tx, &scope, fp, BATCH)? {
                return Ok(b);
            }
            let changed = suspend(tx, &b, Some(meta), reason)?;
            remember(tx, &scope, fp, BATCH, &changed)?;
            Ok(changed)
        })
    }
}
