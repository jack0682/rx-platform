use super::*;
use rx_protocol::executor as wire;
const MAX_PAYLOAD_BYTES: usize = 1_000_000;
fn binding_hash() -> Vec<u8> {
    manifest_hash(include_str!("../../../../spec/executor/v1/binding.json"))
}
fn payload(bytes: Vec<u8>, reference: ArtifactRef) -> Result<Response<wire::ReadPayload>, Status> {
    use sha2::Digest as _;
    if bytes.len() > MAX_PAYLOAD_BYTES {
        return Err(Status::resource_exhausted(
            "execution read payload too large",
        ));
    }
    if reference.size_bytes.0 != bytes.len() as u64
        || reference.sha256 != Digest::from_bytes(sha2::Sha256::digest(&bytes).into())
    {
        return Err(Status::data_loss("artifact/reference mismatch"));
    }
    Ok(Response::new(wire::ReadPayload {
        reference: Some(base::ArtifactRef {
            sha256: reference.sha256.as_bytes().to_vec(),
            schema_id: reference.schema_id.to_string(),
            size_bytes: reference.size_bytes.0,
        }),
        payload: bytes,
    }))
}
#[tonic::async_trait]
impl wire::executor_read_service_server::ExecutorReadService for PlatformIngress {
    async fn get_snapshot(
        &self,
        request: Request<wire::SnapshotRequest>,
    ) -> Result<Response<wire::ReadPayload>, Status> {
        use sha2::Digest as _;
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        if value.binding_hash != binding_hash() {
            return Err(Status::failed_precondition(
                "executor read binding mismatch",
            ));
        }
        if value.visit == 0
            || value
                .context
                .as_ref()
                .is_some_and(|c| c.expected_revision.is_some())
        {
            return Err(Status::invalid_argument(
                "positive visit and read context required",
            ));
        }
        let Reply::ExecutionSnapshot(snapshot) = self
            .call(Command::ExecutionSnapshot {
                identity,
                run: id(&value.run_id)?,
                visit: Counter(value.visit),
            })
            .await?
        else {
            return Err(Status::internal("snapshot reply"));
        };
        let bytes = canonical::bytes(&snapshot)
            .map_err(|_| Status::resource_exhausted("execution snapshot exceeds supported size"))?;
        let reference = ArtifactRef {
            sha256: Digest::from_bytes(sha2::Sha256::digest(&bytes).into()),
            schema_id: snapshot.schema.clone(),
            size_bytes: Counter(bytes.len() as u64),
        };
        payload(bytes, reference)
    }
    async fn get_artifact(
        &self,
        request: Request<wire::ArtifactRequest>,
    ) -> Result<Response<wire::ReadPayload>, Status> {
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        if value.binding_hash != binding_hash() {
            return Err(Status::failed_precondition(
                "executor read binding mismatch",
            ));
        }
        if value
            .context
            .as_ref()
            .is_some_and(|c| c.expected_revision.is_some())
        {
            return Err(Status::invalid_argument("read context required"));
        }
        let reference = rx_protocol_adapter::artifact(
            value
                .reference
                .ok_or_else(|| Status::invalid_argument("artifact reference required"))?,
        )?;
        let Reply::ArtifactBytes(bytes) = self
            .call(Command::ExecutorArtifact {
                identity,
                run: id(&value.run_id)?,
                reference: reference.clone(),
            })
            .await?
        else {
            return Err(Status::internal("artifact reply"));
        };
        payload(bytes, reference)
    }
}
