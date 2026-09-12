//! Test-only remote E client. Operator assignment and handover remain explicit fixture steps.
use super::*;
use rx_api::grpc::{Configuration, PlatformIngress, TlsMaterial};
use rx_protocol::{base, cell};
use rx_runtime::{
    application::{Application, ApplicationPort, CallFuture, Command as Call, Handle, Reply},
    writer::{Status, WriterError},
};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity as TlsIdentity};
type AppHandle = Handle<Application<SqliteRepository, TestClock, Catalog>>;
pub struct TlsFixture {
    pub server: TlsMaterial,
    pub ca: Vec<u8>,
    pub certificate: Vec<u8>,
    pub key: Vec<u8>,
    pub fingerprint: Digest,
    pub release: Digest,
}
struct LoseFirstSubmit {
    handle: AppHandle,
    first: std::sync::atomic::AtomicBool,
}
impl ApplicationPort for LoseFirstSubmit {
    fn request(&self, command: Call) -> CallFuture<'_> {
        let lose = matches!(&command, Call::ExecutorSubmit { .. })
            && self.first.swap(false, Ordering::SeqCst);
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
pub struct RemoteExecutor {
    pub identity: Identity,
    session: base::Session,
    configuration: CellConfiguration,
    cells: cell::cell_service_client::CellServiceClient<Channel>,
    workflow: base::workflow_service_client::WorkflowServiceClient<Channel>,
    operations: base::operation_service_client::OperationServiceClient<Channel>,
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
    first_submit: bool,
}
fn hash(text: &str) -> Vec<u8> {
    use sha2::Digest as _;
    let value: serde_json::Value = serde_json::from_str(text).unwrap();
    sha2::Sha256::digest(rx_domain::canonical::bytes(&value).unwrap()).to_vec()
}
impl RemoteExecutor {
    pub async fn connect(
        handle: AppHandle,
        configuration: CellConfiguration,
        tls: TlsFixture,
    ) -> Self {
        let Reply::Installation(installation) = handle.call(Call::Installation).await.unwrap()
        else {
            panic!("installation")
        };
        let ingress = PlatformIngress::new(
            Arc::new(LoseFirstSubmit {
                handle,
                first: std::sync::atomic::AtomicBool::new(true),
            }),
            Configuration {
                installation: installation.clone(),
                release_digest: tls.release,
                allowed_certificates: BTreeMap::from([(tls.fingerprint, name("executor"))]),
            },
        )
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("https://{}", listener.local_addr().unwrap());
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            ingress
                .serve(listener, tls.server, async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let channel = Channel::from_shared(uri)
            .unwrap()
            .tls_config(
                ClientTlsConfig::new()
                    .domain_name("localhost")
                    .ca_certificate(Certificate::from_pem(tls.ca))
                    .identity(TlsIdentity::from_pem(tls.certificate, tls.key)),
            )
            .unwrap()
            .connect()
            .await
            .unwrap();
        let base_hash = hash(include_str!(
            "../../../../spec/contracts/v1.0/protocol_manifest.json"
        ));
        let cell_hash = hash(include_str!(
            "../../../../spec/cell_operations/v1.0/protocol_manifest.json"
        ));
        let session = base::session_service_client::SessionServiceClient::new(channel.clone())
            .open(base::PeerHello {
                peer_id: "executor".into(),
                role: base::Role::Executor as i32,
                boot_id: id().to_string(),
                installation_id: installation.id.to_string(),
                store_generation: installation.store_generation.to_string(),
                supported_versions: vec![base::Version {
                    major: 1,
                    minor: 0,
                    schema_hash: base_hash.clone(),
                }],
                release_digest: tls.release.as_bytes().to_vec(),
                journal_id: None,
                last_seq: None,
                shared_clock_id: installation.clock_id.clone(),
            })
            .await
            .unwrap()
            .into_inner();
        let mut cells = cell::cell_service_client::CellServiceClient::new(channel.clone());
        cells
            .open(cell::CellHello {
                base_session_id: session.session_id.clone(),
                peer_id: "executor".into(),
                base_manifest_hash: base_hash,
                cell_manifest_hash: cell_hash,
                cell_definition_digest: configuration.definition.sha256.as_bytes().to_vec(),
                shared_clock_id: installation.clock_id,
            })
            .await
            .unwrap();
        Self {
            identity: Identity {
                principal: name("executor"),
                session: Id::new(&session.session_id).unwrap(),
                terminal: None,
            },
            session,
            configuration,
            cells,
            workflow: base::workflow_service_client::WorkflowServiceClient::new(channel.clone()),
            operations: base::operation_service_client::OperationServiceClient::new(channel),
            stop,
            task,
            first_submit: true,
        }
    }
    fn context(&self, key: Option<&Id>, revision: Option<Counter>) -> base::CallContext {
        base::CallContext {
            session_id: self.session.session_id.clone(),
            call_id: id().to_string(),
            request_key: key.map(ToString::to_string),
            expected_revision: revision.map(|r| r.0),
        }
    }
    pub async fn begin_part(&mut self, run: &Run, revision: Counter) -> PartAttempt {
        let key = id();
        let request = cell::BeginPartAttemptRequest {
            call: Some(cell::CellCall {
                context: Some(self.context(Some(&key), None)),
                cell_id: self.configuration.id.to_string(),
                expected_cell_revision: Some(revision.0),
            }),
            run_id: run.id.to_string(),
            mandate_id: run.mandate.as_ref().unwrap().to_string(),
            expected_budget_revision: run.budget.as_ref().unwrap().revision().0,
        };
        let part = self
            .cells
            .begin_part_attempt(request)
            .await
            .unwrap()
            .into_inner();
        assert_eq!(part.disposition, cell::PartDisposition::InProgress as i32);
        PartAttempt {
            id: Id::new(part.part_attempt_id).unwrap(),
            run: Id::new(part.run_id).unwrap(),
            ordinal: Counter(part.ordinal),
            disposition: PartDisposition::InProgress,
        }
    }
    pub async fn activation(&mut self, run: &Id, visit: Counter, revision: Counter) -> Id {
        let key = id();
        let value = self
            .workflow
            .resolve_activation(base::ResolveActivation {
                context: Some(self.context(Some(&key), Some(revision))),
                run_id: run.to_string(),
                node_id: "step/run".into(),
                visit: visit.0,
            })
            .await
            .unwrap()
            .into_inner();
        Id::new(value.activation_id).unwrap()
    }
    pub async fn submit(&mut self, command: &SubmitWork, mandate: &Id) -> Id {
        let key = id();
        let context = self.context(Some(&key), None);
        let intent = rx_protocol::json::from_slice::<base::Intent>(
            &rx_domain::canonical::bytes(&command.intent).unwrap(),
        )
        .unwrap();
        let mut request = cell::SubmitOperationRequest {
            call: Some(cell::CellCall {
                context: Some(context.clone()),
                cell_id: self.configuration.id.to_string(),
                expected_cell_revision: Some(command.expected_cell.0),
            }),
            request: Some(base::SubmitOperation {
                context: Some(context),
                intent: Some(intent),
                run_id: Some(command.run.to_string()),
                activation_id: Some(command.activation.to_string()),
                slot: Some(command.slot.to_string()),
            }),
            parent: Some(cell::PermitParent {
                value: Some(cell::permit_parent::Value::MandateId(mandate.to_string())),
            }),
            part_attempt_id: command.part.as_ref().map(ToString::to_string),
            expected_run_revision: Some(command.expected_run.0),
            expected_case_revision: None,
        };
        let first = self.cells.submit_operation(request.clone()).await;
        let receipt = if self.first_submit {
            self.first_submit = false;
            assert_eq!(first.unwrap_err().code(), tonic::Code::Unavailable);
            let context = self.context(Some(&key), None);
            request.call.as_mut().unwrap().context = Some(context.clone());
            request.request.as_mut().unwrap().context = Some(context);
            self.cells
                .submit_operation(request.clone())
                .await
                .unwrap()
                .into_inner()
        } else {
            first.unwrap().into_inner()
        };
        assert_eq!(receipt.stage, base::ReceiptStage::Admitted as i32);
        assert_eq!(receipt.operation_revision, Some(1));
        assert!(receipt.journal_seq > 0);
        assert!(receipt.invocation_id.is_none() && receipt.host_state.is_none());
        let again = self
            .cells
            .submit_operation(request)
            .await
            .unwrap()
            .into_inner();
        assert_eq!(receipt, again);
        Id::new(receipt.operation_id).unwrap()
    }
    pub async fn reconcile(&mut self, operation: &Id) -> base::OperationView {
        self.operations
            .reconcile(base::OperationRef {
                context: Some(self.context(None, None)),
                operation_id: operation.to_string(),
            })
            .await
            .unwrap()
            .into_inner()
    }
    pub async fn get(&mut self, operation: &Id) -> base::OperationView {
        self.operations
            .get(base::OperationRef {
                context: Some(self.context(None, None)),
                operation_id: operation.to_string(),
            })
            .await
            .unwrap()
            .into_inner()
    }
    pub async fn shutdown(self) {
        let Self { stop, task, .. } = self;
        stop.send(()).unwrap();
        task.await.unwrap();
    }
}
