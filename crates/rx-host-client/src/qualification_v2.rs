//! Explicit execution-v2 qualification transport. Errors never imply acceptance.
use super::*;
use rx_process_contract::execution_v2::host_qualification as data;
use rx_protocol::host_execution_qualification as wire;
pub(super) fn binding_hash() -> Vec<u8> {
    let value: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../spec/host-qualification/v2/binding.json"
    ))
    .expect("build binding");
    canonical::digest("RX-HOST-EXECUTION-QUALIFICATION-BINDING-v2", &value)
        .expect("binding hash")
        .as_bytes()
        .to_vec()
}
fn decode(payload: wire::QualificationPayload, host: &Name) -> Result<data::Observation, Status> {
    use sha2::Digest as _;
    let r = payload
        .reference
        .ok_or_else(|| Status::data_loss("qualification reference missing"))?;
    if payload.payload.len() > 1_000_000
        || r.schema_id != "rx.host-execution-qualification-observation.v2"
        || r.size_bytes != payload.payload.len() as u64
        || r.sha256 != sha2::Sha256::digest(&payload.payload).as_slice()
    {
        return Err(Status::data_loss("qualification observation integrity"));
    }
    let value: data::Observation = canonical::decode_json(&payload.payload)
        .map_err(|_| Status::data_loss("qualification observation shape"))?;
    value.validate().map_err(Status::data_loss)?;
    if &value.snapshot.host != host {
        return Err(Status::data_loss("qualification Host identity"));
    }
    Ok(value)
}
impl HostClient {
    pub async fn inspect_execution_qualification(&self) -> Result<data::Observation, Status> {
        let mut context = self.context(&new_id());
        context.request_key = None;
        let v = wire::host_execution_qualification_service_client::HostExecutionQualificationServiceClient::new(
            self.channel.clone(),
        )
        .max_decoding_message_size(1_048_576)
        .inspect(wire::InspectQualification {
            context: Some(context),
            binding_hash: binding_hash(),
        })
        .await?
        .into_inner();
        decode(v, &self.host_id)
    }
    pub async fn lookup_execution_qualification(
        &self,
        id: &Id,
    ) -> Result<data::Observation, Status> {
        let mut context = self.context(&new_id());
        context.request_key = None;
        let v = wire::host_execution_qualification_service_client::HostExecutionQualificationServiceClient::new(
            self.channel.clone(),
        )
        .max_decoding_message_size(1_048_576)
        .lookup(wire::LookupQualification {
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
    pub async fn accept_execution_qualification(
        &self,
        input: &data::Request,
    ) -> Result<data::Observation, Status> {
        use sha2::Digest as _;
        input.validate().map_err(Status::invalid_argument)?;
        if input.context.host != self.host_id {
            return Err(Status::invalid_argument("qualification Host differs"));
        }
        let bytes = canonical::bytes(input)
            .map_err(|_| Status::invalid_argument("qualification encode"))?;
        if bytes.len() > 1_000_000 {
            return Err(Status::resource_exhausted("qualification request size"));
        }
        let mut call = self.call(&input.context.cells[0].cell, &input.context.id);
        call.expected_cell_revision = None;
        let v = wire::host_execution_qualification_service_client::HostExecutionQualificationServiceClient::new(
            self.channel.clone(),
        )
        .max_decoding_message_size(1_048_576)
        .accept(wire::AcceptQualification {
            call: Some(call),
            binding_hash: binding_hash(),
            reference: Some(base::ArtifactRef {
                sha256: sha2::Sha256::digest(&bytes).to_vec(),
                schema_id: "rx.host-execution-qualification-request.v2".into(),
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
            .ok_or_else(|| Status::data_loss("qualification receipt missing"))?;
        if r.request_digest != input.digest().map_err(Status::invalid_argument)? {
            return Err(Status::data_loss("qualification response correlation"));
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(dead_code)]
    mod fixture {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../rx-process-contract/tests/support/execution_v2_fixture.rs"
        ));
    }
    fn payload(observation: data::Observation) -> wire::QualificationPayload {
        use sha2::Digest as _;
        let bytes = canonical::bytes(&observation).unwrap();
        wire::QualificationPayload {
            reference: Some(base::ArtifactRef {
                sha256: sha2::Sha256::digest(&bytes).to_vec(),
                schema_id: data::OBSERVATION_SCHEMA.into(),
                size_bytes: bytes.len() as u64,
            }),
            payload: bytes,
        }
    }
    #[test]
    fn v2_qualification_decode_preserves_absence_and_rejects_foreign_or_tampered_facts() {
        let (mut observation, _) = fixture::qualification_v2_observation();
        decode(payload(observation.clone()), &fixture::n("host")).unwrap();
        assert!(decode(payload(observation.clone()), &fixture::n("other")).is_err());
        let mut value = payload(observation.clone());
        value.reference.as_mut().unwrap().schema_id = "rx.host-qualification-observation.v1".into();
        assert!(decode(value, &fixture::n("host")).is_err());
        let mut value = payload(observation.clone());
        value.payload.push(b' ');
        assert!(decode(value, &fixture::n("host")).is_err());
        observation.receipt = None;
        observation.receipt_matches_current_host = false;
        assert!(
            decode(payload(observation), &fixture::n("host"))
                .unwrap()
                .receipt
                .is_none()
        );
    }
    #[derive(Clone)]
    struct NewHost;
    #[tonic::async_trait]
    impl wire::host_execution_qualification_service_server::HostExecutionQualificationService
        for NewHost
    {
        async fn inspect(
            &self,
            r: tonic::Request<wire::InspectQualification>,
        ) -> Result<tonic::Response<wire::QualificationPayload>, Status> {
            assert_eq!(r.into_inner().binding_hash, binding_hash());
            Ok(tonic::Response::new(payload(
                fixture::qualification_v2_observation().0,
            )))
        }
        async fn lookup(
            &self,
            r: tonic::Request<wire::LookupQualification>,
        ) -> Result<tonic::Response<wire::QualificationPayload>, Status> {
            let input = r.into_inner();
            assert_eq!(input.binding_hash, binding_hash());
            // Deliberately return the fixed receipt even for a foreign lookup; client must reject.
            Ok(tonic::Response::new(payload(
                fixture::qualification_v2_observation().0,
            )))
        }
        async fn accept(
            &self,
            r: tonic::Request<wire::AcceptQualification>,
        ) -> Result<tonic::Response<wire::QualificationPayload>, Status> {
            use sha2::Digest as _;
            let input = r.into_inner();
            assert_eq!(input.binding_hash, binding_hash());
            let reference = input.reference.unwrap();
            assert_eq!(reference.schema_id, data::REQUEST_SCHEMA);
            assert_eq!(reference.size_bytes, input.payload.len() as u64);
            assert_eq!(
                reference.sha256,
                sha2::Sha256::digest(&input.payload).to_vec()
            );
            let request: data::Request = canonical::decode_json(&input.payload).unwrap();
            let (observed, configured) = fixture::qualification_v2_observation();
            request.matches_configuration(&configured).unwrap();
            // Return the fixed valid receipt; foreign request correlation is checked by the client.
            Ok(tonic::Response::new(payload(observed)))
        }
    }
    #[derive(Clone)]
    struct OldHost(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    #[tonic::async_trait]
    impl
        rx_protocol::host_qualification::host_qualification_service_server::HostQualificationService
        for OldHost
    {
        async fn inspect(
            &self,
            _: tonic::Request<rx_protocol::host_qualification::InspectQualification>,
        ) -> Result<tonic::Response<rx_protocol::host_qualification::QualificationPayload>, Status>
        {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(Status::unimplemented("v1 probe reached"))
        }
        async fn accept(
            &self,
            _: tonic::Request<rx_protocol::host_qualification::AcceptQualification>,
        ) -> Result<tonic::Response<rx_protocol::host_qualification::QualificationPayload>, Status>
        {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(Status::unimplemented("v1 probe reached"))
        }
        async fn lookup(
            &self,
            _: tonic::Request<rx_protocol::host_qualification::LookupQualification>,
        ) -> Result<tonic::Response<rx_protocol::host_qualification::QualificationPayload>, Status>
        {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(Status::unimplemented("v1 probe reached"))
        }
    }
    #[tokio::test]
    async fn v2_qualification_transport_never_falls_back_and_correlates_original_request() {
        // Plain transport mock, not authentication, native Host or frozen-binary evidence.
        for v2 in [true, false] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            drop(listener);
            let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let old = OldHost(calls.clone());
            let (stop, stopped) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                tonic::transport::Server::builder()
                    .add_optional_service(v2.then(|| wire::host_execution_qualification_service_server::HostExecutionQualificationServiceServer::new(NewHost)))
                    .add_service(rx_protocol::host_qualification::host_qualification_service_server::HostQualificationServiceServer::new(old))
                    .serve_with_shutdown(address, async { let _ = stopped.await; }).await.unwrap();
            });
            let endpoint = Channel::from_shared(format!("http://{address}")).unwrap();
            let mut channel = None;
            for _ in 0..50 {
                match endpoint.clone().connect().await {
                    Ok(value) => {
                        channel = Some(value);
                        break;
                    }
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
                }
            }
            let client = HostClient {
                host_id: fixture::n("host"),
                session: base::Session {
                    session_id: fixture::id(90).to_string(),
                    ..Default::default()
                },
                channel: channel.unwrap(),
                transport_pin: None,
            };
            let (observation, _) = fixture::qualification_v2_observation();
            let request = observation.receipt.unwrap().request;
            for result in [
                client.inspect_execution_qualification().await,
                client
                    .lookup_execution_qualification(&request.context.id)
                    .await,
                client.accept_execution_qualification(&request).await,
            ] {
                if v2 {
                    assert!(result.unwrap().current());
                } else {
                    assert_eq!(result.unwrap_err().code(), tonic::Code::Unimplemented);
                }
            }
            if v2 {
                assert_eq!(
                    client
                        .lookup_execution_qualification(&fixture::id(99))
                        .await
                        .unwrap_err()
                        .code(),
                    tonic::Code::DataLoss
                );
                let mut foreign = request;
                foreign.context.id = fixture::id(99);
                assert_eq!(
                    client
                        .accept_execution_qualification(&foreign)
                        .await
                        .unwrap_err()
                        .code(),
                    tonic::Code::DataLoss
                );
            }
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
            let _ = stop.send(());
            server.await.unwrap();
        }
    }
}
