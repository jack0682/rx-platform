//! Actual mTLS + separate Host publisher. Seeded evidence tests delivery, never physical execution.
mod support;
use rx_api::grpc::{Configuration, EvidenceIngress, TlsMaterial};
use rx_application::*;
use rx_domain::types::*;
use rx_protocol::base;
use rx_runtime::{
    application::{Application, ApplicationPort, CallFuture, Command, Handle, Reply},
    writer::{Status, Writer, WriterError},
};
use rx_storage::SqliteRepository;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    process::{Child, Command as ProcessCommand, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tonic::transport::{
    Certificate as TlsCertificate, Channel, ClientTlsConfig, Identity as TlsIdentity,
};

fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
struct TestClock;
impl Clock for TestClock {
    fn now(&self) -> TimePoint {
        TimePoint {
            clock_id: "test-clock".into(),
            ticks_ns: Counter(1000),
        }
    }
}
struct Deny;
impl QualificationAuthority for Deny {
    fn verify(&self, _: &CellConfiguration, _: &[ArtifactRef], _: &[Digest]) -> bool {
        false
    }
}
type AppHandle = Handle<Application<SqliteRepository, TestClock, Deny>>;
struct LoseFirstDataAck {
    handle: AppHandle,
    armed: AtomicBool,
}
impl ApplicationPort for LoseFirstDataAck {
    fn request(&self, command: Command) -> CallFuture<'_> {
        let lose = matches!(&command,Command::PublishEvidence {batch,..} if !batch.records.is_empty())
            && self.armed.swap(false, Ordering::SeqCst);
        Box::pin(async move {
            let reply = self.handle.call(command).await?;
            if lose {
                Err(WriterError::Unavailable)
            } else {
                Ok(reply)
            }
        })
    }
    fn status(&self) -> Status {
        self.handle.status()
    }
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(executable: &str, config: &Path) -> Process {
    Process(
        ProcessCommand::new(executable)
            .arg(config)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    )
}
async fn published(directory: &Path, process: &mut Process, through: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(bytes) = std::fs::read(directory.join("publisher.json"))
            && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
            && value["through"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                == Some(through)
        {
            return;
        }
        assert!(
            process.0.try_wait().unwrap().is_none(),
            "Host fixture exited"
        );
        assert!(
            tokio::time::Instant::now() < deadline,
            "publication did not reach {through}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
#[ignore = "tools/test_evidence_e2e.sh builds and supplies the separate Host publisher"]
async fn publisher_catches_up_over_128_records_after_lost_ack_and_preserves_cursor_on_restart() {
    use rcgen::*;
    use sha2::Digest as _;
    let executable = std::env::var("RX_HOST_SIM_SERVER").expect("separate Host fixture executable");
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("platform.db");
    let db = database.clone();
    let configuration = support::configuration("cell/a");
    let config = configuration.clone();
    let writer = Writer::start(move || {
        let mut engine = Engine::open(
            SqliteRepository::open(db)?,
            TestClock,
            Deny,
            id(),
            Principal {
                id: name("admin"),
                client_namespace: name("admin"),
                roles: [Role::AccountAdmin, Role::Engineer].into_iter().collect(),
                cells: [name("cell/a")].into_iter().collect(),
                active: true,
            },
        )?;
        let session = engine.authenticated_session(
            &name("admin"),
            id(),
            TimePoint {
                clock_id: "test-clock".into(),
                ticks_ns: Counter(u64::MAX),
            },
        )?;
        let admin = Identity {
            principal: name("admin"),
            session: session.id,
            terminal: None,
        };
        engine.put_principal(
            &admin,
            Principal {
                id: name("host/sim"),
                client_namespace: name("host/sim"),
                roles: [Role::Host].into_iter().collect(),
                cells: [name("cell/a")].into_iter().collect(),
                active: true,
            },
            None,
        )?;
        engine.install_cell(&admin, config)?;
        Ok(Application::new(engine))
    })
    .await
    .unwrap();
    let handle = Handle::new(writer);
    let Reply::Installation(installation) = handle.call(Command::Installation).await.unwrap()
    else {
        panic!("installation reply")
    };
    let mut ca_params = CertificateParams::new(vec![]).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();
    let leaf = |label: &str, usage: ExtendedKeyUsagePurpose| {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec![label.into()]).unwrap();
        params.extended_key_usages = vec![usage];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        (params.signed_by(&key, &ca).unwrap(), key)
    };
    let (server, server_key) = leaf("localhost", ExtendedKeyUsagePurpose::ServerAuth);
    let (producer, producer_key) = leaf("host-sim", ExtendedKeyUsagePurpose::ClientAuth);
    let (unregistered, unregistered_key) =
        leaf("unregistered", ExtendedKeyUsagePurpose::ClientAuth);
    let fingerprint = Digest::from_bytes(sha2::Sha256::digest(producer.der().as_ref()).into());
    let release = Digest::from_bytes([8; 32]);
    let port = Arc::new(LoseFirstDataAck {
        handle: handle.clone(),
        armed: AtomicBool::new(true),
    });
    let ingress = EvidenceIngress::new(
        port.clone(),
        Configuration {
            installation: installation.clone(),
            release_digest: release,
            allowed_certificates: BTreeMap::from([(fingerprint, name("host/sim"))]),
        },
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let uri = format!("https://{}", listener.local_addr().unwrap());
    let tls = TlsMaterial {
        server_certificate_pem: server.pem().into_bytes(),
        server_key_pem: server_key.serialize_pem().into_bytes(),
        client_ca_pem: ca.pem().into_bytes(),
    };
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        ingress
            .serve(listener, tls, async {
                let _ = stopped.await;
            })
            .await
            .unwrap()
    });
    for (file, data) in [
        ("server.pem", server.pem()),
        ("server-key.pem", server_key.serialize_pem()),
        ("ca.pem", ca.pem()),
        ("producer.pem", producer.pem()),
        ("producer-key.pem", producer_key.serialize_pem()),
    ] {
        std::fs::write(dir.path().join(file), data).unwrap();
    }
    let operation = id();
    let invocation = id();
    let device = id();
    let last_evidence = id();
    let records:Vec<_>=(0..131).map(|n|json!({"evidence_id":if n==130 {last_evidence.clone()} else {id()},
        "operation":operation,"invocation":invocation,"profile_digest":configuration.steps[0].intent.profile_digest,
        "capture":{"status_schema":"seed/status.v1","status":0,"native_id":format!("seed/{n}"),"device_session":device,
            "captured_at":{"clock_id":"test-clock","ticks_ns":"1000"}}})).collect();
    let host_directory = dir.path().join("host");
    let config_path = dir.path().join("host-config.json");
    let mut host_config = json!({"directory":host_directory,"bindings":[{"host":"host/sim","platform":"platform","cell":"cell/a",
        "definition":configuration.definition,"envelope":configuration.envelope,"qualification":id(),"qualification_revision":"1",
        "allowed_intents":[configuration.steps[0].intent],"scope_ids":configuration.scopes,"condition_ids":["ready"],"environment":"SIMULATION","purposes":["SETUP"]}],
        "installation":installation.id,"release_digest":release,"client_fingerprint":fingerprint,
        "server_certificate":dir.path().join("server.pem"),"server_key":dir.path().join("server-key.pem"),"client_ca":dir.path().join("ca.pem"),
        "clock_id":"test-clock","ticks":1000,"test_seed_evidence":records,
        "publisher":{"uri":uri,"server_name":"localhost","server_ca":dir.path().join("ca.pem"),"client_certificate":dir.path().join("producer.pem"),
            "client_key":dir.path().join("producer-key.pem"),"store_generation":installation.store_generation}});
    std::fs::write(&config_path, serde_json::to_vec(&host_config).unwrap()).unwrap();
    let mut host = start(&executable, &config_path);
    published(&host_directory, &mut host, 131).await;
    assert!(
        !port.armed.load(Ordering::SeqCst),
        "test must lose an acknowledgment after commit"
    );
    let Reply::Producer(known) = handle
        .call(Command::CurrentEvidenceProducer(name("host/sim")))
        .await
        .unwrap()
    else {
        panic!("producer reply")
    };
    let channel = Channel::from_shared(uri.clone())
        .unwrap()
        .tls_config(
            ClientTlsConfig::new()
                .domain_name("localhost")
                .ca_certificate(TlsCertificate::from_pem(ca.pem()))
                .identity(TlsIdentity::from_pem(
                    producer.pem(),
                    producer_key.serialize_pem(),
                )),
        )
        .unwrap()
        .connect()
        .await
        .unwrap();
    let context = base::CallContext {
        session_id: known.session.to_string(),
        call_id: id().to_string(),
        request_key: None,
        expected_revision: None,
    };
    let result = base::evidence_service_client::EvidenceServiceClient::new(channel.clone())
        .get(base::EvidenceRef {
            context: Some(context.clone()),
            evidence_id: last_evidence.to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    let Some(base::evidence_body::Value::NativeResult(result)) = result.body.and_then(|b| b.value)
    else {
        panic!("native evidence")
    };
    assert_eq!(
        result.correlation.unwrap().native_id.as_deref(),
        Some("seed/130")
    );
    let db = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let slots: i64 = db
        .query_row(
            "SELECT count(*) FROM entities WHERE key LIKE 'evidenceslot/%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        slots, 131,
        "duplicate publication must not create another stored slot"
    );
    let work: i64 = db
        .query_row(
            "SELECT count(*) FROM entities WHERE key LIKE 'work/%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(work, 0);
    let cell: Vec<u8> = db
        .query_row(
            "SELECT document FROM entities WHERE key LIKE 'cell/%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let cell: Value = serde_json::from_slice(&cell).unwrap();
    assert!(cell["value"]["qualification"].is_null());
    assert!(!cell["value"]["blocks"].as_array().unwrap().is_empty());
    drop(db);
    // Same CA is insufficient: the leaf must be registered as the actual producer.
    let rogue = Channel::from_shared(uri.clone())
        .unwrap()
        .tls_config(
            ClientTlsConfig::new()
                .domain_name("localhost")
                .ca_certificate(TlsCertificate::from_pem(ca.pem()))
                .identity(TlsIdentity::from_pem(
                    unregistered.pem(),
                    unregistered_key.serialize_pem(),
                )),
        )
        .unwrap()
        .connect()
        .await
        .unwrap();
    let denied = base::evidence_service_client::EvidenceServiceClient::new(rogue)
        .get(base::EvidenceRef {
            context: Some(context.clone()),
            evidence_id: last_evidence.to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(denied.code(), tonic::Code::PermissionDenied);
    drop(host);
    host_config["test_seed_evidence"] = json!([]);
    std::fs::write(&config_path, serde_json::to_vec(&host_config).unwrap()).unwrap();
    std::fs::remove_file(host_directory.join("publisher.json")).unwrap();
    let mut host = start(&executable, &config_path);
    published(&host_directory, &mut host, 131).await;
    let Reply::Producer(reconnected) = handle
        .call(Command::CurrentEvidenceProducer(name("host/sim")))
        .await
        .unwrap()
    else {
        panic!("producer reply")
    };
    assert_ne!(known.peer_boot, reconnected.peer_boot);
    assert_ne!(known.session, reconnected.session);
    assert_eq!(known.journal, reconnected.journal);
    let revoked = base::evidence_service_client::EvidenceServiceClient::new(channel)
        .get(base::EvidenceRef {
            context: Some(context),
            evidence_id: last_evidence.to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(revoked.code(), tonic::Code::Unauthenticated);
    let effects = host_directory.join("device/effects.jsonl");
    assert!(
        std::fs::read_to_string(effects)
            .unwrap_or_default()
            .is_empty(),
        "publisher must not invoke a device"
    );
    drop(host);
    stop.send(()).unwrap();
    task.await.unwrap();
    handle.close();
    handle.closed().await;
}
