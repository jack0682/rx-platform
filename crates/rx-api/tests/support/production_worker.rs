//! Two synthetic Host completions; the real S coordinator must admit and complete both parts.
use super::*;
type Runtime = Handle<Application<SqliteRepository, TestClock, SimulationOnly>>;
#[allow(clippy::too_many_arguments)]
pub async fn finish(
    runtime: &Runtime,
    directory: &tempfile::TempDir,
    mut child: std::process::Child,
    mut config: serde_json::Value,
    output: &std::path::Path,
    mode: &str,
    admin: Identity,
    host: Identity,
    run: &Id,
) {
    let journal = id();
    let mut operations = vec![];
    for ordinal in 1..=2 {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(25);
        let work = loop {
            let Reply::Overview(view) = runtime
                .call(Command::Overview(admin.clone()))
                .await
                .unwrap()
            else {
                panic!("overview")
            };
            let work = view
                .cells
                .iter()
                .flat_map(|c| &c.work)
                .find(|w| !operations.contains(w.operation.id()));
            if let Some(work) = work {
                break work.operation.id().clone();
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "production worker exited before part {ordinal}"
            );
            assert!(
                tokio::time::Instant::now() < deadline,
                "production admission timeout"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        operations.push(work.clone());
        drive(runtime, &host, &work, &journal, ordinal).await;
    }
    wait_child(&mut child).await;
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "production worker: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let before: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("report.json")).unwrap()).unwrap();
    assert_eq!(before["planner_starts"], 2);
    assert_eq!(before["report"]["phase"], "PAUSE_OBSERVED");
    assert_eq!(before["report"]["stop"]["reason"], "COMPLETED");
    assert!(
        before["report"]["stop"]["attempts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let Reply::VersionedRun(_, finished) = runtime
        .call(Command::InspectRun {
            identity: admin.clone(),
            run: run.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("run")
    };
    assert_eq!(finished.state, RunState::Completed);
    assert_eq!(finished.part_ids.len(), 2);
    assert_eq!(finished.budget.as_ref().unwrap().consumed(), Counter(2));
    assert_eq!(finished.budget.as_ref().unwrap().remaining(), Counter(0));
    let attempts = before["coordination_attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 4);
    for attempt in attempts {
        let lost = (mode == "PRODUCTION_LOST_BEGIN"
            && attempt["logical"]["stage"] == "BEGIN_PART"
            && attempt["logical"]["visit"] == "1")
            || (mode == "PRODUCTION_LOST_COMPLETE"
                && attempt["logical"]["stage"] == "COMPLETE_PART"
                && attempt["logical"]["visit"] == "1");
        assert_eq!(
            attempt["resolution"]["state"],
            if lost { "PENDING" } else { "REPLY" }
        );
    }
    let recovered = directory.path().join("production-recovered");
    config["mode"] = serde_json::json!("RECOVER");
    config["output"] = serde_json::json!(recovered);
    config["ready"] = serde_json::json!(directory.path().join("production-recovered-ready"));
    config["pin"]["peer_boot"] = serde_json::json!(id());
    let path = directory.path().join("production-recovery.json");
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    let mut child =
        std::process::Command::new(std::env::var("RX_EXECUTOR_SERVICE_FIXTURE").unwrap())
            .arg(path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
    wait_child(&mut child).await;
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "production recovery: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(recovered.join("report.json")).unwrap()).unwrap();
    assert_eq!(after["planner_starts"], 0);
    assert_eq!(
        after["coordination_attempts"],
        before["coordination_attempts"]
    );
    if let Ok(root) = std::env::var("RX_EXECUTOR_WORKER_OUTPUT") {
        let root = std::path::PathBuf::from(root).join(mode.to_ascii_lowercase());
        std::fs::create_dir(&root).unwrap();
        std::fs::copy(output.join("report.json"), root.join("report.json")).unwrap();
        std::fs::copy(
            recovered.join("report.json"),
            root.join("recovered-report.json"),
        )
        .unwrap();
    }
}
async fn drive(runtime: &Runtime, host: &Identity, operation: &Id, journal: &Id, ordinal: u64) {
    let Reply::DeliveryPlan(plan) = runtime
        .call(Command::PlanDelivery {
            identity: host.clone(),
            message: operation.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("prepare")
    };
    assert!(plan.first_emission);
    let work = plan.work.unwrap();
    let invocation = id();
    let receipt = HostReceipt {
        operation: operation.clone(),
        digest: work.intent.digest().unwrap(),
        invocation: Some(invocation.clone()),
        journal: plan.registration.delivery_journal,
        sequence: Counter(ordinal * 2),
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
    let message = rx_application::engine::authorization_delivery_id(operation);
    let Reply::DeliveryPlan(next) = runtime
        .call(Command::PlanDelivery {
            identity: host.clone(),
            message: message.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("authorize")
    };
    assert!(next.first_emission);
    runtime
        .call(Command::RecordReceipt {
            identity: host.clone(),
            message,
            receipt: HostReceipt {
                sequence: Counter(ordinal * 2 + 1),
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
                journal: journal.clone(),
                first: Counter(ordinal),
                records: vec![evidence.clone()],
            },
        })
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let Reply::ReconciliationRequests(plans) = runtime
            .call(Command::PendingReconciliations {
                identity: host.clone(),
                after: None,
                limit: 8,
            })
            .await
            .unwrap()
        else {
            panic!("queries")
        };
        if plans.iter().any(|p| p.operation == *operation) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "handover request missing"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let Reply::Work(current) = runtime
        .call(Command::InspectWork {
            identity: host.clone(),
            operation: operation.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("work")
    };
    let Reply::Cell(cell_revision, _) = runtime
        .call(Command::InspectCell {
            identity: host.clone(),
            cell: work.cell.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("cell")
    };
    let observations = ["no-pending", "control", "support"]
        .into_iter()
        .map(|value| HandoverObservation {
            id: id(),
            operation: operation.clone(),
            invocation: evidence.invocation.clone(),
            profile_digest: evidence.profile_digest,
            device_session: evidence.device_session.clone(),
            host_boot: plan.registration.boot_id.clone(),
            source: name(&format!("handover/{operation}/{value}")),
            schema: name("rx.handover.v1"),
            value: true,
            observed_at: evidence.captured_at.clone(),
            uncertainty_ns: Counter(0),
            quality_good: true,
            origin_age_bounded: true,
        })
        .collect();
    runtime
        .call(Command::ReleaseResources {
            identity: host.clone(),
            key: id(),
            command: ReleaseResources {
                operation: operation.clone(),
                expected_operation: current.operation.revision(),
                expected_cell: cell_revision,
                observations,
            },
        })
        .await
        .unwrap();
}
