//! Direct terminal HTTPS ingress. Only this verified TLS connection creates TerminalPeer.
use crate::{auth::Credentials, routes};
use axum::{Extension, Router, extract::ConnectInfo};
use hyper_util::{
    rt::{TokioIo, TokioTimer},
    service::TowerToHyperService,
};
use rx_domain::types::Digest;
use rx_runtime::application::ApplicationPort;
use sha2::{Digest as _, Sha256};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, watch},
    task::JoinSet,
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        self,
        pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    },
};
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct HttpsPolicy {
    pub(crate) origin: String,
    pub(crate) host: String,
}
impl HttpsPolicy {
    /// Same-origin deployment only. Reverse-proxy certificate headers are not trusted.
    pub fn new(origin: &str) -> Result<Self, String> {
        let uri: axum::http::Uri = origin.parse().map_err(|_| "invalid HTTPS origin")?;
        let authority = uri.authority().ok_or("HTTPS authority required")?;
        if uri.scheme_str() != Some("https")
            || uri.host().is_none_or(str::is_empty)
            || authority.as_str().contains('@')
            || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
            || origin.ends_with('/')
        {
            return Err("exact HTTPS origin without path/query/userinfo required".into());
        }
        Ok(Self {
            origin: origin.into(),
            host: authority.to_string(),
        })
    }
}
#[derive(Clone)]
pub struct TlsMaterial {
    pub server_certificate_pem: Vec<u8>,
    pub server_key_pem: Vec<u8>,
    pub terminal_ca_pem: Vec<u8>,
}
/// Fields are private to the verified transport, not constructible from request headers or body.
#[derive(Clone)]
pub(crate) struct TerminalPeer {
    fingerprint: Digest,
}
impl TerminalPeer {
    pub(crate) fn fingerprint(&self) -> Digest {
        self.fingerprint
    }
}

pub struct TerminalHttps {
    router: Router,
    acceptor: TlsAcceptor,
}
impl TerminalHttps {
    pub fn new(
        runtime: Arc<dyn ApplicationPort>,
        credentials: Credentials,
        policy: HttpsPolicy,
        material: TlsMaterial,
    ) -> Result<Self, String> {
        Self::new_with_package_intake(runtime, credentials, policy, material, None)
    }
    pub fn new_with_package_intake(
        runtime: Arc<dyn ApplicationPort>,
        credentials: Credentials,
        policy: HttpsPolicy,
        material: TlsMaterial,
        worker: Option<Arc<rx_runtime::package_intake::Worker>>,
    ) -> Result<Self, String> {
        Self::new_with_operator_ui(runtime, credentials, policy, material, worker, None)
    }
    /// Serve an already validated S bundle over the same direct terminal TLS connection.
    pub fn new_with_operator_ui(
        runtime: Arc<dyn ApplicationPort>,
        credentials: Credentials,
        policy: HttpsPolicy,
        material: TlsMaterial,
        worker: Option<Arc<rx_runtime::package_intake::Worker>>,
        bundle: Option<crate::operator_ui::OperatorBundle>,
    ) -> Result<Self, String> {
        Self::new_with_host_recovery(runtime, credentials, policy, material, worker, bundle, None)
    }
    /// Add a release-configured recovery worker without changing terminal or operation authority.
    pub fn new_with_host_recovery(
        runtime: Arc<dyn ApplicationPort>,
        credentials: Credentials,
        policy: HttpsPolicy,
        material: TlsMaterial,
        worker: Option<Arc<rx_runtime::package_intake::Worker>>,
        bundle: Option<crate::operator_ui::OperatorBundle>,
        recovery: Option<Arc<dyn rx_runtime::host_recovery::Service>>,
    ) -> Result<Self, String> {
        Self::new_with_investigation(
            runtime,
            credentials,
            policy,
            material,
            worker,
            bundle,
            recovery,
            None,
        )
    }
    /// Compose the explicitly configured investigation verifier, preserving older constructors.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_investigation(
        runtime: Arc<dyn ApplicationPort>,
        credentials: Credentials,
        policy: HttpsPolicy,
        material: TlsMaterial,
        worker: Option<Arc<rx_runtime::package_intake::Worker>>,
        bundle: Option<crate::operator_ui::OperatorBundle>,
        recovery: Option<Arc<dyn rx_runtime::host_recovery::Service>>,
        investigation: Option<Arc<rx_runtime::investigation::Worker>>,
    ) -> Result<Self, String> {
        let acceptor = prepare_tls(material)?;
        let mut router = routes::terminal_router(
            runtime,
            credentials,
            policy.clone(),
            worker,
            recovery,
            investigation,
        )?;
        if let Some(bundle) = bundle {
            router = router.merge(bundle.router(policy));
        }
        Ok(Self { router, acceptor })
    }
    /// Stops accepting, drains HTTP requests, then closes remaining network connections only.
    /// Dropping an HTTP reply never rolls back an already admitted writer command.
    pub async fn serve(
        self,
        listener: TcpListener,
        shutdown: impl std::future::Future<Output = ()> + Send,
    ) -> std::io::Result<()> {
        const CONNECTIONS: usize = 64;
        let permits = Arc::new(Semaphore::new(CONNECTIONS));
        let (stop, stopped) = watch::channel(false);
        let mut tasks = JoinSet::new();
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                _ = tasks.join_next(), if !tasks.is_empty() => {},
                result = listener.accept(), if tasks.len() < CONNECTIONS => {
                    let (socket, address) = result?;
                    let Ok(permit) = permits.clone().try_acquire_owned() else {drop(socket); continue;};
                    let acceptor = self.acceptor.clone(); let router = self.router.clone(); let mut stopped = stopped.clone();
                    tasks.spawn(async move {
                        let _permit = permit;
                        let accepted = tokio::select! {
                            result = tokio::time::timeout(Duration::from_secs(5), acceptor.accept(socket)) => result,
                            _ = stopped.changed() => return,
                        };
                        let Ok(Ok(stream)) = accepted else {return;};
                        if *stopped.borrow() {return;}
                        let Some(leaf) = stream.get_ref().1.peer_certificates().and_then(|chain| chain.first()) else {return;};
                        let peer = TerminalPeer {fingerprint: Digest::from_bytes(Sha256::digest(leaf.as_ref()).into())};
                        let service = TowerToHyperService::new(router.layer(Extension(peer)).layer(Extension(ConnectInfo::<SocketAddr>(address))));
                        let mut builder = hyper::server::conn::http1::Builder::new();
                        builder.timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(5)).max_buf_size(32768).max_headers(64);
                        let connection = builder.serve_connection(TokioIo::new(stream), service);
                        tokio::pin!(connection);
                        tokio::select! {
                            _ = &mut connection => {},
                            _ = tokio::time::sleep(Duration::from_secs(60)) => {},
                            _ = stopped.changed() => {
                                connection.as_mut().graceful_shutdown();
                                let _ = tokio::time::timeout(Duration::from_secs(5), &mut connection).await;
                            }
                        }
                    });
                }
            }
        }
        let _ = stop.send(true);
        let drain = async { while tasks.join_next().await.is_some() {} };
        if tokio::time::timeout(Duration::from_secs(6), drain)
            .await
            .is_err()
        {
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        }
        Ok(())
    }
}
fn certificates(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, String> {
    let certificates = CertificateDer::pem_slice_iter(pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "invalid certificate PEM")?;
    if certificates.is_empty() || certificates.len() > 16 {
        return Err("certificate chain count outside bounds".into());
    }
    Ok(certificates)
}

/// Preflight the same TLS policy without opening listeners or granting any identity.
pub fn validate_material(material: TlsMaterial) -> Result<(), String> {
    prepare_tls(material).map(|_| ())
}
fn prepare_tls(material: TlsMaterial) -> Result<TlsAcceptor, String> {
    if [
        material.server_certificate_pem.len(),
        material.server_key_pem.len(),
        material.terminal_ca_pem.len(),
    ]
    .into_iter()
    .any(|n| n == 0 || n > 131_072)
    {
        return Err("TLS material size outside bounds".into());
    }
    let chain = certificates(&material.server_certificate_pem)?;
    let private_pem = Zeroizing::new(material.server_key_pem);
    let key =
        PrivateKeyDer::from_pem_slice(&private_pem).map_err(|_| "invalid server private key")?;
    let mut roots = rustls::RootCertStore::empty();
    for certificate in certificates(&material.terminal_ca_pem)? {
        roots.add(certificate).map_err(|_| "invalid terminal CA")?;
    }
    let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .map_err(|_| "terminal verifier configuration failed")?;
    let mut config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(chain, key)
        .map_err(|_| "server key/certificate mismatch")?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config.max_early_data_size = 0;
    config.send_tls13_tickets = 0;
    config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    Ok(TlsAcceptor::from(Arc::new(config)))
}
