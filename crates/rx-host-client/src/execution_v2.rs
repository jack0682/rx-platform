//! Operation-scoped v2 only. Never retry these calls through a v1 service.
use super::*;
use rx_protocol::host_execution as wire;
use wire::host_execution_service_client::HostExecutionServiceClient;
fn protocol() -> Vec<u8> {
    let value: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../spec/host-execution/v2/binding.json"
    ))
    .expect("binding manifest");
    canonical::digest("RX-HOST-EXECUTION-BINDING-v2", &value)
        .expect("static binding")
        .as_bytes()
        .to_vec()
}
fn binding(
    work: &app::Work,
) -> Result<&rx_process_contract::execution_v2::OperationBinding, Status> {
    let b = work
        .execution
        .as_deref()
        .ok_or_else(|| Status::invalid_argument("v2 work required"))?;
    b.validate().map_err(Status::invalid_argument)?;
    if b.operation != *work.operation.id()
        || b.selection.run != work.run
        || work.part.as_ref() != Some(&b.selection.part)
        || b.selection.intent_digest
            != work
                .intent
                .digest()
                .map_err(|e| Status::invalid_argument(e.to_string()))?
    {
        return Err(Status::invalid_argument("v2 Work binding differs"));
    }
    Ok(b)
}
fn binding_digest(work: &app::Work) -> Result<Vec<u8>, Status> {
    Ok(binding(work)?
        .digest()
        .map_err(Status::invalid_argument)?
        .as_bytes()
        .to_vec())
}
fn bound_receipt(
    value: wire::ExecutionReceipt,
    work: &app::Work,
) -> Result<app::HostReceipt, Status> {
    if value.operation_binding != binding_digest(work)? {
        return Err(Status::data_loss("v2 receipt binding differs"));
    }
    let r = receipt(
        value
            .receipt
            .ok_or_else(|| Status::data_loss("v2 receipt missing"))?,
    )?;
    if r.operation != *work.operation.id() || r.digest != work.operation.intent_digest() {
        return Err(Status::data_loss("v2 receipt operation differs"));
    }
    Ok(r)
}
impl HostClient {
    pub async fn prepare_execution(
        &self,
        key: &Id,
        work: &app::Work,
        permit: &app::Permit,
        parameters: &[u8],
    ) -> Result<app::HostReceipt, Status> {
        use sha2::Digest as _;
        let b = binding(work)?;
        let r = &b.selection.parameter;
        if parameters.len() as u64 != r.size_bytes.0
            || parameters.len() as u64 > rx_process_contract::execution_v2::MAX_PARAMETER_BYTES
            || sha2::Sha256::digest(parameters).as_slice() != r.sha256.as_bytes()
            || b.mandate != permit.mandate
            || permit.operation != b.operation
            || permit.intent_digest != b.selection.intent_digest
        {
            return Err(Status::invalid_argument(
                "v2 parameter/permit binding differs",
            ));
        }
        let bytes = canonical::bytes(b).map_err(|e| Status::invalid_argument(e.to_string()))?;
        let value = HostExecutionServiceClient::new(self.channel.clone())
            .prepare(wire::PrepareExecution {
                request: Some(self.prepare_request(key, work, permit)?),
                binding_hash: protocol(),
                reference: Some(base::ArtifactRef {
                    schema_id: rx_process_contract::execution_v2::OPERATION_SCHEMA.into(),
                    sha256: sha2::Sha256::digest(&bytes).to_vec(),
                    size_bytes: bytes.len() as u64,
                }),
                payload: bytes,
                parameters: parameters.to_vec(),
            })
            .await?
            .into_inner();
        bound_receipt(value, work)
    }
    pub(super) async fn authorize_execution(
        &self,
        key: &Id,
        work: &app::Work,
        permit: &app::Permit,
    ) -> Result<app::HostReceipt, Status> {
        let b = binding(work)?;
        if b.mandate != permit.mandate
            || permit.operation != b.operation
            || permit.intent_digest != b.selection.intent_digest
        {
            return Err(Status::invalid_argument("v2 permit binding differs"));
        }
        let value = HostExecutionServiceClient::new(self.channel.clone())
            .authorize(wire::AuthorizeExecution {
                request: Some(self.authorize_request(key, work, permit)?),
                binding_hash: protocol(),
                operation_binding: binding_digest(work)?,
            })
            .await?
            .into_inner();
        bound_receipt(value, work)
    }
    fn execution_query(&self, work: &app::Work) -> Result<wire::ReadExecution, Status> {
        let mut context = self.context(&new_id());
        context.request_key = None;
        Ok(wire::ReadExecution {
            request: Some(base::OperationRef {
                context: Some(context),
                operation_id: work.operation.id().to_string(),
            }),
            binding_hash: protocol(),
            operation_binding: binding_digest(work)?,
        })
    }
    pub async fn work_receipt(&self, work: &app::Work) -> Result<app::HostReceipt, Status> {
        if work.execution.is_none() {
            return self.receipt(work.operation.id()).await;
        }
        let value = HostExecutionServiceClient::new(self.channel.clone())
            .get_receipt(self.execution_query(work)?)
            .await?
            .into_inner();
        bound_receipt(value, work)
    }
    pub async fn work_evidence(&self, work: &app::Work) -> Result<app::EvidenceBatch, Status> {
        if work.execution.is_none() {
            return self.reconcile(work.operation.id()).await;
        }
        let value = HostExecutionServiceClient::new(self.channel.clone())
            .reconcile(self.execution_query(work)?)
            .await?
            .into_inner();
        if value.operation_binding != binding_digest(work)? {
            return Err(Status::data_loss("v2 evidence binding differs"));
        }
        rx_protocol_adapter::evidence_batch(
            value
                .batch
                .ok_or_else(|| Status::data_loss("v2 evidence missing"))?,
        )
    }
}

#[cfg(test)]
#[path = "execution_v2_tests.rs"]
mod tests;
