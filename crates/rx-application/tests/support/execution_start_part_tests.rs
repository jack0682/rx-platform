use super::*;
use rx_application::execution_inventory as inventory;
use rx_process_contract::execution_v2::executor as wire;
fn revise(f: &mut Fixture, reference: &rx_domain::definition::Reference) {
    let old = f
        .app
        .definition(
            &f.admin,
            &reference.catalog,
            &reference.id,
            Some(reference.revision),
        )
        .unwrap()
        .version
        .definition;
    let mut changed = save(&reference.catalog, old.body);
    changed.id = reference.id.clone();
    changed.expected = Some(reference.revision);
    changed.label = "Instance revised with the same values".into();
    put(&mut f.app, &f.admin, changed);
}
pub(super) fn exercise(
    f: &mut Fixture,
    target: &CellConfiguration,
    published: &publication::Publication,
    inputs: &v2::InputClosure,
    policy: &v2::Policy,
    case: u8,
) {
    let layouts = inputs.slot_resources(policy).unwrap();
    for layout in &layouts {
        f.app
            .initialize_execution_slots(
                &f.operator,
                &id(),
                inventory::Initialize {
                    cell: target.id.clone(),
                    resource: layout.resource.clone(),
                    rule: layout.rule.clone(),
                    expected_generation: None,
                    reason: "SIMULATION stock before negotiated execution".into(),
                },
            )
            .unwrap();
    }
    let cell_revision = f.app.inspect_cell(&f.admin, &target.id).unwrap().0;
    let binding = f
        .app
        .create_execution_run(
            &f.operator,
            &id(),
            inventory::CreateRun {
                cell: target.id.clone(),
                publication: published.reference.clone(),
                expected_cell: cell_revision,
                count: Counter(2),
            },
        )
        .unwrap();
    let actual = put(
        &mut f.app,
        &f.admin,
        save(
            &policy.workflow.catalog,
            Body::ObjectInstance {
                base: policy.candidates[0].object_model.clone(),
                values: BTreeMap::new(),
            },
        ),
    )
    .version
    .definition
    .reference;
    f.app
        .bind_execution_object(
            &f.operator,
            &id(),
            inventory::BindObject {
                run: binding.run.clone(),
                ordinal: Counter(1),
                object: actual.clone(),
            },
        )
        .unwrap();
    f.app
        .report_fact(
            &f.hosts[0],
            FactRecord {
                cell: target.id.clone(),
                id: name("ready"),
                source_host: f.hosts[0].principal.clone(),
                source_generation: f.registrations[0].source_sessions[&name("ready")].clone(),
                schema: name("boolean/v1"),
                unit: name("unitless"),
                acquired_at: f.clock.now(),
                maximum_age_ns: Counter(20000),
                acquisition_uncertainty_ns: Counter(0),
                quality_good: true,
                origin_age_bounded: true,
                disputed: false,
                value: TypedValue::Boolean(true),
                evidence_id: id(),
            },
        )
        .unwrap();
    let command = StartRun {
        run: binding.run.clone(),
        envelope_digest: target.envelope.sha256,
        purpose: Purpose::Production,
        budget_unit: rx_domain::budget::BudgetUnit::PartAttempt,
        budget_limit: Counter(2),
        expected_cell: cell_revision,
        expected_run: Counter(1),
    };
    assert!(
        f.app
            .start_execution_run(&f.operator, id().as_str(), command.clone())
            .is_err(),
        "base negotiation alone cannot declare v2 support"
    );
    assert!(
        f.app
            .negotiate_execution_session(&f.operator, &target.id, wire::binding_hash())
            .is_err()
    );
    assert!(
        f.app
            .negotiate_execution_session(&f.executor, &target.id, Digest::from_bytes([99; 32]))
            .is_err()
    );
    let session = f
        .app
        .negotiate_execution_session(&f.executor, &target.id, wire::binding_hash())
        .unwrap();
    assert_eq!(session.session, f.executor.session);
    let key = id();
    let attempt = f
        .app
        .start_execution_run(&f.operator, key.as_str(), command.clone())
        .unwrap();
    assert_eq!(
        f.app
            .start_execution_run(&f.operator, key.as_str(), command)
            .unwrap()
            .id,
        attempt.id
    );
    if case == 9 {
        revise(f, &actual);
    }
    let acknowledgement = ArmAcknowledgment {
        attempt: attempt.id.clone(),
        host_boot: f.registrations[0].boot_id.clone(),
        delivery_journal: f.registrations[0].delivery_journal.clone(),
        sequence: Counter(901),
        epoch: attempt.epoch,
        scopes: attempt.scopes.clone(),
    };
    if case == 9 {
        assert!(f.app.acknowledge_arm(&f.hosts[0], acknowledgement).is_err());
        assert!(
            f.app
                .inspect_run(&f.operator, &binding.run)
                .unwrap()
                .1
                .mandate
                .is_none()
        );
        return;
    }
    let started = f.app.acknowledge_arm(&f.hosts[0], acknowledgement).unwrap();
    assert_eq!(started.status, StartStatus::Started);
    let (_, run) = f.app.inspect_run(&f.operator, &binding.run).unwrap();
    assert_eq!(run.state, RunState::Executing);
    let request = BeginPartRequest {
        cell: target.id.clone(),
        run: run.id.clone(),
        mandate: run.mandate.clone().unwrap(),
        expected_budget: run.budget.as_ref().unwrap().revision(),
        expected_cell: None,
    };
    assert!(
        f.app
            .executor_begin_part(&f.executor, id().as_str(), request.clone())
            .is_err(),
        "legacy Part path cannot consume a v2 reservation"
    );
    let key = id();
    let inventory::PartPreparation::Compute(ticket) = f
        .app
        .prepare_execution_part(&f.executor, &key, request.clone())
        .unwrap()
    else {
        panic!("part computation")
    };
    let prepared = inventory::PreparedPart::prepare(*ticket).unwrap();
    if case == 10 {
        revise(f, &actual);
        assert!(f.app.commit_execution_part(prepared).is_err());
        assert!(
            f.app
                .inspect_run(&f.operator, &run.id)
                .unwrap()
                .1
                .part_ids
                .is_empty()
        );
        for layout in &layouts {
            assert!(
                f.app
                    .execution_slot_pool(&f.operator, &layout.resource)
                    .unwrap()
                    .holds[&0]
                    .part
                    .is_none()
            );
        }
        return;
    }
    f.failure.store(1, Ordering::SeqCst);
    assert!(f.app.commit_execution_part(prepared).is_err());
    assert!(
        f.app
            .inspect_run(&f.operator, &run.id)
            .unwrap()
            .1
            .part_ids
            .is_empty()
    );
    let inventory::PartPreparation::Compute(ticket) = f
        .app
        .prepare_execution_part(&f.executor, &key, request.clone())
        .unwrap()
    else {
        panic!("rolled back Part")
    };
    let prepared = inventory::PreparedPart::prepare(*ticket).unwrap();
    f.failure.store(2, Ordering::SeqCst);
    assert!(f.app.commit_execution_part(prepared).is_err());
    let inventory::PartPreparation::Recorded(part) = f
        .app
        .prepare_execution_part(&f.executor, &key, request.clone())
        .unwrap()
    else {
        panic!("original Part recovery")
    };
    assert_eq!(part.binding.object, actual);
    assert_eq!(part.binding.ordinal, Counter(1));
    assert_eq!(part.binding.slot, 0);
    let after = f.app.inspect_run(&f.operator, &run.id).unwrap().1;
    assert_eq!(after.part_ids, vec![part.binding.part.clone()]);
    assert_eq!(
        after.budget.as_ref().unwrap().revision(),
        request.expected_budget.increment().unwrap()
    );
    for layout in &layouts {
        assert_eq!(
            f.app
                .execution_slot_pool(&f.operator, &layout.resource)
                .unwrap()
                .holds[&0]
                .part,
            Some(part.binding.part.clone())
        );
    }
    let bytes = f
        .app
        .execution_part_artifact(
            &f.executor,
            &run.id,
            &part.binding.part,
            &part.binding.report,
        )
        .unwrap();
    assert_eq!(
        rx_package::content_digest(&bytes),
        part.binding.report.sha256
    );
    let mut wrong = part.binding.report.clone();
    wrong.schema_id = name("foreign/schema");
    assert!(
        f.app
            .execution_part_artifact(&f.executor, &run.id, &part.binding.part, &wrong)
            .is_err()
    );
    assert!(
        f.app
            .execution_part(&f.executor, &id(), &part.binding.part)
            .is_err()
    );
    let request = BeginPartRequest {
        expected_budget: after.budget.as_ref().unwrap().revision(),
        ..request
    };
    assert!(
        f.app
            .prepare_execution_part(&f.executor, &id(), request)
            .is_err(),
        "unfinished Part cannot advance to another slot"
    );
    if case >= 11 {
        operation_tests::exercise(f, target, &part, case);
        return;
    }
    revise(f, &actual);
    assert_eq!(
        f.app
            .execution_part(&f.executor, &run.id, &part.binding.part)
            .unwrap()
            .binding
            .object,
        actual
    );
    assert_eq!(
        f.app
            .execution_part_artifact(
                &f.executor,
                &run.id,
                &part.binding.part,
                &part.binding.report
            )
            .unwrap(),
        bytes
    );
}

#[path = "execution_operation_tests.rs"]
mod operation_tests;
