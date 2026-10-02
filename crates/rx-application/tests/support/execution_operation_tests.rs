use super::*;
pub(super) fn exercise(f: &mut Fixture, target: &CellConfiguration, part: &wire::Part, case: u8) {
    let (revision, run) = f.app.inspect_run(&f.operator, &part.binding.run).unwrap();
    let node = target
        .execution
        .as_ref()
        .unwrap()
        .nodes
        .keys()
        .next()
        .unwrap()
        .clone();
    let request = inventory::SubmitNode {
        cell: target.id.clone(),
        run: run.id.clone(),
        part: part.binding.part.clone(),
        node,
        mandate: run.mandate.clone().unwrap(),
        expected_cell: f.app.inspect_cell(&f.operator, &target.id).unwrap().0,
        expected_run: revision,
    };
    let key = id();
    let mut wrong = request.clone();
    wrong.part = id();
    assert!(
        f.app
            .prepare_execution_operation(&f.executor, &id(), wrong)
            .is_err()
    );
    let mut wrong = request.clone();
    wrong.run = id();
    assert!(
        f.app
            .prepare_execution_operation(&f.executor, &id(), wrong)
            .is_err()
    );
    assert!(
        f.app
            .prepare_execution_operation(&f.operator, &id(), request.clone())
            .is_err()
    );
    let inventory::OperationPreparation::Compute(ticket) = f
        .app
        .prepare_execution_operation(&f.executor, &key, request.clone())
        .unwrap()
    else {
        panic!("compute");
    };
    let prepared = inventory::PreparedOperation::prepare(*ticket).unwrap();
    if case == 12 {
        revise(f, &part.binding.object);
        assert!(f.app.commit_execution_operation(prepared).is_err());
        assert_eq!(f.app.inspect_run(&f.operator, &run.id).unwrap().0, revision);
        return;
    }
    f.failure.store(1, Ordering::SeqCst);
    assert!(f.app.commit_execution_operation(prepared).is_err());
    assert_eq!(f.app.inspect_run(&f.operator, &run.id).unwrap().0, revision);
    let inventory::OperationPreparation::Compute(ticket) = f
        .app
        .prepare_execution_operation(&f.executor, &key, request.clone())
        .unwrap()
    else {
        panic!("retry rolled back compute");
    };
    let prepared = inventory::PreparedOperation::prepare(*ticket).unwrap();
    f.failure.store(2, Ordering::SeqCst);
    assert!(f.app.commit_execution_operation(prepared).is_err());
    let inventory::OperationPreparation::Recorded(work) = f
        .app
        .prepare_execution_operation(&f.executor, &key, request.clone())
        .unwrap()
    else {
        panic!("recover original");
    };
    let binding = work.execution.as_ref().unwrap();
    binding.validate().unwrap();
    assert_eq!(binding.selection.object, part.binding.object);
    assert_eq!(binding.report, part.binding.report);
    assert_eq!(
        binding.selection.parameter,
        part.binding.parameters[&binding.selection.node]
    );
    assert_eq!(
        binding.selection.intent_digest,
        work.intent.digest().unwrap()
    );
    assert_eq!(binding.selection.run, run.id);
    let progress = f
        .app
        .process_progress(&f.executor, &run.id, part.binding.ordinal)
        .unwrap();
    assert_eq!(progress.view.operations.len(), 1);
    assert_ne!(
        progress.frontier.state,
        rx_process_contract::frontier::State::Completed,
        "admission is not completion"
    );
    assert_eq!(
        f.app
            .inspect_run(&f.operator, &run.id)
            .unwrap()
            .1
            .budget
            .unwrap()
            .revision(),
        run.budget.unwrap().revision()
    );
    if case == 13 {
        complete(f, target, part, &work);
        return;
    }
    if case == 15 {
        assert!(f.app.begin_delivery(work.operation.id()).unwrap());
        f.app
            .note_delivery_attention(work.operation.id(), DeliveryIssue::ResponseUnknown)
            .unwrap();
        let unknown = f
            .app
            .inspect_work(&f.executor, work.operation.id())
            .unwrap();
        assert_eq!(
            unknown.operation.knowledge(),
            rx_domain::operation::Knowledge::Unknown
        );
        assert_eq!(
            unknown.operation.disposition(),
            rx_domain::operation::Disposition::Quarantined
        );
        let binding = f.app.execution_run(&f.operator, &work.run).unwrap();
        for pin in binding.pools {
            let pool = f
                .app
                .execution_slot_pool(&f.operator, &pin.resource)
                .unwrap();
            assert!(!pool.holds[&part.binding.slot].consumed);
            assert_eq!(pool.holds[&part.binding.slot].part, work.part);
        }
        let (_, active) = f.app.inspect_run(&f.executor, &work.run).unwrap();
        assert!(
            f.app
                .prepare_execution_part(
                    &f.executor,
                    &id(),
                    BeginPartRequest {
                        cell: work.cell.clone(),
                        run: work.run.clone(),
                        mandate: active.mandate.unwrap(),
                        expected_budget: active.budget.unwrap().revision(),
                        expected_cell: None
                    }
                )
                .is_err()
        );
        let inventory::OperationPreparation::Recorded(recovered) = f
            .app
            .prepare_execution_operation(&f.executor, &id(), request)
            .unwrap()
        else {
            panic!("original unknown");
        };
        assert_eq!(recovered.operation.id(), work.operation.id());
        return;
    }
    if case == 14 {
        revise(f, &part.binding.object);
        assert!(f.app.begin_delivery(work.operation.id()).is_err());
        assert_eq!(
            f.app
                .inspect_work(&f.executor, work.operation.id())
                .unwrap()
                .operation
                .knowledge(),
            rx_domain::operation::Knowledge::NotSent
        );
    } else {
        assert!(f.app.begin_delivery(work.operation.id()).unwrap());
        revise(f, &part.binding.object);
    }
    let inventory::OperationPreparation::Recorded(same) = f
        .app
        .prepare_execution_operation(&f.executor, &id(), request)
        .unwrap()
    else {
        panic!("occupied node recovers after revision drift");
    };
    assert_eq!(same.operation.id(), work.operation.id());
    assert_eq!(same.permit, work.permit);
}

fn complete(f: &mut Fixture, target: &CellConfiguration, part: &wire::Part, work: &Work) {
    let op = work.operation.id().clone();
    let revision = f.app.inspect_run(&f.executor, &work.run).unwrap().0;
    assert!(
        f.app
            .complete_part(&f.executor, id().as_str(), &part.binding.part, revision)
            .is_err()
    );
    let plan = f.app.plan_delivery(&f.hosts[0], &op).unwrap();
    assert!(plan.first_emission);
    assert_eq!(
        rx_package::content_digest(plan.execution_parameters.as_ref().unwrap()),
        work.execution.as_ref().unwrap().selection.parameter.sha256
    );
    let invocation = id();
    f.app
        .record_host_receipt(
            &f.hosts[0],
            &op,
            HostReceipt {
                operation: op.clone(),
                digest: work.intent.digest().unwrap(),
                invocation: Some(invocation.clone()),
                journal: f.registrations[0].delivery_journal.clone(),
                sequence: Counter(910),
                state: ReceiptState::Prepared,
            },
        )
        .unwrap();
    let next = f
        .app
        .pending_deliveries(128)
        .unwrap()
        .into_iter()
        .find(|d| matches!(&d.payload, Delivery::Authorize {operation,..} if *operation==op))
        .unwrap();
    assert!(f.app.begin_delivery(&next.id).unwrap());
    f.app
        .record_host_receipt(
            &f.hosts[0],
            &next.id,
            HostReceipt {
                operation: op.clone(),
                digest: work.intent.digest().unwrap(),
                invocation: Some(invocation.clone()),
                journal: f.registrations[0].delivery_journal.clone(),
                sequence: Counter(911),
                state: ReceiptState::ResultCaptured,
            },
        )
        .unwrap();
    let evidence = NativeEvidence {
        native_details: None,
        id: id(),
        operation: op.clone(),
        invocation,
        profile_digest: work.intent.profile_digest,
        device_session: id(),
        status_schema: name("rx.sim.completed.v1"),
        status: Integer(0),
        captured_at: f.clock.now(),
    };
    f.app
        .ingest_evidence(
            &f.hosts[0],
            EvidenceBatch {
                journal: id(),
                first: Counter(1),
                records: vec![evidence.clone()],
            },
        )
        .unwrap();
    let done = f.app.inspect_work(&f.executor, &op).unwrap();
    assert_eq!(
        done.operation.outcome(),
        rx_domain::operation::Outcome::Succeeded
    );
    assert!(
        f.app
            .complete_part(&f.executor, id().as_str(), &part.binding.part, revision)
            .is_err()
    );
    let mut proof = handover_proof(f, &done, &evidence);
    for o in &mut proof.observations {
        o.observed_at = f.clock.now();
    }
    f.app
        .release_resources(&f.hosts[0], id().as_str(), proof)
        .unwrap();
    f.app
        .complete_part(&f.executor, id().as_str(), &part.binding.part, revision)
        .unwrap();
    let binding = f.app.execution_run(&f.operator, &part.binding.run).unwrap();
    for pin in binding.pools {
        let pool = f
            .app
            .execution_slot_pool(&f.operator, &pin.resource)
            .unwrap();
        assert!(pool.holds[&part.binding.slot].consumed);
        assert!(!pool.holds[&1].consumed);
    }
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &target.id)
            .unwrap()
            .1
            .configuration
            .execution
            .as_ref()
            .unwrap()
            .policy,
        part.binding.policy
    );
}
