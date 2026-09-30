//! Focused invariant scenario tests (see docs/invariant-traceability.json).
use super::*;
use rx_domain::budget::BudgetUnit;

fn rows_of<T: serde::de::DeserializeOwned>(rows: &[Record], schema: &str) -> Vec<T> {
    rows.iter()
        .filter(|r| r.document.schema.as_str() == schema)
        .map(|r| serde_json::from_value(r.document.value.clone()).unwrap())
        .collect()
}

fn submit_step(
    f: &mut Fixture,
    a: &Activation,
    step: usize,
    key: &str,
    part: Option<Id>,
) -> rx_ports::Result<Work> {
    let cr = f.app.inspect_cell(&f.executor, &f.configuration.id)?.0;
    let rr = f.app.inspect_run(&f.executor, &a.run)?.0;
    f.app.submit(
        &f.executor,
        key,
        SubmitWork {
            run: a.run.clone(),
            activation: a.id.clone(),
            part,
            slot: name("main"),
            intent: f.configuration.steps[step].intent.clone(),
            expected_cell: cr,
            expected_run: rr,
        },
    )
}

fn resolve(f: &mut Fixture, run: &Id, step: &str, visit: Counter) -> rx_ports::Result<Activation> {
    let rev = f.app.inspect_run(&f.executor, run)?.0;
    f.app
        .resolve_activation(&f.executor, run, &name(step), visit, rev)
}

/// OI01: the admitted operation's permit carries the current qualification, the envelope that
/// qualification covers and the cell's current epoch vector; an intent whose digests differ
/// from the qualified step is not admitted, and a later epoch change voids the admitted permit.
#[test]
fn admitted_permit_binds_current_qualification_envelope_and_scope_vector() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let a = activation(&mut f, &run);
    let step = f.configuration.steps[0].intent.clone();
    let changed = [
        Intent {
            site_config_digest: Digest::from_bytes([40; 32]),
            ..step.clone()
        },
        Intent {
            profile_digest: Digest::from_bytes([41; 32]),
            ..step.clone()
        },
        Intent {
            calibration_digests: vec![Digest::from_bytes([42; 32])],
            ..step.clone()
        },
    ];
    for intent in changed {
        let cr = f
            .app
            .inspect_cell(&f.executor, &f.configuration.id)
            .unwrap()
            .0;
        let rr = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
        assert!(matches!(
            f.app.submit(
                &f.executor,
                id().as_str(),
                SubmitWork {
                    intent,
                    ..work_command(&f, &a, cr, rr)
                },
            ),
            Err(StoreError::Rejected(Rejection::CapabilityMissing))
        ));
    }
    assert!(
        f.app.overview(&f.operator).unwrap().cells[0]
            .work
            .is_empty()
    );

    let work = submit(&mut f, &a, id().as_str()).unwrap();
    let (_, cell) = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    let q = cell.qualification.clone().unwrap();
    let permit = f.app.inspect_permit(&f.operator, &work.permit).unwrap();
    assert_eq!(permit.operation, *work.operation.id());
    assert_eq!(permit.intent_digest, step.digest().unwrap());
    assert_eq!(permit.qualification, q.id);
    assert_eq!(permit.qualification_revision, q.revision);
    assert_eq!(permit.envelope_digest, cell.configuration.envelope.sha256);
    assert_eq!(q.envelope_digest, cell.configuration.envelope.sha256);
    assert_eq!(run.envelope_digest, cell.configuration.envelope.sha256);
    for dependency in [
        cell.configuration.definition.sha256,
        cell.configuration.envelope.sha256,
        cell.configuration.recipe.sha256,
        cell.configuration.site_config_digest,
    ] {
        assert!(q.dependencies.contains(&dependency));
    }
    assert_eq!(permit.epoch, cell.epoch);
    assert_eq!(permit.scopes, cell.scope_epochs);
    assert_eq!(permit.state, PermitState::Issued);

    // Advancing the cell's epoch vector leaves no Issued permit bound to the old vector.
    let held = f
        .app
        .hold(&f.operator, id().as_str(), &f.configuration.id)
        .unwrap();
    assert!(held.epoch > permit.epoch);
    assert!(
        held.scope_epochs
            .iter()
            .all(|(scope, epoch)| epoch > &permit.scopes[scope])
    );
    assert_eq!(
        f.app
            .inspect_permit(&f.operator, &work.permit)
            .unwrap()
            .state,
        PermitState::Voided
    );

    let mut repo = f.app.into_repository();
    let rows = repo.snapshot().unwrap().1;
    let mandates: Vec<Mandate> = rows_of(&rows, "rx.internal.mandate.v1");
    assert_eq!(mandates.len(), 1);
    assert_eq!(Some(&mandates[0].id), run.mandate.as_ref());
    assert_eq!(mandates[0].epoch, permit.epoch);
    assert_eq!(mandates[0].scopes, permit.scopes);
    assert_eq!(mandates[0].state, MandateState::Revoked);
    assert_eq!(rows_of::<Permit>(&rows, "rx.internal.permit.v1").len(), 1);
}

/// OI03 (platform admission half): a permit is minted only through cell-aware admission that
/// binds the run's purpose and its parent — a part of the same run for PRODUCTION, no part for
/// SETUP — and a request whose parent does not match that purpose writes no operation.
#[test]
fn admission_binds_permit_purpose_and_rejects_a_parent_of_the_other_purpose() {
    // SETUP: operation-count budget, one activation visit and no part parent.
    let mut f = fixture(1, true);
    let rev = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .0;
    let run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, rev))
        .unwrap();
    let mut command = start_command(&f, &run, rev, 2);
    command.purpose = Purpose::Setup;
    command.budget_unit = BudgetUnit::OperationCount;
    let attempt = f
        .app
        .start_run(&f.operator, id().as_str(), command)
        .unwrap();
    let r = f.registrations[0].clone();
    f.app
        .acknowledge_arm(
            &f.hosts[0],
            ArmAcknowledgment {
                attempt: attempt.id,
                host_boot: r.boot_id.clone(),
                delivery_journal: r.delivery_journal.clone(),
                sequence: Counter(1),
                epoch: r.epoch,
                scopes: r.scopes.clone(),
            },
        )
        .unwrap();
    let run = f.app.inspect_run(&f.operator, &run.id).unwrap().1;
    assert_eq!(run.state, RunState::Executing);
    assert_eq!(run.purpose, Some(Purpose::Setup));
    assert!(matches!(
        f.app.begin_part(
            &f.executor,
            id().as_str(),
            &run.id,
            run.budget.as_ref().unwrap().revision(),
        ),
        Err(StoreError::Rejected(Rejection::InvalidInput))
    ));
    assert!(matches!(
        resolve(&mut f, &run.id, "step/0", Counter(2)),
        Err(StoreError::Rejected(Rejection::InvalidInput))
    ));
    let a = resolve(&mut f, &run.id, "step/0", Counter(1)).unwrap();
    assert_eq!(a.part, None);
    assert!(matches!(
        submit_step(&mut f, &a, 0, id().as_str(), Some(id())),
        Err(StoreError::Rejected(Rejection::InvalidInput))
    ));
    assert!(
        f.app.overview(&f.operator).unwrap().cells[0]
            .work
            .is_empty()
    );
    let work = submit_step(&mut f, &a, 0, id().as_str(), None).unwrap();
    assert_eq!(work.part, None);
    let permit = f.app.inspect_permit(&f.operator, &work.permit).unwrap();
    assert_eq!(permit.purpose, Purpose::Setup);
    assert_eq!(permit.mandate, run.mandate.clone().unwrap());
    let budget = f
        .app
        .inspect_run(&f.operator, &run.id)
        .unwrap()
        .1
        .budget
        .unwrap();
    assert_eq!(budget.consumed(), Counter(1));

    // PRODUCTION: the activation and the operation must name a part of the same run.
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    assert_eq!(run.purpose, Some(Purpose::Production));
    assert!(matches!(
        resolve(&mut f, &run.id, "step/0", Counter(1)),
        Err(StoreError::Rejected(Rejection::InvalidInput))
    ));
    let a = activation(&mut f, &run);
    let part = a.part.clone().unwrap();
    assert_eq!(
        f.app.inspect_run(&f.operator, &run.id).unwrap().1.part_ids,
        vec![part.clone()]
    );
    for parent in [None, Some(id())] {
        assert!(matches!(
            submit_step(&mut f, &a, 0, id().as_str(), parent),
            Err(StoreError::Rejected(Rejection::InvalidInput))
        ));
    }
    assert!(
        f.app.overview(&f.operator).unwrap().cells[0]
            .work
            .is_empty()
    );
    let work = submit_step(&mut f, &a, 0, id().as_str(), Some(part.clone())).unwrap();
    assert_eq!(work.part, Some(part));
    let permit = f.app.inspect_permit(&f.operator, &work.permit).unwrap();
    assert_eq!(permit.purpose, Purpose::Production);
    assert_eq!(permit.mandate, run.mandate.unwrap());
}

const SUPPORT_SCHEMA: &str = "boolean/v1";

fn support_fact(id_: &str, host: &str) -> FactSpec {
    FactSpec {
        id: name(id_),
        host: name(host),
        schema: name(SUPPORT_SCHEMA),
        unit: name("unitless"),
        maximum_age_ns: Counter(20000),
        maximum_uncertainty_ns: Counter(0),
    }
}

fn holds(fact: &str) -> Condition {
    Condition::Eq {
        fact: name(fact),
        schema: name(SUPPORT_SCHEMA),
        unit: name("unitless"),
        expected: TypedValue::Boolean(true),
    }
}

fn report_support(f: &mut Fixture, host: usize, fact: &str) {
    let r = f.registrations[host].clone();
    f.app
        .report_fact(
            &f.hosts[host],
            FactRecord {
                cell: f.configuration.id.clone(),
                id: name(fact),
                source_host: f.hosts[host].principal.clone(),
                source_generation: r.source_sessions[&name(fact)].clone(),
                schema: name(SUPPORT_SCHEMA),
                unit: name("unitless"),
                acquired_at: expiry(1000),
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
}

/// OI07 (platform T1 half): a gripper release on host/0 and a chuck release on host/1 each
/// require the other device's support PASS. When both intents name one common support
/// resource, the second is refused Busy even though its PASS condition was evaluated first,
/// and nothing of it is reserved; with distinct support resources both are admitted.
#[test]
fn reciprocal_support_pass_does_not_admit_two_releases_of_one_common_support_resource() {
    for shared in [true, false] {
        let mut f = fixture_configured((2, true, false, false, None, false, true), |mut c| {
            c.fact_specs.push(support_fact("support/gripper", "host/0"));
            c.fact_specs.push(support_fact("support/chuck", "host/1"));
            // Each release requires the receiver's current support PASS.
            let plan = [
                ("support/chuck", "sim/chuck-holds", "support/material-1"),
                (
                    "support/gripper",
                    "sim/gripper-holds",
                    if shared {
                        "support/material-1"
                    } else {
                        "support/material-2"
                    },
                ),
            ];
            for (step, (fact, condition, support)) in c.steps.iter_mut().zip(plan) {
                step.intent.resource_set.push(name(support));
                step.conditions = vec![holds(fact)];
                step.condition_ids = vec![name(condition)];
            }
            c
        });
        report_support(&mut f, 0, "support/gripper");
        report_support(&mut f, 1, "support/chuck");
        let run = start(&mut f, 1);
        let gripper = activation(&mut f, &run);
        let part = gripper.part.clone();
        let chuck = resolve(&mut f, &run.id, "step/1", Counter(1)).unwrap();
        let first = submit_step(&mut f, &gripper, 0, id().as_str(), part.clone()).unwrap();
        let second = submit_step(&mut f, &chuck, 1, id().as_str(), part);
        if shared {
            assert!(matches!(second, Err(StoreError::Rejected(Rejection::Busy))));
        } else {
            assert_eq!(second.unwrap().host, name("host/1"));
        }
        assert_eq!(first.host, name("host/0"));
        let mut repo = f.app.into_repository();
        let rows = repo.snapshot().unwrap().1;
        let works: Vec<Work> = rows_of(&rows, "rx.internal.work.v1");
        let permits: Vec<Permit> = rows_of(&rows, "rx.internal.permit.v1");
        let resources: Vec<Resource> = rows_of(&rows, "rx.internal.resource.v1");
        let holder = |resource: &str| {
            resources
                .iter()
                .find(|r| r.id.as_str() == resource)
                .and_then(|r| r.holder.clone())
        };
        assert_eq!(
            holder("support/material-1").as_ref(),
            Some(first.operation.id())
        );
        if shared {
            assert_eq!(works.len(), 1);
            assert_eq!(permits.len(), 1);
            assert_eq!(holder("controller/1"), None);
        } else {
            assert_eq!(works.len(), 2);
            assert_eq!(permits.len(), 2);
            assert!(holder("controller/1").is_some());
            assert!(holder("support/material-2").is_some());
        }
    }
}
