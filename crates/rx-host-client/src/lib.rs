//! Authenticated Host transport. Business state is changed only by rx-application transactions.
pub mod bootstrap;
pub mod connection;
pub mod delivery;
use rx_application as app;
use rx_domain::{canonical, types::*};
use rx_protocol::{base, cell};
use std::collections::BTreeMap;
use tonic::{
    Status,
    transport::{Certificate, Channel, ClientTlsConfig, Identity},
};

#[derive(Clone)]
pub struct TlsEndpoint {
    pub uri: String,
    pub server_name: String,
    pub server_ca_pem: Vec<u8>,
    pub client_certificate_pem: Vec<u8>,
    pub client_key_pem: Vec<u8>,
}
#[derive(Clone)]
pub struct HostClient {
    pub host_id: Name,
    pub session: base::Session,
    channel: Channel,
    transport_pin: Option<app::host_link::TransportPin>,
}
#[derive(Clone)]
pub struct Hello {
    pub peer_id: Name,
    pub boot_id: Id,
    pub installation: Id,
    pub store_generation: Id,
    pub release_digest: Digest,
    pub clock_id: String,
}
impl HostClient {
    /// Present only after the exact release pin was checked on the TLS stream.
    pub fn transport_pin(&self) -> Option<&app::host_link::TransportPin> {
        self.transport_pin.as_ref()
    }
    pub async fn inspect_cell(&self, cell_id: &Name) -> Result<cell::HostCellState, Status> {
        cell::cell_host_service_client::CellHostServiceClient::new(self.channel.clone())
            .inspect(self.call(cell_id, &new_id()))
            .await
            .map(|r| r.into_inner())
    }
    pub async fn connect(
        endpoint: TlsEndpoint,
        host_id: Name,
        hello: Hello,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let channel = Channel::from_shared(endpoint.uri)?
            .tls_config(
                ClientTlsConfig::new()
                    .domain_name(endpoint.server_name)
                    .ca_certificate(Certificate::from_pem(endpoint.server_ca_pem))
                    .identity(Identity::from_pem(
                        endpoint.client_certificate_pem,
                        endpoint.client_key_pem,
                    )),
            )?
            .timeout(std::time::Duration::from_secs(3))
            .connect()
            .await?;
        Self::negotiate(channel, host_id, hello).await
    }
    async fn negotiate(
        channel: Channel,
        host_id: Name,
        hello: Hello,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let session = base::session_service_client::SessionServiceClient::new(channel.clone())
            .open(base::PeerHello {
                peer_id: hello.peer_id.to_string(),
                role: base::Role::Platform as i32,
                boot_id: hello.boot_id.to_string(),
                installation_id: hello.installation.to_string(),
                store_generation: hello.store_generation.to_string(),
                supported_versions: vec![base::Version {
                    major: 1,
                    minor: 0,
                    schema_hash: base_hash(),
                }],
                release_digest: hello.release_digest.as_bytes().to_vec(),
                journal_id: None,
                last_seq: None,
                shared_clock_id: hello.clock_id,
            })
            .await?
            .into_inner();
        if session.peer_id != hello.peer_id.as_str()
            || session.boot_id != hello.boot_id.as_str()
            || session
                .selected_version
                .as_ref()
                .map(|v| v.schema_hash.as_slice())
                != Some(base_hash().as_slice())
        {
            return Err("Host selected an unexpected peer or contract".into());
        }
        parse_id(&session.session_id)?;
        Ok(Self {
            host_id,
            session,
            channel,
            transport_pin: None,
        })
    }
    pub async fn open_cell(
        &self,
        configuration: &app::CellConfiguration,
        clock_id: &str,
    ) -> Result<cell::HostCellState, Status> {
        self.open_configuration_cell(&configuration.id, configuration.definition.sha256, clock_id)
            .await
    }
    pub async fn open_configuration_cell(
        &self,
        cell_id: &Name,
        definition: Digest,
        clock_id: &str,
    ) -> Result<cell::HostCellState, Status> {
        cell::cell_service_client::CellServiceClient::new(self.channel.clone())
            .open(cell::CellHello {
                base_manifest_hash: base_hash(),
                cell_manifest_hash: cell_hash(),
                peer_id: self.session.peer_id.clone(),
                base_session_id: self.session.session_id.clone(),
                cell_definition_digest: definition.as_bytes().to_vec(),
                shared_clock_id: clock_id.into(),
            })
            .await?;
        let state =
            cell::cell_host_service_client::CellHostServiceClient::new(self.channel.clone())
                .inspect(self.call(cell_id, &new_id()))
                .await?
                .into_inner();
        if state.definition_digest != definition.as_bytes() {
            return Err(Status::failed_precondition("Host CellDefinition mismatch"));
        }
        Ok(state)
    }
    pub async fn acquire_grant(
        &self,
        key: &Id,
        resources: Vec<Name>,
        fence: Counter,
        ttl_ms: Counter,
        sent_at: TimePoint,
    ) -> Result<(app::Grant, Id), Status> {
        let result = base::host_service_client::HostServiceClient::new(self.channel.clone())
            .acquire_grant(base::GrantRequest {
                context: Some(self.context(key)),
                resource_set: resources.iter().map(ToString::to_string).collect(),
                fence: fence.0,
                owner_id: self.session.peer_id.clone(),
                requested_ttl_ms: ttl_ms.0,
            })
            .await?
            .into_inner();
        let mut received = names(&result.resource_set)?;
        let mut expected = resources;
        received.sort();
        expected.sort();
        if received != expected
            || result.fence != fence.0
            || result.owner_id != self.session.peer_id
            || result.ttl_ms != ttl_ms.0
        {
            return Err(Status::failed_precondition(
                "grant response differs from requested scope",
            ));
        }
        let duration = ttl_ms
            .0
            .checked_mul(1_000_000)
            .ok_or_else(|| Status::invalid_argument("TTL overflow"))?;
        let expiry = sent_at
            .ticks_ns
            .0
            .checked_add(duration)
            .ok_or_else(|| Status::invalid_argument("expiry overflow"))?;
        Ok((
            app::Grant {
                id: parse_id(&result.grant_id)?,
                fence,
                resources: received,
                owner: parse_name(&result.owner_id)?,
                ttl_ms,
                valid_until: TimePoint {
                    clock_id: sent_at.clock_id,
                    ticks_ns: Counter(expiry),
                },
            },
            parse_id(&result.host_boot_id)?,
        ))
    }
    pub async fn arm(
        &self,
        key: &Id,
        attempt: &Id,
        cell_id: &Name,
        epoch: Counter,
        scopes: &BTreeMap<Name, Counter>,
    ) -> Result<app::ArmAcknowledgment, Status> {
        self.arm_clearing(key, attempt, cell_id, epoch, scopes, &[])
            .await
    }
    pub async fn arm_clearing(
        &self,
        key: &Id,
        attempt: &Id,
        cell_id: &Name,
        epoch: Counter,
        scopes: &BTreeMap<Name, Counter>,
        allowed_clear: &[Id],
    ) -> Result<app::ArmAcknowledgment, Status> {
        let current = self.inspect_cell(cell_id).await?;
        if current.cell.as_ref() != Some(&reference(cell_id, epoch, scopes)) {
            return Err(Status::failed_precondition(
                "Arm inspection generation changed",
            ));
        }
        let blocked = current
            .block_ids
            .iter()
            .map(|id| parse_id(id))
            .collect::<Result<Vec<_>, _>>()?;
        if blocked.iter().any(|id| !allowed_clear.contains(id)) {
            return Err(Status::failed_precondition(
                "Host has a block outside the approved clear plan",
            ));
        }
        let result =
            cell::cell_host_service_client::CellHostServiceClient::new(self.channel.clone())
                .arm_cell(cell::HostArmCellRequest {
                    call: Some(self.call(cell_id, key)),
                    attempt_id: attempt.to_string(),
                    target: Some(reference(cell_id, epoch, scopes)),
                    clearance_ids: vec![],
                    block_ids_to_clear: blocked.iter().map(ToString::to_string).collect(),
                })
                .await?
                .into_inner();
        let target = result
            .target
            .ok_or_else(|| Status::data_loss("Arm target absent"))?;
        if target != reference(cell_id, epoch, scopes)
            || result.attempt_id != attempt.as_str()
            || result.seq == 0
        {
            return Err(Status::data_loss("Arm response mismatch"));
        }
        Ok(app::ArmAcknowledgment {
            attempt: attempt.clone(),
            host_boot: parse_id(&result.host_boot_id)?,
            delivery_journal: parse_id(&result.delivery_journal_id)?,
            sequence: Counter(result.seq),
            epoch,
            scopes: scopes.clone(),
        })
    }
    pub async fn fence(
        &self,
        key: &Id,
        cell_id: &Name,
        epoch: Counter,
        scopes: &BTreeMap<Name, Counter>,
        blocks: &[Id],
    ) -> Result<cell::FenceReceipt, Status> {
        let result =
            cell::cell_host_service_client::CellHostServiceClient::new(self.channel.clone())
                .fence_cell(cell::HostFenceCellRequest {
                    call: Some(self.call(cell_id, key)),
                    target: Some(reference(cell_id, epoch, scopes)),
                    block_ids: blocks.iter().map(ToString::to_string).collect(),
                    invalidation_id: key.to_string(),
                })
                .await
                .map(|r| r.into_inner())?;
        if result.target.as_ref() != Some(&reference(cell_id, epoch, scopes))
            || result.invalidation_id != key.as_str()
            || result.seq == 0
        {
            return Err(Status::data_loss("Fence response mismatch"));
        }
        parse_id(&result.host_boot_id)?;
        parse_id(&result.delivery_journal_id)?;
        Ok(result)
    }
    pub async fn prepare(
        &self,
        key: &Id,
        work: &app::Work,
        permit: &app::Permit,
    ) -> Result<app::HostReceipt, Status> {
        let context = self.context(key);
        let intent = rx_protocol::json::from_slice(
            &canonical::bytes(&work.intent).map_err(|e| Status::invalid_argument(e.to_string()))?,
        )?;
        let result =
            cell::cell_host_service_client::CellHostServiceClient::new(self.channel.clone())
                .prepare(cell::HostPrepareRequest {
                    call: Some(cell::CellCall {
                        context: Some(context.clone()),
                        cell_id: work.cell.to_string(),
                        expected_cell_revision: None,
                    }),
                    base_request: Some(base::PrepareOperation {
                        context: Some(context),
                        operation_id: work.operation.id().to_string(),
                        intent: Some(intent),
                        intent_digest: work
                            .intent
                            .digest()
                            .map_err(|e| Status::invalid_argument(e.to_string()))?
                            .as_bytes()
                            .to_vec(),
                        grant: Some(grant_wire(permit)),
                    }),
                    permit: Some(permit_wire(permit)),
                })
                .await?
                .into_inner();
        receipt(result)
    }
    pub async fn authorize(
        &self,
        key: &Id,
        work: &app::Work,
        permit: &app::Permit,
    ) -> Result<app::HostReceipt, Status> {
        let context = self.context(key);
        let result =
            cell::cell_host_service_client::CellHostServiceClient::new(self.channel.clone())
                .authorize(cell::HostAuthorizeRequest {
                    call: Some(cell::CellCall {
                        context: Some(context.clone()),
                        cell_id: work.cell.to_string(),
                        expected_cell_revision: None,
                    }),
                    base_request: Some(base::AuthorizeDispatch {
                        context: Some(context),
                        operation_id: work.operation.id().to_string(),
                        invocation_id: work
                            .invocation
                            .as_ref()
                            .ok_or_else(|| {
                                Status::failed_precondition("Host invocation not yet known")
                            })?
                            .to_string(),
                        intent_digest: work
                            .intent
                            .digest()
                            .map_err(|e| Status::invalid_argument(e.to_string()))?
                            .as_bytes()
                            .to_vec(),
                        grant: Some(grant_wire(permit)),
                    }),
                    permit: Some(permit_wire(permit)),
                })
                .await?
                .into_inner();
        receipt(result)
    }
    pub async fn receipt(&self, operation: &Id) -> Result<app::HostReceipt, Status> {
        let value = base::host_service_client::HostServiceClient::new(self.channel.clone())
            .get_receipt(base::OperationRef {
                context: Some(self.context(&new_id())),
                operation_id: operation.to_string(),
            })
            .await?
            .into_inner();
        receipt(value)
    }
    pub async fn reconcile(&self, operation: &Id) -> Result<app::EvidenceBatch, Status> {
        let value = base::host_service_client::HostServiceClient::new(self.channel.clone())
            .reconcile(base::OperationRef {
                context: Some(self.context(&new_id())),
                operation_id: operation.to_string(),
            })
            .await?
            .into_inner();
        rx_protocol_adapter::evidence_batch(value)
    }

    pub async fn handover(&self, operation: &Id) -> Result<Vec<app::HandoverObservation>, Status> {
        let sources: Vec<_> = ["no-pending", "control", "support"]
            .into_iter()
            .map(|s| format!("handover/{operation}/{s}"))
            .collect();
        let mut stream = base::host_service_client::HostServiceClient::new(self.channel.clone())
            .watch_observations(base::ObservationQuery {
                context: Some(self.context(&new_id())),
                source_ids: sources.clone(),
                value_schemas: vec!["rx.handover.v1".into()],
            })
            .await?
            .into_inner();
        let batch = stream
            .message()
            .await?
            .ok_or_else(|| Status::data_loss("handover observation missing"))?;
        if batch.dropped_count != 0 || batch.observations.len() != 3 {
            return Err(Status::data_loss("incomplete handover observation"));
        }
        let mut result = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for observation in batch.observations {
            if !sources.contains(&observation.source_id)
                || !seen.insert(observation.source_id.clone())
                || observation.value_schema != "rx.handover.v1"
            {
                return Err(Status::data_loss("handover source/schema mismatch"));
            }
            let correlation = observation
                .correlation
                .ok_or_else(|| Status::data_loss("handover correlation missing"))?;
            if correlation.operation_id.as_deref() != Some(operation.as_str())
                || correlation.cancel_id.is_some()
            {
                return Err(Status::data_loss("handover operation mismatch"));
            }
            let Some(base::typed_value::Value::Boolean(value)) =
                observation.value.and_then(|v| v.value)
            else {
                return Err(Status::data_loss("handover value is not boolean"));
            };
            let at = observation
                .receive_time
                .ok_or_else(|| Status::data_loss("handover clock missing"))?;
            let uncertainty = observation
                .uncertainty_ms
                .ok_or_else(|| Status::data_loss("acquisition uncertainty missing"))?
                .checked_mul(1_000_000)
                .ok_or_else(|| Status::data_loss("uncertainty overflow"))?;
            result.push(app::HandoverObservation {
                id: parse_id(&observation.observation_id)?,
                operation: operation.clone(),
                invocation: parse_id(
                    correlation
                        .invocation_id
                        .as_deref()
                        .ok_or_else(|| Status::data_loss("handover invocation missing"))?,
                )?,
                profile_digest: digest(&correlation.profile_digest)?,
                device_session: parse_id(&correlation.device_session_id)?,
                host_boot: parse_id(&observation.receive_boot_id)?,
                source: parse_name(&observation.source_id)?,
                schema: parse_name(&observation.value_schema)?,
                value,
                observed_at: TimePoint {
                    clock_id: at.clock_id,
                    ticks_ns: Counter(at.ticks_ns),
                },
                uncertainty_ns: Counter(uncertainty),
                quality_good: observation.quality == base::Quality::Good as i32,
                origin_age_bounded: observation.freshness_basis
                    == base::FreshnessBasis::ReadTransaction as i32,
            });
        }
        Ok(result)
    }
    fn context(&self, key: &Id) -> base::CallContext {
        base::CallContext {
            session_id: self.session.session_id.clone(),
            call_id: new_id().to_string(),
            request_key: Some(key.to_string()),
            expected_revision: None,
        }
    }
    fn call(&self, cell: &Name, key: &Id) -> cell::CellCall {
        cell::CellCall {
            context: Some(self.context(key)),
            cell_id: cell.to_string(),
            expected_cell_revision: None,
        }
    }
}
fn reference(id: &Name, epoch: Counter, scopes: &BTreeMap<Name, Counter>) -> cell::CellRef {
    cell::CellRef {
        cell_id: id.to_string(),
        cell_epoch: epoch.0,
        scopes: scopes
            .iter()
            .map(|(s, e)| cell::ScopeEpoch {
                scope_id: s.to_string(),
                epoch: e.0,
            })
            .collect(),
    }
}
fn grant_wire(p: &app::Permit) -> base::Grant {
    base::Grant {
        grant_id: p.grant.id.to_string(),
        host_boot_id: p.host_boot.to_string(),
        fence: p.grant.fence.0,
        resource_set: p.grant.resources.iter().map(ToString::to_string).collect(),
        ttl_ms: p.grant.ttl_ms.0,
        owner_id: p.grant.owner.to_string(),
    }
}
fn permit_wire(p: &app::Permit) -> cell::DispatchPermit {
    let target = reference(&p.cell, p.epoch, &p.scopes);
    let at = base::TimePoint {
        clock_id: p.issued_at.clock_id.clone(),
        ticks_ns: p.issued_at.ticks_ns.0,
    };
    let until = base::TimePoint {
        clock_id: p.expires_at.clock_id.clone(),
        ticks_ns: p.expires_at.ticks_ns.0,
    };
    cell::DispatchPermit {
        permit_id: p.id.to_string(),
        operation_id: p.operation.to_string(),
        intent_digest: p.intent_digest.as_bytes().to_vec(),
        cell: Some(target.clone()),
        envelope_digest: p.envelope_digest.as_bytes().to_vec(),
        qualification_id: p.qualification.to_string(),
        qualification_revision: p.qualification_revision.0,
        purpose: match p.purpose {
            app::Purpose::Production => cell::Purpose::Production,
            app::Purpose::Setup => cell::Purpose::Setup,
        } as i32,
        parent: Some(cell::PermitParent {
            value: Some(cell::permit_parent::Value::MandateId(p.mandate.to_string())),
        }),
        grant: Some(grant_wire(p)),
        host_boot_id: p.host_boot.to_string(),
        issued_at: Some(at.clone()),
        expires_at: Some(until.clone()),
        conditions: p
            .condition_ids
            .iter()
            .map(|id| cell::ConditionEvaluation {
                condition_id: id.to_string(),
                condition_revision: p.condition_revision.0,
                verdict: cell::Verdict::Pass as i32,
                reason: cell::CellReason::None as i32,
                evidence_ids: p.evidence_ids.iter().map(ToString::to_string).collect(),
                evaluated_at: Some(at.clone()),
                valid_until: Some(until.clone()),
                cell: Some(target.clone()),
            })
            .collect(),
        state: cell::PermitState::Issued as i32,
    }
}
fn receipt(r: base::Receipt) -> Result<app::HostReceipt, Status> {
    if r.operation_revision.is_some() || r.cancel_id.is_some() || r.journal_seq == 0 {
        return Err(Status::data_loss("not a Host production receipt"));
    }
    let state = match r
        .host_state
        .and_then(|s| base::HostReceiptState::try_from(s).ok())
    {
        Some(base::HostReceiptState::Prepared) => app::ReceiptState::Prepared,
        Some(base::HostReceiptState::SendEntered) => app::ReceiptState::SendEntered,
        Some(base::HostReceiptState::NativeAccepted) => app::ReceiptState::NativeAccepted,
        Some(base::HostReceiptState::NativeRejected) => app::ReceiptState::NativeRejected,
        Some(base::HostReceiptState::ResultCaptured) => app::ReceiptState::ResultCaptured,
        Some(base::HostReceiptState::VoidedBeforeSend) => app::ReceiptState::VoidedBeforeSend,
        _ => return Err(Status::data_loss("Host receipt state missing")),
    };
    let stage = base::ReceiptStage::try_from(r.stage)
        .map_err(|_| Status::data_loss("unknown receipt stage"))?;
    let coherent = match state {
        app::ReceiptState::Prepared => stage == base::ReceiptStage::HostPrepared,
        app::ReceiptState::SendEntered | app::ReceiptState::NativeRejected => {
            stage == base::ReceiptStage::SendEntered
        }
        app::ReceiptState::NativeAccepted => stage == base::ReceiptStage::NativeAccepted,
        app::ReceiptState::ResultCaptured => matches!(
            stage,
            base::ReceiptStage::SendEntered | base::ReceiptStage::NativeAccepted
        ),
        app::ReceiptState::VoidedBeforeSend => stage == base::ReceiptStage::NotDispatched,
    };
    if !coherent {
        return Err(Status::data_loss("incoherent Host receipt stage"));
    }
    Ok(app::HostReceipt {
        operation: parse_id(&r.operation_id)?,
        digest: digest(&r.intent_digest)?,
        invocation: r.invocation_id.as_deref().map(parse_id).transpose()?,
        journal: parse_id(&r.journal_id)?,
        sequence: Counter(r.journal_seq),
        state,
    })
}
fn new_id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID")
}
fn parse_id(s: &str) -> Result<Id, Status> {
    Id::new(s).map_err(|e| Status::data_loss(e.to_string()))
}
fn parse_name(s: &str) -> Result<Name, Status> {
    Name::new(s).map_err(|e| Status::data_loss(e.to_string()))
}
fn digest(b: &[u8]) -> Result<Digest, Status> {
    Ok(Digest::from_bytes(
        b.try_into()
            .map_err(|_| Status::data_loss("Digest length"))?,
    ))
}
fn names(values: &[String]) -> Result<Vec<Name>, Status> {
    values.iter().map(|s| parse_name(s)).collect()
}
fn base_hash() -> Vec<u8> {
    hash_manifest(include_str!(
        "../../../spec/contracts/v1.0/protocol_manifest.json"
    ))
}
fn cell_hash() -> Vec<u8> {
    hash_manifest(include_str!(
        "../../../spec/cell_operations/v1.0/protocol_manifest.json"
    ))
}
fn hash_manifest(s: &str) -> Vec<u8> {
    use sha2::Digest as _;
    let v: serde_json::Value = serde_json::from_str(s).expect("manifest");
    sha2::Sha256::digest(canonical::bytes(&v).expect("JCS")).to_vec()
}

pub mod observation;

pub mod service_health;

pub mod configuration;

pub mod configuration_worker;

pub mod qualification;

pub mod qualification_worker;

pub mod recovery;
