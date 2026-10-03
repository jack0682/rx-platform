use super::*;
use rx_api::grpc::{Configuration, PlatformIngress, TlsMaterial};
use rx_application::{definition_catalog as defs, execution_inventory as inv};
use rx_domain::{
    canonical,
    definition::{Body as DefinitionBody, Reference},
};
use rx_runtime::{
    application::{Application, ApplicationPort, Command, Handle, Reply},
    writer::Writer,
};
use std::path::{Path, PathBuf};
struct LossPort {
    inner: Arc<dyn ApplicationPort>,
    mode: String,
    dropped: std::sync::atomic::AtomicBool,
}
impl ApplicationPort for LossPort {
    fn request(&self, command: Command) -> rx_runtime::application::CallFuture<'_> {
        let selected = match self.mode.as_str() {
            "begin" => matches!(&command, Command::CommitExecutionPart(_)),
            "submit" => matches!(&command, Command::CommitExecutionOperation(_)),
            "complete" => matches!(&command, Command::ExecutorCompletePart { .. }),
            _ => false,
        };
        Box::pin(async move {
            let reply = self.inner.request(command).await?;
            if selected && !self.dropped.swap(true, Ordering::SeqCst) {
                Err(rx_runtime::writer::WriterError::Unavailable)
            } else {
                Ok(reply)
            }
        })
    }
    fn status(&self) -> rx_runtime::writer::Status {
        self.inner.status()
    }
}
struct ChildGuard(std::process::Child);
impl std::ops::Deref for ChildGuard {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
async fn call(runtime: &dyn ApplicationPort, c: Command) -> Reply {
    let label = match &c {
        Command::StartExecutionRun { .. } => "start2",
        Command::PlanDelivery { .. } => "plan-delivery",
        Command::FinishArm { .. } => "finish-arm",
        Command::GetExecutionPart { .. } => "get-part2",
        Command::RecordReceipt { .. } => "receipt",
        Command::ReleaseResources { .. } => "release",
        Command::BindExecutionObject { .. } => "bind-object",
        _ => "read/evidence",
    };
    runtime
        .request(c)
        .await
        .unwrap_or_else(|e| panic!("{label}: {e:?}"))
}
fn actual(f: &mut Fixture, model: &Reference, label: &str) -> Reference {
    f.app
        .save_definition(
            &f.admin,
            &id(),
            defs::Prepared::prepare(defs::Save {
                catalog: model.catalog.clone(),
                id: id(),
                expected: None,
                label: label.into(),
                body: DefinitionBody::ObjectInstance {
                    base: model.clone(),
                    values: BTreeMap::new(),
                },
                archived: false,
            })
            .unwrap(),
        )
        .unwrap()
        .version
        .definition
        .reference
}
async fn wait_marker(p: &Path, child: &mut std::process::Child) {
    for _ in 0..500 {
        if p.exists() {
            return;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "S fixture exited before ready"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("S fixture ready timeout");
}
pub async fn run() {
    use rcgen::*;
    use sha2::Digest as _;
    let executable = PathBuf::from(std::env::args().nth(1).expect("S fixture binary required"));
    let output = PathBuf::from(
        std::env::args()
            .nth(2)
            .expect("new evidence directory required"),
    );
    std::fs::create_dir(&output).unwrap();
    let mut params = CertificateParams::new(vec![]).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
    let leaf = |server: bool| {
        let key = KeyPair::generate().unwrap();
        let mut p = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        p.extended_key_usages = vec![if server {
            ExtendedKeyUsagePurpose::ServerAuth
        } else {
            ExtendedKeyUsagePurpose::ClientAuth
        }];
        (p.signed_by(&key, &ca).unwrap(), key)
    };
    let (server, server_key) = leaf(true);
    let (peer, peer_key) = leaf(false);
    let fingerprint = Digest::from_bytes(sha2::Sha256::digest(peer.der().as_ref()).into());
    let peer_boot = id();
    let mode = std::env::args().nth(3).unwrap_or_else(|| "normal".into());
    assert!(["normal", "begin", "submit", "complete"].contains(&mode.as_str()));
    let setup =
        definition_catalog_tests::workflow_model_tests::execution_change_tests::network_setup((
            peer_boot.clone(),
            fingerprint,
            Digest::from_bytes([71; 32]),
        ));
    let mut f = setup.fixture;
    let _sources = setup._sources;
    let layouts = setup.inputs.slot_resources(&setup.policy).unwrap();
    let (_, cell) = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap();
    let config = cell.configuration;
    let plan = rx_process_contract::execution_v2::Plan {
        schema: name(rx_process_contract::execution_v2::PLAN_SCHEMA),
        binding: (**config.execution.as_ref().unwrap()).clone(),
        process: (**config.process.as_ref().unwrap()).clone(),
    };
    assert_eq!(plan.reference().unwrap(), config.recipe);
    std::fs::write(
        output.join("plan-v2.json"),
        canonical::bytes(&plan).unwrap(),
    )
    .unwrap();
    std::fs::write(
        output.join("publication-v2.json"),
        canonical::bytes(&setup.publication).unwrap(),
    )
    .unwrap();
    std::fs::write(
        output.join("policy-v2.json"),
        canonical::bytes(&setup.policy).unwrap(),
    )
    .unwrap();
    for layout in &layouts {
        f.app
            .initialize_execution_slots(
                &f.operator,
                &id(),
                inv::Initialize {
                    cell: config.id.clone(),
                    resource: layout.resource.clone(),
                    rule: layout.rule.clone(),
                    expected_generation: None,
                    reason: "SIMULATION registered Executor fixture".into(),
                },
            )
            .unwrap();
    }
    let (cell_revision, _) = f.app.inspect_cell(&f.admin, &config.id).unwrap();
    let binding = f
        .app
        .create_execution_run(
            &f.operator,
            &id(),
            inv::CreateRun {
                cell: config.id.clone(),
                publication: setup.publication.reference.clone(),
                expected_cell: cell_revision,
                count: Counter(2),
            },
        )
        .unwrap();
    let objects = [
        actual(
            &mut f,
            &setup.policy.candidates[0].object_model,
            "SIM fixture part 1",
        ),
        actual(
            &mut f,
            &setup.policy.candidates[0].object_model,
            "SIM fixture part 2",
        ),
    ];
    f.app
        .bind_execution_object(
            &f.operator,
            &id(),
            inv::BindObject {
                run: binding.run.clone(),
                ordinal: Counter(1),
                object: objects[0].clone(),
            },
        )
        .unwrap();
    let now = f.clock.now();
    f.app
        .report_fact(
            &f.hosts[0],
            FactRecord {
                cell: config.id.clone(),
                id: name("ready"),
                source_host: f.hosts[0].principal.clone(),
                source_generation: f.registrations[0].source_sessions[&name("ready")].clone(),
                schema: name("boolean/v1"),
                unit: name("unitless"),
                acquired_at: now.clone(),
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
    let install = f.app.installation.clone();
    let operator = f.operator;
    let admin = f.admin;
    let host = f.hosts[0].clone();
    let registration = f.registrations[0].clone();
    let _directory = f._directory;
    let app = f.app;
    let writer = Writer::start(move || Ok(Application::new(app)))
        .await
        .unwrap();
    let runtime: Arc<dyn ApplicationPort> = Arc::new(Handle::new(writer));
    let port = Arc::new(LossPort {
        inner: runtime.clone(),
        mode: mode.clone(),
        dropped: std::sync::atomic::AtomicBool::new(false),
    });
    let ingress = PlatformIngress::new(
        port.clone(),
        Configuration {
            installation: install.clone(),
            release_digest: Digest::from_bytes([71; 32]),
            allowed_certificates: [(fingerprint, name("executor"))].into(),
        },
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let uri = format!("https://{}", listener.local_addr().unwrap());
    let ca_pem = ca.pem();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server_task = tokio::spawn(async move {
        ingress
            .serve(
                listener,
                TlsMaterial {
                    server_certificate_pem: server.pem().into_bytes(),
                    server_key_pem: server_key.serialize_pem().into_bytes(),
                    client_ca_pem: ca.pem().into_bytes(),
                },
                async {
                    let _ = stopped.await;
                },
            )
            .await
            .unwrap();
    });
    for (file, bytes) in [
        ("ca.pem", ca_pem),
        ("peer.pem", peer.pem()),
        ("peer.key", peer_key.serialize_pem()),
    ] {
        let path = output.join(file);
        std::fs::write(&path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
    let ready = output.join("ready");
    let report = output.join("report.json");
    let input = output.join("config.json");
    std::fs::write(&input,serde_json::to_vec_pretty(&serde_json::json!({"uri":uri,"ca":output.join("ca.pem"),"certificate":output.join("peer.pem"),"key":output.join("peer.key"),"pin":{"principal":"executor","peer_boot":peer_boot,"installation":install.id,"store_generation":install.store_generation,"release":Digest::from_bytes([71;32]),"clock_id":now.clock_id,"cell":config.id,"definition":config.definition.sha256},"run":binding.run,"plan":config.recipe.sha256,"ticks":now.ticks_ns,"ready":ready,"output":report,"journal":output.join("s-journal.sqlite3")})).unwrap()).unwrap();
    let mut child = ChildGuard(
        std::process::Command::new(&executable)
            .arg(&input)
            .stdout(std::fs::File::create(output.join("s.stdout")).unwrap())
            .stderr(std::fs::File::create(output.join("s.stderr")).unwrap())
            .spawn()
            .unwrap(),
    );
    wait_marker(&ready, &mut child).await;
    let executor = Identity {
        principal: name("executor"),
        session: Id::new(std::fs::read_to_string(&ready).unwrap()).unwrap(),
        terminal: None,
    };
    let Reply::Cell(cr, _) = call(
        runtime.as_ref(),
        Command::InspectCell {
            identity: operator.clone(),
            cell: config.id.clone(),
        },
    )
    .await
    else {
        panic!("cell")
    };
    let Reply::VersionedRun(rr, _) = call(
        runtime.as_ref(),
        Command::InspectRun {
            identity: operator.clone(),
            run: binding.run.clone(),
        },
    )
    .await
    else {
        panic!("run")
    };
    let Reply::Attempt(attempt) = call(
        runtime.as_ref(),
        Command::StartExecutionRun {
            identity: operator.clone(),
            request_key: id().to_string(),
            command: StartRun {
                run: binding.run.clone(),
                envelope_digest: config.envelope.sha256,
                purpose: Purpose::Production,
                budget_unit: rx_domain::budget::BudgetUnit::PartAttempt,
                budget_limit: Counter(2),
                expected_cell: cr,
                expected_run: rr,
            },
        },
    )
    .await
    else {
        panic!("start")
    };
    let Reply::PendingDeliveries(deliveries) = call(
        runtime.as_ref(),
        Command::PendingDeliveries {
            after: None,
            limit: 128,
        },
    )
    .await
    else {
        panic!("outbox")
    };
    let arm = deliveries
        .into_iter()
        .find(|d| matches!(&d.payload,Delivery::Arm {attempt:a,..} if a==&attempt.id))
        .unwrap();
    call(
        runtime.as_ref(),
        Command::PlanDelivery {
            identity: host.clone(),
            message: arm.id.clone(),
        },
    )
    .await;
    call(
        runtime.as_ref(),
        Command::FinishArm {
            identity: host.clone(),
            message: arm.id,
            ack: ArmAcknowledgment {
                attempt: attempt.id,
                host_boot: registration.boot_id,
                delivery_journal: registration.delivery_journal,
                sequence: Counter(950),
                epoch: attempt.epoch,
                scopes: attempt.scopes,
            },
        },
    )
    .await;
    let mut operations = vec![];
    let evidence_journal = id();
    for ordinal in 1..=2 {
        let mut found = None;
        for _ in 0..500 {
            let Reply::Overview(overview) =
                call(runtime.as_ref(), Command::Overview(admin.clone())).await
            else {
                panic!("overview")
            };
            found = overview
                .cells
                .iter()
                .flat_map(|c| &c.work)
                .find(|w| !operations.contains(w.operation.id()))
                .map(|w| w.operation.id().clone());
            if found.is_some() {
                break;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "S exited before operation {ordinal}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let op = found.expect("operation timeout");
        operations.push(op.clone());
        if ordinal == 1 {
            negative_reads(
                &uri,
                &output,
                &executor,
                &binding.run,
                runtime.as_ref(),
                &op,
            )
            .await;
        }
        drive(
            runtime.as_ref(),
            &host,
            &op,
            &evidence_journal,
            ordinal,
            &now,
        )
        .await;
        for i in 0..500 {
            let Reply::VersionedRun(_, r) = call(
                runtime.as_ref(),
                Command::InspectRun {
                    identity: operator.clone(),
                    run: binding.run.clone(),
                },
            )
            .await
            else {
                panic!("run")
            };
            let part = r.part_ids[(ordinal - 1) as usize].clone();
            let Reply::ExecutionPart(p) = call(
                runtime.as_ref(),
                Command::GetExecutionPart {
                    identity: executor.clone(),
                    run: binding.run.clone(),
                    part,
                },
            )
            .await
            else {
                panic!("part")
            };
            if p.state.disposition == PartDisposition::ConfirmedCompleted {
                break;
            }
            assert!(i < 499, "completion timeout");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        if ordinal == 1 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let Reply::VersionedRun(_, r) = call(
                runtime.as_ref(),
                Command::InspectRun {
                    identity: operator.clone(),
                    run: binding.run.clone(),
                },
            )
            .await
            else {
                panic!("waiting Run")
            };
            assert_eq!(r.state, RunState::Executing);
            assert_eq!(r.part_ids.len(), 1);
            assert!(child.try_wait().unwrap().is_none());
            call(
                runtime.as_ref(),
                Command::BindExecutionObject {
                    identity: operator.clone(),
                    key: id(),
                    input: inv::BindObject {
                        run: binding.run.clone(),
                        ordinal: Counter(2),
                        object: objects[1].clone(),
                    },
                },
            )
            .await;
        }
    }
    for i in 0..500 {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(i < 499, "service exit timeout");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        child.wait().unwrap().success(),
        "S failure; inspect s.stderr"
    );
    let result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    assert_eq!(result["report"]["stop"]["reason"], "COMPLETED");
    let Reply::VersionedRun(_, run) = call(
        runtime.as_ref(),
        Command::InspectRun {
            identity: operator,
            run: binding.run.clone(),
        },
    )
    .await
    else {
        panic!("final Run")
    };
    assert_eq!(run.state, RunState::Completed);
    assert_eq!(run.budget.as_ref().unwrap().consumed(), Counter(2));
    assert_eq!(port.dropped.load(Ordering::SeqCst), mode != "normal");
    let attempts = result["attempts"].as_array().unwrap();
    if mode != "normal" {
        let stage = match mode.as_str() {
            "begin" => "BEGIN_PART",
            "submit" => "SUBMIT_OPERATION",
            _ => "COMPLETE_PART",
        };
        let first = attempts
            .iter()
            .find(|a| a["logical"]["stage"] == stage && a["logical"]["visit"] == "1")
            .unwrap();
        assert_eq!(
            first["resolution"]["state"], "PENDING",
            "query recovery must not fabricate the lost reply"
        );
    }
    std::fs::write(output.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"REGISTERED_P_EXECUTOR_V2_PASS","mode":mode,"reply_loss_injected":port.dropped.load(Ordering::SeqCst),"parts":run.part_ids,"operations":operations,"run":run.id,"budget_consumed":run.budget.unwrap().consumed(),"actual_object_wait_checked":true,"negative_rpc_checks":["v2-binding-mismatch","v1-snapshot-refusal","foreign-part-artifact"],"executor_binary_sha256":format!("{:x}",sha2::Sha256::digest(std::fs::read(executable).unwrap())),"evidence_boundary":"real P signed-publication/qualification APIs, enrolled mTLS, real S worker/service; synthetic registered Host receipts/evidence, frozen test clock; not native Host or M3"})).unwrap()).unwrap();
    let _ = stop.send(());
    server_task.await.unwrap();
    println!("registered P/Executor2 pass");
}
async fn drive(
    runtime: &dyn ApplicationPort,
    host: &Identity,
    operation: &Id,
    journal: &Id,
    ordinal: u64,
    now: &TimePoint,
) {
    let Reply::DeliveryPlan(plan) = call(
        runtime,
        Command::PlanDelivery {
            identity: host.clone(),
            message: operation.clone(),
        },
    )
    .await
    else {
        panic!("prepare")
    };
    assert!(plan.first_emission);
    let work = plan.work.unwrap();
    assert!(work.execution.is_some());
    let invocation = id();
    let receipt = HostReceipt {
        operation: operation.clone(),
        digest: work.intent.digest().unwrap(),
        invocation: Some(invocation.clone()),
        journal: plan.registration.delivery_journal,
        sequence: Counter(1000 + ordinal * 2),
        state: ReceiptState::Prepared,
    };
    call(
        runtime,
        Command::RecordReceipt {
            identity: host.clone(),
            message: operation.clone(),
            receipt: receipt.clone(),
        },
    )
    .await;
    let message = rx_application::engine::authorization_delivery_id(operation);
    call(
        runtime,
        Command::PlanDelivery {
            identity: host.clone(),
            message: message.clone(),
        },
    )
    .await;
    call(
        runtime,
        Command::RecordReceipt {
            identity: host.clone(),
            message,
            receipt: HostReceipt {
                sequence: Counter(receipt.sequence.0 + 1),
                state: ReceiptState::ResultCaptured,
                ..receipt
            },
        },
    )
    .await;
    let evidence = NativeEvidence {
        native_details: None,
        id: id(),
        operation: operation.clone(),
        invocation,
        profile_digest: work.intent.profile_digest,
        device_session: id(),
        status_schema: name("rx.sim.completed.v1"),
        status: Integer(0),
        captured_at: now.clone(),
    };
    call(
        runtime,
        Command::IngestEvidence {
            identity: host.clone(),
            batch: EvidenceBatch {
                journal: journal.clone(),
                first: Counter(ordinal),
                records: vec![evidence.clone()],
            },
        },
    )
    .await;
    let Reply::Work(proven) = call(
        runtime,
        Command::InspectWork {
            identity: host.clone(),
            operation: operation.clone(),
        },
    )
    .await
    else {
        panic!("proven work")
    };
    assert_eq!(
        proven.operation.outcome(),
        rx_domain::operation::Outcome::Succeeded,
        "native fixture did not prove success: {:?}",
        proven.operation
    );
    assert_eq!(
        proven.operation.integrity(),
        rx_domain::operation::Integrity::Valid
    );
    for i in 0..500 {
        let Reply::ReconciliationRequests(plans) = call(
            runtime,
            Command::PendingReconciliations {
                identity: host.clone(),
                after: None,
                limit: 8,
            },
        )
        .await
        else {
            panic!("queries")
        };
        if plans.iter().any(|p| p.operation == *operation) {
            break;
        }
        assert!(i < 499, "original reconciliation missing");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let Reply::Work(current) = call(
        runtime,
        Command::InspectWork {
            identity: host.clone(),
            operation: operation.clone(),
        },
    )
    .await
    else {
        panic!("work")
    };
    let Reply::Cell(cr, _) = call(
        runtime,
        Command::InspectCell {
            identity: host.clone(),
            cell: work.cell.clone(),
        },
    )
    .await
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
            observed_at: now.clone(),
            uncertainty_ns: Counter(0),
            quality_good: true,
            origin_age_bounded: true,
        })
        .collect();
    call(
        runtime,
        Command::ReleaseResources {
            identity: host.clone(),
            key: id(),
            command: ReleaseResources {
                operation: operation.clone(),
                expected_operation: current.operation.revision(),
                expected_cell: cr,
                observations,
            },
        },
    )
    .await;
}

async fn negative_reads(
    uri: &str,
    output: &Path,
    executor: &Identity,
    run: &Id,
    runtime: &dyn ApplicationPort,
    operation: &Id,
) {
    use rx_protocol::{base, execution_v2 as wire, executor as old};
    use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity as TlsIdentity};
    let channel = Channel::from_shared(uri.to_owned())
        .unwrap()
        .tls_config(
            ClientTlsConfig::new()
                .domain_name("localhost")
                .ca_certificate(Certificate::from_pem(
                    std::fs::read(output.join("ca.pem")).unwrap(),
                ))
                .identity(TlsIdentity::from_pem(
                    std::fs::read(output.join("peer.pem")).unwrap(),
                    std::fs::read(output.join("peer.key")).unwrap(),
                )),
        )
        .unwrap()
        .connect()
        .await
        .unwrap();
    let context = || base::CallContext {
        session_id: executor.session.to_string(),
        call_id: id().to_string(),
        request_key: None,
        expected_revision: None,
    };
    let mut new =
        wire::execution_control_service_client::ExecutionControlServiceClient::new(channel.clone());
    assert_eq!(
        new.get_snapshot(wire::ReadExecutionSnapshot {
            context: Some(context()),
            run_id: run.to_string(),
            visit: 1,
            binding_hash: vec![0; 32]
        })
        .await
        .unwrap_err()
        .code(),
        tonic::Code::FailedPrecondition
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(include_bytes!("../../../../spec/executor/v1/binding.json"))
            .unwrap();
    use sha2::Digest as _;
    let old_hash = sha2::Sha256::digest(canonical::bytes(&manifest).unwrap()).to_vec();
    let mut old = old::executor_read_service_client::ExecutorReadServiceClient::new(channel);
    assert_eq!(
        old.get_snapshot(rx_protocol::executor::SnapshotRequest {
            context: Some(context()),
            run_id: run.to_string(),
            visit: 1,
            binding_hash: old_hash
        })
        .await
        .unwrap_err()
        .code(),
        tonic::Code::FailedPrecondition
    );
    let Reply::Work(work) = call(
        runtime,
        Command::InspectWork {
            identity: executor.clone(),
            operation: operation.clone(),
        },
    )
    .await
    else {
        panic!("work")
    };
    let r = work.execution.as_ref().unwrap().report.clone();
    assert_eq!(
        new.get_artifact(wire::ReadExecutionArtifact {
            context: Some(context()),
            run_id: run.to_string(),
            part_id: work.part.clone().unwrap().to_string(),
            reference: Some(base::ArtifactRef {
                schema_id: "foreign/schema".into(),
                sha256: r.sha256.as_bytes().to_vec(),
                size_bytes: r.size_bytes.0
            }),
            binding_hash: rx_process_contract::execution_v2::executor::binding_hash()
                .as_bytes()
                .to_vec()
        })
        .await
        .unwrap_err()
        .code(),
        tonic::Code::PermissionDenied
    );
}
