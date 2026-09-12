use super::*;
#[tonic::async_trait]
impl base::evidence_service_server::EvidenceService for PlatformIngress {
    async fn publish(
        &self,
        request: Request<base::EvidenceBatch>,
    ) -> Result<Response<base::DurableAck>, Status> {
        let identity = self
            .validated_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let batch = rx_protocol_adapter::evidence_batch(request.into_inner())?;
        let Reply::EvidenceCommit(commit) = self
            .call(Command::PublishEvidence { identity, batch })
            .await?
        else {
            return Err(Status::internal("evidence commit reply"));
        };
        Ok(Response::new(rx_protocol_adapter::durable_ack(commit)))
    }
    async fn get(
        &self,
        request: Request<base::EvidenceRef>,
    ) -> Result<Response<base::Evidence>, Status> {
        let identity = self
            .validated_identity(&request, request.get_ref().context.as_ref())
            .await?;
        let evidence = id(&request.into_inner().evidence_id)?;
        let Reply::NativeEvidence(value) = self
            .call(Command::GetNativeEvidence { identity, evidence })
            .await?
        else {
            return Err(Status::internal("evidence reply"));
        };
        Ok(Response::new(rx_protocol_adapter::evidence_wire(value)?))
    }
}
