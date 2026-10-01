//! Optional mTLS reporter ingress. It cannot impersonate a frozen Host or user session.
use super::*;
use rx_application::resident_reporting::ReporterIdentity;
use rx_protocol::resident_reporting as wire;
use serde::Serialize;
use sha2::Digest as _;

fn binding() -> Vec<u8> {
    sha2::Sha256::digest(include_bytes!(
        "../../../../spec/resident-reporting/v1/binding.json"
    ))
    .to_vec()
}
fn check_binding(value: &[u8]) -> Result<(), Status> {
    if value != binding() {
        return Err(Status::failed_precondition(
            "resident reporting binding differs",
        ));
    }
    Ok(())
}
fn payload(schema: &str, value: &impl Serialize) -> Result<Response<wire::Payload>, Status> {
    let data = canonical::bytes(value).map_err(|_| Status::internal("resident payload"))?;
    if data.len() > 65536 {
        return Err(Status::resource_exhausted("resident payload exceeds limit"));
    }
    Ok(Response::new(wire::Payload {
        schema: schema.into(),
        sha256: sha2::Sha256::digest(&data).to_vec(),
        data,
    }))
}
impl PlatformIngress {
    fn reporter_authentication(&self, fingerprint: Digest) -> Result<Digest, Status> {
        canonical::digest(
            "RX-RESIDENT-REPORTER-AUTH-v1",
            &(
                fingerprint,
                &self.configuration.installation,
                self.configuration.release_digest,
                binding(),
            ),
        )
        .map_err(|_| Status::internal("reporter authentication binding"))
    }
    fn reporter_identity<T>(
        &self,
        request: &Request<T>,
        session: &str,
    ) -> Result<ReporterIdentity, Status> {
        let (fingerprint, principal) = self.certificate(request)?;
        Ok(ReporterIdentity {
            principal,
            session: id(session)?,
            authentication_binding: self.reporter_authentication(fingerprint)?,
        })
    }
}
#[tonic::async_trait]
impl wire::resident_reporting_service_server::ResidentReportingService for PlatformIngress {
    async fn open(
        &self,
        request: Request<wire::OpenReporter>,
    ) -> Result<Response<wire::Payload>, Status> {
        let (fingerprint, principal) = self.certificate(&request)?;
        let hello = request.into_inner();
        check_binding(&hello.binding_hash)?;
        let meta = &self.configuration.installation;
        if hello.peer_id != principal.as_str()
            || hello.installation_id != meta.id.as_str()
            || hello.store_generation != meta.store_generation.as_str()
            || hello.shared_clock_id != meta.clock_id
            || digest(&hello.release_digest)? != self.configuration.release_digest
        {
            return Err(Status::failed_precondition(
                "reporter installation context differs",
            ));
        }
        let Reply::ResidentReporter(peer) = self
            .call(Command::OpenResidentReporter {
                principal,
                peer_boot: id(&hello.peer_boot)?,
                authentication_binding: self.reporter_authentication(fingerprint)?,
            })
            .await?
        else {
            return Err(Status::internal("reporter reply"));
        };
        payload("rx.resident-reporter.v1", &peer)
    }
    async fn inspect(
        &self,
        request: Request<wire::InspectScope>,
    ) -> Result<Response<wire::Payload>, Status> {
        let identity = self.reporter_identity(&request, &request.get_ref().session_id)?;
        let input = request.into_inner();
        check_binding(&input.binding_hash)?;
        let Reply::ResidentReportingScope(scope) = self
            .call(Command::InspectResidentReporting {
                identity,
                scope: id(&input.scope_id)?,
            })
            .await?
        else {
            return Err(Status::internal("reporting scope reply"));
        };
        payload("rx.resident-reporting-scope.v1", &scope)
    }
    async fn head(
        &self,
        request: Request<wire::ReadHead>,
    ) -> Result<Response<wire::Payload>, Status> {
        let identity = self.reporter_identity(&request, &request.get_ref().session_id)?;
        let input = request.into_inner();
        check_binding(&input.binding_hash)?;
        let Reply::ResidentReportHead(head) = self
            .call(Command::ResidentReportHead {
                identity,
                scope: id(&input.scope_id)?,
                instance: id(&input.instance_id)?,
            })
            .await?
        else {
            return Err(Status::internal("resident head reply"));
        };
        payload("rx.resident-report-head.v1", &head)
    }

    async fn publish(
        &self,
        request: Request<wire::PublishReport>,
    ) -> Result<Response<wire::Payload>, Status> {
        let identity = self.reporter_identity(&request, &request.get_ref().session_id)?;
        let input = request.into_inner();
        check_binding(&input.binding_hash)?;
        if input.payload.len() > 65536 {
            return Err(Status::resource_exhausted("resident payload exceeds limit"));
        }
        if digest(&input.payload_sha256)?
            != Digest::from_bytes(sha2::Sha256::digest(&input.payload).into())
        {
            return Err(Status::invalid_argument("resident payload digest differs"));
        }
        let report: rx_domain::resident_reporting::Report = canonical::decode_json(&input.payload)
            .map_err(|_| Status::invalid_argument("resident report schema"))?;
        if canonical::bytes(&report).map_err(|_| Status::invalid_argument("resident report"))?
            != input.payload
        {
            return Err(Status::invalid_argument(
                "resident report must use canonical representation",
            ));
        }
        let Reply::ResidentReportReceipt(receipt) = self
            .call(Command::PublishResidentReport {
                identity,
                key: id(&input.request_key)?,
                report,
            })
            .await?
        else {
            return Err(Status::internal("resident report reply"));
        };
        payload("rx.resident-report-receipt.v1", &receipt)
    }
}
