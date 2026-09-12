//! Best-effort latest-state relay. It cannot issue or recover execution authority.
use crate::{
    connection::{ConnectionService, ConnectionStatus},
    observation::ObservationStatus,
};
use rx_application::service_health::*;
use rx_domain::types::Counter;
use rx_runtime::application::{ApplicationPort, Command};
use std::{sync::Arc, time::Duration};
pub struct Relay {
    runtime: Arc<dyn ApplicationPort>,
    owner: Owner,
    connection: tokio::sync::watch::Receiver<ConnectionStatus>,
    observation: tokio::sync::watch::Receiver<ObservationStatus>,
    delivery: tokio::sync::watch::Receiver<crate::delivery::Report>,
}
impl Relay {
    pub fn new(
        runtime: Arc<dyn ApplicationPort>,
        owner: Owner,
        service: &ConnectionService,
    ) -> Result<Self, crate::connection::Error> {
        if owner.target != service.target() {
            return Err("diagnostic source binding differs".into());
        }
        Ok(Self {
            runtime,
            owner,
            connection: service.subscribe(),
            observation: service.subscribe_observations(),
            delivery: service.subscribe_delivery(),
        })
    }
    pub async fn run(mut self, mut stopped: tokio::sync::watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut sequence = Counter(0);
        let mut last_received = None;
        loop {
            let ending = *stopped.borrow()
                || stopped.has_changed().is_err()
                || self.connection.has_changed().is_err();
            let connection = if ending {
                Connection::Stopped
            } else {
                match *self.connection.borrow() {
                    ConnectionStatus::WaitingForProducer => Connection::WaitingForPeer,
                    ConnectionStatus::Connecting => Connection::Connecting,
                    ConnectionStatus::Bound { .. } => Connection::Bound,
                    ConnectionStatus::Attention => Connection::Attention,
                    ConnectionStatus::Stopped => Connection::Stopped,
                }
            };
            let (observation, received) = match &*self.observation.borrow() {
                ObservationStatus::Waiting => (Observation::Waiting, None),
                ObservationStatus::Received(value) => (
                    if value.continuity_lost() {
                        Observation::IntegrityAttention
                    } else {
                        Observation::Received
                    },
                    Some(value.received_at.clone()),
                ),
                ObservationStatus::Unavailable => (Observation::Unavailable, None),
                ObservationStatus::Stopped => (Observation::Stopped, None),
            };
            if received.is_some() {
                last_received = received;
            }
            let delivery = self.delivery.borrow().clone();
            let Ok(next) = sequence.increment() else {
                return;
            };
            sequence = next;
            let message = Publish {
                owner: self.owner.clone(),
                sequence,
                report: Report {
                    connection,
                    observation: if ending {
                        Observation::Stopped
                    } else {
                        observation
                    },
                    delivery: if ending {
                        Delivery::Stopped
                    } else if delivery.last_pass_at.is_some() {
                        Delivery::Active
                    } else {
                        Delivery::NotStarted
                    },
                    last_observation_at: last_received.clone(),
                    last_delivery_pass_at: delivery.last_pass_at,
                    delivery_error_history: delivery.attention > 0,
                },
            };
            // No backlog of telemetry. A busy/unavailable writer yields a stale display, not invented health.
            let result = self
                .runtime
                .request(Command::PublishHostService(Box::new(message)))
                .await;
            if matches!(
                result,
                Err(rx_runtime::writer::WriterError::Rejected(
                    rx_ports::StoreError::Rejected(rx_domain::fault::Rejection::ContinuityUnproven)
                ))
            ) {
                return;
            }
            if ending {
                return;
            }
            tokio::select! {
                _=interval.tick()=>{},
                _=stopped.changed()=>{},
                _=self.connection.changed()=>{},
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rx_domain::types::*;
    use rx_runtime::{
        application::{CallFuture, Reply},
        writer::Status,
    };
    struct Sink;
    impl ApplicationPort for Sink {
        fn request(&self, _: Command) -> CallFuture<'_> {
            Box::pin(async { Ok(Reply::Done) })
        }
        fn status(&self) -> Status {
            Status::Running
        }
    }
    #[test]
    fn a_relay_cannot_label_another_services_samples_with_its_owner() {
        let runtime: Arc<dyn ApplicationPort> = Arc::new(Sink);
        let n = |s: &str| Name::new(s).unwrap();
        let id = || Id::new(uuid::Uuid::new_v4().to_string()).unwrap();
        let target = Target {
            host: n("host/a"),
            cell: n("cell/a"),
        };
        let service = ConnectionService::new(
            runtime.clone(),
            crate::connection::ConnectionConfiguration {
                host: target.host.clone(),
                cell: target.cell.clone(),
                endpoint: crate::TlsEndpoint {
                    uri: "https://127.0.0.1:1".into(),
                    server_name: "localhost".into(),
                    server_ca_pem: vec![],
                    client_certificate_pem: vec![],
                    client_key_pem: vec![],
                },
                server_pin: Digest::from_bytes([1; 32]),
                release: Digest::from_bytes([2; 32]),
                ttl_ms: Counter(1000),
            },
        )
        .unwrap();
        let mut owner = Owner {
            target,
            id: id(),
            runtime_boot: id(),
            definition: Digest::from_bytes([3; 32]),
            envelope: Digest::from_bytes([4; 32]),
        };
        assert!(Relay::new(runtime.clone(), owner.clone(), &service).is_ok());
        owner.target.cell = n("cell/b");
        assert!(Relay::new(runtime, owner, &service).is_err());
    }
}
