//! Synthetic Host facts for the S-worker wire test; actual Host queries have a separate E2E.
use super::*;
type Runtime = Handle<Application<SqliteRepository, TestClock, SimulationOnly>>;

async fn marker(path: &std::path::Path, child: &mut std::process::Child) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(25);
    while !path.exists() {
        assert!(
            child.try_wait().unwrap().is_none(),
            "worker exited before {}",
            path.display()
        );
        assert!(
            tokio::time::Instant::now() < deadline,
            "worker marker timeout: {}",
            path.display()
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

pub async fn complete_and_release(
    runtime: &Runtime,
    admin: Identity,
    host: Identity,
    output: &std::path::Path,
    child: &mut std::process::Child,
) {
    marker(&output.join("admitted.json"), child).await;
    let operation: Id =
        canonical::decode_json(&std::fs::read(output.join("admitted.json")).unwrap()).unwrap();
    let Reply::DeliveryPlan(plan) = runtime
        .call(Command::PlanDelivery {
            identity: host.clone(),
            message: operation.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("prepare plan")
    };
    assert!(plan.first_emission);
    let work = plan.work.unwrap();
    let invocation = id();
    let receipt = HostReceipt {
        operation: operation.clone(),
        digest: work.intent.digest().unwrap(),
        invocation: Some(invocation.clone()),
        journal: plan.registration.delivery_journal.clone(),
        sequence: Counter(1),
        state: ReceiptState::Prepared,
    };
    runtime
        .call(Command::RecordReceipt {
            identity: host.clone(),
            message: operation.clone(),
            receipt: receipt.clone(),
        })
        .await
        .unwrap();
    let message = rx_application::engine::authorization_delivery_id(&operation);
    let Reply::DeliveryPlan(auth) = runtime
        .call(Command::PlanDelivery {
            identity: host.clone(),
            message: message.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("authorize plan")
    };
    assert!(auth.first_emission);
    runtime
        .call(Command::RecordReceipt {
            identity: host.clone(),
            message,
            receipt: HostReceipt {
                sequence: Counter(2),
                state: ReceiptState::ResultCaptured,
                ..receipt
            },
        })
        .await
        .unwrap();
    let evidence = NativeEvidence {
        native_details: None,
        id: id(),
        operation: operation.clone(),
        invocation,
        profile_digest: work.intent.profile_digest,
        device_session: id(),
        status_schema: name("rx.sim.completed.v1"),
        status: Integer(0),
        captured_at: TimePoint {
            clock_id: "test-clock".into(),
            ticks_ns: Counter(1000),
        },
    };
    runtime
        .call(Command::IngestEvidence {
            identity: host.clone(),
            batch: EvidenceBatch {
                journal: id(),
                first: Counter(1),
                records: vec![evidence.clone()],
            },
        })
        .await
        .unwrap();
    std::fs::write(
        output.join("handover-ready"),
        b"synthetic completion committed",
    )
    .unwrap();
    marker(&output.join("handover-requested"), child).await;
    let Reply::ReconciliationRequests(plans) = runtime
        .call(Command::PendingReconciliations {
            identity: host.clone(),
            after: None,
            limit: 4,
        })
        .await
        .unwrap()
    else {
        panic!("query plans")
    };
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].operation, operation);
    assert_eq!(plans[0].generation, Counter(1));
    let Reply::ReconciliationPlan(current) = runtime
        .call(Command::PlanReconciliation {
            identity: host.clone(),
            operation: operation.clone(),
            request: plans[0].id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("query plan")
    };
    assert_ne!(
        current.work.operation.disposition(),
        rx_domain::operation::Disposition::Released
    );
    let observations: Vec<_> = ["no-pending", "control", "support"]
        .into_iter()
        .map(|predicate| HandoverObservation {
            id: id(),
            operation: operation.clone(),
            invocation: evidence.invocation.clone(),
            profile_digest: evidence.profile_digest,
            device_session: evidence.device_session.clone(),
            host_boot: plan.registration.boot_id.clone(),
            source: name(&format!("handover/{operation}/{predicate}")),
            schema: name("rx.handover.v1"),
            value: true,
            observed_at: TimePoint {
                clock_id: "test-clock".into(),
                ticks_ns: Counter(1000),
            },
            uncertainty_ns: Counter(0),
            quality_good: true,
            origin_age_bounded: true,
        })
        .collect();
    runtime
        .call(Command::RecordReconciliationObservations {
            identity: host.clone(),
            operation: operation.clone(),
            request: plans[0].id.clone(),
            observations: observations.clone(),
        })
        .await
        .unwrap();
    let Reply::Work(released) = runtime
        .call(Command::ReleaseResources {
            identity: host.clone(),
            key: id(),
            command: ReleaseResources {
                operation: operation.clone(),
                expected_operation: current.work.operation.revision(),
                expected_cell: current.cell_revision,
                observations,
            },
        })
        .await
        .unwrap()
    else {
        panic!("released work")
    };
    assert_eq!(
        released.operation.disposition(),
        rx_domain::operation::Disposition::Released
    );
    runtime
        .call(Command::UpdateReconciliation {
            identity: host,
            operation: operation.clone(),
            request: plans[0].id.clone(),
            state: ReconciliationState::Complete,
            issue: None,
        })
        .await
        .unwrap();
    let Reply::Work(actual) = runtime
        .call(Command::InspectWork {
            identity: admin,
            operation,
        })
        .await
        .unwrap()
    else {
        panic!("actual work")
    };
    assert_eq!(
        actual.operation.disposition(),
        rx_domain::operation::Disposition::Released
    );
    std::fs::write(output.join("handover-released"), b"P release committed").unwrap();
}
