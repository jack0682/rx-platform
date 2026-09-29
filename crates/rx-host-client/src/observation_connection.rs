//! Narrow pinned read-only connection. No grant, fence, Arm or dispatcher is created.
use crate::{Hello, HostClient, TlsEndpoint, connection::Error};
use rx_application::{CellConfiguration, observation::BatchReceipt, observation_link};
use rx_domain::types::*;
use rx_runtime::application::{ApplicationPort, Command, Reply};
use std::sync::Arc;
pub struct ObservedHost {
    client: HostClient,
    runtime: Arc<dyn ApplicationPort>,
    link: observation_link::Link,
}
async fn now(runtime: &Arc<dyn ApplicationPort>) -> Result<TimePoint, Error> {
    let Reply::Time(now) = runtime.request(Command::CurrentTime).await? else {
        return Err("clock reply".into());
    };
    Ok(now)
}
impl ObservedHost {
    pub async fn establish(
        runtime: Arc<dyn ApplicationPort>,
        endpoint: TlsEndpoint,
        server_pin: Digest,
        hello: Hello,
        host: Name,
        configuration: &CellConfiguration,
    ) -> Result<Self, Error> {
        let client = HostClient::connect_pinned(endpoint, host, hello, server_pin).await?;
        client
            .open_cell(configuration, &now(&runtime).await?.clock_id)
            .await?;
        Self::from_client(runtime, client, configuration).await
    }
    /// A CA-only client is insufficient. The actual pinned transport provides the provenance.
    pub async fn from_client(
        runtime: Arc<dyn ApplicationPort>,
        client: HostClient,
        configuration: &CellConfiguration,
    ) -> Result<Self, Error> {
        let transport = client
            .transport_pin()
            .ok_or("observation link requires pinned transport")?
            .clone();
        let configuration_read_started = now(&runtime).await?;
        let observed_configuration = client.inspect_process_configuration().await?;
        let configuration_read_finished = now(&runtime).await?;
        let read_started = now(&runtime).await?;
        let snapshot = client
            .read_bootstrap(
                &configuration.id,
                configuration
                    .fact_specs
                    .iter()
                    .filter(|s| s.host == client.host_id)
                    .map(|s| s.id.clone())
                    .collect(),
            )
            .await?;
        let Reply::ObservationLink(link) = runtime
            .request(Command::RegisterObservationLink(Box::new(
                observation_link::Register {
                    host: client.host_id.clone(),
                    platform_session: Id::new(&client.session.session_id)?,
                    read_started,
                    provenance: rx_application::host_link::BootstrapProvenance {
                        transport,
                        configuration: observed_configuration,
                        configuration_read_started,
                        configuration_read_finished,
                        host_read: snapshot,
                    },
                },
            )))
            .await?
        else {
            return Err("observation registration reply".into());
        };
        Ok(Self {
            client,
            runtime,
            link: *link,
        })
    }
    pub fn link(&self) -> &observation_link::Link {
        &self.link
    }
    pub async fn read_once(&self) -> Result<BatchReceipt, Error> {
        let read_started = now(&self.runtime).await?;
        let snapshot = self
            .client
            .read_bootstrap(
                &self.link.cell,
                self.link.source_sessions.keys().cloned().collect(),
            )
            .await?;
        let Reply::Observations(receipt) = self
            .runtime
            .request(Command::IngestObservationRead(Box::new(
                observation_link::Read {
                    link: self.link.id.clone(),
                    snapshot,
                    read_started,
                },
            )))
            .await?
        else {
            return Err("observation ingestion reply".into());
        };
        Ok(receipt)
    }
}
