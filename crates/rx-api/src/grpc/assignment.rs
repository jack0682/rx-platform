use super::*;
use rx_protocol::assignment as wire;
#[tonic::async_trait]
impl wire::executor_assignment_service_server::ExecutorAssignmentService for PlatformIngress {
    async fn inspect(
        &self,
        request: Request<wire::InspectCell>,
    ) -> Result<Response<rx_protocol::executor::ReadPayload>, Status> {
        use sha2::Digest as _;
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        if value.binding_hash
            != manifest_hash(include_str!("../../../../spec/assignment/v1/binding.json"))
        {
            return Err(Status::failed_precondition("assignment binding mismatch"));
        }
        let context = value
            .context
            .ok_or_else(|| Status::invalid_argument("context required"))?;
        if context.request_key.is_some() || context.expected_revision.is_some() {
            return Err(Status::invalid_argument("read context required"));
        }
        let Reply::ExecutorAssignment(view) = self
            .call(Command::ExecutorAssignment {
                identity,
                cell: rx_protocol_adapter::name(&value.cell_id)?,
            })
            .await?
        else {
            return Err(Status::internal("assignment reply"));
        };
        let payload = canonical::bytes(&view)
            .map_err(|_| Status::resource_exhausted("assignment payload"))?;
        if payload.len() > rx_process_contract::assignment::MAX_PAYLOAD {
            return Err(Status::resource_exhausted("assignment payload limit"));
        }
        Ok(Response::new(rx_protocol::executor::ReadPayload {
            reference: Some(base::ArtifactRef {
                sha256: sha2::Sha256::digest(&payload).to_vec(),
                schema_id: view.schema.to_string(),
                size_bytes: payload.len() as u64,
            }),
            payload,
        }))
    }
}
