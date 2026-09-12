//! Authenticated platform peer ingress. Admission and dispatch decisions remain in the writer.
mod assignment;
mod cell_negotiation;
mod evidence;
mod execution_read;
mod executor_plan;
mod operation;
mod production;
mod session;
mod workflow;
use rx_application::{Identity, Installation};
use rx_domain::{canonical, types::*};
use rx_ports::StoreError;
use rx_protocol::{base, cell};
use rx_protocol_adapter::{digest, id};
use rx_runtime::{
    application::{ApplicationPort, Command, Reply},
    writer::WriterError,
};
use std::{collections::BTreeMap, sync::Arc};
use tonic::{
    Request, Response, Status,
    transport::{Certificate, Identity as TlsIdentity, Server, ServerTlsConfig},
};

#[derive(Clone)]
pub struct Configuration {
    pub installation: Installation,
    pub release_digest: Digest,
    pub allowed_certificates: BTreeMap<Digest, Name>,
}
pub struct TlsMaterial {
    pub server_certificate_pem: Vec<u8>,
    pub server_key_pem: Vec<u8>,
    pub client_ca_pem: Vec<u8>,
}
#[derive(Clone)]
pub struct PlatformIngress {
    runtime: Arc<dyn ApplicationPort>,
    configuration: Arc<Configuration>,
}
/// Compatibility name for the previously evidence-only ingress.
pub type EvidenceIngress = PlatformIngress;
impl PlatformIngress {
    pub async fn new(
        runtime: Arc<dyn ApplicationPort>,
        configuration: Configuration,
    ) -> Result<Self, Status> {
        if configuration.allowed_certificates.is_empty() {
            return Err(Status::invalid_argument(
                "registered service certificates required",
            ));
        }
        let Reply::Installation(actual) = runtime
            .request(Command::Installation)
            .await
            .map_err(failure)?
        else {
            return Err(Status::internal("installation reply"));
        };
        let expected = &configuration.installation;
        if actual.id != expected.id
            || actual.store_generation != expected.store_generation
            || actual.runtime_boot != expected.runtime_boot
            || actual.clock_id != expected.clock_id
        {
            return Err(Status::failed_precondition(
                "ingress installation is not the writer's current incarnation",
            ));
        }
        Ok(Self {
            runtime,
            configuration: Arc::new(configuration),
        })
    }
    pub async fn serve(
        self,
        listener: tokio::net::TcpListener,
        tls: TlsMaterial,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Result<(), tonic::transport::Error> {
        Server::builder()
            .tls_config(
                ServerTlsConfig::new()
                    .identity(TlsIdentity::from_pem(
                        tls.server_certificate_pem,
                        tls.server_key_pem,
                    ))
                    .client_ca_root(Certificate::from_pem(tls.client_ca_pem))
                    .client_auth_optional(false),
            )?
            .add_service(
                base::session_service_server::SessionServiceServer::new(self.clone())
                    .max_decoding_message_size(1_048_576)
                    .max_encoding_message_size(1_048_576),
            )
            .add_service(
                cell::cell_service_server::CellServiceServer::new(self.clone())
                    .max_decoding_message_size(1_048_576)
                    .max_encoding_message_size(1_048_576),
            )
            .add_service(
                base::workflow_service_server::WorkflowServiceServer::new(self.clone())
                    .max_decoding_message_size(1_048_576)
                    .max_encoding_message_size(1_048_576),
            )
            .add_service(
                base::operation_service_server::OperationServiceServer::new(self.clone())
                    .max_decoding_message_size(1_048_576)
                    .max_encoding_message_size(1_048_576),
            )
            .add_service(
                rx_protocol::executor::executor_read_service_server::ExecutorReadServiceServer::new(self.clone())
                    .max_decoding_message_size(1_048_576).max_encoding_message_size(1_048_576),
            )
            .add_service(
                rx_protocol::executor_plan::executor_plan_service_server::ExecutorPlanServiceServer::new(self.clone())
                    .max_decoding_message_size(1_048_576).max_encoding_message_size(1_048_576),
            )
            .add_service(rx_protocol::production::production_service_server::ProductionServiceServer::new(self.clone()).max_decoding_message_size(1_048_576).max_encoding_message_size(1_048_576))
            .add_service(rx_protocol::assignment::executor_assignment_service_server::ExecutorAssignmentServiceServer::new(self.clone()).max_decoding_message_size(1_048_576).max_encoding_message_size(1_048_576))
            .add_service(
                base::evidence_service_server::EvidenceServiceServer::new(self)
                    .max_decoding_message_size(1_048_576)
                    .max_encoding_message_size(1_048_576),
            )
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                shutdown,
            )
            .await
    }
    fn certificate<T>(&self, request: &Request<T>) -> Result<(Digest, Name), Status> {
        use sha2::Digest as _;
        use tonic::transport::server::{TcpConnectInfo, TlsConnectInfo};
        let connection = request
            .extensions()
            .get::<TlsConnectInfo<TcpConnectInfo>>()
            .ok_or_else(|| Status::unauthenticated("mTLS required"))?;
        let certificates = connection
            .peer_certs()
            .ok_or_else(|| Status::unauthenticated("service certificate required"))?;
        let leaf = certificates
            .first()
            .ok_or_else(|| Status::unauthenticated("certificate missing"))?;
        let fingerprint = Digest::from_bytes(sha2::Sha256::digest(leaf.as_ref()).into());
        let principal = self
            .configuration
            .allowed_certificates
            .get(&fingerprint)
            .cloned()
            .ok_or_else(|| Status::permission_denied("certificate is not registered"))?;
        Ok((fingerprint, principal))
    }
    fn identity<T>(
        &self,
        request: &Request<T>,
        context: Option<&base::CallContext>,
    ) -> Result<Identity, Status> {
        let (_, principal) = self.certificate(request)?;
        let context = context.ok_or_else(|| Status::invalid_argument("CallContext required"))?;
        id(&context.call_id)?;
        if let Some(key) = &context.request_key {
            id(key)?;
        }
        Ok(Identity {
            principal,
            session: id(&context.session_id)?,
            terminal: None,
        })
    }
    fn authentication_binding(&self, fingerprint: Digest) -> Result<Digest, Status> {
        let meta = &self.configuration.installation;
        canonical::digest(
            "RX-PRODUCER-AUTH-v1",
            &(
                fingerprint,
                meta.id.clone(),
                meta.store_generation.clone(),
                self.configuration.release_digest,
            ),
        )
        .map_err(|_| Status::invalid_argument("authentication binding"))
    }
    fn executor_authentication_binding(&self, fingerprint: Digest) -> Result<Digest, Status> {
        let meta = &self.configuration.installation;
        canonical::digest(
            "RX-EXECUTOR-AUTH-v1",
            &(
                fingerprint,
                meta.id.clone(),
                meta.store_generation.clone(),
                self.configuration.release_digest,
            ),
        )
        .map_err(|_| Status::invalid_argument("executor authentication binding"))
    }
    fn operator_authentication_binding(&self, fingerprint: Digest) -> Result<Digest, Status> {
        let meta = &self.configuration.installation;
        canonical::digest(
            "RX-OPERATOR-API-AUTH-v1",
            &(
                fingerprint,
                &meta.id,
                &meta.store_generation,
                self.configuration.release_digest,
            ),
        )
        .map_err(|_| Status::invalid_argument("operator authentication binding"))
    }
    async fn validated_cell_peer_identity<T>(
        &self,
        request: &Request<T>,
        context: Option<&base::CallContext>,
    ) -> Result<Identity, Status> {
        let (fingerprint, _) = self.certificate(request)?;
        let identity = self.identity(request, context)?;
        let Reply::ServicePeer(peer) = self
            .call(Command::InspectServicePeer(identity.clone()))
            .await?
        else {
            return Err(Status::unauthenticated("service session required"));
        };
        let (actual, expected) = match peer {
            rx_application::ServicePeer::Executor(p) => (
                p.authentication_binding,
                self.executor_authentication_binding(fingerprint)?,
            ),
            rx_application::ServicePeer::Host(p) => (
                p.authentication_binding,
                self.authentication_binding(fingerprint)?,
            ),
            rx_application::ServicePeer::OperatorApi(p) => (
                p.authentication_binding,
                self.operator_authentication_binding(fingerprint)?,
            ),
        };
        if actual != expected {
            return Err(Status::unauthenticated("session/certificate mismatch"));
        }
        Ok(identity)
    }
    async fn validated_executor_identity<T>(
        &self,
        request: &Request<T>,
        context: Option<&base::CallContext>,
    ) -> Result<Identity, Status> {
        let (fingerprint, _) = self.certificate(request)?;
        let identity = self.identity(request, context)?;
        let Reply::ServicePeer(rx_application::ServicePeer::Executor(peer)) = self
            .call(Command::InspectServicePeer(identity.clone()))
            .await?
        else {
            return Err(Status::permission_denied("executor session required"));
        };
        if peer.authentication_binding != self.executor_authentication_binding(fingerprint)? {
            return Err(Status::unauthenticated("session/certificate mismatch"));
        }
        Ok(identity)
    }
    async fn validated_identity<T>(
        &self,
        request: &Request<T>,
        context: Option<&base::CallContext>,
    ) -> Result<Identity, Status> {
        if context.is_some_and(|c| c.expected_revision.is_some()) {
            return Err(Status::invalid_argument(
                "producer call does not accept entity revision",
            ));
        }
        let (fingerprint, _) = self.certificate(request)?;
        let identity = self.identity(request, context)?;
        let Reply::Producer(producer) = self
            .call(Command::InspectEvidenceProducer(identity.clone()))
            .await?
        else {
            return Err(Status::internal("producer reply"));
        };
        if producer.authentication_binding != self.authentication_binding(fingerprint)? {
            return Err(Status::unauthenticated("session/certificate mismatch"));
        }
        Ok(identity)
    }
    async fn call(&self, command: Command) -> Result<Reply, Status> {
        self.runtime.request(command).await.map_err(failure)
    }
}
fn failure(error: WriterError<StoreError>) -> Status {
    use rx_domain::fault::Rejection as R;
    match error {
        WriterError::Busy => Status::resource_exhausted("BUSY"),
        WriterError::Rejected(StoreError::Rejected(R::Unauthenticated)) => {
            Status::unauthenticated("UNAUTHENTICATED")
        }
        WriterError::Rejected(StoreError::Rejected(R::Forbidden)) => {
            Status::permission_denied("FORBIDDEN")
        }
        WriterError::Rejected(StoreError::Rejected(R::NotFound)) => Status::not_found("NOT_FOUND"),
        WriterError::Rejected(StoreError::Rejected(R::StaleRevision)) => {
            Status::aborted("STALE_REVISION")
        }
        WriterError::Rejected(StoreError::Rejected(R::InvalidInput)) => {
            Status::invalid_argument("INVALID_INPUT")
        }
        WriterError::Rejected(StoreError::Rejected(_)) => {
            Status::failed_precondition("service or cell precondition failed")
        }
        WriterError::Rejected(StoreError::Invalid(message))
            if message.starts_with("GAP: next expected ") =>
        {
            let mut result = Status::failed_precondition("GAP");
            if let Some(next) = message
                .strip_prefix("GAP: next expected ")
                .and_then(|s| s.parse::<u64>().ok())
                && let Ok(value) = next.to_string().parse()
            {
                result.metadata_mut().insert("rx-next-expected", value);
            }
            result
        }
        WriterError::Rejected(StoreError::Invalid(_)) => Status::invalid_argument("INVALID_INPUT"),
        WriterError::Rejected(StoreError::Integrity(_)) => {
            Status::data_loss("INTEGRITY_CONFLICT_OR_CONTINUITY_UNPROVEN")
        }
        WriterError::Rejected(StoreError::KeyConflict) => Status::already_exists("KEY_CONFLICT"),
        WriterError::Rejected(StoreError::RevisionConflict(_) | StoreError::OutboxConflict) => {
            Status::aborted("REVISION_OR_DELIVERY_CONFLICT")
        }
        _ => Status::unavailable("no durable response; reconcile before retrying a mutation"),
    }
}
fn base_hash() -> Vec<u8> {
    manifest_hash(include_str!(
        "../../../../spec/contracts/v1.0/protocol_manifest.json"
    ))
}
fn cell_hash() -> Vec<u8> {
    manifest_hash(include_str!(
        "../../../../spec/cell_operations/v1.0/protocol_manifest.json"
    ))
}
fn manifest_hash(source: &str) -> Vec<u8> {
    use sha2::Digest as _;
    let value: serde_json::Value =
        serde_json::from_str(source).expect("embedded contract manifest");
    sha2::Sha256::digest(canonical::bytes(&value).expect("manifest JCS")).to_vec()
}
