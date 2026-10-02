use super::*;
use rx_process_contract::execution_v2::executor::PartBinding;

struct Current {
    cell: Cell,
    run: Run,
    part: PartBinding,
    domain: execution_configuration::Domain,
}
fn current(
    tx: &mut dyn Transaction,
    context: ProcessingContext<'_>,
    c: &data::SubmitNode,
) -> Result<Current> {
    let (_, revision, cell) = executor_peer::executor_scope(
        tx,
        ProcessingContext {
            identity: context.identity,
            meta: context.meta,
            now: context.now,
        },
        &c.cell,
    )?;
    execution_session::require(tx, context.identity, context.meta, context.now, &cell)?;
    check_revision(revision, c.expected_cell)?;
    let (revision, run): (_, Run) = load(tx, "run", &c.run, RUN)?;
    check_revision(revision, c.expected_run)?;
    if run.cell != c.cell || run.mandate.as_ref() != Some(&c.mandate) {
        return reject(Reject::MandateRevoked);
    }
    active_run(tx, &cell, &run, context.identity, context.meta, context.now)?;
    validate_part(tx, &run, &Some(c.part.clone()))?;
    let (_, attempt): (_, PartAttempt) = load(tx, "part", &c.part, PART)?;
    if attempt.disposition != PartDisposition::InProgress {
        return reject(Reject::ConditionUnknown);
    }
    let (_, part): (_, PartBinding) = load(
        tx,
        "executionpart",
        &c.part,
        v2::executor::PART_BINDING_SCHEMA,
    )?;
    part.validate().map_err(StoreError::Integrity)?;
    let (_, binding): (_, data::RunBinding) = load(tx, "executionrun", &c.run, BINDING)?;
    let object = object_current(tx, &cell, &binding, part.ordinal)?;
    if part.run != run.id
        || part.part != c.part
        || run.part_ids.last() != Some(&c.part)
        || binding.configuration
            != cell
                .configuration
                .reference()
                .map_err(StoreError::Invalid)?
        || part.configuration != binding.configuration
        || part.publication != binding.publication
        || part.policy != binding.policy
        || part.object != object.object
        || part.model != object.model
        || part.object_values_digest != object.values_digest
        || part.candidate != object.candidate
        || part.slot != object.slot
        || part.slot_ordinal != object.slot_ordinal
    {
        return reject(Reject::StaleRevision);
    }
    for pin in &binding.pools {
        let (_, pool) = load_pool(tx, &pin.resource)?;
        if pool
            .holds
            .get(&part.slot)
            .is_none_or(|h| h.part.as_ref() != Some(&part.part))
        {
            return reject(Reject::ContinuityUnproven);
        }
    }
    process::eligible_node(tx, &run, &cell, &c.node, part.ordinal)?;
    let domain = execution_configuration::domain(tx, &cell.configuration)?
        .ok_or(StoreError::Rejected(Reject::UnsupportedSchema))?;
    Ok(Current {
        cell,
        run,
        part,
        domain,
    })
}
/// Recover an already admitted node before checking eligibility/current definitions again.
fn occupied(tx: &mut dyn Transaction, c: &data::SubmitNode) -> Result<Option<Work>> {
    let (_, part): (_, PartAttempt) = load(tx, "part", &c.part, PART)?;
    if part.run != c.run {
        return reject(Reject::Forbidden);
    }
    let Some(row) = tx.get(&key("activation", (&c.run, &c.node, part.ordinal)))? else {
        return Ok(None);
    };
    let activation: Activation = decode(&row, ACTIVATION)?;
    let Some(operation) = activation.slots.get(&name("main")) else {
        return Ok(None);
    };
    let (_, work): (_, Work) = load(tx, "work", operation, WORK)?;
    let (_, permit): (_, Permit) = load(tx, "permit", &work.permit, PERMIT)?;
    let binding = work
        .execution
        .as_ref()
        .ok_or(StoreError::Rejected(Reject::UnsupportedSchema))?;
    binding.validate().map_err(StoreError::Integrity)?;
    if work.cell != c.cell
        || work.run != c.run
        || work.part.as_ref() != Some(&c.part)
        || work.activation != activation.id
        || work.operation.id() != operation
        || activation.run != c.run
        || activation.part.as_ref() != Some(&c.part)
        || activation.node != c.node
        || work.slot.as_str() != "main"
        || permit.cell != c.cell
        || permit.intent_digest != work.operation.intent_digest()
        || work.operation.intent_digest() != binding.selection.intent_digest
        || permit.operation != *operation
        || binding.operation != *operation
        || binding.selection.run != c.run
        || binding.selection.part != c.part
        || binding.selection.ordinal != part.ordinal
        || permit.mandate != c.mandate
        || binding.mandate != c.mandate
        || binding.selection.intent_digest != work.intent.digest().map_err(domain_error)?
    {
        return reject(Reject::InvalidInput);
    }
    Ok(Some(work))
}
impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn prepare_execution_operation(
        &mut self,
        identity: &Identity,
        key_: &Id,
        command: data::SubmitNode,
    ) -> Result<data::OperationPreparation> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let context = || ProcessingContext {
                identity,
                meta,
                now: &now,
            };
            let (actor, _, cell) = executor_peer::executor_scope(tx, context(), &command.cell)?;
            execution_session::require(tx, identity, meta, &now, &cell)?;
            let (scope, digest) = request(
                meta,
                &actor,
                "Execution.SubmitNode",
                key_.as_str(),
                &command,
            )?;
            if let Some(saved) = prior(tx, &scope, digest, WORK)? {
                return Ok(data::OperationPreparation::Recorded(Box::new(saved)));
            }
            if let Some(work) = occupied(tx, &command)? {
                remember(tx, &scope, digest, WORK, &work)?;
                return Ok(data::OperationPreparation::Recorded(Box::new(work)));
            }
            let c = current(tx, context(), &command)?;
            Ok(data::OperationPreparation::Compute(Box::new(
                data::OperationTicket {
                    identity: identity.clone(),
                    key: key_.clone(),
                    command,
                    part: c.part,
                    policy: c.domain.policy,
                    inputs: c.domain.inputs,
                    index: c.domain.index,
                    boot: meta.runtime_boot.clone(),
                    issued: now,
                },
            )))
        })
    }
    pub fn commit_execution_operation(
        &mut self,
        prepared: data::PreparedOperation,
    ) -> Result<Work> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let t = &prepared.ticket;
            let now = clock.now();
            let context = || ProcessingContext {
                identity: &t.identity,
                meta,
                now: &now,
            };
            let (actor, _, cell) = executor_peer::executor_scope(tx, context(), &t.command.cell)?;
            execution_session::require(tx, &t.identity, meta, &now, &cell)?;
            let (scope, digest) = request(
                meta,
                &actor,
                "Execution.SubmitNode",
                t.key.as_str(),
                &t.command,
            )?;
            if let Some(saved) = prior(tx, &scope, digest, WORK)? {
                return Ok(saved);
            }
            if let Some(work) = occupied(tx, &t.command)? {
                remember(tx, &scope, digest, WORK, &work)?;
                return Ok(work);
            }
            if t.boot != meta.runtime_boot
                || now.age_ns(&t.issued).is_none_or(|a| a >= TICKET_TTL_NS)
            {
                return reject(Reject::StaleRevision);
            }
            let c = current(tx, context(), &t.command)?;
            if canonical::bytes(&c.part).map_err(domain_error)?
                != canonical::bytes(&t.part).map_err(domain_error)?
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
                    c.part.candidate,
                    c.part.slot,
                )
                .map_err(StoreError::Invalid)?;
            let node = c
                .cell
                .configuration
                .execution
                .as_ref()
                .and_then(|e| e.nodes.get(&t.command.node))
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            let action = prepared
                .materialized
                .actions()
                .get(node)
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            let bytes = prepared
                .materialized
                .parameters()
                .get(node)
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            let parameter = c
                .part
                .parameters
                .get(node)
                .ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            if prepared.materialized.report_digest() != c.part.report.sha256
                || rx_package::content_digest(bytes) != parameter.sha256
                || bytes.len() as u64 != parameter.size_bytes.0
            {
                return reject(Reject::InvalidInput);
            }
            let selection = v2::Selection {
                schema: name("rx.execution-selection.v2"),
                publication: c.part.publication.id.clone(),
                policy_digest: c.domain.policy.digest().map_err(StoreError::Invalid)?,
                configuration_digest: c.part.configuration.sha256,
                run: c.run.id.clone(),
                part: c.part.part.clone(),
                ordinal: c.part.ordinal,
                slot_ordinal: c.part.slot_ordinal,
                object: c.part.object.clone(),
                object_values_digest: c.part.object_values_digest,
                candidate: c.part.candidate,
                slot: c.part.slot,
                report_digest: c.part.report.sha256,
                node: node.clone(),
                parameter: parameter.clone(),
                intent_digest: action.intent.digest().map_err(domain_error)?,
                authority_generation: c.cell.epoch,
            };
            let activation = workflow::resolve_activation_transition(
                tx,
                &c.run,
                t.command.expected_run,
                &t.command.node,
                c.part.ordinal,
                t.command.expected_run,
                context(),
            )?;
            let (activation_revision, mut activation): (_, Activation) =
                load(tx, "activationid", &activation.id, ACTIVATION)?;
            let (run_revision, mut run): (_, Run) = load(tx, "run", &c.run.id, RUN)?;
            let command = SubmitWork {
                run: run.id.clone(),
                activation: activation.id.clone(),
                part: Some(c.part.part),
                slot: name("main"),
                intent: action.intent.clone(),
                expected_cell: t.command.expected_cell,
                expected_run: run_revision,
            };
            let work = dispatch::submit_transition(
                tx,
                dispatch::DispatchState {
                    execution: Some(dispatch::ExecutionAdmission {
                        selection,
                        publication: c.part.publication,
                        policy_reference: c.part.policy,
                        policy: c.domain.policy,
                        index: c.domain.index,
                        report: c.part.report,
                        parameter_bytes: bytes.clone(),
                    }),
                    activation: &mut activation,
                    activation_revision,
                    run: &mut run,
                    run_revision,
                },
                &command,
                context(),
            )?;
            remember(tx, &scope, digest, WORK, &work)?;
            Ok(work)
        })
    }
}
