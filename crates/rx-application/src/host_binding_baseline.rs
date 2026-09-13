//! Immutable evidence of one normal Host binding. Historical context is not current authority.
use rx_domain::{canonical, host_configuration, host_snapshot, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA: &str = "rx.host-binding-baseline.v1";
pub const PREFIX: &str = "host-binding-baseline";
pub const SOURCE_SCHEMA: &str = "rx.host-binding-baseline-source.v1";
pub const SOURCE_PREFIX: &str = "host-binding-baseline-source";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportPin {
    pub uri: String,
    pub server_name: String,
    /// SHA-256 of the exact configured public CA PEM bytes.
    pub server_ca_digest: Digest,
    /// SHA-256 of the authenticated Host leaf DER.
    pub server_leaf_digest: Digest,
    /// SHA-256 of the P client leaf DER.
    pub platform_client_leaf_digest: Digest,
    /// SHA-256 of the exact configured public P client certificate PEM chain bytes.
    pub platform_client_chain_digest: Digest,
    pub release: Digest,
    pub base_manifest: Digest,
    pub cell_manifest: Digest,
    pub host_read_binding: Digest,
    pub host_configuration_binding: Digest,
}
impl TransportPin {
    pub fn validate(&self) -> Result<(), String> {
        let authority = self
            .uri
            .strip_prefix("https://")
            .and_then(|s| s.split('/').next());
        if authority.is_none_or(str::is_empty)
            || self
                .uri
                .strip_prefix("https://")
                .and_then(|s| s.split_once('/'))
                .is_some_and(|(_, path)| !path.is_empty())
            || self.uri.len() > 4096
            || self
                .uri
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
            || self.uri.contains(['@', '?', '#', '\\'])
            || self.server_name.is_empty()
            || self.server_name.len() > 253
            || self
                .server_name
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err("Host transport pin differs".into());
        }
        Ok(())
    }
    pub(crate) fn validate_current_bindings(&self) -> Result<(), String> {
        self.validate()?;
        if self.base_manifest
            != manifest(include_str!(
                "../../../spec/contracts/v1.0/protocol_manifest.json"
            ))?
            || self.cell_manifest
                != manifest(include_str!(
                    "../../../spec/cell_operations/v1.0/protocol_manifest.json"
                ))?
            || self.host_read_binding
                != manifest(include_str!("../../../spec/host-read/v1/binding.json"))?
            || self.host_configuration_binding != configuration_binding()?
        {
            return Err("Host transport pin or negotiated binding differs".into());
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-HOST-TRANSPORT-PIN-v1", self).map_err(|e| e.to_string())
    }
}
fn manifest(text: &str) -> Result<Digest, String> {
    let value: serde_json::Value =
        canonical::decode_json(text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(rx_package::content_digest(
        &canonical::bytes(&value).map_err(|e| e.to_string())?,
    ))
}
fn configuration_binding() -> Result<Digest, String> {
    let value: serde_json::Value = canonical::decode_json(include_bytes!(
        "../../../spec/host-configuration/v1/binding.json"
    ))
    .map_err(|e| e.to_string())?;
    canonical::digest("RX-HOST-CONFIGURATION-BINDING-v1", &value).map_err(|e| e.to_string())
}

/// Internal composition input, supplied only after actual pinned transport and validated reads.
/// Legacy non-pinned clients supply None; their history must never be backfilled.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapProvenance {
    pub transport: TransportPin,
    pub configuration: host_configuration::Observation,
    pub configuration_read_started: TimePoint,
    pub configuration_read_finished: TimePoint,
    pub host_read: host_snapshot::HostSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StableHostIdentity {
    pub installation: Id,
    pub store_generation: Id,
    pub host: Name,
    pub cell: Name,
    pub host_boot: Id,
    pub evidence_journal: Id,
    pub delivery_journal: Id,
    pub source_sessions: BTreeMap<Name, Id>,
    pub transport_digest: Digest,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceReference {
    pub generation: Id,
    pub evidence: Id,
    pub observation: ArtifactRef,
}

/// Original bound objects and read bytes retained independently of mutable live projections.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineSource {
    pub installation: crate::Installation,
    pub producer: crate::EvidenceProducer,
    pub configuration: crate::CellConfiguration,
    pub plan: crate::host_link::Plan,
    pub receipt: crate::host_link::BoundReceipt,
    pub bound_at: TimePoint,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostBindingBaseline {
    pub schema: Name,
    pub plan: Id,
    /// Hash/size of the entire original sidecar, including fields not separately projected.
    pub source: ArtifactRef,
    /// Digest of the exact initial bound Plan; renewal may later change that Plan's lease fields.
    pub original_plan_digest: Digest,
    /// Digest of the exact initial HostRegistration; never compared to a later full registration.
    pub original_registration_digest: Digest,
    pub installation: Id,
    pub store_generation: Id,
    pub runtime_boot: Id,
    pub host: Name,
    pub cell: Name,
    pub producer_authentication_binding: Digest,
    pub producer_session: Id,
    pub platform_session: Id,
    pub host_boot: Id,
    pub evidence_journal: Id,
    pub delivery_journal: Id,
    pub source_sessions: BTreeMap<Name, Id>,
    pub transport: TransportPin,
    pub transport_digest: Digest,
    /// Original P CellConfiguration, not a claim that later Apply preserved this value.
    pub original_configuration_digest: Digest,
    pub host_binding_digest: Digest,
    pub applied_context: Option<host_configuration::AppliedContext>,
    /// Exact bound P/Fence cut, distinct from the pre-Fence observations below.
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub host_read: ArtifactRef,
    pub host_read_captured_at: TimePoint,
    pub configuration_read: ArtifactRef,
    pub configuration_read_started: TimePoint,
    pub configuration_read_finished: TimePoint,
    pub source_references: BTreeMap<Name, SourceReference>,
    pub bound_at: TimePoint,
}
impl HostBindingBaseline {
    pub fn stable_identity(&self) -> StableHostIdentity {
        StableHostIdentity {
            installation: self.installation.clone(),
            store_generation: self.store_generation.clone(),
            host: self.host.clone(),
            cell: self.cell.clone(),
            host_boot: self.host_boot.clone(),
            evidence_journal: self.evidence_journal.clone(),
            delivery_journal: self.delivery_journal.clone(),
            source_sessions: self.source_sessions.clone(),
            transport_digest: self.transport_digest,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != SCHEMA
            || self.source.schema_id.as_str() != SOURCE_SCHEMA
            || self.source.size_bytes.0 == 0
            || self.transport.digest()? != self.transport_digest
            || self.epoch.0 == 0
            || self.scopes.is_empty()
            || self.scopes.values().any(|e| e.0 == 0)
            || self.host_read.schema_id.as_str() != "rx.host-snapshot.v1"
            || self.host_read.size_bytes.0 == 0
            || self.configuration_read.schema_id.as_str()
                != "rx.host-process-configuration-observation.v1"
            || self.configuration_read.size_bytes.0 == 0
            || self.source_references.len() != self.source_sessions.len()
            || self.source_references.iter().any(|(source, reference)| {
                self.source_sessions.get(source) != Some(&reference.generation)
                    || reference.observation.schema_id.as_str() != "rx.host-source-observation.v1"
                    || reference.observation.size_bytes.0 == 0
            })
            || self.applied_context.as_ref().is_some_and(|a| {
                a.cell != self.cell
                    || a.binding_digest != self.host_binding_digest
                    || a.receipt_sequence.0 == 0
                    || a.configuration != self.original_configuration_digest
            })
            || self
                .configuration_read_finished
                .age_ns(&self.configuration_read_started)
                .is_none_or(|n| n > 100_000_000)
            || self
                .host_read_captured_at
                .age_ns(&self.configuration_read_finished)
                .is_none()
            || self.bound_at.age_ns(&self.host_read_captured_at).is_none()
        {
            return Err("Host binding baseline identity/reference/cut differs".into());
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-HOST-BINDING-BASELINE-v1", self).map_err(|e| e.to_string())
    }
    /// Verify retained original objects, never a renewed Plan or current full registration.
    pub fn verify_source(&self, source: &BaselineSource) -> Result<(), String> {
        self.validate()?;
        if reference(SOURCE_SCHEMA, source)? != self.source {
            return Err("baseline complete original source hash/size differs".into());
        }
        let plan = &source.plan;
        let provenance = plan
            .provenance
            .as_ref()
            .ok_or("baseline original provenance missing")?;
        provenance.configuration.validate()?;
        provenance.host_read.validate().map_err(|e| e.to_string())?;
        let observed = provenance
            .configuration
            .snapshot
            .cells
            .iter()
            .find(|c| c.cell == self.cell)
            .ok_or("baseline original configuration cell missing")?;
        let sources = provenance
            .host_read
            .observations
            .iter()
            .map(|o| {
                Ok((
                    o.source.clone(),
                    SourceReference {
                        generation: o.generation.clone(),
                        evidence: o.evidence_id.clone(),
                        observation: reference("rx.host-source-observation.v1", o)?,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        if !plan.bound
            || plan.id != self.plan
            || canonical::digest("RX-HOST-LINK-BOUND-PLAN-v1", plan).map_err(|e| e.to_string())?
                != self.original_plan_digest
            || canonical::digest("RX-HOST-LINK-REGISTRATION-v1", &source.receipt.registration)
                .map_err(|e| e.to_string())?
                != self.original_registration_digest
            || source.installation.id != self.installation
            || source.installation.store_generation != self.store_generation
            || source.installation.runtime_boot != self.runtime_boot
            || source.bound_at != self.bound_at
            || source.producer.authentication_binding != self.producer_authentication_binding
            || source.producer.principal != self.host
            || source.producer.session != self.producer_session
            || source.producer.peer_boot != self.host_boot
            || source.producer.journal != self.evidence_journal
            || plan.host != self.host
            || plan.cell != self.cell
            || plan.producer_session != self.producer_session
            || plan.platform_session != self.platform_session
            || plan.host_boot != self.host_boot
            || plan.evidence_journal != self.evidence_journal
            || plan.delivery_journal != self.delivery_journal
            || plan.source_sessions != self.source_sessions
            || plan.epoch != self.epoch
            || plan.scopes != self.scopes
            || plan.valid_until != source.receipt.registration.grant.valid_until
            || provenance.transport != self.transport
            || crate::runtime_invalidation::configuration_digest(&source.configuration)?
                != self.original_configuration_digest
            || source.configuration.id != self.cell
            || source.configuration.definition.sha256 != plan.definition
            || provenance.configuration.snapshot.binding_digest != self.host_binding_digest
            || reference("rx.host-snapshot.v1", &provenance.host_read)? != self.host_read
            || reference(
                "rx.host-process-configuration-observation.v1",
                &provenance.configuration,
            )? != self.configuration_read
            || provenance.host_read.captured_at != self.host_read_captured_at
            || provenance.configuration_read_started != self.configuration_read_started
            || provenance.configuration_read_finished != self.configuration_read_finished
            || !same_serialized(&sources, &self.source_references)?
            || !same_serialized(&observed.applied, &self.applied_context)?
        {
            return Err("baseline retained original objects or reference bytes differ".into());
        }
        Ok(())
    }
}

fn same_serialized<T: Serialize>(left: &T, right: &T) -> Result<bool, String> {
    Ok(canonical::bytes(left).map_err(|e| e.to_string())?
        == canonical::bytes(right).map_err(|e| e.to_string())?)
}

pub(crate) fn reference<T: Serialize>(schema: &str, value: &T) -> Result<ArtifactRef, String> {
    let bytes = canonical::bytes(value).map_err(|e| e.to_string())?;
    Ok(ArtifactRef {
        sha256: rx_package::content_digest(&bytes),
        schema_id: Name::new(schema).map_err(|e| e.to_string())?,
        size_bytes: Counter(bytes.len() as u64),
    })
}

/// Historical-only lookup; absence is not a request to create or reconstruct a baseline.
pub fn load(
    tx: &mut dyn rx_ports::Transaction,
    plan: &Id,
) -> rx_ports::Result<Option<HostBindingBaseline>> {
    let key = crate::persistence::key(PREFIX, plan);
    let Some(row) = tx.get(&key)? else {
        if tx
            .get(&crate::persistence::key(SOURCE_PREFIX, plan))?
            .is_some()
        {
            return Err(rx_ports::StoreError::Integrity(
                "baseline missing for retained source".into(),
            ));
        }
        let receipt_exists = tx
            .get(&crate::persistence::key("host-link-receipt", plan))?
            .is_some();
        let plan_key = crate::persistence::key("host-link-plan", plan);
        if let Some(row) = tx.get(&plan_key)? {
            let stored: crate::host_link::Plan =
                crate::persistence::decode(&row, "rx.internal.host-link-plan.v1")?;
            if row.key != plan_key
                || stored.id != *plan
                || (stored.bound && stored.provenance.is_some())
                || (!stored.bound && receipt_exists)
            {
                return Err(rx_ports::StoreError::Integrity(
                    "baseline missing for known pinned binding or inconsistent plan".into(),
                ));
            }
        } else if receipt_exists {
            return Err(rx_ports::StoreError::Integrity(
                "baseline and plan missing for retained bound receipt".into(),
            ));
        }
        return Ok(None);
    };
    let value: HostBindingBaseline = crate::persistence::decode(&row, SCHEMA)?;
    if row.key != key || row.revision != Counter(1) || value.plan != *plan {
        return Err(rx_ports::StoreError::Integrity(
            "Host binding baseline immutable identity differs".into(),
        ));
    }
    value.validate().map_err(rx_ports::StoreError::Integrity)?;
    let source_key = crate::persistence::key(SOURCE_PREFIX, plan);
    let source_row = tx
        .get(&source_key)?
        .ok_or_else(|| rx_ports::StoreError::Integrity("baseline source missing".into()))?;
    let source: BaselineSource = crate::persistence::decode(&source_row, SOURCE_SCHEMA)?;
    if source_row.key != source_key || source_row.revision != Counter(1) || source.plan.id != *plan
    {
        return Err(rx_ports::StoreError::Integrity(
            "baseline source immutable identity differs".into(),
        ));
    }
    value
        .verify_source(&source)
        .map_err(rx_ports::StoreError::Integrity)?;
    let receipt_key = crate::persistence::key("host-link-receipt", plan);
    let receipt_row = tx
        .get(&receipt_key)?
        .ok_or_else(|| rx_ports::StoreError::Integrity("baseline bound receipt missing".into()))?;
    let receipt: crate::host_link::BoundReceipt =
        crate::persistence::decode(&receipt_row, "rx.internal.host-link-receipt.v1")?;
    if receipt_row.key != receipt_key
        || receipt_row.revision != Counter(1)
        || canonical::bytes(&receipt).map_err(|e| rx_ports::StoreError::Integrity(e.to_string()))?
            != canonical::bytes(&source.receipt)
                .map_err(|e| rx_ports::StoreError::Integrity(e.to_string()))?
    {
        return Err(rx_ports::StoreError::Integrity(
            "baseline bound receipt differs".into(),
        ));
    }
    Ok(Some(value))
}
