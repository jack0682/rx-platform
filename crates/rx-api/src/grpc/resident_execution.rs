//! Supervisor-only execution control ingress. Reports and Host sessions cannot impersonate it.
use super::*;
use rx_application::resident_execution::Identity;
use rx_protocol::resident_execution as wire;
use serde::{Serialize, de::DeserializeOwned};
use sha2::Digest as _;
const MAX: usize = 262_144;
fn binding() -> Vec<u8> {
    sha2::Sha256::digest(include_bytes!(
        "../../../../spec/resident-execution/v1/binding.json"
    ))
    .to_vec()
}
fn check_binding(value: &[u8]) -> Result<(), Status> {
    if value != binding() {
        return Err(Status::failed_precondition(
            "resident execution binding differs",
        ));
    }
    Ok(())
}
fn payload(schema: &str, value: &impl Serialize) -> Result<Response<wire::Payload>, Status> {
    let data = canonical::bytes(value).map_err(|_| Status::internal("execution payload"))?;
    if data.len() > MAX {
        return Err(Status::resource_exhausted(
            "execution payload exceeds limit",
        ));
    }
    Ok(Response::new(wire::Payload {
        schema: schema.into(),
        sha256: sha2::Sha256::digest(&data).to_vec(),
        data,
    }))
}
fn input<T: DeserializeOwned + Serialize>(v: &wire::Mutation) -> Result<T, Status> {
    check_binding(&v.binding_hash)?;
    if v.payload.len() > MAX {
        return Err(Status::resource_exhausted(
            "execution payload exceeds limit",
        ));
    }
    if digest(&v.payload_sha256)? != Digest::from_bytes(sha2::Sha256::digest(&v.payload).into()) {
        return Err(Status::invalid_argument("execution payload digest"));
    }
    let value = canonical::decode_json(&v.payload)
        .map_err(|_| Status::invalid_argument("execution value"))?;
    if canonical::bytes(&value).map_err(|_| Status::invalid_argument("execution value"))?
        != v.payload
    {
        return Err(Status::invalid_argument(
            "canonical execution value required",
        ));
    }
    Ok(value)
}
impl PlatformIngress {
    fn execution_authentication(&self, fingerprint: Digest) -> Result<Digest, Status> {
        canonical::digest(
            "RX-RESIDENT-EXECUTION-AUTH-v1",
            &(
                fingerprint,
                &self.configuration.installation,
                self.configuration.release_digest,
                binding(),
            ),
        )
        .map_err(|_| Status::internal("execution authentication"))
    }
    fn execution_identity<T>(
        &self,
        request: &Request<T>,
        session: &str,
    ) -> Result<Identity, Status> {
        let (fingerprint, principal) = self.certificate(request)?;
        Ok(Identity {
            principal,
            session: id(session)?,
            authentication_binding: self.execution_authentication(fingerprint)?,
        })
    }
}
#[tonic::async_trait]
impl wire::resident_execution_service_server::ResidentExecutionService for PlatformIngress {
    async fn open(
        &self,
        request: Request<wire::OpenSupervisor>,
    ) -> Result<Response<wire::Payload>, Status> {
        let (fingerprint, principal) = self.certificate(&request)?;
        let v = request.into_inner();
        check_binding(&v.binding_hash)?;
        let m = &self.configuration.installation;
        if v.peer_id != principal.as_str()
            || v.installation_id != m.id.as_str()
            || v.store_generation != m.store_generation.as_str()
            || v.shared_clock_id != m.clock_id
            || digest(&v.release_digest)? != self.configuration.release_digest
        {
            return Err(Status::failed_precondition(
                "execution installation context differs",
            ));
        }
        let Reply::ResidentSupervisor(peer) = self
            .call(Command::OpenResidentSupervisor {
                principal,
                peer_boot: id(&v.peer_boot)?,
                authentication_binding: self.execution_authentication(fingerprint)?,
                registry: digest(&v.registry_binding)?,
            })
            .await?
        else {
            return Err(Status::internal("execution peer reply"));
        };
        payload("rx.resident-execution-peer.v1", &peer)
    }
    async fn inspect(
        &self,
        request: Request<wire::InspectAssignment>,
    ) -> Result<Response<wire::Payload>, Status> {
        let identity = self.execution_identity(&request, &request.get_ref().session_id)?;
        let v = request.into_inner();
        check_binding(&v.binding_hash)?;
        let Reply::ResidentExecution(view) = self
            .call(Command::InspectResidentExecution {
                identity,
                id: id(&v.assignment_id)?,
            })
            .await?
        else {
            return Err(Status::internal("execution view reply"));
        };
        payload("rx.resident-execution-view.v1", &view)
    }
    async fn prepare(
        &self,
        request: Request<wire::Mutation>,
    ) -> Result<Response<wire::Payload>, Status> {
        let identity = self.execution_identity(&request, &request.get_ref().session_id)?;
        let v = request.into_inner();
        let Reply::ResidentExecutionContent(receipt) = self
            .call(Command::PrepareResidentExecution {
                identity,
                key: id(&v.request_key)?,
                input: input(&v)?,
            })
            .await?
        else {
            return Err(Status::internal("execution preparation reply"));
        };
        payload("rx.resident-execution-content.v1", &receipt)
    }
    async fn observe(
        &self,
        request: Request<wire::Mutation>,
    ) -> Result<Response<wire::Payload>, Status> {
        let identity = self.execution_identity(&request, &request.get_ref().session_id)?;
        let v = request.into_inner();
        let Reply::ResidentExecutionObservation(receipt) = self
            .call(Command::ObserveResidentExecution {
                identity,
                key: id(&v.request_key)?,
                input: input(&v)?,
            })
            .await?
        else {
            return Err(Status::internal("execution observation reply"));
        };
        payload("rx.resident-execution-observation.v1", &receipt)
    }
}
