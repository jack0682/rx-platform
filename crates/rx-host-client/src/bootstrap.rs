//! Pinned, read-only Host bootstrap transport. No grant/Arm or native submission is hidden here.
use super::*;
use rx_domain::host_snapshot::HostSnapshot;
use std::{io, sync::Arc};
use tokio_rustls::{
    TlsConnector,
    rustls::{
        self,
        pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject},
    },
};
fn host_read_binding_hash() -> Vec<u8> {
    use sha2::Digest as _;
    let binding: serde_json::Value =
        serde_json::from_slice(include_bytes!("../../../spec/host-read/v1/binding.json"))
            .expect("build Host-read binding");
    sha2::Sha256::digest(canonical::bytes(&binding).expect("Host-read binding bytes")).to_vec()
}

impl HostClient {
    /// A declaration from release configuration, not proof that a peer was contacted.
    pub fn expected_transport_pin(
        endpoint: &TlsEndpoint,
        release: Digest,
        server_fingerprint: Digest,
    ) -> Result<app::host_link::TransportPin, Box<dyn std::error::Error + Send + Sync>> {
        use sha2::Digest as _;
        let origin: tonic::codegen::http::Uri = endpoint
            .uri
            .parse()
            .map_err(|_| "Host endpoint must be an HTTPS origin")?;
        if origin.scheme_str() != Some("https")
            || origin.authority().is_none_or(|a| a.as_str().contains('@'))
            || origin
                .path_and_query()
                .is_some_and(|p| p.path() != "/" || p.query().is_some())
            || endpoint.uri.contains('#')
        {
            return Err(
                "Host endpoint must be an HTTPS origin without user info, path, query or fragment"
                    .into(),
            );
        }
        let _ = ServerName::try_from(endpoint.server_name.clone())?;
        let certificates = CertificateDer::pem_slice_iter(&endpoint.client_certificate_pem)
            .collect::<Result<Vec<_>, _>>()?;
        let client_leaf = certificates
            .first()
            .ok_or("platform client certificate missing")?;
        let pin = app::host_link::TransportPin {
            uri: endpoint.uri.clone(),
            server_name: endpoint.server_name.clone(),
            server_ca_digest: Digest::from_bytes(
                sha2::Sha256::digest(&endpoint.server_ca_pem).into(),
            ),
            server_leaf_digest: server_fingerprint,
            platform_client_leaf_digest: Digest::from_bytes(
                sha2::Sha256::digest(client_leaf.as_ref()).into(),
            ),
            platform_client_chain_digest: Digest::from_bytes(
                sha2::Sha256::digest(&endpoint.client_certificate_pem).into(),
            ),
            release,
            base_manifest: digest(&base_hash())?,
            cell_manifest: digest(&cell_hash())?,
            host_read_binding: digest(&host_read_binding_hash())?,
            host_configuration_binding: digest(&super::configuration::binding_hash())?,
        };
        pin.validate()?;
        Ok(pin)
    }
    pub async fn connect_pinned(
        endpoint: TlsEndpoint,
        host: Name,
        hello: Hello,
        server_fingerprint: Digest,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        use sha2::Digest as _;
        let transport_pin =
            Self::expected_transport_pin(&endpoint, hello.release_digest, server_fingerprint)?;
        let origin: tonic::codegen::http::Uri = endpoint
            .uri
            .parse()
            .map_err(|_| "Host endpoint must be an HTTPS origin")?;
        if origin.scheme_str() != Some("https")
            || origin.authority().is_none_or(|a| a.as_str().contains('@'))
            || origin
                .path_and_query()
                .is_some_and(|p| p.path() != "/" || p.query().is_some())
            || endpoint.uri.contains('#')
        {
            return Err(
                "Host endpoint must be an HTTPS origin without user info, path, query or fragment"
                    .into(),
            );
        }
        // The custom connector returns an already authenticated TLS stream. Use an internal
        // HTTP transport URI so tonic does not add a second TLS layer; keep the wire origin HTTPS.
        let mut parts = origin.clone().into_parts();
        parts.scheme = Some(tonic::codegen::http::uri::Scheme::HTTP);
        let transport_uri = tonic::codegen::http::Uri::from_parts(parts)?;
        let remote = tonic::transport::Endpoint::from(transport_uri)
            .origin(origin)
            .connect_timeout(std::time::Duration::from_secs(3))
            .timeout(std::time::Duration::from_secs(3));
        let mut roots = rustls::RootCertStore::empty();
        for cert in CertificateDer::pem_slice_iter(&endpoint.server_ca_pem) {
            roots.add(cert?)?;
        }
        let certificates = CertificateDer::pem_slice_iter(&endpoint.client_certificate_pem)
            .collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_slice(&endpoint.client_key_pem)?;
        let mut config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_client_auth_cert(certificates, key)?;
        config.alpn_protocols = vec![b"h2".to_vec()];
        let connector = TlsConnector::from(Arc::new(config));
        let server_name = ServerName::try_from(endpoint.server_name)?;
        let channel = remote
            .connect_with_connector(tower::service_fn(move |uri: tonic::codegen::http::Uri| {
                let connector = connector.clone();
                let server_name = server_name.clone();
                async move {
                    let hostname = uri
                        .host()
                        .ok_or_else(|| io::Error::other("Host address missing"))?
                        .trim_start_matches('[')
                        .trim_end_matches(']');
                    let socket =
                        tokio::net::TcpStream::connect((hostname, uri.port_u16().unwrap_or(443)))
                            .await?;
                    // Custom connectors bypass tonic's default TCP_NODELAY setting.
                    // Keep small HTTP/2/TLS records from waiting behind Nagle/delayed ACK.
                    socket.set_nodelay(true)?;
                    let stream = connector.connect(server_name, socket).await?;
                    let leaf = stream
                        .get_ref()
                        .1
                        .peer_certificates()
                        .and_then(|v| v.first())
                        .ok_or_else(|| io::Error::other("Host certificate missing"))?;
                    if Digest::from_bytes(sha2::Sha256::digest(leaf.as_ref()).into())
                        != server_fingerprint
                    {
                        return Err(io::Error::other("Host certificate pin mismatch"));
                    }
                    Ok::<_, io::Error>(hyper_util::rt::TokioIo::new(stream))
                }
            }))
            .await?;
        let mut client = Self::negotiate(channel, host, hello).await?;
        client.transport_pin = Some(transport_pin);
        Ok(client)
    }
    pub async fn read_bootstrap(
        &self,
        cell_id: &Name,
        sources: Vec<Name>,
    ) -> Result<HostSnapshot, Status> {
        use sha2::Digest as _;
        let binding_hash = host_read_binding_hash();
        let reply = rx_protocol::host_read::host_read_service_client::HostReadServiceClient::new(
            self.channel.clone(),
        )
        .inspect(rx_protocol::host_read::InspectHost {
            call: Some(cell::CellCall {
                context: Some(base::CallContext {
                    session_id: self.session.session_id.clone(),
                    call_id: new_id().to_string(),
                    request_key: None,
                    expected_revision: None,
                }),
                cell_id: cell_id.to_string(),
                expected_cell_revision: None,
            }),
            source_ids: sources.iter().map(ToString::to_string).collect(),
            binding_hash,
        })
        .await?
        .into_inner();
        let reference = reply
            .reference
            .ok_or_else(|| Status::data_loss("Host snapshot reference missing"))?;
        if reply.payload.len() > 1_000_000
            || reference.schema_id != "rx.host-snapshot.v1"
            || reference.size_bytes != reply.payload.len() as u64
            || reference.sha256 != sha2::Sha256::digest(&reply.payload).as_slice()
        {
            return Err(Status::data_loss("Host snapshot artifact mismatch"));
        }
        let snapshot: HostSnapshot = canonical::decode_json(&reply.payload)
            .map_err(|_| Status::data_loss("Host snapshot decode"))?;
        snapshot
            .validate()
            .map_err(|_| Status::data_loss("Host snapshot fields"))?;
        if snapshot.host != self.host_id
            || snapshot.cell != *cell_id
            || (snapshot.sources_available
                && snapshot
                    .observations
                    .iter()
                    .map(|v| &v.source)
                    .collect::<std::collections::BTreeSet<_>>()
                    != sources.iter().collect())
        {
            return Err(Status::data_loss("Host snapshot identity/sources mismatch"));
        }
        Ok(snapshot)
    }
}

impl HostClient {
    pub async fn renew_bootstrap(
        &self,
        renewal: &app::host_link::Renewal,
    ) -> Result<(app::Grant, Id), Status> {
        let response = base::host_service_client::HostServiceClient::new(self.channel.clone())
            .renew_grant(base::RenewGrant {
                context: Some(self.context(&renewal.request)),
                grant_id: renewal.grant.id.to_string(),
                renew_seq: renewal.sequence.0,
            })
            .await?
            .into_inner();
        let mut resources = names(&response.resource_set)?;
        resources.sort();
        if response.grant_id != renewal.grant.id.as_str()
            || response.owner_id != renewal.grant.owner.as_str()
            || response.fence != renewal.grant.fence.0
            || response.ttl_ms != renewal.grant.ttl_ms.0
            || resources != renewal.grant.resources
        {
            return Err(Status::data_loss("grant renewal scope mismatch"));
        }
        let mut grant = renewal.grant.clone();
        grant.valid_until = TimePoint {
            clock_id: renewal.sent_at.clock_id.clone(),
            ticks_ns: Counter(
                renewal
                    .sent_at
                    .ticks_ns
                    .0
                    .checked_add(
                        grant
                            .ttl_ms
                            .0
                            .checked_mul(1_000_000)
                            .ok_or_else(|| Status::data_loss("renewal TTL"))?,
                    )
                    .ok_or_else(|| Status::data_loss("renewal expiry"))?,
            ),
        };
        Ok((grant, parse_id(&response.host_boot_id)?))
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::*;

    #[tokio::test]
    async fn credential_bearing_and_non_origin_endpoints_are_rejected_before_tls() {
        for uri in [
            "http://localhost:7444",
            "https://user:secret@localhost:7444",
            "https://localhost:7444/private",
            "https://localhost:7444/?token=private",
            "https://localhost:7444/#private",
        ] {
            let result = HostClient::connect_pinned(
                TlsEndpoint {
                    uri: uri.into(),
                    server_name: "localhost".into(),
                    server_ca_pem: vec![],
                    client_certificate_pem: vec![],
                    client_key_pem: vec![],
                },
                Name::new("host/test").unwrap(),
                Hello {
                    peer_id: Name::new("platform/test").unwrap(),
                    boot_id: new_id(),
                    installation: new_id(),
                    store_generation: new_id(),
                    release_digest: Digest::from_bytes([1; 32]),
                    clock_id: "test-clock".into(),
                },
                Digest::from_bytes([2; 32]),
            )
            .await;
            let error = match result {
                Ok(_) => panic!("invalid origin created a client"),
                Err(error) => error,
            };
            assert!(error.to_string().contains("HTTPS origin"), "{error}");
            assert!(!error.to_string().contains("secret"));
        }
    }
}
