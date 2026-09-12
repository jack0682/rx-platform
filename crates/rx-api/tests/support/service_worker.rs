use super::*;
type Runtime = Handle<Application<SqliteRepository, TestClock, SimulationOnly>>;
pub async fn finish(
    runtime: &Runtime,
    directory: &tempfile::TempDir,
    mut child: std::process::Child,
    mut config: serde_json::Value,
    output: &std::path::Path,
    mode: &str,
    admin: Identity,
) {
    let expected_work = usize::from(mode != "SERVICE_BEFORE_ASSIGN");
    if expected_work > 0 {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(25);
        loop {
            let Reply::Overview(view) = runtime
                .call(Command::Overview(admin.clone()))
                .await
                .unwrap()
            else {
                panic!("overview")
            };
            if view.cells.iter().map(|c| c.work.len()).sum::<usize>() == expected_work {
                break;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "service exited before work"
            );
            assert!(
                tokio::time::Instant::now() < deadline,
                "service work timeout"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    std::fs::write(config["stop_file"].as_str().unwrap(), b"stop requested").unwrap();
    wait_child(&mut child).await;
    let result = child.wait_with_output().unwrap();
    assert_eq!(
        result.status.code(),
        Some(if mode == "SERVICE_CRASH_STOP" { 88 } else { 0 }),
        "service {mode}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let source = if mode == "SERVICE_CRASH_STOP" {
        "boundary.json"
    } else {
        "report.json"
    };
    let before: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join(source)).unwrap()).unwrap();
    if mode != "SERVICE_CRASH_STOP" {
        let report = &before["report"];
        assert_eq!(
            report["phase"],
            if mode == "SERVICE_PAUSE_UNAVAILABLE" {
                "PENDING"
            } else {
                "PAUSE_OBSERVED"
            }
        );
        if mode == "SERVICE_STORE_FAULT" {
            assert!(report["durability_fault"].is_string());
            assert!(before["saved"].is_null());
        } else {
            assert!(report["durability_fault"].is_null());
            assert_eq!(report["stop"], before["saved"]);
        }
        if mode == "SERVICE_BEFORE_ASSIGN" {
            assert_eq!(before["planner_starts"], 0);
            assert!(report["stop"]["origin_context"].is_null());
        }
        if matches!(mode, "SERVICE_LOST_PAUSE" | "SERVICE_PAUSE_UNAVAILABLE") {
            let attempts = report["stop"]["attempts"].as_array().unwrap();
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0]["state"], "ENTERED");
            assert!(attempts[0]["response"].is_null());
        }
    }
    let Reply::Overview(view) = runtime
        .call(Command::Overview(admin.clone()))
        .await
        .unwrap()
    else {
        panic!("overview")
    };
    assert_eq!(
        view.cells.iter().map(|c| c.work.len()).sum::<usize>(),
        expected_work
    );
    let recovered = directory.path().join("service-recovered");
    config["output"] = serde_json::json!(recovered);
    config["ready"] = serde_json::json!(directory.path().join("service-recovered-ready"));
    config["mode"] = serde_json::json!("RECOVER");
    config["pin"]["peer_boot"] = serde_json::json!(id());
    let path = directory.path().join("service-recovered-config.json");
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
        "service recovery: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(recovered.join("report.json")).unwrap()).unwrap();
    assert_eq!(after["planner_starts"], 0);
    assert_eq!(after["report"]["phase"], "PAUSE_OBSERVED");
    if mode == "SERVICE_CRASH_STOP" {
        assert_eq!(after["saved"]["id"], before["id"]);
        assert!(after["saved"]["attempts"].as_array().unwrap().is_empty());
    } else if mode != "SERVICE_STORE_FAULT" {
        assert_eq!(after["saved"]["id"], before["saved"]["id"]);
        assert_eq!(after["saved"]["attempts"], before["saved"]["attempts"]);
    }
    let Reply::Overview(view) = runtime.call(Command::Overview(admin)).await.unwrap() else {
        panic!("overview")
    };
    assert_eq!(
        view.cells.iter().map(|c| c.work.len()).sum::<usize>(),
        expected_work
    );
    if let Ok(root) = std::env::var("RX_EXECUTOR_WORKER_OUTPUT") {
        let root = std::path::PathBuf::from(root).join(mode.to_ascii_lowercase());
        std::fs::create_dir(&root).unwrap();
        std::fs::copy(output.join(source), root.join(source)).unwrap();
        std::fs::copy(
            recovered.join("report.json"),
            root.join("recovered-report.json"),
        )
        .unwrap();
    }
}
