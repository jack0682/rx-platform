use super::*;
use rx_process_contract::execution_v2::executor as wire;
const PART_BINDING: &str = "rx.execution-part-binding.v2";
const REPLY: &str = "rx.execution-part.v2";
const BLOBS: crate::artifact_storage::BlobStore =
    crate::artifact_storage::BlobStore::new("executionpart", v2::MAX_DEFINITION_BYTES);
struct Current {
    revision: Counter,
    run: Run,
    cell: Cell,
    binding: data::RunBinding,
    object: data::ObjectBinding,
    domain: execution_configuration::Domain,
}
fn current(
    tx: &mut dyn Transaction,
    context: ProcessingContext<'_>,
    command: &BeginPartRequest,
) -> Result<Current> {
    let (_, cell_revision, cell) = executor_peer::executor_scope(tx, context, &command.cell)?;
    let (revision, run): (_, Run) = load(tx, "run", &command.run, RUN)?;
    if run.cell != command.cell || run.mandate.as_ref() != Some(&command.mandate) {
        return reject(Reject::MandateRevoked);
    }
    if let Some(expected) = command.expected_cell {
        check_revision(cell_revision, expected)?;
    }
    let (_, binding): (_, data::RunBinding) = load(tx, "executionrun", &run.id, BINDING)?;
    if binding.run != run.id
        || binding.cell != run.cell
        || binding.configuration
            != cell
                .configuration
                .reference()
                .map_err(StoreError::Integrity)?
    {
        return reject(Reject::StaleRevision);
    }
    if let Some(last) = run.part_ids.last() {
        let (_, part): (_, PartAttempt) = load(tx, "part", last, PART)?;
        if part.disposition != PartDisposition::ConfirmedCompleted {
            return reject(Reject::Busy);
        }
    }
    let object = object_current(tx, &cell, &binding, Counter(run.part_ids.len() as u64 + 1))?;
    for pin in &binding.pools {
        let (_, pool) = load_pool(tx, &pin.resource)?;
        if pool
            .holds
            .get(&object.slot)
            .is_none_or(|h| h.part.is_some())
        {
            return reject(Reject::ContinuityUnproven);
        }
    }
    let domain = execution_configuration::domain(tx, &cell.configuration)?
        .ok_or(StoreError::Rejected(Reject::UnsupportedSchema))?;
    Ok(Current {
        revision,
        run,
        cell,
        binding,
        object,
        domain,
    })
}
fn snapshot(binding: wire::PartBinding, revision: Counter, part: PartAttempt) -> wire::Part {
    wire::Part {
        schema: name(wire::PART_SCHEMA),
        binding,
        state: rx_process_contract::production::Part {
            id: part.id,
            run: part.run,
            ordinal: part.ordinal,
            revision,
            disposition: part.disposition,
        },
    }
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn prepare_execution_part(
        &mut self,
        identity: &Identity,
        key_: &Id,
        command: BeginPartRequest,
    ) -> Result<data::PartPreparation> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (actor, _, cell) = executor_peer::executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                &command.cell,
            )?;
            let (scope, digest) =
                request(meta, &actor, "Execution.BeginPart", key_.as_str(), &command)?;
            if let Some(saved) = prior(tx, &scope, digest, REPLY)? {
                return Ok(data::PartPreparation::Recorded(Box::new(saved)));
            }
            execution_session::require(tx, identity, meta, &now, &cell)?;
            let c = current(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                &command,
            )?;
            active_run(tx, &c.cell, &c.run, identity, meta, &now)?;
            check_revision(
                c.run
                    .budget
                    .as_ref()
                    .ok_or(StoreError::Rejected(Reject::InvalidInput))?
                    .revision(),
                command.expected_budget,
            )?;
            Ok(data::PartPreparation::Compute(Box::new(data::PartTicket {
                identity: identity.clone(),
                key: key_.clone(),
                command,
                run_revision: c.revision,
                object: c.object,
                binding: c.binding,
                policy: c.domain.policy,
                inputs: c.domain.inputs,
                index: c.domain.index,
                boot: meta.runtime_boot.clone(),
                issued: now,
            })))
        })
    }
    pub fn commit_execution_part(&mut self, prepared: data::PreparedPart) -> Result<wire::Part> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let t = &prepared.ticket;
            let now = clock.now();
            let (actor, _, cell) = executor_peer::executor_scope(
                tx,
                ProcessingContext {
                    identity: &t.identity,
                    meta,
                    now: &now,
                },
                &t.command.cell,
            )?;
            let (scope, digest) = request(
                meta,
                &actor,
                "Execution.BeginPart",
                t.key.as_str(),
                &t.command,
            )?;
            if let Some(saved) = prior(tx, &scope, digest, REPLY)? {
                return Ok(saved);
            }
            execution_session::require(tx, &t.identity, meta, &now, &cell)?;
            if t.boot != meta.runtime_boot
                || now.age_ns(&t.issued).is_none_or(|age| age >= TICKET_TTL_NS)
            {
                return reject(Reject::StaleRevision);
            }
            let mut c = current(
                tx,
                ProcessingContext {
                    identity: &t.identity,
                    meta,
                    now: &now,
                },
                &t.command,
            )?;
            if c.revision != t.run_revision
                || canonical::bytes(&c.binding).map_err(domain_error)?
                    != canonical::bytes(&t.binding).map_err(domain_error)?
                || canonical::bytes(&c.object).map_err(domain_error)?
                    != canonical::bytes(&t.object).map_err(domain_error)?
                || c.domain.policy.digest().map_err(StoreError::Invalid)?
                    != t.policy.digest().map_err(StoreError::Invalid)?
            {
                return reject(Reject::StaleRevision);
            }
            prepared
                .materialized
                .verify_index(
                    &c.domain.policy,
                    &c.domain.index,
                    c.object.candidate,
                    c.object.slot,
                )
                .map_err(StoreError::Invalid)?;
            let part = workflow::begin_part_transition(
                tx,
                &c.cell,
                &mut c.run,
                c.revision,
                t.command.expected_budget,
                true,
                ProcessingContext {
                    identity: &t.identity,
                    meta,
                    now: &now,
                },
            )?;
            let report = ArtifactRef {
                schema_id: name("rx.execution-report.v2"),
                sha256: prepared.materialized.report_digest(),
                size_bytes: Counter(prepared.materialized.report().len() as u64),
            };
            BLOBS.put(tx, report.sha256, prepared.materialized.report())?;
            let mut parameters = BTreeMap::new();
            for (node, bytes) in prepared.materialized.parameters() {
                let r = ArtifactRef {
                    schema_id: name(v2::PARAMETER_SCHEMA),
                    sha256: rx_package::content_digest(bytes),
                    size_bytes: Counter(bytes.len() as u64),
                };
                BLOBS.put(tx, r.sha256, bytes)?;
                parameters.insert(node.clone(), r);
            }
            for pin in &c.binding.pools {
                let (revision, mut pool) = load_pool(tx, &pin.resource)?;
                pool.holds
                    .get_mut(&c.object.slot)
                    .ok_or(StoreError::Rejected(Reject::ContinuityUnproven))?
                    .part = Some(part.part.id.clone());
                tx.put(&pool_key(&pin.resource), Some(revision), &doc(POOL, &pool)?)?;
            }
            let binding = wire::PartBinding {
                schema: name(wire::PART_BINDING_SCHEMA),
                run: c.run.id.clone(),
                part: part.part.id.clone(),
                ordinal: part.part.ordinal,
                slot_ordinal: c.object.slot_ordinal,
                slot: c.object.slot,
                object: c.object.object,
                model: c.object.model,
                object_values_digest: c.object.values_digest,
                candidate: c.object.candidate,
                publication: c.binding.publication,
                policy: c.binding.policy,
                configuration: c.binding.configuration,
                report,
                parameters,
            };
            binding.validate().map_err(StoreError::Invalid)?;
            save(
                tx,
                "executionpart",
                &binding.part,
                None,
                PART_BINDING,
                &binding,
            )?;
            let result = snapshot(binding, part.revision, part.part);
            event(tx, "rx.event.execution-part-admitted.v2", &result)?;
            remember(tx, &scope, digest, REPLY, &result)?;
            Ok(result)
        })
    }
    pub fn execution_part(
        &mut self,
        identity: &Identity,
        run: &Id,
        part: &Id,
    ) -> Result<wire::Part> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (_, binding): (_, wire::PartBinding) =
                load(tx, "executionpart", part, PART_BINDING)?;
            binding.validate().map_err(StoreError::Integrity)?;
            let (revision, value): (_, PartAttempt) = load(tx, "part", part, PART)?;
            let (_, owner): (_, Run) = load(tx, "run", run, RUN)?;
            let now = clock.now();
            let (_, _, cell) = executor_peer::executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                &owner.cell,
            )?;
            execution_session::require(tx, identity, meta, &now, &cell)?;
            if binding.run != *run
                || value.run != *run
                || binding.part != *part
                || value.id != *part
                || binding.ordinal != value.ordinal
                || !owner.part_ids.contains(part)
            {
                return reject(Reject::Forbidden);
            }
            Ok(snapshot(binding, revision, value))
        })
    }
    pub fn execution_part_artifact(
        &mut self,
        identity: &Identity,
        run: &Id,
        part: &Id,
        reference: &ArtifactRef,
    ) -> Result<Vec<u8>> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let (_, binding): (_, wire::PartBinding) =
                load(tx, "executionpart", part, PART_BINDING)?;
            let (_, owner): (_, Run) = load(tx, "run", run, RUN)?;
            let now = clock.now();
            let (_, _, cell) = executor_peer::executor_scope(
                tx,
                ProcessingContext {
                    identity,
                    meta,
                    now: &now,
                },
                &owner.cell,
            )?;
            execution_session::require(tx, identity, meta, &now, &cell)?;
            if binding.run != *run
                || binding.part != *part
                || !owner.part_ids.contains(part)
                || (reference != &binding.report
                    && !binding.parameters.values().any(|p| p == reference))
            {
                return reject(Reject::Forbidden);
            }
            BLOBS.read(tx, reference)
        })
    }
}
