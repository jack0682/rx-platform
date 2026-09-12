//! Process-context transport only. An RPC error is an unknown outcome, not NOT_APPLIED.
use super::*;
use rx_domain::host_configuration as data;
use rx_protocol::host_configuration as wire;
fn binding_hash() -> Vec<u8> {
    let value: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../spec/host-configuration/v1/binding.json"
    ))
    .expect("build binding");
    canonical::digest("RX-HOST-CONFIGURATION-BINDING-v1", &value)
        .expect("binding hash")
        .as_bytes()
        .to_vec()
}
fn decode(payload: wire::ConfigurationPayload, host: &Name) -> Result<data::Observation, Status> {
    use sha2::Digest as _;
    let r = payload
        .reference
        .ok_or_else(|| Status::data_loss("configuration reference missing"))?;
    if payload.payload.len() > 1_000_000
        || r.schema_id != "rx.host-process-configuration-observation.v1"
        || r.size_bytes != payload.payload.len() as u64
        || r.sha256 != sha2::Sha256::digest(&payload.payload).as_slice()
    {
        return Err(Status::data_loss("configuration observation integrity"));
    }
    let value: data::Observation = canonical::decode_json(&payload.payload)
        .map_err(|_| Status::data_loss("configuration observation shape"))?;
    value.validate().map_err(Status::data_loss)?;
    if &value.snapshot.host != host {
        return Err(Status::data_loss("configuration Host identity"));
    }
    Ok(value)
}
impl HostClient {
    pub async fn inspect_process_configuration(&self) -> Result<data::Observation, Status> {
        let mut context = self.context(&new_id());
        context.request_key = None;
        let v = wire::host_configuration_service_client::HostConfigurationServiceClient::new(
            self.channel.clone(),
        )
        .max_decoding_message_size(1_048_576)
        .inspect(wire::InspectConfiguration {
            context: Some(context),
            binding_hash: binding_hash(),
        })
        .await?
        .into_inner();
        decode(v, &self.host_id)
    }
    pub async fn lookup_process_configuration(&self, id: &Id) -> Result<data::Observation, Status> {
        let mut context = self.context(&new_id());
        context.request_key = None;
        let v = wire::host_configuration_service_client::HostConfigurationServiceClient::new(
            self.channel.clone(),
        )
        .max_decoding_message_size(1_048_576)
        .lookup(wire::LookupConfiguration {
            context: Some(context),
            request_id: id.to_string(),
            binding_hash: binding_hash(),
        })
        .await?
        .into_inner();
        let v = decode(v, &self.host_id)?;
        if v.receipt.as_ref().is_some_and(|r| &r.request.id != id) {
            return Err(Status::data_loss("lookup request identity differs"));
        }
        Ok(v)
    }
    /// Caller must durably save the full request before calling, and retain it after any error.
    pub async fn accept_process_configuration(
        &self,
        input: &data::Request,
    ) -> Result<data::Observation, Status> {
        use sha2::Digest as _;
        input.validate().map_err(Status::invalid_argument)?;
        if input.host != self.host_id {
            return Err(Status::invalid_argument("configuration Host differs"));
        }
        let bytes = canonical::bytes(input)
            .map_err(|_| Status::invalid_argument("configuration encode"))?;
        if bytes.len() > 1_000_000 {
            return Err(Status::resource_exhausted("configuration request size"));
        }
        let mut call = self.call(&input.cells[0].cell, &input.id);
        call.expected_cell_revision = None;
        let v = wire::host_configuration_service_client::HostConfigurationServiceClient::new(
            self.channel.clone(),
        )
        .max_decoding_message_size(1_048_576)
        .apply(wire::ApplyConfiguration {
            call: Some(call),
            binding_hash: binding_hash(),
            reference: Some(base::ArtifactRef {
                sha256: sha2::Sha256::digest(&bytes).to_vec(),
                schema_id: "rx.host-process-configuration-request.v1".into(),
                size_bytes: bytes.len() as u64,
            }),
            payload: bytes,
        })
        .await?
        .into_inner();
        let v = decode(v, &self.host_id)?;
        let r = v
            .receipt
            .as_ref()
            .ok_or_else(|| Status::data_loss("configuration receipt missing"))?;
        if r.request_digest != input.digest().map_err(Status::invalid_argument)? {
            return Err(Status::data_loss("configuration response correlation"));
        }
        Ok(v)
    }
}
