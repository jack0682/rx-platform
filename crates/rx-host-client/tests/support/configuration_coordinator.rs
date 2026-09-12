use super::{id, name, qsupport, review_support};
use rx_application::*;
use rx_domain::{canonical, host_configuration as wire, types::*};
use rx_host_client::{HostClient, configuration_worker::Coordinator};
use rx_runtime::{
    application::{Application, ApplicationPort, Command, Handle, Reply},
    writer::Writer,
};
use rx_storage::SqliteRepository;
use std::{collections::BTreeMap, path::Path, sync::Arc};
#[derive(Clone)]
struct TestClock;
pub(super) fn shared_time() -> TimePoint {
    if std::env::var_os("RX_HOST_PRODUCT_SERVER").is_some() {
        #[cfg(target_os = "linux")]
        {
            let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
            let t = rustix::time::clock_gettime(rustix::time::ClockId::Boottime);
            return TimePoint {
                clock_id: format!("linux-boottime/{}", boot.trim()),
                ticks_ns: Counter(t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64),
            };
        }
        #[cfg(not(target_os = "linux"))]
        panic!("product Host test requires Linux");
    }
    TimePoint {
        clock_id: "simulation/boottime".into(),
        ticks_ns: Counter(1000),
    }
}
impl Clock for TestClock {
    fn now(&self) -> TimePoint {
        shared_time()
    }
}
struct NoQualification;
impl QualificationAuthority for NoQualification {
    fn verify(&self, _: &CellConfiguration, _: &[ArtifactRef], _: &[Digest]) -> bool {
        false
    }
}
type App = Engine<SqliteRepository, TestClock, NoQualification>;
fn principal(label: &str, roles: &[Role], cell: &Name) -> Principal {
    Principal {
        id: name(label),
        client_namespace: name(&format!("client/{label}")),
        roles: roles.iter().copied().collect(),
        cells: [cell.clone()].into_iter().collect(),
        active: true,
    }
}
fn login(app: &mut App, who: &str) -> Identity {
    let session = app
        .authenticated_session(&name(who), id(), {
            let now = shared_time();
            TimePoint {
                clock_id: now.clock_id,
                ticks_ns: Counter(now.ticks_ns.0 + 300_000_000_000),
            }
        })
        .unwrap();
    Identity {
        principal: name(who),
        session: session.id,
        terminal: None,
    }
}
fn target(c: &process_change::Change) -> process_change::Transition {
    process_change::Transition {
        change: c.id.clone(),
        cell: c.cell.clone(),
        expected: c.revision,
        plan_digest: c.plan_digest,
    }
}
fn validate(p: &review_support::Fixture, job: &process_review::Job) -> process_review::Validated {
    let (report, signature, bytes) = p.report(job);
    process_review::Validated::check(
        job,
        p.store.verify_owned(&p.object, &p.policy).unwrap(),
        &p.authority,
        report,
        signature,
        Some(&bytes),
    )
    .unwrap()
}
fn prepare(
    app: &mut App,
    admin: &Identity,
    reviewer: &Identity,
    release: &Identity,
    cfg: &CellConfiguration,
    p: &review_support::Fixture,
) -> (process_change::Change, process_review::Job) {
    assert!(p.package.is_dir() && p.policy_path.is_file());
    app.configure_package_intake(Some((
        p.store.owner().clone(),
        p.policy.fingerprint().unwrap(),
        rx_package::content_digest(&p.policy_bytes),
    )))
    .unwrap();
    app.configure_process_review(Some(p.authority.digest().unwrap()))
        .unwrap();
    let ctx = app.package_intake_context(admin, &cfg.id).unwrap();
    let intake = id();
    let package_intake::Preflight::Verify(t) = app
        .prepare_package_intake(
            admin,
            &id(),
            package_intake::Submit {
                id: intake.clone(),
                cell: cfg.id.clone(),
                title: "Host coordinator integration".into(),
                relative_path: rx_package::PackagePath::new("package").unwrap(),
                object: p.object.clone(),
                configuration_digest: ctx.configuration_digest,
                policy_generation: ctx.registration.as_ref().unwrap().generation.clone(),
            },
        )
        .unwrap()
    else {
        panic!("intake")
    };
    app.commit_package_intake(
        package_intake::Prepared::new(*t, p.store.verify_owned(&p.object, &p.policy).unwrap())
            .unwrap(),
    )
    .unwrap();
    let job = app
        .create_process_review(
            admin,
            &id(),
            process_review::Create {
                device_plans: vec![],
                id: id(),
                intake,
                cell: cfg.id.clone(),
                configuration_digest: ctx.configuration_digest,
                policy_generation: ctx.registration.unwrap().generation,
                binding_selections: BTreeMap::from([(name("load"), cfg.steps[0].id.clone())]),
            },
        )
        .unwrap();
    let (report, _, _) = p.report(&job);
    let process_review::Preflight::Verify(t) = app
        .prepare_review_report(
            admin,
            &id(),
            process_review::Submit {
                review: job.request.id.clone(),
                cell: cfg.id.clone(),
                expected: None,
                directory: rx_package::PackagePath::new("report").unwrap(),
                report_digest: report.digest().unwrap(),
            },
        )
        .unwrap()
    else {
        panic!("review")
    };
    let v = app
        .commit_review_report(process_review::Prepared::new(*t, validate(p, &job)).unwrap())
        .unwrap();
    let process_review::DecisionPreflight::Verify(t) = app
        .prepare_review_decision(
            reviewer,
            &id(),
            process_review::Decide {
                review: job.request.id.clone(),
                cell: cfg.id.clone(),
                report_revision: v.revision,
                review_digest: v.review_digest,
                expected: None,
                choice: process_review::Choice::Approve,
                note: "Review immutable software materials".into(),
            },
        )
        .unwrap()
    else {
        panic!("decision")
    };
    let d = app
        .commit_review_decision(
            process_review::PreparedDecision::approve(*t, validate(p, &job)).unwrap(),
        )
        .unwrap();
    let process_change::Preflight::Verify(t) = app
        .prepare_process_change(
            admin,
            &id(),
            process_change::Create {
                mode: process_change::Mode::Replace,
                id: id(),
                cell: cfg.id.clone(),
                review: process_change::ReviewRef {
                    id: job.request.id.clone(),
                    revision: v.revision,
                    review_digest: v.review_digest,
                    decision_revision: d.revision,
                },
                reason: "Integrate durable Host process context without native effects".into(),
            },
        )
        .unwrap()
    else {
        panic!("change")
    };
    let c = app
        .commit_process_change(process_change::Prepared::new(*t, validate(p, &job)).unwrap())
        .unwrap();
    let c = app
        .review_process_change_impact(
            reviewer,
            &id(),
            process_change::ReviewImpact {
                target: target(&c),
                note: "Entire affected cell remains blocked until requalification".into(),
            },
        )
        .unwrap();
    let process_change::Preflight::Verify(t) = app
        .prepare_process_change_stage(release, &id(), target(&c))
        .unwrap()
    else {
        panic!("stage")
    };
    let c = app
        .commit_process_change_stage(process_change::Prepared::new(*t, validate(p, &job)).unwrap())
        .unwrap();
    let c = app
        .begin_process_change_preparation(
            release,
            &id(),
            process_change::BeginPreparation {
                target: target(&c),
                refresh: false,
            },
        )
        .unwrap();
    (c, job)
}
pub async fn run(
    host: &HostClient,
    cfg: &CellConfiguration,
    snapshot: &wire::Snapshot,
    installation: &Id,
    directory: &Path,
    host_state: &Path,
    terminal_fingerprint: Digest,
) {
    let database = directory.join("platform.db");
    let admin_p = principal(
        "engineer",
        &[Role::AccountAdmin, Role::Engineer, Role::Observer],
        &cfg.id,
    );
    let mut app = Engine::open(
        SqliteRepository::open(&database).unwrap(),
        TestClock,
        NoQualification,
        installation.clone(),
        admin_p.clone(),
    )
    .unwrap();
    let admin = login(&mut app, "engineer");
    for (label, roles) in [
        ("reviewer", vec![Role::Verifier]),
        ("release", vec![Role::ReleaseManager]),
        ("host/sim", vec![Role::Host]),
        ("operator", vec![Role::Operator]),
        ("executor", vec![Role::Executor, Role::Observer]),
    ] {
        app.put_principal(&admin, principal(label, &roles, &cfg.id), None)
            .unwrap();
    }
    let reviewer = login(&mut app, "reviewer");
    let host_id = login(&mut app, "host/sim");
    app.put_terminal(
        &admin,
        Terminal {
            id: name("panel/test"),
            certificate_digest: terminal_fingerprint,
            cells: [cfg.id.clone()].into_iter().collect(),
            active: true,
        },
        None,
    )
    .unwrap();
    let session = app
        .authenticated_terminal_user_session(
            &name("release"),
            id(),
            Counter(300_000_000_000),
            terminal_fingerprint,
        )
        .unwrap();
    let release = Identity {
        principal: name("release"),
        session: session.id,
        terminal: Some((name("panel/test"), terminal_fingerprint)),
    };
    let executor = login(&mut app, "executor");
    let operator_session = app
        .authenticated_terminal_user_session(
            &name("operator"),
            id(),
            Counter(300_000_000_000),
            terminal_fingerprint,
        )
        .unwrap();
    let operator = Identity {
        principal: name("operator"),
        session: operator_session.id,
        terminal: Some((name("panel/test"), terminal_fingerprint)),
    };
    app.install_cell(&admin, cfg.clone()).unwrap();
    let cell = app.inspect_cell(&admin, &cfg.id).unwrap().1;
    let initial_read = host
        .read_bootstrap(
            &cfg.id,
            cfg.fact_specs.iter().map(|s| s.id.clone()).collect(),
        )
        .await
        .unwrap();
    let (grant, _) = host
        .acquire_grant(
            &id(),
            cfg.steps
                .iter()
                .flat_map(|s| s.intent.resource_set.iter().cloned())
                .collect(),
            Counter(1),
            Counter(30000),
            TestClock.now(),
        )
        .await
        .unwrap();
    let reg = HostRegistration {
        id: host_id.principal.clone(),
        session: host_id.session.clone(),
        boot_id: snapshot.host_boot.clone(),
        delivery_journal: snapshot.delivery_journal.clone(),
        cell: cfg.id.clone(),
        epoch: cell.epoch,
        scopes: cell.scope_epochs.clone(),
        source_sessions: initial_read
            .observations
            .iter()
            .map(|o| (o.source.clone(), o.generation.clone()))
            .collect(),
        grant,
    };
    app.register_host(&host_id, reg.clone()).unwrap();
    let package = review_support::fixture(cfg, Digest::from_bytes([71; 32]));
    let (change, job) = prepare(&mut app, &admin, &reviewer, &release, cfg, &package);
    for fence in &change.preparation.as_ref().unwrap().fences {
        app.plan_delivery(&host_id, &fence.message).unwrap();
        let bound = change
            .preparation
            .as_ref()
            .unwrap()
            .cells
            .iter()
            .find(|c| c.cell == fence.cell)
            .unwrap();
        let receipt = host
            .fence(
                &fence.message,
                &fence.cell,
                fence.epoch,
                &fence.scopes,
                &bound.change_blocks,
            )
            .await
            .unwrap();
        app.finish_fence_delivery(
            &host_id,
            &fence.message,
            FenceAcknowledgment {
                cell: fence.cell.clone(),
                invalidation: fence.message.clone(),
                epoch: fence.epoch,
                scopes: fence.scopes.clone(),
                host_boot: Id::new(receipt.host_boot_id).unwrap(),
                journal: Id::new(receipt.delivery_journal_id).unwrap(),
                sequence: Counter(receipt.seq),
            },
        )
        .unwrap();
    }
    let batch = app
        .authorize_host_configuration(&release, &id(), target(&change))
        .unwrap();
    let runtime = Arc::new(Handle::new(
        Writer::start(move || Ok(Application::new(app)))
            .await
            .unwrap(),
    ));
    let mut coordinator = if std::env::var_os("RX_HOST_PRODUCT_SERVER").is_some() {
        Coordinator::with_transport(
            runtime.clone(),
            Arc::new(LossOnce::new(host.clone())),
            host_id.clone(),
        )
        .unwrap()
    } else {
        Coordinator::new(runtime.clone(), host.clone(), host_id.clone()).unwrap()
    };
    coordinator.step().await.unwrap();
    let Reply::HostConfigurationTasks(tasks) = runtime
        .request(Command::HostConfigurationTasks {
            identity: host_id.clone(),
            after: None,
        })
        .await
        .unwrap()
    else {
        panic!("tasks")
    };
    let pending = &tasks[0];
    assert_eq!(pending.id, batch.tasks[0]);
    assert_eq!(pending.phase, configuration_dispatch::Phase::SendEntered);
    assert!(pending.receipt.is_none());
    assert_eq!(
        pending.issue,
        Some(configuration_dispatch::Issue::TransportUnavailable)
    );
    let request = pending.request.clone().unwrap();
    // Recover with a newly created worker, so no sender in-memory response can supply the result.
    drop(coordinator);
    let mut coordinator = Coordinator::new(runtime.clone(), host.clone(), host_id.clone()).unwrap();
    coordinator.step().await.unwrap();
    let Reply::HostConfigurationTasks(tasks) = runtime
        .request(Command::HostConfigurationTasks {
            identity: host_id.clone(),
            after: None,
        })
        .await
        .unwrap()
    else {
        panic!("tasks")
    };
    let recovered = &tasks[0];
    assert_eq!(recovered.request_digest, Some(request.digest().unwrap()));
    assert_eq!(
        recovered.receipt.as_ref().unwrap().status,
        wire::Status::AppliedUnqualified
    );
    let Reply::ProcessChangeDetail(detail) = runtime
        .request(Command::GetProcessChange {
            identity: admin.clone(),
            cell: cfg.id.clone(),
            id: change.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("detail")
    };
    assert!(detail.host_configuration.all_hosts_acknowledged);
    assert!(!detail.applied && !detail.activation_authorized);
    let key = id();
    let Reply::ProcessChangePreflight(process_change::Preflight::Verify(ticket)) = runtime
        .request(Command::PrepareProcessChangeApply {
            identity: release.clone(),
            key: key.clone(),
            input: target(&change),
        })
        .await
        .unwrap()
    else {
        panic!("apply ticket")
    };
    let prepared = process_change::Prepared::new(*ticket, validate(&package, &job)).unwrap();
    let Reply::ProcessChange(applied) = runtime
        .request(Command::CommitProcessChangeApply(Box::new(prepared)))
        .await
        .unwrap()
    else {
        panic!("applied")
    };
    assert_eq!(applied.state, process_change::State::AppliedUnqualified);
    let Reply::ProcessChangePreflight(process_change::Preflight::Recorded(repeated)) = runtime
        .request(Command::PrepareProcessChangeApply {
            identity: release.clone(),
            key,
            input: target(&change),
        })
        .await
        .unwrap()
    else {
        panic!("same apply result")
    };
    assert_eq!(applied.revision, repeated.revision);
    let Reply::Cell(current_revision, current) = runtime
        .request(Command::InspectCell {
            identity: admin.clone(),
            cell: cfg.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("cell")
    };
    assert_eq!(
        current.configuration.process.as_ref().unwrap().root.id,
        package.resolved.root.id
    );
    assert_ne!(current.configuration.recipe, cfg.recipe);
    assert!(current.qualification.is_none());
    for fence in &applied.application.as_ref().unwrap().fences {
        runtime
            .request(Command::PlanDelivery {
                identity: host_id.clone(),
                message: fence.message.clone(),
            })
            .await
            .unwrap();
        let receipt = host
            .fence(
                &fence.message,
                &fence.cell,
                fence.epoch,
                &fence.scopes,
                &current
                    .blocks
                    .iter()
                    .map(|b| b.id.clone())
                    .collect::<Vec<_>>(),
            )
            .await
            .unwrap();
        runtime
            .request(Command::FinishFence {
                identity: host_id.clone(),
                message: fence.message.clone(),
                ack: FenceAcknowledgment {
                    cell: fence.cell.clone(),
                    invalidation: fence.message.clone(),
                    epoch: fence.epoch,
                    scopes: fence.scopes.clone(),
                    host_boot: Id::new(receipt.host_boot_id).unwrap(),
                    journal: Id::new(receipt.delivery_journal_id).unwrap(),
                    sequence: Counter(receipt.seq),
                },
            })
            .await
            .unwrap();
    }
    let proof = qsupport::policy(&current.configuration, qsupport::configure(cfg.clone()).1);
    let policy_path = directory.join("qualification-policy.json");
    let policy_bytes = canonical::bytes(&proof.policy).unwrap();
    std::fs::write(&policy_path, &policy_bytes).unwrap();
    let worker = rx_runtime::requalification::Worker::new(
        directory.to_owned(),
        policy_path.clone(),
        rx_package::content_digest(&policy_bytes),
    )
    .unwrap();
    runtime
        .request(Command::ConfigureRequalification(Some(worker.policy())))
        .await
        .unwrap();
    let Reply::RequalificationJob(jobq) = runtime
        .request(Command::BeginRequalification {
            identity: release.clone(),
            key: id(),
            input: requalification::Begin {
                runtime_restrictions: BTreeMap::new(),
                id: id(),
                change: applied.id.clone(),
                cell: cfg.id.clone(),
                expected_change: applied.revision,
                expected_cells: BTreeMap::from([(cfg.id.clone(), current_revision)]),
                policy_digest: proof.policy.digest().unwrap(),
            },
        })
        .await
        .unwrap()
    else {
        panic!("qualification job")
    };
    let Reply::Cell(_, cellq) = runtime
        .request(Command::InspectCell {
            identity: admin.clone(),
            cell: cfg.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("cell")
    };
    for f in &jobq.request.fences {
        runtime
            .request(Command::PlanDelivery {
                identity: host_id.clone(),
                message: f.message.clone(),
            })
            .await
            .unwrap();
        let r = host
            .fence(
                &f.message,
                &f.cell,
                f.epoch,
                &f.scopes,
                &cellq
                    .blocks
                    .iter()
                    .map(|b| b.id.clone())
                    .collect::<Vec<_>>(),
            )
            .await
            .unwrap();
        runtime
            .request(Command::FinishFence {
                identity: host_id.clone(),
                message: f.message.clone(),
                ack: FenceAcknowledgment {
                    cell: f.cell.clone(),
                    invalidation: f.message.clone(),
                    epoch: f.epoch,
                    scopes: f.scopes.clone(),
                    host_boot: Id::new(r.host_boot_id).unwrap(),
                    journal: Id::new(r.delivery_journal_id).unwrap(),
                    sequence: Counter(r.seq),
                },
            })
            .await
            .unwrap();
    }
    let report = proof.write(&directory.join("qualification"), &jobq);
    let input = requalification::Submit {
        review: jobq.request.id.clone(),
        cell: cfg.id.clone(),
        expected: None,
        directory: rx_package::PackagePath::new("qualification").unwrap(),
        report_digest: report.digest().unwrap(),
    };
    let key = id();
    let Reply::RequalificationPreflight(requalification::Preflight::Verify(t)) = runtime
        .request(Command::PrepareRequalificationReport {
            identity: admin.clone(),
            key: key.clone(),
            input: input.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("report ticket")
    };
    let artifact = &report.checks[0].evidence[0];
    let artifact_path = directory.join(format!("qualification/artifacts/{}.bin", artifact.sha256));
    let original = std::fs::read(&artifact_path).unwrap();
    std::fs::write(&artifact_path, b"corrupted").unwrap();
    assert!(worker.prepare(*t).await.is_err());
    std::fs::write(&artifact_path, &original).unwrap();
    let Reply::RequalificationPreflight(requalification::Preflight::Verify(t)) = runtime
        .request(Command::PrepareRequalificationReport {
            identity: admin.clone(),
            key,
            input,
        })
        .await
        .unwrap()
    else {
        panic!("retry ticket")
    };
    let p = worker.prepare(*t).await.unwrap();
    let Reply::RequalificationVersion(version) = runtime
        .request(Command::CommitRequalificationReport(Box::new(p)))
        .await
        .unwrap()
    else {
        panic!("version")
    };
    let decide = requalification::Decide {
        review: jobq.request.id.clone(),
        cell: cfg.id.clone(),
        report_revision: version.revision,
        report_digest: version.digest,
        expected: None,
        choice: requalification::Choice::Approve,
        note: "Independent test review of complete simulation evidence; no field activation".into(),
    };
    let Reply::RequalificationDecisionPreflight(requalification::DecisionPreflight::Verify(t)) =
        runtime
            .request(Command::PrepareRequalificationDecision {
                identity: reviewer.clone(),
                key: id(),
                input: decide.clone(),
            })
            .await
            .unwrap()
    else {
        panic!("review ticket")
    };
    std::fs::write(&policy_path, b"{}").unwrap();
    assert!(worker.decide(*t).await.is_err());
    std::fs::write(&policy_path, &policy_bytes).unwrap();
    let Reply::RequalificationDecisionPreflight(requalification::DecisionPreflight::Verify(t)) =
        runtime
            .request(Command::PrepareRequalificationDecision {
                identity: reviewer.clone(),
                key: id(),
                input: decide,
            })
            .await
            .unwrap()
    else {
        panic!("fresh review ticket")
    };
    let p = worker.decide(*t).await.unwrap();
    runtime
        .request(Command::CommitRequalificationDecision(Box::new(p)))
        .await
        .unwrap();
    let Reply::RequalificationDetail(qdetail) = runtime
        .request(Command::GetRequalification {
            identity: admin.clone(),
            cell: cfg.id.clone(),
            id: jobq.request.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("review detail")
    };
    assert!(qdetail.approval_current && !qdetail.activation_authorized);
    let package_worker = rx_runtime::package_intake::Worker::new(
        directory.to_owned(),
        package.store,
        package.policy.clone(),
        rx_package::content_digest(&package.policy_bytes),
    )
    .unwrap();
    let package_worker = rx_runtime::package_intake::Worker::with_requalification(
        package_worker,
        policy_path.clone(),
        rx_package::content_digest(&policy_bytes),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("https://{}", listener.local_addr().unwrap());
    let password = "test-qualification-http-only";
    let hash = rx_api::auth::password_hash(password).unwrap();
    let credentials = rx_api::auth::Credentials {
        schema: "rx.local-credentials.v1".into(),
        accounts: ["engineer", "reviewer", "release"]
            .into_iter()
            .map(|v| rx_api::auth::LocalAccount {
                principal: name(v),
                password_hash: hash.clone(),
            })
            .collect(),
    };
    let service = rx_api::terminal_https::TerminalHttps::new_with_package_intake(
        runtime.clone(),
        credentials,
        rx_api::terminal_https::HttpsPolicy::new(&origin).unwrap(),
        rx_api::terminal_https::TlsMaterial {
            server_certificate_pem: std::fs::read(directory.join("server.pem")).unwrap(),
            server_key_pem: std::fs::read(directory.join("server.key")).unwrap(),
            terminal_ca_pem: std::fs::read(directory.join("ca.pem")).unwrap(),
        },
        Some(package_worker.clone()),
    )
    .unwrap();
    let (stop_http, http_stopped) = tokio::sync::oneshot::channel();
    let http_server = tokio::spawn(async move {
        service
            .serve(listener, async {
                let _ = http_stopped.await;
            })
            .await
            .unwrap()
    });
    let identity = [
        std::fs::read(directory.join("terminal.pem")).unwrap(),
        std::fs::read(directory.join("terminal.key")).unwrap(),
    ]
    .concat();
    let http = reqwest::Client::builder()
        .tls_built_in_root_certs(false)
        .add_root_certificate(
            reqwest::Certificate::from_pem(&std::fs::read(directory.join("ca.pem")).unwrap())
                .unwrap(),
        )
        .identity(reqwest::Identity::from_pem(&identity).unwrap())
        .build()
        .unwrap();
    let mut cookies = Vec::new();
    for principal in ["engineer", "reviewer", "release"] {
        let response = http
            .post(format!("{origin}/api/v1/session"))
            .header("origin", &origin)
            .header("x-rx-client", "browser-v1")
            .json(&serde_json::json!({"principal":principal,"password":password}))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        cookies.push(
            response.headers()[reqwest::header::SET_COOKIE]
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_owned(),
        );
    }
    let post = |path: &str, who: usize| {
        http.post(format!("{origin}{path}"))
            .header("origin", &origin)
            .header("x-rx-client", "browser-v1")
            .header(reqwest::header::COOKIE, &cookies[who])
    };
    let response=post("/api/v1/qualification-review/reports",0).json(&serde_json::json!({"request_key":id(),"command":{"review":jobq.request.id,"cell":cfg.id,"expected":version.revision,"directory":"qualification","report_digest":report.digest().unwrap()}})).send().await.unwrap();
    assert!(response.status().is_success());
    let v: requalification::Version = response.json().await.unwrap();
    assert_eq!(v.revision.0, version.revision.0 + 1);
    let response=post("/api/v1/qualification-review/decisions",1).json(&serde_json::json!({"request_key":id(),"command":{"review":jobq.request.id,"cell":cfg.id,"report_revision":v.revision,"report_digest":v.digest,"expected":qdetail.decision.as_ref().unwrap().revision,"choice":"APPROVE","note":"Independent HTTP review of exact signed simulation evidence"}})).send().await.unwrap();
    assert!(response.status().is_success());
    let response = post("/api/v1/qualification-review/artifact", 0)
        .json(&serde_json::json!({"review":jobq.request.id,"cell":cfg.id,"reference":artifact}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    assert_eq!(
        response.bytes().await.unwrap().as_ref(),
        original.as_slice()
    );
    let response = http
        .get(format!("{origin}/api/v1/qualification-review"))
        .query(&[
            ("cell", cfg.id.to_string()),
            ("id", jobq.request.id.to_string()),
        ])
        .header("origin", &origin)
        .header("x-rx-client", "browser-v1")
        .header(reqwest::header::COOKIE, &cookies[0])
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let http_detail: serde_json::Value = response.json().await.unwrap();
    assert_eq!(http_detail["approval_current"], true);
    assert_eq!(http_detail["activation_authorized"], false);
    let response=post("/api/v1/qualification-reviews",0).json(&serde_json::json!({"request_key":id(),"command":{"id":id(),"change":applied.id,"cell":cfg.id,"expected_change":applied.revision,"expected_cells":{cfg.id.to_string():jobq.request.cells[0].expected_revision},"policy_digest":proof.policy.digest().unwrap()}})).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    assert!(
        host.arm(&id(), &id(), &cfg.id, cellq.epoch, &cellq.scope_epochs)
            .await
            .is_err()
    );
    let approved: requalification::Detail = serde_json::from_value(http_detail).unwrap();
    let reviewed = approved.version.as_ref().unwrap();
    let decision = approved.decision.as_ref().unwrap();
    let Reply::Cell(cell_revision, cell_now) = runtime
        .request(Command::InspectCell {
            identity: admin.clone(),
            cell: cfg.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("cell")
    };
    let input = qualification_activation::IssueRequest {
        review: jobq.request.id.clone(),
        cell: cfg.id.clone(),
        report_revision: reviewed.revision,
        report_digest: reviewed.digest,
        decision_revision: decision.revision,
        expected_cells: BTreeMap::from([(cfg.id.clone(), cell_revision)]),
        clear_blocks: BTreeMap::from([(
            cfg.id.clone(),
            cell_now.blocks.iter().map(|b| b.id.clone()).collect(),
        )]),
    };
    let issue_key = id();
    let response = post("/api/v1/qualification-activations", 2)
        .json(&serde_json::json!({"request_key":issue_key,"command":input}))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "issue HTTP: {}",
        response.text().await.unwrap_or_default()
    );
    let issued: Box<qualification_activation::Batch> = response.json().await.unwrap();
    let mut qualifier = if std::env::var_os("RX_HOST_PRODUCT_SERVER").is_some() {
        rx_host_client::qualification_worker::Coordinator::with_transport(
            runtime.clone(),
            Arc::new(LossOnce::new(host.clone())),
            host_id.clone(),
        )
        .unwrap()
    } else {
        rx_host_client::qualification_worker::Coordinator::new(
            runtime.clone(),
            host.clone(),
            host_id.clone(),
        )
        .unwrap()
    };
    qualifier.step().await.unwrap();
    let Reply::QualificationView(waiting) = runtime
        .request(Command::GetQualificationBatch {
            identity: admin.clone(),
            cell: cfg.id.clone(),
            id: issued.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("waiting")
    };
    assert!(waiting.outcome_unknown);
    assert_eq!(
        waiting.batch.state,
        qualification_activation::State::Pending
    );
    let qualification_request = waiting.hosts[0].request.clone().unwrap();
    drop(qualifier);
    let mut qualifier = rx_host_client::qualification_worker::Coordinator::new(
        runtime.clone(),
        host.clone(),
        host_id.clone(),
    )
    .unwrap();
    qualifier.step().await.unwrap();
    let Reply::QualificationView(ready) = runtime
        .request(Command::GetQualificationBatch {
            identity: admin.clone(),
            cell: cfg.id.clone(),
            id: issued.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("ready")
    };
    assert_eq!(ready.accepted_hosts, 1);
    assert!(!ready.outcome_unknown);
    let qualified = ready.hosts[0].observation.clone().unwrap();
    let final_input = qualification_activation::Finalize {
        batch: issued.id.clone(),
        cell: cfg.id.clone(),
        expected: issued.revision,
        expected_cells: input.expected_cells,
    };
    let final_key = id();
    let response = post("/api/v1/qualification-activation/activate", 2)
        .json(&serde_json::json!({"request_key":final_key,"command":final_input}))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "activate HTTP: {}",
        response.text().await.unwrap_or_default()
    );
    let activated: Box<qualification_activation::Batch> = response.json().await.unwrap();
    assert_eq!(activated.state, qualification_activation::State::Active);
    let response = post("/api/v1/qualification-activation/activate", 2)
        .json(&serde_json::json!({"request_key":final_key,"command":final_input}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let repeated: qualification_activation::Batch = response.json().await.unwrap();
    assert_eq!(repeated.revision, activated.revision);
    let Reply::Cell(cell_revision, cell_active) = runtime
        .request(Command::InspectCell {
            identity: admin.clone(),
            cell: cfg.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("active cell")
    };
    assert_eq!(
        cell_active.qualification.as_ref().unwrap().id,
        issued.cells[0].qualification.id
    );
    assert!(cell_active.blocks.is_empty());
    assert!(!host_state.join("device/effects.jsonl").exists());
    let read = host
        .read_bootstrap(
            &cfg.id,
            cfg.fact_specs.iter().map(|s| s.id.clone()).collect(),
        )
        .await
        .unwrap();
    for o in read.observations {
        let spec = cfg.fact_specs.iter().find(|s| s.id == o.source).unwrap();
        runtime
            .request(Command::ReportFact {
                identity: host_id.clone(),
                fact: FactRecord {
                    cell: cfg.id.clone(),
                    id: o.source,
                    source_host: host_id.principal.clone(),
                    source_generation: o.generation,
                    schema: o.schema,
                    unit: o.unit,
                    acquired_at: o.acquired_at,
                    maximum_age_ns: spec.maximum_age_ns,
                    acquisition_uncertainty_ns: o.uncertainty_ns,
                    quality_good: o.quality_good,
                    origin_age_bounded: o.origin_age_bounded,
                    disputed: o.disputed,
                    value: o.value,
                    evidence_id: o.evidence_id,
                },
            })
            .await
            .unwrap();
    }
    let Reply::Run(run) = runtime
        .request(Command::CreateRun {
            identity: operator.clone(),
            key: id(),
            command: CreateRun {
                cell: cfg.id.clone(),
                expected_cell: cell_revision,
                recipe_digest: cell_active.configuration.recipe.sha256,
                site_config_digest: cell_active.configuration.site_config_digest,
            },
        })
        .await
        .unwrap()
    else {
        panic!("run")
    };
    runtime
        .request(Command::StartRun {
            identity: operator.clone(),
            key: id(),
            command: StartRun {
                run: run.id.clone(),
                expected_cell: cell_revision,
                expected_run: Counter(1),
                envelope_digest: cfg.envelope.sha256,
                purpose: Purpose::Production,
                budget_unit: rx_domain::budget::BudgetUnit::PartAttempt,
                budget_limit: Counter(1),
            },
        })
        .await
        .unwrap();
    let mut dispatcher =
        rx_host_client::delivery::Dispatcher::new(runtime.clone(), host.clone(), host_id.clone())
            .unwrap();
    dispatcher.tick().await.unwrap();
    let Reply::VersionedRun(_, running) = runtime
        .request(Command::InspectRun {
            identity: operator.clone(),
            run: run.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("running")
    };
    assert_eq!(running.state, RunState::Executing);
    let Reply::Part(part) = runtime
        .request(Command::BeginPart {
            identity: executor.clone(),
            key: id(),
            run: run.id.clone(),
            expected_budget: running.budget.as_ref().unwrap().revision(),
        })
        .await
        .unwrap()
    else {
        panic!("part")
    };
    let Reply::VersionedRun(revision, _) = runtime
        .request(Command::InspectRun {
            identity: executor.clone(),
            run: run.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("revision")
    };
    let step = &cell_active.configuration.steps[0];
    let Reply::Activation(activation) = runtime
        .request(Command::ResolveActivation {
            identity: executor.clone(),
            run: run.id.clone(),
            node: step.id.clone(),
            visit: part.ordinal,
            expected_run: revision,
        })
        .await
        .unwrap()
    else {
        panic!("activation")
    };
    let Reply::Cell(current_cell_revision, _) = runtime
        .request(Command::InspectCell {
            identity: admin.clone(),
            cell: cfg.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("revision")
    };
    let Reply::VersionedRun(submit_revision, _) = runtime
        .request(Command::InspectRun {
            identity: executor.clone(),
            run: run.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("submit revision")
    };
    let Reply::Work(work) = runtime
        .request(Command::SubmitWork {
            identity: executor.clone(),
            key: id(),
            command: Box::new(SubmitWork {
                run: run.id.clone(),
                activation: activation.id,
                part: Some(part.id.clone()),
                slot: name("main"),
                intent: step.intent.clone(),
                expected_cell: current_cell_revision,
                expected_run: submit_revision,
            }),
        })
        .await
        .unwrap()
    else {
        panic!("work")
    };
    for _ in 0..4 {
        dispatcher.tick().await.unwrap();
    }
    let Reply::Work(done) = runtime
        .request(Command::InspectWork {
            identity: operator.clone(),
            operation: work.operation.id().clone(),
        })
        .await
        .unwrap()
    else {
        panic!("done")
    };
    assert_eq!(
        done.operation.outcome(),
        rx_domain::operation::Outcome::Succeeded
    );
    assert_eq!(
        done.operation.disposition(),
        rx_domain::operation::Disposition::Held
    );
    let observations = host.handover(done.operation.id()).await.unwrap();
    let Reply::Cell(revision, _) = runtime
        .request(Command::InspectCell {
            identity: admin.clone(),
            cell: cfg.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("handover revision")
    };
    let Reply::Work(released) = runtime
        .request(Command::ReleaseResources {
            identity: host_id.clone(),
            key: id(),
            command: ReleaseResources {
                operation: done.operation.id().clone(),
                expected_operation: done.operation.revision(),
                expected_cell: revision,
                observations,
            },
        })
        .await
        .unwrap()
    else {
        panic!("released")
    };
    assert_eq!(
        released.operation.disposition(),
        rx_domain::operation::Disposition::Released
    );
    let Reply::VersionedRun(revision, _) = runtime
        .request(Command::InspectRun {
            identity: executor.clone(),
            run: run.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("complete revision")
    };
    runtime
        .request(Command::CompletePart {
            identity: executor.clone(),
            key: id(),
            part: part.id.clone(),
            expected_run: revision,
        })
        .await
        .unwrap();
    let Reply::VersionedRun(_, finished) = runtime
        .request(Command::InspectRun {
            identity: executor.clone(),
            run: run.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("finished")
    };
    assert_eq!(finished.state, RunState::Completed);
    assert_eq!(
        std::fs::read_to_string(host_state.join("device/effects.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    qualifier.step().await.unwrap();
    drop(qualifier);
    drop(dispatcher);
    drop(http);
    let _ = stop_http.send(());
    http_server.await.unwrap();
    drop(coordinator);
    runtime.close();
    runtime.closed().await;
    drop(runtime);
    let mut restarted = Engine::open(
        SqliteRepository::open(&database).unwrap(),
        TestClock,
        NoQualification,
        installation.clone(),
        admin_p,
    )
    .unwrap();
    let admin = login(&mut restarted, "engineer");
    let current_host = login(&mut restarted, "host/sim");
    let tasks = restarted
        .host_configuration_tasks(&current_host, None)
        .unwrap();
    assert!(tasks.is_empty());
    assert!(
        restarted
            .enter_host_configuration_send(&current_host, &request.id, true)
            .is_err()
    );
    let qafter = restarted
        .requalification(&admin, &cfg.id, &jobq.request.id)
        .unwrap();
    assert!(!qafter.context_current && !qafter.approval_current && !qafter.activation_authorized);
    assert!(qafter.decision.is_some());
    let after = restarted
        .process_change(&admin, &cfg.id, &change.id)
        .unwrap();
    assert!(
        !after.host_configuration.all_hosts_acknowledged
            && after.applied
            && !after.activation_authorized
    );
    assert_eq!(
        after.change.application.as_ref().unwrap().host_proofs[0].request_digest,
        request.digest().unwrap()
    );
    if let Ok(out) = std::env::var("RX_HOST_CONFIGURATION_EVIDENCE") {
        let out = std::path::PathBuf::from(format!("{out}-coordinator"));
        std::fs::create_dir(&out).unwrap();
        for (file, value) in [
            (
                "p-qualification-active.json",
                serde_json::to_value(&activated).unwrap(),
            ),
            (
                "completed-run.json",
                serde_json::to_value(&finished).unwrap(),
            ),
            ("request.json", serde_json::to_value(&request).unwrap()),
            (
                "qualification-request.json",
                serde_json::to_value(&qualification_request).unwrap(),
            ),
            (
                "qualification-accepted.json",
                serde_json::to_value(&qualified).unwrap(),
            ),
            ("recovered.json", serde_json::to_value(recovered).unwrap()),
            (
                "qualification-review.json",
                serde_json::to_value(qdetail).unwrap(),
            ),
            (
                "qualification-after-restart.json",
                serde_json::to_value(qafter).unwrap(),
            ),
            (
                "restarted-change.json",
                serde_json::to_value(after).unwrap(),
            ),
        ] {
            std::fs::write(out.join(file), canonical::bytes(&value).unwrap()).unwrap();
        }
        let mut result:serde_json::Value=serde_json::from_slice(b"{\"status\":\"PASS\",\"actual_platform_writer\":true,\"real_mtls_host\":true,\"reply_lost_after_commit\":true,\"worker_recreated\":true,\"platform_restart_preserves_fact\":true,\"native_effects\":1,\"platform_applied\":true,\"qualification_was_activated\":true,\"requalification_review_approved\":true,\"corrupt_artifact_rejected\":true,\"changed_policy_rejected\":true,\"http_report_review_artifact_read\":true,\"wrong_role_https_begin_rejected\":true,\"host_qualification_reply_lost_and_recovered\":true,\"host_qualification_accepted\":true,\"production_p_qualification_coordinator\":true,\"terminal_https_issue_and_activate\":true}").unwrap();
        let product = std::env::var_os("RX_HOST_PRODUCT_SERVER").is_some();
        result["host_runtime"] =
            serde_json::json!(if product { "rx-hostd" } else { "test-harness" });
        result["real_shared_clock"] = serde_json::json!(product);
        result["server_fault_injection"] = serde_json::json!(!product);
        result["fault_injection_location"] = serde_json::json!(if product {
            "test client after actual RPC response"
        } else {
            "test server after commit"
        });
        std::fs::write(out.join("result.json"), canonical::bytes(&result).unwrap()).unwrap();
    }
}

struct LossOnce {
    client: HostClient,
    host: Name,
    used: std::sync::atomic::AtomicBool,
}
impl LossOnce {
    fn new(client: HostClient) -> Self {
        Self {
            client,
            host: name("host/sim"),
            used: std::sync::atomic::AtomicBool::new(false),
        }
    }
}
#[tonic::async_trait]
impl rx_host_client::configuration_worker::Transport for LossOnce {
    fn host(&self) -> &Name {
        &self.host
    }
    async fn inspect(&self) -> Result<rx_domain::host_configuration::Observation, tonic::Status> {
        self.client.inspect_process_configuration().await
    }
    async fn open(
        &self,
        cells: &[configuration_dispatch::CellProjection],
        clock: &str,
    ) -> Result<(), tonic::Status> {
        for c in cells {
            self.client
                .open_configuration_cell(&c.cell, c.definition, clock)
                .await?;
        }
        Ok(())
    }
    async fn lookup(
        &self,
        id: &Id,
    ) -> Result<rx_domain::host_configuration::Observation, tonic::Status> {
        self.client.lookup_process_configuration(id).await
    }
    async fn apply(
        &self,
        r: &rx_domain::host_configuration::Request,
    ) -> Result<rx_domain::host_configuration::Observation, tonic::Status> {
        let value = self.client.accept_process_configuration(r).await?;
        if !self.used.swap(true, std::sync::atomic::Ordering::SeqCst) {
            Err(tonic::Status::unavailable(
                "test client lost processed reply",
            ))
        } else {
            Ok(value)
        }
    }
}
#[tonic::async_trait]
impl rx_host_client::qualification_worker::Transport for LossOnce {
    fn host(&self) -> &Name {
        &self.host
    }
    async fn inspect(&self) -> Result<rx_domain::host_qualification::Observation, tonic::Status> {
        self.client.inspect_qualification().await
    }
    async fn open(
        &self,
        cells: &[rx_domain::host_qualification::CellTarget],
        clock: &str,
    ) -> Result<(), tonic::Status> {
        for c in cells {
            self.client
                .open_configuration_cell(&c.cell, c.definition, clock)
                .await?;
        }
        Ok(())
    }
    async fn lookup(
        &self,
        id: &Id,
    ) -> Result<rx_domain::host_qualification::Observation, tonic::Status> {
        self.client.lookup_qualification(id).await
    }
    async fn apply(
        &self,
        r: &rx_domain::host_qualification::Request,
    ) -> Result<rx_domain::host_qualification::Observation, tonic::Status> {
        let value = self.client.accept_qualification(r).await?;
        if !self.used.swap(true, std::sync::atomic::Ordering::SeqCst) {
            Err(tonic::Status::unavailable(
                "test client lost processed reply",
            ))
        } else {
            Ok(value)
        }
    }
}
