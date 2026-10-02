use super::*;
use rx_process_contract::execution_v2::executor as data;
use rx_protocol::execution_v2 as wire;
fn binding(value: &[u8]) -> Result<(), Status> {
    if value != data::binding_hash().as_bytes() {
        return Err(Status::failed_precondition("execution v2 binding mismatch"));
    }
    Ok(())
}
fn payload(schema: &str, bytes: Vec<u8>) -> Result<Response<wire::ExecutionPayload>, Status> {
    use sha2::Digest as _;
    if bytes.len() > 1_000_000 {
        return Err(Status::resource_exhausted("execution payload bound"));
    }
    Ok(Response::new(wire::ExecutionPayload {
        reference: Some(base::ArtifactRef {
            schema_id: schema.into(),
            sha256: sha2::Sha256::digest(&bytes).to_vec(),
            size_bytes: bytes.len() as u64,
        }),
        payload: bytes,
    }))
}
fn encoded<T: serde::Serialize>(
    schema: &str,
    value: &T,
) -> Result<Response<wire::ExecutionPayload>, Status> {
    payload(
        schema,
        canonical::bytes(value).map_err(|_| Status::internal("execution encode"))?,
    )
}
#[tonic::async_trait]
impl wire::execution_control_service_server::ExecutionControlService for PlatformIngress {
    async fn submit_node(
        &self,
        request: Request<wire::SubmitExecutionNode>,
    ) -> Result<Response<wire::ExecutionPayload>, Status> {
        let identity = self
            .validated_executor_identity(
                &request,
                request
                    .get_ref()
                    .call
                    .as_ref()
                    .and_then(|c| c.context.as_ref()),
            )
            .await?;
        let value = request.into_inner();
        binding(&value.binding_hash)?;
        let call = value
            .call
            .ok_or_else(|| Status::invalid_argument("cell call required"))?;
        let context = call
            .context
            .ok_or_else(|| Status::invalid_argument("context required"))?;
        if context.expected_revision.is_some()
            || value.expected_run == 0
            || call.expected_cell_revision.is_none_or(|r| r == 0)
        {
            return Err(Status::invalid_argument("body revisions required"));
        }
        let key = id(context
            .request_key
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("request key required"))?)?;
        let command = rx_application::execution_inventory::SubmitNode {
            cell: rx_protocol_adapter::name(&call.cell_id)?,
            run: id(&value.run_id)?,
            part: id(&value.part_id)?,
            node: rx_protocol_adapter::name(&value.node)?,
            mandate: id(&value.mandate_id)?,
            expected_cell: Counter(call.expected_cell_revision.unwrap()),
            expected_run: Counter(value.expected_run),
        };
        let Reply::ExecutionOperationPreparation(preparation) = self
            .call(Command::PrepareExecutionOperation {
                identity,
                key,
                command,
            })
            .await?
        else {
            return Err(Status::internal("node preparation reply"));
        };
        let ticket = match *preparation {
            rx_application::execution_inventory::OperationPreparation::Recorded(work) => {
                return encoded("rx.execution-work.v2", &work);
            }
            rx_application::execution_inventory::OperationPreparation::Compute(ticket) => ticket,
        };
        let prepared = tokio::task::spawn_blocking(move || {
            rx_application::execution_inventory::PreparedOperation::prepare(*ticket)
        })
        .await
        .map_err(|_| Status::unavailable("node computation unavailable"))?
        .map_err(Status::failed_precondition)?;
        let Reply::Work(work) = self
            .call(Command::CommitExecutionOperation(Box::new(prepared)))
            .await?
        else {
            return Err(Status::internal("node commit reply"));
        };
        encoded("rx.execution-work.v2", &work)
    }
    async fn negotiate(
        &self,
        request: Request<wire::NegotiateExecution>,
    ) -> Result<Response<wire::ExecutionPayload>, Status> {
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        binding(&value.binding_hash)?;
        if value
            .context
            .as_ref()
            .is_some_and(|c| c.expected_revision.is_some() || c.request_key.is_some())
        {
            return Err(Status::invalid_argument("negotiation context"));
        }
        let Reply::ExecutionSession(session) = self
            .call(Command::NegotiateExecutionSession {
                identity,
                cell: rx_protocol_adapter::name(&value.cell)?,
                binding: digest(&value.binding_hash)?,
            })
            .await?
        else {
            return Err(Status::internal("negotiation reply"));
        };
        encoded(data::SESSION_SCHEMA, &session)
    }
    async fn begin_part(
        &self,
        request: Request<wire::BeginExecutionPart>,
    ) -> Result<Response<wire::ExecutionPayload>, Status> {
        let identity = self
            .validated_executor_identity(
                &request,
                request
                    .get_ref()
                    .call
                    .as_ref()
                    .and_then(|c| c.context.as_ref()),
            )
            .await?;
        let value = request.into_inner();
        binding(&value.binding_hash)?;
        let call = value
            .call
            .ok_or_else(|| Status::invalid_argument("cell call required"))?;
        let context = call
            .context
            .ok_or_else(|| Status::invalid_argument("context required"))?;
        if context.expected_revision.is_some() || value.expected_budget == 0 {
            return Err(Status::invalid_argument("budget/body revision required"));
        }
        let key = id(context
            .request_key
            .as_deref()
            .ok_or_else(|| Status::invalid_argument("request key required"))?)?;
        let command = rx_application::BeginPartRequest {
            cell: rx_protocol_adapter::name(&call.cell_id)?,
            run: id(&value.run_id)?,
            mandate: id(&value.mandate_id)?,
            expected_budget: Counter(value.expected_budget),
            expected_cell: call.expected_cell_revision.map(Counter),
        };
        let Reply::ExecutionPartPreparation(preparation) = self
            .call(Command::PrepareExecutionPart {
                identity,
                key,
                command,
            })
            .await?
        else {
            return Err(Status::internal("Part preparation reply"));
        };
        let work = match *preparation {
            rx_application::execution_inventory::PartPreparation::Recorded(part) => {
                return encoded(data::PART_SCHEMA, &part);
            }
            rx_application::execution_inventory::PartPreparation::Compute(ticket) => ticket,
        };
        let prepared = tokio::task::spawn_blocking(move || {
            rx_application::execution_inventory::PreparedPart::prepare(*work)
        })
        .await
        .map_err(|_| Status::unavailable("Part computation unavailable"))?
        .map_err(Status::failed_precondition)?;
        let Reply::ExecutionPart(part) = self
            .call(Command::CommitExecutionPart(Box::new(prepared)))
            .await?
        else {
            return Err(Status::internal("Part commit reply"));
        };
        encoded(data::PART_SCHEMA, &part)
    }
    async fn get_snapshot(
        &self,
        request: Request<wire::ReadExecutionSnapshot>,
    ) -> Result<Response<wire::ExecutionPayload>, Status> {
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        binding(&value.binding_hash)?;
        if value.visit == 0
            || value
                .context
                .as_ref()
                .is_some_and(|c| c.expected_revision.is_some() || c.request_key.is_some())
        {
            return Err(Status::invalid_argument(
                "read context and actual visit required",
            ));
        }
        let Reply::ExecutionSnapshotV2(snapshot) = self
            .call(Command::GetExecutionSnapshotV2 {
                identity,
                run: id(&value.run_id)?,
                visit: Counter(value.visit),
            })
            .await?
        else {
            return Err(Status::internal("v2 snapshot reply"));
        };
        encoded(
            rx_process_contract::execution_v2::snapshot::SNAPSHOT_SCHEMA,
            &snapshot,
        )
    }
    async fn get_part(
        &self,
        request: Request<wire::ReadExecutionPart>,
    ) -> Result<Response<wire::ExecutionPayload>, Status> {
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        binding(&value.binding_hash)?;
        if value
            .context
            .as_ref()
            .is_some_and(|c| c.expected_revision.is_some() || c.request_key.is_some())
        {
            return Err(Status::invalid_argument("read context required"));
        }
        let Reply::ExecutionPart(part) = self
            .call(Command::GetExecutionPart {
                identity,
                run: id(&value.run_id)?,
                part: id(&value.part_id)?,
            })
            .await?
        else {
            return Err(Status::internal("Part read reply"));
        };
        encoded(data::PART_SCHEMA, &part)
    }
    async fn get_artifact(
        &self,
        request: Request<wire::ReadExecutionArtifact>,
    ) -> Result<Response<wire::ExecutionPayload>, Status> {
        let identity = self
            .validated_executor_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let value = request.into_inner();
        binding(&value.binding_hash)?;
        if value
            .context
            .as_ref()
            .is_some_and(|c| c.expected_revision.is_some() || c.request_key.is_some())
        {
            return Err(Status::invalid_argument("read context required"));
        }
        let r = value
            .reference
            .ok_or_else(|| Status::invalid_argument("artifact required"))?;
        let reference = ArtifactRef {
            schema_id: rx_protocol_adapter::name(&r.schema_id)?,
            sha256: digest(&r.sha256)?,
            size_bytes: Counter(r.size_bytes),
        };
        let Reply::ExecutionPartArtifact(bytes) = self
            .call(Command::GetExecutionPartArtifact {
                identity,
                run: id(&value.run_id)?,
                part: id(&value.part_id)?,
                reference: reference.clone(),
            })
            .await?
        else {
            return Err(Status::internal("artifact reply"));
        };
        payload(reference.schema_id.as_str(), bytes)
    }
}
