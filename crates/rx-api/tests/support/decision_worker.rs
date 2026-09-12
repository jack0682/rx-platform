use super::*;
type Runtime = Handle<Application<SqliteRepository, TestClock, SimulationOnly>>;
pub async fn finish(
    _runtime: &Runtime,
    directory: &tempfile::TempDir,
    mut child: std::process::Child,
    mut config: serde_json::Value,
    output: &std::path::Path,
    mode: &str,
) {
    wait_child(&mut child).await;
    let result = child.wait_with_output().unwrap();
    let exit = match mode {
        "CHECKPOINT_CRASH_BEFORE" => 75,
        "CHECKPOINT_CRASH_AFTER" => 76,
        _ => 0,
    };
    assert_eq!(
        result.status.code(),
        Some(exit),
        "decision worker {mode}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let source = if exit == 0 {
        "report.json"
    } else {
        "boundary.json"
    };
    let before: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join(source)).unwrap()).unwrap();
    config["mode"] = serde_json::json!("RECOVER");
    config["pin"]["peer_boot"] = serde_json::json!(id());
    let recovered = directory.path().join("decision-recovered");
    config["output"] = serde_json::json!(recovered);
    config["ready"] = serde_json::json!(directory.path().join("decision-recovered-ready"));
    let path = directory.path().join("decision-recover-config.json");
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    let mut child =
        std::process::Command::new(std::env::var("RX_EXECUTOR_DECISION_FIXTURE").unwrap())
            .arg(path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
    wait_child(&mut child).await;
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "decision recovery: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(recovered.join("report.json")).unwrap()).unwrap();
    let work = usize::from(!matches!(
        mode,
        "CHECKPOINT_CRASH_BEFORE"
            | "CHECKPOINT_CRASH_AFTER"
            | "CHECKPOINT_WAIT_PENDING"
            | "CHECKPOINT_WAIT_TIMEOUT"
    ));
    assert_eq!(report["work_count"], work);
    assert_eq!(report["admission"], false);
    let entries = report["entries"].as_array().unwrap();
    let checkpoints: Vec<_> = entries
        .iter()
        .filter(|e| {
            e["logical"]["stage"]
                .as_str()
                .unwrap()
                .starts_with("CHECKPOINT_")
        })
        .collect();
    assert!(!checkpoints.is_empty());
    if exit != 0 {
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(checkpoints[0]["key"], before["key"]);
        assert_eq!(checkpoints[0]["send"], "EMIT_ENTERED");
        assert_eq!(checkpoints[0]["resolution"]["state"], "PENDING");
        let branch_count = report["process_checkpoint"]["branches"]
            .as_object()
            .unwrap()
            .len();
        assert_eq!(branch_count, usize::from(mode == "CHECKPOINT_CRASH_AFTER"));
    } else {
        assert_eq!(report["entries"], before["entries"]);
        assert_eq!(report["process_checkpoint"], before["process_checkpoint"]);
    }
    if mode == "CHECKPOINT_LOST_REPLY" {
        assert_eq!(checkpoints[0]["resolution"]["state"], "PENDING");
    }
    if mode == "CHECKPOINT_EXPIRED" {
        let attempts: Vec<_> = report["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["logical"]["stage"] == "CHECKPOINT_BRANCH")
            .collect();
        assert_eq!(attempts.len(), 2);
        let rejected = attempts.iter().find(|e| e["generation"] == "1").unwrap();
        assert_eq!(rejected["resolution"]["state"], "CHECKPOINT_REJECTED");
        assert_eq!(rejected["resolution"]["reason"], "EXPIRED");
        assert_eq!(checkpoints[0]["generation"], "2");
        assert_eq!(checkpoints[0]["resolution"]["state"], "REPLY");
        assert_ne!(rejected["key"], checkpoints[0]["key"]);
    }
    if exit == 0 {
        assert!(before["engine_pid"].as_u64().is_some());
        assert_eq!(before["pause_observed"], true);
        let replies: Vec<serde_json::Value> = std::fs::read_dir(output)
            .unwrap()
            .filter_map(|f| {
                let f = f.unwrap();
                f.file_name()
                    .to_str()
                    .unwrap()
                    .starts_with("engine-reply-")
                    .then(|| serde_json::from_slice(&std::fs::read(f.path()).unwrap()).unwrap())
            })
            .collect();
        assert!(!replies.is_empty());
        if mode.contains("WAIT") {
            let waits = replies
                .iter()
                .flat_map(|r| r["requests"].as_array().unwrap())
                .filter(|r| r["kind"] == "BEGIN_WAIT")
                .count();
            assert_eq!(
                waits, 1,
                "persistent BT emits the wait suggestion once; Rust retains it"
            );
        }
    }
    if mode == "CHECKPOINT_WAIT_PENDING" {
        assert!(before["waiting"].as_u64().unwrap() >= 3);
        assert_eq!(checkpoints.len(), 1);
        assert!(
            report["process_checkpoint"]["waits"]
                .as_object()
                .unwrap()
                .is_empty()
        );
    }
    if mode == "CHECKPOINT_BRANCH_FALSE" {
        assert!(
            report["process_checkpoint"]["branches"]
                .as_object()
                .unwrap()
                .values()
                .all(|b| b["chosen"] == false)
        );
    }
    // Every actual stored P process fact is recovered independently of local reply state.
    let observations = report["observations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| {
            o["logical"]["stage"]
                .as_str()
                .unwrap()
                .starts_with("CHECKPOINT_")
        })
        .count();
    let cp = &report["process_checkpoint"];
    assert_eq!(
        observations,
        cp["branches"].as_object().unwrap().len()
            + cp["wait_windows"].as_object().unwrap().len()
            + cp["waits"].as_object().unwrap().len()
    );
    if let Ok(root) = std::env::var("RX_EXECUTOR_WORKER_OUTPUT") {
        let root = std::path::PathBuf::from(root).join(mode.to_ascii_lowercase());
        std::fs::create_dir(&root).unwrap();
        for file in std::fs::read_dir(output).unwrap() {
            let file = file.unwrap();
            if file.file_type().unwrap().is_file() {
                std::fs::copy(file.path(), root.join(file.file_name())).unwrap();
            }
        }
        std::fs::copy(
            recovered.join("report.json"),
            root.join("recovered-report.json"),
        )
        .unwrap();
    }
}
