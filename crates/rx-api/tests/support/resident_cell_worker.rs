//! Real P mTLS/SQLite and one actual S CellService process; Host results are explicit simulation.
//! This fixture observes requests and drives existing application commands, never edits P/S rows.
use super::*;
use rx_application::projection::Overview;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, Stdio},
    sync::Mutex,
    time::Duration,
};

type Runtime = Handle<Application<SqliteRepository, TestClock, SimulationOnly>>;

#[derive(Clone, Default, Debug, Serialize)]
struct Counts {
    assignment_reads: usize,
    none_reads: usize,
    executor_sessions: BTreeSet<Id>,
    begin_parts: usize,
    complete_parts: usize,
    resolves: usize,
    submits: usize,
    pauses: usize,
}
#[derive(Default)]
pub(super) struct Audit(Mutex<Counts>);
impl Audit {
    fn snapshot(&self) -> Counts {
        self.0.lock().unwrap().clone()
    }
}
pub(super) struct ObservedPort {
    inner: Arc<dyn ApplicationPort>,
    audit: Arc<Audit>,
}
impl ObservedPort {
    pub(super) fn new(inner: Arc<dyn ApplicationPort>, audit: Arc<Audit>) -> Self {
        Self { inner, audit }
    }
}
impl ApplicationPort for ObservedPort {
    fn request(&self, command: Command) -> CallFuture<'_> {
        {
            let mut count = self.audit.0.lock().unwrap();
            match &command {
                Command::ExecutorAssignment { .. } => count.assignment_reads += 1,
                Command::ExecutorBeginPart { .. } => count.begin_parts += 1,
                Command::ExecutorCompletePart { .. } => count.complete_parts += 1,
                Command::ExecutorResolveActivation { .. } => count.resolves += 1,
                Command::ExecutorSubmit { .. } => count.submits += 1,
                Command::PauseExecutorRun { .. } => count.pauses += 1,
                _ => {}
            }
        }
        Box::pin(async move {
            let reply = self.inner.request(command).await?;
            if let Reply::ExecutorAssignment(view) = &reply {
                let mut count = self.audit.0.lock().unwrap();
                count.executor_sessions.insert(view.caller_session.clone());
                if view.cardinality == rx_process_contract::assignment::Cardinality::None {
                    count.none_reads += 1;
                }
            }
            Ok(reply)
        })
    }
    fn status(&self) -> Status {
        self.inner.status()
    }
}

struct ChildOwner {
    child: Child,
    stop: PathBuf,
    stderr: PathBuf,
    output: PathBuf,
    diagnostics: Vec<(PathBuf, &'static str)>,
}
impl ChildOwner {
    fn preserve_diagnostics(&self, prefix: &str) {
        for (source, label) in &self.diagnostics {
            if std::fs::metadata(source).is_ok_and(|m| m.is_file() && m.len() <= 4_194_304) {
                let _ = std::fs::copy(source, self.output.join(format!("{prefix}-{label}")));
            }
        }
    }
    fn assert_running(&mut self) {
        if let Some(exit) = self.child.try_wait().unwrap() {
            self.preserve_diagnostics("exited");
            let stdout =
                std::fs::read_to_string(self.output.join("child.stdout")).unwrap_or_default();
            panic!(
                "resident S process exited ({exit}); stderr: {}; stdout: {}; diagnostics: {}",
                std::fs::read_to_string(&self.stderr).unwrap_or_default(),
                stdout.chars().take(12000).collect::<String>(),
                self.output.display()
            );
        }
    }
}
impl Drop for ChildOwner {
    fn drop(&mut self) {
        self.preserve_diagnostics("before-cleanup");
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = std::fs::write(&self.stop, b"stop");
            for _ in 0..100 {
                if self.child.try_wait().ok().flatten().is_some() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
fn read_json(path: &Path) -> Option<Value> {
    match std::fs::read(path) {
        Ok(bytes) => Some(serde_json::from_slice(&bytes).expect("atomic fixture JSON")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => panic!("{}: {e}", path.display()),
    }
}
async fn overview(runtime: &Runtime, admin: &Identity) -> Overview {
    let Reply::Overview(view) = runtime
        .call(Command::Overview(admin.clone()))
        .await
        .unwrap()
    else {
        panic!("overview reply")
    };
    *view
}
fn all_parts(view: &Overview) -> BTreeSet<Id> {
    view.cells
        .iter()
        .flat_map(|c| &c.runs)
        .flat_map(|r| r.value.part_ids.iter().cloned())
        .collect()
}
async fn await_status(
    child: &mut ChildOwner,
    path: &Path,
    session: Option<&Id>,
    completed: u64,
    planners: usize,
) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(35);
    let expected_completed = completed.to_string();
    loop {
        child.assert_running();
        if let Some(value) = read_json(path) {
            assert_ne!(
                value["status"]["phase"], "ATTENTION",
                "S service attention: {value}"
            );
            if value["status"]["phase"] == "IDLE"
                && value["status"]["completed_runs"].as_str() == Some(expected_completed.as_str())
                && value["planner_starts"] == planners
            {
                assert!(value["status"]["run"].is_null());
                assert!(value["status"]["attachment"].is_null());
                assert!(value["status"]["active"].is_null());
                if let Some(session) = session {
                    assert_eq!(value["status"]["session"], session.as_str());
                }
                return value;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "resident idle status timeout: {:?}",
            read_json(path)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
#[allow(clippy::too_many_arguments)]
async fn verify_idle(
    runtime: &Runtime,
    admin: &Identity,
    child: &mut ChildOwner,
    status: &Path,
    audit: &Audit,
    completed: u64,
    planners: usize,
    session: &Id,
) -> Value {
    let status = await_status(child, status, Some(session), completed, planners).await;
    let before = audit.snapshot();
    let view = overview(runtime, admin).await;
    let parts = all_parts(&view);
    assert_eq!(parts.len(), completed as usize * 2);
    assert!(
        view.cells
            .iter()
            .all(|c| !c.runs_truncated && !c.work_truncated)
    );
    assert!(
        view.cells
            .iter()
            .flat_map(|c| &c.runs)
            .all(|r| r.value.state == RunState::Completed)
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        child.assert_running();
        if audit.snapshot().none_reads >= before.none_reads + 5 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "S did not poll NONE while idle"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let after = audit.snapshot();
    assert_eq!(
        (
            after.begin_parts,
            after.complete_parts,
            after.resolves,
            after.submits,
            after.pauses
        ),
        (
            before.begin_parts,
            before.complete_parts,
            before.resolves,
            before.submits,
            before.pauses
        ),
        "idle discovery must not invoke production or PauseRun"
    );
    assert_eq!(all_parts(&overview(runtime, admin).await), parts);
    assert_eq!(after.executor_sessions, BTreeSet::from([session.clone()]));
    json!({"status":status,"counts_before":before,"counts_after":after,"parts":parts})
}

async fn start_run(
    runtime: &Runtime,
    configuration: &CellConfiguration,
    admin: &Identity,
    host: &Identity,
    session: &Id,
    arm_sequence: u64,
) -> Run {
    let Reply::Cell(cell_revision, _) = runtime
        .call(Command::InspectCell {
            identity: admin.clone(),
            cell: configuration.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("cell")
    };
    let Reply::Run(run) = runtime
        .call(Command::CreateRun {
            identity: admin.clone(),
            key: id(),
            command: CreateRun {
                cell: configuration.id.clone(),
                recipe_digest: configuration.recipe.sha256,
                site_config_digest: configuration.site_config_digest,
                expected_cell: cell_revision,
            },
        })
        .await
        .unwrap()
    else {
        panic!("created Run")
    };
    let Reply::VersionedRun(run_revision, _) = runtime
        .call(Command::InspectRun {
            identity: admin.clone(),
            run: run.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("Run revision")
    };
    let Reply::Attempt(attempt) = runtime
        .call(Command::StartRun {
            identity: admin.clone(),
            key: id(),
            command: StartRun {
                run: run.id.clone(),
                envelope_digest: configuration.envelope.sha256,
                purpose: Purpose::Production,
                budget_unit: rx_domain::budget::BudgetUnit::PartAttempt,
                budget_limit: Counter(2),
                expected_cell: cell_revision,
                expected_run: run_revision,
            },
        })
        .await
        .unwrap()
    else {
        panic!("StartRun")
    };
    assert_eq!(&attempt.executor_session, session);
    assert_eq!(attempt.status, StartStatus::Arming);
    let Reply::PendingDeliveries(pending) = runtime
        .call(Command::PendingDeliveries {
            after: None,
            limit: 128,
        })
        .await
        .unwrap()
    else {
        panic!("Arm outbox")
    };
    let message = pending.iter().find(|p| matches!(&p.payload, Delivery::Arm { attempt: target, .. } if target == &attempt.id)).unwrap().id.clone();
    let Reply::DeliveryPlan(plan) = runtime
        .call(Command::PlanDelivery {
            identity: host.clone(),
            message: message.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("Arm delivery")
    };
    let reg = plan.registration;
    runtime
        .call(Command::FinishArm {
            identity: host.clone(),
            message,
            ack: ArmAcknowledgment {
                attempt: attempt.id,
                host_boot: reg.boot_id,
                delivery_journal: reg.delivery_journal,
                sequence: Counter(arm_sequence),
                epoch: reg.epoch,
                scopes: reg.scopes,
            },
        })
        .await
        .unwrap();
    let Reply::VersionedRun(_, running) = runtime
        .call(Command::InspectRun {
            identity: admin.clone(),
            run: run.id,
        })
        .await
        .unwrap()
    else {
        panic!("started Run")
    };
    assert_eq!(running.state, RunState::Executing);
    assert_eq!(running.executor_session.as_ref(), Some(session));
    assert_eq!(running.budget.as_ref().unwrap().limit(), Counter(2));
    running
}
async fn complete_two_parts(
    runtime: &Runtime,
    admin: &Identity,
    host: &Identity,
    child: &mut ChildOwner,
    run: &Id,
    journal: &Id,
    run_index: u64,
) -> Run {
    let mut operations = BTreeSet::new();
    for part in 1..=2 {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let operation = loop {
            child.assert_running();
            let view = overview(runtime, admin).await;
            if let Some(work) = view
                .cells
                .iter()
                .flat_map(|c| &c.work)
                .find(|w| &w.run == run && !operations.contains(w.operation.id()))
            {
                break work.operation.id().clone();
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "resident Run {run} part {part} admission timeout"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        operations.insert(operation.clone());
        let ordinal = run_index * 2 + part;
        production_worker::drive_with_sequence(
            runtime,
            host,
            &operation,
            journal,
            ordinal,
            ordinal * 2 + run_index,
        )
        .await;
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        child.assert_running();
        let Reply::VersionedRun(_, value) = runtime
            .call(Command::InspectRun {
                identity: admin.clone(),
                run: run.clone(),
            })
            .await
            .unwrap()
        else {
            panic!("completed Run")
        };
        if value.state == RunState::Completed {
            assert_eq!(value.part_ids.len(), 2);
            assert_eq!(value.budget.as_ref().unwrap().consumed(), Counter(2));
            assert_eq!(value.budget.as_ref().unwrap().remaining(), Counter(0));
            return value;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "P Run {run} did not complete"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    runtime: &Runtime,
    directory: &tempfile::TempDir,
    configuration: &CellConfiguration,
    installation: &Installation,
    uri: &str,
    release: Digest,
    ca: &str,
    certificate: &str,
    private_key: &str,
    admin_session: Id,
    host_session: Id,
    clock_file: &Path,
    audit: Arc<Audit>,
) {
    let evidence_output = PathBuf::from(
        std::env::var("RX_EXECUTOR_CELL_OUTPUT").expect("new evidence directory supplied by tool"),
    );
    assert!(evidence_output.is_dir());
    let output = evidence_output.join("resident-cell");
    std::fs::create_dir(&output).expect("resident evidence destination must be new");
    let fixture = directory.path();
    for (name, data) in [
        ("cell-ca.pem", ca),
        ("cell-client.pem", certificate),
        ("cell-key.pem", private_key),
    ] {
        std::fs::write(fixture.join(name), data).unwrap();
    }
    let root = fixture.join("resident-journals");
    std::fs::create_dir(&root).unwrap();
    let ready = fixture.join("resident-ready.json");
    let status = fixture.join("resident-status.json");
    let stop_file = fixture.join("resident-stop");
    let report_root = fixture.join("resident-output");
    let image = std::env::var("RX_BT_REQUEST_IMAGE").expect("observed immutable C++ image ID");
    assert!(
        image.starts_with("sha256:")
            && image.len() == 71
            && image[7..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
    let config = json!({
        "uri":uri,"server_name":"localhost","ca":fixture.join("cell-ca.pem"),
        "certificate":fixture.join("cell-client.pem"),"key":fixture.join("cell-key.pem"),
        "pin":{"principal":"executor","peer_boot":id(),"installation":installation.id,
            "store_generation":installation.store_generation,"release":release,"clock_id":"test-clock",
            "cell":configuration.id,"definition":configuration.definition.sha256},
        "service_journal":id(),"root":root,"clock_file":clock_file,"cpp_image":image,
        "ready":ready,"stop_file":stop_file,"status_file":status,"output":report_root,"initialize":true,
    });
    assert!(config.get("run").is_none() && config.get("visit").is_none());
    let config_path = fixture.join("resident-config.json");
    std::fs::write(&config_path, canonical::bytes(&config).unwrap()).unwrap();
    let stderr = output.join("child.stderr");
    let child = ProcessCommand::new(
        std::env::var("RX_EXECUTOR_CELL_SERVICE_FIXTURE")
            .expect("actual S resident fixture binary"),
    )
    .arg(&config_path)
    .stdout(Stdio::from(
        std::fs::File::create(output.join("child.stdout")).unwrap(),
    ))
    .stderr(Stdio::from(std::fs::File::create(&stderr).unwrap()))
    .spawn()
    .unwrap();
    let pid = child.id();
    let mut child = ChildOwner {
        child,
        stop: stop_file.clone(),
        stderr,
        output: output.clone(),
        diagnostics: vec![
            (ready.clone(), "ready.json"),
            (status.clone(), "status.json"),
            (report_root.join("report.json"), "report.json"),
        ],
    };
    let ready_value = await_status(&mut child, &ready, None, 0, 0).await;
    let session = Id::new(ready_value["status"]["session"].as_str().unwrap()).unwrap();
    let admin = Identity {
        principal: name("admin"),
        session: admin_session,
        terminal: None,
    };
    let first_idle =
        verify_idle(runtime, &admin, &mut child, &status, &audit, 0, 0, &session).await;
    assert_eq!(audit.snapshot().begin_parts, 0);
    assert_eq!(audit.snapshot().resolves, 0);
    assert_eq!(audit.snapshot().submits, 0);
    let Reply::Session(bound) = runtime
        .call(Command::OpenTerminalUserSession {
            principal: name("admin"),
            id: id(),
            ttl_ns: Counter(3_600_000_000_000),
            certificate: Digest::from_bytes([77; 32]),
        })
        .await
        .unwrap()
    else {
        panic!("simulation operator terminal")
    };
    let operator = Identity {
        principal: name("admin"),
        session: bound.id,
        terminal: Some((name("test/panel"), Digest::from_bytes([77; 32]))),
    };
    let host = Identity {
        principal: name("host/sim"),
        session: host_session,
        terminal: None,
    };
    let journal = id();
    let a = start_run(runtime, configuration, &operator, &host, &session, 1).await;
    let a = complete_two_parts(runtime, &admin, &host, &mut child, &a.id, &journal, 0).await;
    let between = verify_idle(runtime, &admin, &mut child, &status, &audit, 1, 2, &session).await;
    assert_eq!(child.child.id(), pid);
    let b = start_run(runtime, configuration, &operator, &host, &session, 6).await;
    assert_ne!(a.id, b.id);
    let b = complete_two_parts(runtime, &admin, &host, &mut child, &b.id, &journal, 1).await;
    let final_idle =
        verify_idle(runtime, &admin, &mut child, &status, &audit, 2, 4, &session).await;
    let before_stop = audit.snapshot();
    assert_eq!(
        before_stop.pauses, 0,
        "normal completion must not request PauseRun"
    );
    let p_before_stop = overview(runtime, &admin).await;
    let parts = all_parts(&p_before_stop);
    assert_eq!(parts.len(), 4);
    let executor_identity = Identity {
        principal: name("executor"),
        session: session.clone(),
        terminal: None,
    };
    let mut productions = Vec::new();
    for run in [&a, &b] {
        let Reply::ProductionView(view) = runtime
            .call(Command::ProductionView {
                identity: executor_identity.clone(),
                run: run.id.clone(),
            })
            .await
            .unwrap()
        else {
            panic!("P production proof")
        };
        rx_process_contract::production::validate(&view).unwrap();
        assert_eq!(view.run.run.state, RunState::Completed);
        assert_eq!(view.caller_session, session);
        assert!(
            view.parts
                .iter()
                .all(|p| p.disposition == PartDisposition::ConfirmedCompleted)
        );
        productions.push(*view);
    }
    std::fs::write(&stop_file, b"explicit idle shutdown").unwrap();
    wait_child(&mut child.child).await;
    assert!(
        child.child.wait().unwrap().success(),
        "resident fixture: {}",
        std::fs::read_to_string(&child.stderr).unwrap()
    );
    let after_stop = audit.snapshot();
    assert_eq!(
        after_stop.pauses, before_stop.pauses,
        "idle shutdown must not create PauseRun"
    );
    assert_eq!(
        (
            after_stop.begin_parts,
            after_stop.complete_parts,
            after_stop.resolves,
            after_stop.submits
        ),
        (
            before_stop.begin_parts,
            before_stop.complete_parts,
            before_stop.resolves,
            before_stop.submits
        )
    );
    assert_eq!(
        after_stop.executor_sessions,
        BTreeSet::from([session.clone()])
    );
    let p_after_stop = overview(runtime, &admin).await;
    assert_eq!(
        canonical::bytes(&p_before_stop.cells).unwrap(),
        canonical::bytes(&p_after_stop.cells).unwrap()
    );
    let report = read_json(&report_root.join("report.json")).expect("S completion report");
    assert_eq!(report["report"]["status"]["phase"], "STOPPED");
    assert_eq!(report["report"]["status"]["session"], session.as_str());
    assert_eq!(report["report"]["status"]["completed_runs"], "2");
    assert_eq!(report["planner_starts"], 4);
    assert!(report["current"].is_null());
    let attachments = report["attachments"].as_array().unwrap();
    assert_eq!(attachments.len(), 2);
    let expected_runs = BTreeSet::from([a.id.to_string(), b.id.to_string()]);
    let mut attached_runs = BTreeSet::new();
    for record in attachments {
        let value = &record["document"]["value"];
        assert_eq!(value["phase"], "CLOSED");
        assert_eq!(value["preparation"]["executor_session"], session.as_str());
        assert_eq!(value["completion"]["run"]["run"]["state"], "COMPLETED");
        attached_runs.insert(
            value["preparation"]["scope"]["run"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    assert_eq!(attached_runs, expected_runs);
    let stops = report["stops"].as_array().unwrap();
    assert_eq!(stops.len(), 2);
    let mut stopped_runs = BTreeSet::new();
    for value in stops {
        let stop = &value["stop"]["document"]["value"];
        assert_eq!(stop["phase"], "PAUSE_OBSERVED");
        assert_eq!(stop["reason"], "COMPLETED");
        assert!(stop["attempts"].as_array().unwrap().is_empty());
        stopped_runs.insert(value["run"].as_str().unwrap().to_owned());
    }
    assert_eq!(stopped_runs, expected_runs);
    std::fs::copy(
        report_root.join("report.json"),
        output.join("service-report.json"),
    )
    .unwrap();
    std::fs::write(output.join("p-evidence.json"), canonical::bytes(&json!({
        "schema":"rx.test.resident-cell-e2e.v1","simulation_only":true,
        "product_image_acceptance":false,"s_process_id":pid,"executor_session":session,
        "cpp_image":image,"configuration_has_run_or_visit":false,
        "idle_before_a":first_idle,"idle_between_runs":between,"idle_after_b":final_idle,
        "before_stop":before_stop,"after_stop":after_stop,"parts":parts,"production":productions,
        "planner_retirement_basis":"Both durable attachments CLOSED only after confirmed local planner cleanup and fresh exact P COMPLETED views",
    })).unwrap()).unwrap();
}
