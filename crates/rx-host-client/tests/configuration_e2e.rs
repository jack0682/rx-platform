//! Cross-repository mTLS test. File-device only, no equipment connection.
#[path = "support/configuration_coordinator.rs"]
mod coordinator;
#[path = "../../rx-application/tests/support/requalification_fixture.rs"]
mod qsupport;
#[path = "../../rx-application/tests/support/process_review.rs"]
mod review_support;
use rx_domain::{canonical, host_configuration as config, intent::*, types::*};
use rx_host_client::{Hello, HostClient, TlsEndpoint};
use std::{
    collections::BTreeMap,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[tokio::test]
#[ignore = "tools/test_host_configuration_e2e.sh supplies S simulation server"]
async fn configuration_reply_loss_recovers_same_receipt_and_restarted_host_stays_unqualified() {
    run_fixture(false).await;
}
#[tokio::test]
#[ignore = "tools/test_host_configuration_e2e.sh supplies S simulation server"]
async fn configuration_coordinator_persists_request_and_recovers_real_host_reply_loss() {
    run_fixture(true).await;
}
async fn run_fixture(coordinated: bool) {
    use rcgen::*;
    use sha2::Digest as _;
    let product = std::env::var("RX_HOST_PRODUCT_SERVER").ok();
    let executable = product
        .clone()
        .unwrap_or_else(|| std::env::var("RX_HOST_SIM_SERVER").unwrap());
    let dir = tempfile::tempdir().unwrap();
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
    let (client, client_key) = leaf(false);
    let (terminal, terminal_key) = leaf(false);
    let terminal_fingerprint =
        Digest::from_bytes(sha2::Sha256::digest(terminal.der().as_ref()).into());
    for (label, data) in [
        ("server.pem", server.pem()),
        ("server.key", server_key.serialize_pem()),
        ("ca.pem", ca.pem()),
        ("terminal.pem", terminal.pem()),
        ("terminal.key", terminal_key.serialize_pem()),
    ] {
        let path = dir.path().join(label);
        std::fs::write(&path, data).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if label.ends_with(".key") {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
    }
    let intent = Intent {
        kind: Kind::FiniteAction,
        target: name("sim/chuck"),
        profile_digest: qsupport::material("profile", "rx.device-profile.v1").sha256,
        site_config_digest: qsupport::material("site", "rx.site-config.v1").sha256,
        calibration_digests: vec![],
        resource_set: vec![name("sim/controller")],
        execution_timeout_ms: Counter(1000),
        prepare_validity_ms: Counter(1000),
        completion_rule: name("sim/closed"),
        cancel_rule: name("sim/stop"),
        body: Body::Program(ProgramGoal {
            program: qsupport::material("program", "rx.sim.program.v1"),
            parameter_set: qsupport::material("parameters", "rx.sim.parameters.v1"),
        }),
    };
    let definition = qsupport::material("definition", "rx.cell-definition.v1");
    let envelope = qsupport::material("envelope", "rx.operating-envelope.v1");
    let installation = id();
    let binding = serde_json::json!({"host":"host/sim","platform":installation.to_string(),"cell":"cell/sim","definition":definition,"envelope":envelope,"qualification":id(),"qualification_revision":"1","allowed_intents":[intent],"scope_ids":["scope/main"],"condition_ids":["sim/ready"],"environment":"SIMULATION","purposes":["PRODUCTION","SETUP"]});
    let state = dir.path().join("host");
    let file = dir.path().join("server.json");
    let configuration = serde_json::json!({"directory":state,"bindings":[binding],"installation":installation,"release_digest":Digest::from_bytes([8;32]),"client_fingerprint":Digest::from_bytes(sha2::Sha256::digest(client.der().as_ref()).into()),"server_certificate":dir.path().join("server.pem"),"server_key":dir.path().join("server.key"),"client_ca":dir.path().join("ca.pem"),"clock_id":"simulation/boottime","ticks":1000,"publisher":null,"lose_configuration_reply_once":true,"lose_qualification_reply_once":true});
    let runtime_directory = dir.path().join("host-runtime");
    std::fs::create_dir(&runtime_directory).unwrap();
    let ready_file = if product.is_some() {
        runtime_directory.join("host-status.json")
    } else {
        state.join("ready.json")
    };
    let configuration = if product.is_some() {
        let binding_file = dir.path().join("bindings.json");
        let bytes = canonical::bytes(&serde_json::json!([binding])).unwrap();
        std::fs::write(&binding_file, &bytes).unwrap();
        let pinned = |label: &str| {
            let path = dir.path().join(label);
            serde_json::json!({"path":path,"sha256":rx_package::content_digest(&std::fs::read(&path).unwrap())})
        };
        serde_json::json!({"schema":"rx.host-startup.v1","installation":installation,"release_digest":Digest::from_bytes([8;32]),"host":"host/sim","bind":"127.0.0.1:0","data_directory":state,"runtime_directory":runtime_directory,"bindings":{"path":binding_file,"sha256":rx_package::content_digest(&bytes)},"backend":{"kind":"FILE_SIMULATION"},"tls":{"certificate":pinned("server.pem"),"key":pinned("server.key"),"ca":pinned("ca.pem")},"allowed_platform_certificates":{Digest::from_bytes(sha2::Sha256::digest(client.der().as_ref()).into()).to_string():installation.to_string()},"publisher":null,"publication_drain_ms":"0"})
    } else {
        configuration
    };
    std::fs::write(&file, canonical::bytes(&configuration).unwrap()).unwrap();
    if product.is_some() {
        let result = Command::new(&executable)
            .arg("init")
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    let start = || {
        let mut command = Command::new(&executable);
        if product.is_some() {
            command.arg("run");
        }
        command.arg(&file).stdout(Stdio::null());
        let mut child = ChildGuard(command.spawn().unwrap());
        let end = Instant::now() + Duration::from_secs(15);
        while !ready_file.exists() {
            assert!(Instant::now() < end && child.0.try_wait().unwrap().is_none());
            std::thread::sleep(Duration::from_millis(10));
        }
        child
    };
    let mut process = start();
    let endpoint = |uri: String| TlsEndpoint {
        uri,
        server_name: "localhost".into(),
        server_ca_pem: ca.pem().into_bytes(),
        client_certificate_pem: client.pem().into_bytes(),
        client_key_pem: client_key.serialize_pem().into_bytes(),
    };
    let ready: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&ready_file).unwrap()).unwrap();
    let hello = || Hello {
        peer_id: name(installation.as_str()),
        boot_id: id(),
        installation: installation.clone(),
        store_generation: id(),
        release_digest: Digest::from_bytes([8; 32]),
        clock_id: coordinator::shared_time().clock_id,
    };
    let host = HostClient::connect(
        endpoint(
            ready[if product.is_some() {
                "endpoint"
            } else {
                "address"
            }]
            .as_str()
            .unwrap()
            .into(),
        ),
        name("host/sim"),
        hello(),
    )
    .await
    .unwrap();
    let cfg:rx_application::CellConfiguration=serde_json::from_value(serde_json::json!({"id":"cell/sim","environment":"SIMULATION","definition":definition,"envelope":envelope,"recipe":definition,"site_config_digest":intent.site_config_digest,"scopes":["scope/main"],"hosts":["host/sim"],"executor":"executor","maximum_budget":"1","permit_ttl_ns":"1000000000","start_timeout_ns":"5000000000","start_conditions":[{"op":"EQ","fact":"ready","schema":"boolean/v1","unit":"unitless","expected":{"boolean":true}}],"steps":[{"id":"step","host":"host/sim","intent":intent,"predecessors":[],"conditions":[{"op":"EQ","fact":"ready","schema":"boolean/v1","unit":"unitless","expected":{"boolean":true}}],"completion":{"kind":"NATIVE","schema":"rx.sim.completed.v1","success":["0"],"failure":["1"],"postconditions":[]},"condition_ids":["sim/ready"],"condition_revision":"1","handover_max_age_ns":"1000000000"}],"maintained_conditions":[],"fact_specs":[{"id":"ready","host":"host/sim","schema":"boolean/v1","unit":"unitless","maximum_age_ns":"1000000000","maximum_uncertainty_ns":"0"}]})).unwrap();
    host.open_cell(&cfg, &coordinator::shared_time().clock_id)
        .await
        .unwrap();
    let before = host.inspect_process_configuration().await.unwrap().snapshot;
    if coordinated {
        coordinator::run(
            &host,
            &cfg,
            &before,
            &installation,
            dir.path(),
            &state,
            terminal_fingerprint,
        )
        .await;
        if product.is_some() {
            #[cfg(target_os = "linux")]
            {
                let pid = rustix::process::Pid::from_raw(process.0.id() as i32).unwrap();
                rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
                let until = Instant::now() + Duration::from_secs(5);
                loop {
                    if let Some(exit) = process.0.try_wait().unwrap() {
                        assert!(exit.success());
                        break;
                    }
                    assert!(Instant::now() < until);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                let status: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&ready_file).unwrap()).unwrap();
                assert_eq!(status["phase"], "STOPPED_WITH_RECONCILIATION_REQUIRED");
                assert_eq!(status["stop"]["safe_to_drop"], true);
                assert_eq!(status["admission_open"], false);
                if let Ok(output) = std::env::var("RX_HOST_CONFIGURATION_EVIDENCE") {
                    let path = std::path::PathBuf::from(format!("{output}-coordinator"));
                    std::fs::write(
                        path.join("host-service-stop.json"),
                        canonical::bytes(&status).unwrap(),
                    )
                    .unwrap();
                }
            }
        }
        return;
    }
    let scopes = BTreeMap::from([(name("scope/main"), Counter(2))]);
    let fence = id();
    host.fence(&fence, &cfg.id, Counter(2), &scopes, &[id()])
        .await
        .unwrap();
    let request = config::Request {
        schema: name("rx.host-process-configuration-request.v1"),
        id: id(),
        change: id(),
        preparation: Counter(1),
        plan_digest: Digest::from_bytes([9; 32]),
        host: name("host/sim"),
        expected_host_boot: before.host_boot,
        expected_delivery_journal: before.delivery_journal,
        binding_digest: before.binding_digest,
        cells: vec![config::CellTarget {
            cell: cfg.id.clone(),
            expected_context: None,
            before_configuration: Digest::from_bytes([6; 32]),
            after_configuration: Digest::from_bytes([7; 32]),
            recipe: cfg.recipe.clone(),
            definition: cfg.definition.sha256,
            envelope: cfg.envelope.sha256,
            environment: name("SIMULATION"),
            required_intents: vec![intent.digest().unwrap()],
            required_conditions: vec![name("sim/ready")],
            epoch: Counter(2),
            scopes,
            fence_request: fence,
        }],
    };
    // Persist exact request bytes before first send; response loss must not produce a new ID.
    {
        use std::io::Write;
        let mut file =
            std::fs::File::create(dir.path().join("outgoing-configuration.json")).unwrap();
        file.write_all(&canonical::bytes(&request).unwrap())
            .unwrap();
        file.sync_all().unwrap();
    }
    if product.is_some() {
        let _ = host.accept_process_configuration(&request).await.unwrap();
    } else {
        assert!(host.accept_process_configuration(&request).await.is_err());
    }
    let recovered = host
        .lookup_process_configuration(&request.id)
        .await
        .unwrap();
    let first = recovered.receipt.as_ref().unwrap();
    assert_eq!(first.status, config::Status::AppliedUnqualified);
    assert!(recovered.context_matches_current_host && !recovered.activation_authorized);
    let repeated = host.accept_process_configuration(&request).await.unwrap();
    assert_eq!(repeated.receipt.as_ref().unwrap().sequence, first.sequence);
    let mut conflict = request.clone();
    conflict.cells[0].after_configuration = Digest::from_bytes([55; 32]);
    assert!(host.accept_process_configuration(&conflict).await.is_err());
    assert!(!state.join("device/effects.jsonl").exists());
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    std::fs::remove_file(&ready_file).unwrap();
    let restarted = start();
    let ready: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&ready_file).unwrap()).unwrap();
    let next = HostClient::connect(
        endpoint(
            ready[if product.is_some() {
                "endpoint"
            } else {
                "address"
            }]
            .as_str()
            .unwrap()
            .into(),
        ),
        name("host/sim"),
        hello(),
    )
    .await
    .unwrap();
    let old = next
        .lookup_process_configuration(&request.id)
        .await
        .unwrap();
    assert_eq!(old.receipt.as_ref().unwrap().sequence, first.sequence);
    assert!(!old.context_matches_current_host && !old.activation_authorized);
    if let Ok(out) = std::env::var("RX_HOST_CONFIGURATION_EVIDENCE") {
        let out = std::path::PathBuf::from(out);
        std::fs::create_dir(&out).unwrap();
        for (label, value) in [
            ("request.json", serde_json::to_value(&request).unwrap()),
            ("recovered.json", serde_json::to_value(&recovered).unwrap()),
            ("restart.json", serde_json::to_value(&old).unwrap()),
        ] {
            std::fs::write(out.join(label), canonical::bytes(&value).unwrap()).unwrap();
        }
        let mut result:serde_json::Value=serde_json::from_slice(b"{\"status\":\"PASS\",\"real_mtls\":true,\"reply_lost_after_commit\":true,\"same_request_recovered\":true,\"native_effects\":0,\"restart_not_qualified\":true}").unwrap();
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
        if product {
            result["reply_lost_after_commit"] = serde_json::json!(false);
            result["fault_injection_location"] = serde_json::json!("none in raw-client case");
        }
        std::fs::write(out.join("result.json"), canonical::bytes(&result).unwrap()).unwrap();
    }
    drop(restarted);
}
