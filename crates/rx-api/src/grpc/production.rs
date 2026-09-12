use super::*;
use rx_protocol::production as wire;
fn binding() -> Vec<u8> {
    manifest_hash(include_str!("../../../../spec/production/v1/binding.json"))
}
#[tonic::async_trait]
impl wire::production_service_server::ProductionService for PlatformIngress {
    async fn inspect(
        &self,
        request: Request<wire::InspectRun>,
    ) -> Result<Response<rx_protocol::executor::ReadPayload>, Status> {
        use sha2::Digest as _;
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        if value.binding_hash != binding() {
            return Err(Status::failed_precondition("production binding mismatch"));
        }
        let context = value
            .context
            .ok_or_else(|| Status::invalid_argument("context required"))?;
        if context.request_key.is_some() || context.expected_revision.is_some() {
            return Err(Status::invalid_argument("read context required"));
        }
        let Reply::ProductionView(view) = self
            .call(Command::ProductionView {
                identity,
                run: id(&value.run_id)?,
            })
            .await?
        else {
            return Err(Status::internal("production view reply"));
        };
        let payload = canonical::bytes(&view)
            .map_err(|_| Status::resource_exhausted("production view size"))?;
        if payload.len() > 1_000_000 {
            return Err(Status::resource_exhausted("production view limit"));
        }
        let reference = base::ArtifactRef {
            sha256: sha2::Sha256::digest(&payload).to_vec(),
            schema_id: view.schema.to_string(),
            size_bytes: payload.len() as u64,
        };
        Ok(Response::new(rx_protocol::executor::ReadPayload {
            reference: Some(reference),
            payload,
        }))
    }
    async fn complete_part(
        &self,
        request: Request<wire::CompletePart>,
    ) -> Result<Response<cell::PartAttempt>, Status> {
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        if value.binding_hash != binding() {
            return Err(Status::failed_precondition("production binding mismatch"));
        }
        let context = value
            .context
            .ok_or_else(|| Status::invalid_argument("context required"))?;
        if context.expected_revision.is_some()
            || value.expected_run_revision == 0
            || value.expected_part_revision == 0
        {
            return Err(Status::invalid_argument("body revisions required"));
        }
        let key = id(context
            .request_key
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("request key required"))?)?;
        let command = rx_application::ProductionCompletePart {
            cell: rx_protocol_adapter::name(&value.cell_id)?,
            run: id(&value.run_id)?,
            part: id(&value.part_attempt_id)?,
            expected_run: Counter(value.expected_run_revision),
            expected_part: Counter(value.expected_part_revision),
        };
        let Reply::PartSnapshot(part) = self
            .call(Command::ExecutorCompletePart {
                identity,
                key,
                command,
            })
            .await?
        else {
            return Err(Status::internal("part response"));
        };
        let response = rx_protocol_adapter::workflow::part_view(&part);
        rx_protocol::json::to_value(&response)?;
        Ok(Response::new(response))
    }
}
