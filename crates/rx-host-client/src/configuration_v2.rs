//! Process-context transport only. An RPC error is an unknown outcome, not NOT_APPLIED.
use super::*;
use rx_process_contract::execution_v2::host_configuration as data;
use rx_protocol::host_execution_configuration as wire;
pub(super) fn binding_hash() -> Vec<u8> {
    let value: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../spec/host-configuration/v2/binding.json"
    ))
    .expect("build binding");
    canonical::digest("RX-HOST-EXECUTION-CONFIGURATION-BINDING-v2", &value)
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
        || r.schema_id != "rx.host-execution-configuration-observation.v2"
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
    pub async fn inspect_execution_configuration(&self) -> Result<data::Observation, Status> {
        let mut context = self.context(&new_id());
        context.request_key = None;
        let v = wire::host_execution_configuration_service_client::HostExecutionConfigurationServiceClient::new(
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
    pub async fn lookup_execution_configuration(
        &self,
        id: &Id,
    ) -> Result<data::Observation, Status> {
        let mut context = self.context(&new_id());
        context.request_key = None;
        let v = wire::host_execution_configuration_service_client::HostExecutionConfigurationServiceClient::new(
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
        if v.receipt
            .as_ref()
            .is_some_and(|r| &r.request.context.id != id)
        {
            return Err(Status::data_loss("lookup request identity differs"));
        }
        Ok(v)
    }
    /// Caller must durably save the full request before calling, and retain it after any error.
    pub async fn accept_execution_configuration(
        &self,
        input: &data::Request,
    ) -> Result<data::Observation, Status> {
        use sha2::Digest as _;
        input.validate().map_err(Status::invalid_argument)?;
        if input.context.host != self.host_id {
            return Err(Status::invalid_argument("configuration Host differs"));
        }
        let bytes = canonical::bytes(input)
            .map_err(|_| Status::invalid_argument("configuration encode"))?;
        if bytes.len() > 1_000_000 {
            return Err(Status::resource_exhausted("configuration request size"));
        }
        let mut call = self.call(&input.context.cells[0].cell, &input.context.id);
        call.expected_cell_revision = None;
        let v = wire::host_execution_configuration_service_client::HostExecutionConfigurationServiceClient::new(
            self.channel.clone(),
        )
        .max_decoding_message_size(1_048_576)
        .apply(wire::ApplyConfiguration {
            call: Some(call),
            binding_hash: binding_hash(),
            reference: Some(base::ArtifactRef {
                sha256: sha2::Sha256::digest(&bytes).to_vec(),
                schema_id: "rx.host-execution-configuration-request.v2".into(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use rx_domain::host_configuration as old;
    fn n(s: &str) -> Name {
        Name::new(s).unwrap()
    }
    fn id(i: u8) -> Id {
        Id::new(format!("00000000-0000-4000-8000-{i:012}")).unwrap()
    }
    fn payload() -> wire::ConfigurationPayload {
        use sha2::Digest as _;
        let observation = data::Observation {
            schema: n(data::OBSERVATION_SCHEMA),
            snapshot: old::Snapshot {
                schema: n("rx.host-process-configuration-snapshot.v1"),
                host: n("host/a"),
                host_boot: id(1),
                delivery_journal: id(2),
                binding_digest: Digest::from_bytes([3; 32]),
                cells: vec![old::CellObservation {
                    cell: n("cell/a"),
                    definition: Digest::from_bytes([4; 32]),
                    envelope: Digest::from_bytes([5; 32]),
                    environment: n("SIMULATION"),
                    epoch: Counter(1),
                    scopes: [(n("scope"), Counter(1))].into(),
                    blocked: vec![],
                    applied: None,
                }],
                evidence_journal: None,
                installation_identity: None,
                binding_commit: None,
            },
            policies: Default::default(),
            receipt: None,
            context_matches_current_host: false,
            activation_authorized: false,
        };
        let bytes = canonical::bytes(&observation).unwrap();
        wire::ConfigurationPayload {
            reference: Some(base::ArtifactRef {
                sha256: sha2::Sha256::digest(&bytes).to_vec(),
                schema_id: data::OBSERVATION_SCHEMA.into(),
                size_bytes: bytes.len() as u64,
            }),
            payload: bytes,
        }
    }
    #[test]
    fn decode_requires_v2_identity_integrity_and_does_not_infer_application() {
        let decoded = decode(payload(), &n("host/a")).unwrap();
        assert!(decoded.receipt.is_none());
        assert!(!decoded.context_matches_current_host);
        assert!(decode(payload(), &n("host/foreign")).is_err());
        let mut old = payload();
        old.reference.as_mut().unwrap().schema_id =
            "rx.host-process-configuration-observation.v1".into();
        assert!(decode(old, &n("host/a")).is_err());
        let mut tampered = payload();
        tampered.payload.push(b' ');
        assert!(decode(tampered, &n("host/a")).is_err());
        let mut forged = payload();
        forged.reference.as_mut().unwrap().sha256 = vec![0; 32];
        assert!(decode(forged, &n("host/a")).is_err());
    }
    #[derive(Clone)]
    struct NewHost;
    #[tonic::async_trait]
    impl wire::host_execution_configuration_service_server::HostExecutionConfigurationService
        for NewHost
    {
        async fn inspect(
            &self,
            r: tonic::Request<wire::InspectConfiguration>,
        ) -> Result<tonic::Response<wire::ConfigurationPayload>, Status> {
            assert_eq!(r.into_inner().binding_hash, binding_hash());
            Ok(tonic::Response::new(payload()))
        }
        async fn apply(
            &self,
            _: tonic::Request<wire::ApplyConfiguration>,
        ) -> Result<tonic::Response<wire::ConfigurationPayload>, Status> {
            Err(Status::unimplemented("read-only transport probe"))
        }
        async fn lookup(
            &self,
            _: tonic::Request<wire::LookupConfiguration>,
        ) -> Result<tonic::Response<wire::ConfigurationPayload>, Status> {
            Err(Status::unimplemented("read-only transport probe"))
        }
    }
    #[derive(Clone)]
    struct OldHost(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    #[tonic::async_trait]
    impl
        rx_protocol::host_configuration::host_configuration_service_server::HostConfigurationService
        for OldHost
    {
        async fn inspect(
            &self,
            _: tonic::Request<rx_protocol::host_configuration::InspectConfiguration>,
        ) -> Result<tonic::Response<rx_protocol::host_configuration::ConfigurationPayload>, Status>
        {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(Status::unimplemented("v1 probe reached"))
        }
        async fn apply(
            &self,
            _: tonic::Request<rx_protocol::host_configuration::ApplyConfiguration>,
        ) -> Result<tonic::Response<rx_protocol::host_configuration::ConfigurationPayload>, Status>
        {
            Err(Status::unimplemented("read-only probe"))
        }
        async fn lookup(
            &self,
            _: tonic::Request<rx_protocol::host_configuration::LookupConfiguration>,
        ) -> Result<tonic::Response<rx_protocol::host_configuration::ConfigurationPayload>, Status>
        {
            Err(Status::unimplemented("read-only probe"))
        }
    }
    #[tokio::test]
    async fn v2_transport_uses_its_own_service_and_does_not_fall_back_to_v1() {
        // Transport-only mock. No Host authority/native implementation or frozen binary claim.
        for v2 in [true, false] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            drop(listener);
            let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let old = OldHost(calls.clone());
            let (stop, stopped) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                tonic::transport::Server::builder()
                    .add_optional_service(v2.then(||wire::host_execution_configuration_service_server::HostExecutionConfigurationServiceServer::new(NewHost)))
                    .add_service(rx_protocol::host_configuration::host_configuration_service_server::HostConfigurationServiceServer::new(old))
                    .serve_with_shutdown(addr,async {let _=stopped.await;}).await.unwrap();
            });
            let endpoint = Channel::from_shared(format!("http://{addr}")).unwrap();
            let mut channel = None;
            for _ in 0..50 {
                match endpoint.clone().connect().await {
                    Ok(c) => {
                        channel = Some(c);
                        break;
                    }
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
                }
            }
            let client = HostClient {
                host_id: n("host/a"),
                session: base::Session {
                    session_id: id(9).to_string(),
                    ..Default::default()
                },
                channel: channel.expect("probe server"),
                transport_pin: None,
            };
            let result = client.inspect_execution_configuration().await;
            if v2 {
                assert!(result.is_ok());
            } else {
                assert_eq!(result.unwrap_err().code(), tonic::Code::Unimplemented);
            }
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
            let _ = stop.send(());
            server.await.unwrap();
        }
    }
}
