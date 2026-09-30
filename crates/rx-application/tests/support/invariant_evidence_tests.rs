//! Focused invariant scenario tests (see docs/invariant-traceability.json).
use super::*;
use rx_application::control_journal::{CHANGE_SCHEMA, ChangeKind, ControlChange, EntityKind};
use rx_application::persistence::decode;
use rx_domain::operation::{Integrity, Knowledge, Outcome, Phase};

const WORK_SCHEMA: &str = "rx.internal.work.v1";
const APPLIED: &str = "rx.event.native-evidence-applied.v1";

fn every_event(repository: &mut FaultRepository, control: bool) -> Vec<StoredEvent> {
    let mut events: Vec<StoredEvent> = Vec::new();
    loop {
        let after = events.last().map_or(Counter(0), |event| event.seq);
        let page = if control {
            repository.control_events_after(after, 128)
        } else {
            repository.events_after(after, 128)
        }
        .unwrap();
        if page.is_empty() {
            return events;
        }
        events.extend(page);
    }
}

fn event_record(event: &StoredEvent) -> Record {
    Record {
        key: name("event"),
        revision: Counter(1),
        document: event.document.clone(),
    }
}

/// I01, platform half only: P emits a Prepare only from its committed intent, an Authorize only
/// after that intent's Prepared receipt, and refuses SEND_ENTERED for a send it never entered.
/// The Host's SEND_ENTERED journal commit before the native write is outside this repository.
#[test]
fn intent_commit_precedes_every_emission_and_an_unentered_send_is_refused() {
    let mut f = fixture_complete(1, true, false, true);
    let run = start(&mut f, 1);
    let a = activation(&mut f, &run);
    let messages = |f: &mut Fixture| {
        f.app
            .pending_deliveries(128)
            .unwrap()
            .into_iter()
            .filter(|d| {
                matches!(
                    d.payload,
                    Delivery::Prepare { .. } | Delivery::Authorize { .. }
                )
            })
            .collect::<Vec<_>>()
    };
    let key_ = id().to_string();
    let cr = f
        .app
        .inspect_cell(&f.executor, &f.configuration.id)
        .unwrap()
        .0;
    let rr = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
    f.failure.store(1, Ordering::SeqCst);
    assert!(
        f.app
            .submit(&f.executor, &key_, work_command(&f, &a, cr, rr))
            .is_err()
    );
    assert!(messages(&mut f).is_empty());

    let work = f
        .app
        .submit(&f.executor, &key_, work_command(&f, &a, cr, rr))
        .unwrap();
    let op = work.operation.id().clone();
    let pending = messages(&mut f);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, op);
    assert!(matches!(
        &pending[0].payload,
        Delivery::Prepare { operation, permit, .. } if operation == &op && permit == &work.permit
    ));
    let permit = f.app.inspect_permit(&f.operator, &work.permit).unwrap();
    assert_eq!(permit.operation, op);
    assert_eq!(permit.intent_digest, work.intent.digest().unwrap());
    assert_eq!(permit.state, PermitState::Issued);

    let journal = f.registrations[0].delivery_journal.clone();
    let invocation = id();
    let receipt = |sequence, state| HostReceipt {
        operation: op.clone(),
        digest: work.intent.digest().unwrap(),
        invocation: Some(invocation.clone()),
        journal: journal.clone(),
        sequence: Counter(sequence),
        state,
    };
    assert!(matches!(
        f.app
            .record_host_receipt(&f.hosts[0], &op, receipt(9, ReceiptState::SendEntered)),
        Err(StoreError::Rejected(Rejection::InvalidInput))
    ));
    let unsent = f.app.inspect_work(&f.operator, &op).unwrap();
    assert_eq!(unsent.operation.phase(), Phase::Admitted);
    assert_eq!(unsent.operation.knowledge(), Knowledge::NotSent);
    assert_eq!(
        f.app
            .inspect_permit(&f.operator, &work.permit)
            .unwrap()
            .state,
        PermitState::Issued
    );

    assert!(f.app.begin_delivery(&op).unwrap());
    assert!(
        !messages(&mut f)
            .iter()
            .any(|d| matches!(d.payload, Delivery::Authorize { .. }))
    );
    f.app
        .record_host_receipt(&f.hosts[0], &op, receipt(10, ReceiptState::Prepared))
        .unwrap();
    let authorization = rx_application::engine::authorization_delivery_id(&op);
    assert!(messages(&mut f).iter().any(|d| d.id == authorization
        && matches!(
            &d.payload,
            Delivery::Authorize { operation, permit, .. }
                if operation == &op && permit == &work.permit
        )));
    assert!(f.app.begin_delivery(&authorization).unwrap());
    let entered = f
        .app
        .record_host_receipt(
            &f.hosts[0],
            &authorization,
            receipt(11, ReceiptState::SendEntered),
        )
        .unwrap();
    assert_eq!(entered.operation.phase(), Phase::Active);
    assert_eq!(entered.operation.knowledge(), Knowledge::MayHaveExecuted);
    assert_eq!(
        f.app
            .inspect_permit(&f.operator, &work.permit)
            .unwrap()
            .state,
        PermitState::Consumed
    );
}

/// I04: SUCCEEDED is settled only by evidence that satisfies the step's completion rule. A
/// captured-result receipt, the executor's part claim, a GOOD idle observation, the elapsed
/// execution timeout with its watchdog write, and a native return the rule does not name all
/// leave the operation unsettled and without evidence.
#[test]
fn succeeded_comes_only_from_rule_evidence_not_timeout_idle_or_executor_claims() {
    let mut f = fixture_configured((1, true, true, true, None, false, true), |mut c| {
        c.fact_specs.push(FactSpec {
            id: name("idle"),
            host: name("host/0"),
            schema: name("boolean/v1"),
            unit: name("unitless"),
            maximum_age_ns: Counter(20000),
            maximum_uncertainty_ns: Counter(0),
        });
        c
    });
    // Prepared, then a ResultCaptured receipt for the entered authorization.
    let (work, evidence) = native_started(&mut f);
    let op = work.operation.id().clone();
    let unsettled = |f: &mut Fixture, reader: &Identity| {
        let current = f.app.inspect_work(reader, &op).unwrap();
        assert_eq!(current.operation.outcome(), Outcome::None);
        assert_ne!(current.operation.phase(), Phase::Settled);
        assert!(current.operation.evidence_ids().is_empty());
    };
    let operator = f.operator.clone();
    unsettled(&mut f, &operator);

    let revision = f.app.inspect_run(&f.executor, &work.run).unwrap().0;
    assert!(matches!(
        f.app.complete_part(
            &f.executor,
            id().as_str(),
            work.part.as_ref().unwrap(),
            revision
        ),
        Err(StoreError::Rejected(Rejection::ConditionUnknown))
    ));
    unsettled(&mut f, &operator);

    let idle = FactRecord {
        cell: f.configuration.id.clone(),
        id: name("idle"),
        source_host: name("host/0"),
        source_generation: f.registrations[0].source_sessions[&name("idle")].clone(),
        schema: name("boolean/v1"),
        unit: name("unitless"),
        acquired_at: expiry(1000),
        maximum_age_ns: Counter(20000),
        acquisition_uncertainty_ns: Counter(0),
        quality_good: true,
        origin_age_bounded: true,
        disputed: false,
        value: TypedValue::Boolean(true),
        evidence_id: id(),
    };
    f.app.report_fact(&f.hosts[0], idle).unwrap();
    unsettled(&mut f, &operator);

    let now = f.clock.now().ticks_ns.0 + work.intent.execution_timeout_ms.0 * 1_000_000 + 1;
    f.clock.0.store(now, Ordering::SeqCst);
    assert_eq!(
        f.app.check_maintained_conditions().unwrap(),
        vec![f.configuration.id.clone()]
    );
    let later = expiry(now + 1_000_000);
    let reader = Identity {
        principal: name("admin"),
        session: f
            .app
            .authenticated_session(&name("admin"), id(), later.clone())
            .unwrap()
            .id,
        terminal: None,
    };
    unsettled(&mut f, &reader);
    assert_eq!(
        f.app
            .inspect_work(&reader, &op)
            .unwrap()
            .operation
            .knowledge(),
        Knowledge::Unknown
    );

    let host = Identity {
        principal: name("host/0"),
        session: f
            .app
            .authenticated_session(&name("host/0"), id(), later)
            .unwrap()
            .id,
        terminal: None,
    };
    let journal = id();
    let mut unnamed = evidence.clone();
    unnamed.id = id();
    unnamed.status_schema = name("rx.sim.idle.v1");
    f.app
        .ingest_evidence(
            &host,
            EvidenceBatch {
                journal: journal.clone(),
                first: Counter(1),
                records: vec![unnamed],
            },
        )
        .unwrap();
    unsettled(&mut f, &reader);

    f.app
        .ingest_evidence(
            &host,
            EvidenceBatch {
                journal,
                first: Counter(2),
                records: vec![evidence.clone()],
            },
        )
        .unwrap();
    let done = f.app.inspect_work(&reader, &op).unwrap();
    assert_eq!(done.operation.outcome(), Outcome::Succeeded);
    assert_eq!(
        done.operation.evidence_ids(),
        std::slice::from_ref(&evidence.id)
    );
}

/// I07: one T2 commit records the native evidence, the outcome it decides together with the
/// condition evidence it used, the audit and control events, and the new revisions; a fault
/// before commit leaves none of them and the retry records each exactly once.
#[test]
fn t2_fault_leaves_no_partial_decision_and_the_retry_records_the_bundle_once() {
    for fault in [1, 2] {
        let mut f = fixture_configured((1, true, false, true, None, false, true), |mut c| {
            let conditions = c.steps[0].conditions.clone();
            let CompletionRule::Native { postconditions, .. } = &mut c.steps[0].completion else {
                panic!("native completion rule");
            };
            *postconditions = conditions;
            c
        });
        let (work, evidence) = native_started(&mut f);
        let op = work.operation.id().clone();
        let condition = f
            .app
            .inspect_fact(&f.operator, &f.configuration.id, &name("ready"))
            .unwrap()
            .evidence_id;
        let before = f.app.inspect_work(&f.operator, &op).unwrap();
        assert_eq!(before.operation.outcome(), Outcome::None);
        assert!(before.operation.evidence_ids().is_empty());
        let batch = EvidenceBatch {
            journal: id(),
            first: Counter(1),
            records: vec![evidence.clone()],
        };

        f.failure.store(fault, Ordering::SeqCst);
        assert!(f.app.ingest_evidence(&f.hosts[0], batch.clone()).is_err());
        let between = f.app.inspect_work(&f.operator, &op).unwrap();
        if fault == 1 {
            assert_eq!(between.operation, before.operation);
            assert!(
                f.app
                    .inspect_native_evidence(&f.hosts[0], &evidence.id)
                    .is_err()
            );
        } else {
            assert_eq!(between.operation.outcome(), Outcome::Succeeded);
        }

        assert_eq!(
            f.app.ingest_evidence(&f.hosts[0], batch).unwrap().through,
            Counter(1)
        );
        let done = f.app.inspect_work(&f.operator, &op).unwrap();
        assert_eq!(done.operation.outcome(), Outcome::Succeeded);
        assert_eq!(done.operation.integrity(), Integrity::Valid);
        assert_eq!(
            done.operation.revision(),
            Counter(before.operation.revision().0 + 1)
        );
        let mut used = vec![evidence.id.clone(), condition];
        used.sort();
        assert_eq!(done.operation.evidence_ids(), used.as_slice());
        assert_eq!(
            f.app
                .inspect_native_evidence(&f.hosts[0], &evidence.id)
                .unwrap()
                .operation,
            op
        );

        let mut repository = f.app.into_repository();
        let (_, rows) = repository.snapshot().unwrap();
        let stored = rows
            .iter()
            .find(|row| {
                row.document.schema.as_str() == WORK_SCHEMA
                    && decode::<Work>(row, WORK_SCHEMA).unwrap().operation.id() == &op
            })
            .unwrap()
            .clone();
        let stored_work: Work = decode(&stored, WORK_SCHEMA).unwrap();
        assert_eq!(stored_work.operation, done.operation);
        for prefix in ["evidence/", "evidenceslot/", "evidencecursor/"] {
            assert_eq!(
                rows.iter()
                    .filter(|row| row.key.as_str().starts_with(prefix))
                    .count(),
                1,
                "{prefix}"
            );
        }

        let concluded = every_event(&mut repository, true)
            .iter()
            .map(|event| decode::<ControlChange>(&event_record(event), CHANGE_SCHEMA).unwrap())
            .filter(|change| {
                change.entity_kind == EntityKind::Work && {
                    let value: Work = decode(&change.entity, WORK_SCHEMA).unwrap();
                    value.operation.id() == &op && value.operation.outcome() != Outcome::None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(concluded.len(), 1);
        assert_eq!(concluded[0].change, ChangeKind::EvidenceRecorded);
        assert_eq!(concluded[0].evidence_ids, vec![evidence.id.clone()]);
        assert_eq!(concluded[0].entity, stored);
        assert!(repository.control_snapshot().unwrap().1.contains(&stored));

        let audit = every_event(&mut repository, false);
        let applied = audit
            .iter()
            .filter(|event| event.document.schema.as_str() == APPLIED)
            .map(|event| decode::<Work>(&event_record(event), APPLIED).unwrap())
            .filter(|value| value.operation.id() == &op)
            .collect::<Vec<_>>();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].operation, stored_work.operation);
        if fault == 1 {
            assert_eq!(
                audit
                    .iter()
                    .filter(|event| event.document.schema.as_str()
                        == "rx.event.evidence-batch-committed.v1")
                    .count(),
                1
            );
        }
    }
}

/// I11: part completion reads the platform conclusion ledger. A Host native-return receipt, the
/// executor's part claim and an operator-identity attempt complete nothing; after the ledger
/// concludes from evidence, completion still waits for release and then proceeds.
#[test]
fn part_completion_follows_the_conclusion_ledger_not_receipts_executor_or_operator() {
    let mut f = fixture_complete(1, true, false, true);
    let (work, evidence) = native_started(&mut f);
    let op = work.operation.id().clone();
    let part = work.part.clone().unwrap();
    let received = f.app.inspect_work(&f.operator, &op).unwrap();
    assert_eq!(received.operation.outcome(), Outcome::None);
    assert_eq!(received.operation.knowledge(), Knowledge::MayHaveExecuted);

    let revision = f.app.inspect_run(&f.executor, &work.run).unwrap().0;
    assert!(matches!(
        f.app
            .complete_part(&f.executor, id().as_str(), &part, revision),
        Err(StoreError::Rejected(Rejection::ConditionUnknown))
    ));
    assert!(matches!(
        f.app
            .complete_part(&f.operator, id().as_str(), &part, revision),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    assert_eq!(
        f.app.inspect_run(&f.operator, &work.run).unwrap().1.state,
        RunState::Executing
    );

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
    let concluded = f.app.inspect_work(&f.operator, &op).unwrap();
    assert_eq!(concluded.operation.outcome(), Outcome::Succeeded);
    assert!(concluded.operation.evidence_ids().contains(&evidence.id));
    let revision = f.app.inspect_run(&f.executor, &work.run).unwrap().0;
    assert!(matches!(
        f.app
            .complete_part(&f.executor, id().as_str(), &part, revision),
        Err(StoreError::Rejected(Rejection::ConditionUnknown))
    ));

    let proof = handover_proof(&mut f, &concluded, &evidence);
    f.app
        .release_resources(&f.hosts[0], id().as_str(), proof)
        .unwrap();
    let revision = f.app.inspect_run(&f.executor, &work.run).unwrap().0;
    let completed = f
        .app
        .complete_part(&f.executor, id().as_str(), &part, revision)
        .unwrap();
    assert_eq!(completed.disposition, PartDisposition::ConfirmedCompleted);
    assert_eq!(
        f.app.inspect_run(&f.operator, &work.run).unwrap().1.state,
        RunState::Completed
    );
}
