//! Explicit v2 data contracts. Validation alone grants no execution authority.
//! P must obtain these records from the admitted publication and durable Run.
use crate::model::ActionBinding;
use rx_domain::{
    canonical, definition::Reference, intent::Body, intent::Intent, intent::Kind, types::*,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
pub mod host_configuration;
pub mod host_qualification;
mod materialize;
mod plan;
mod templates;
pub use materialize::{
    InputClosure, Materialized, NodeContract, ParameterContract, compiler_digest, context_digest,
    materialize,
};
pub use plan::{BINDING_SCHEMA, Binding, PLAN_SCHEMA, Plan};
pub use templates::{
    TEMPLATE_CATALOG_SCHEMA, TemplateCatalog, TemplateDeclaration, TemplateDocument,
};

pub const POLICY_SCHEMA: &str = "rx.execution-policy.v2";
pub const INDEX_SCHEMA: &str = "rx.execution-report-index.v2";
pub const PARAMETER_SCHEMA: &str = "rx.workflow-parameters.v2";
pub const MAX_VARIANTS: usize = 8;
pub const MAX_SLOTS: usize = 2400;
pub const MAX_NODES: usize = 16;
pub const MAX_INDEX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_POLICY_BYTES: usize = 128 * 1024;
pub const MAX_REPORT_BYTES: usize = 900_000;
pub const MAX_PARAMETER_BYTES: u64 = 64 * 1024;
pub const MAX_DEFINITIONS: usize = 512;
pub const MAX_DEFINITION_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_DEPENDENCIES: usize = 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub key: Name,
    pub object_model: Reference,
    pub context_digest: Digest,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema: Name,
    pub workflow: Reference,
    pub definition_closure: ArtifactRef,
    pub resolver_digest: Digest,
    pub compiler_digest: Digest,
    pub candidates: Vec<Candidate>,
    pub slot_order: Vec<u16>,
    pub templates: BTreeMap<Name, ActionBinding>,
    pub node_contracts: BTreeMap<Name, NodeContract>,
    pub report_index: ArtifactRef,
}

fn nonzero(digest: Digest) -> Result<(), String> {
    if digest == Digest::from_bytes([0; 32]) {
        Err("zero content digest".into())
    } else {
        Ok(())
    }
}

fn reference(reference: &ArtifactRef, schema: &str, max: u64) -> Result<(), String> {
    nonzero(reference.sha256)?;
    if reference.schema_id.as_str() != schema
        || reference.size_bytes.0 == 0
        || reference.size_bytes.0 > max
    {
        return Err(format!("{schema}: artifact schema or size differs"));
    }
    Ok(())
}

impl Policy {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != POLICY_SCHEMA
            || self.candidates.is_empty()
            || self.candidates.len() > MAX_VARIANTS
            || self.slot_order.is_empty()
            || self.slot_order.len() > MAX_SLOTS
            || self.templates.is_empty()
            || self.templates.len() > MAX_NODES
            || self.workflow.revision.0 == 0
            || self.templates.keys().ne(self.node_contracts.keys())
        {
            return Err("execution v2 policy schema/domain bound differs".into());
        }
        nonzero(self.workflow.digest)?;
        nonzero(self.resolver_digest)?;
        nonzero(self.compiler_digest)?;
        reference(
            &self.definition_closure,
            "rx.execution-input-closure.v2",
            MAX_DEFINITION_BYTES,
        )?;
        reference(&self.report_index, INDEX_SCHEMA, MAX_INDEX_BYTES as u64)?;
        let mut keys = BTreeSet::new();
        let mut variants = BTreeSet::new();
        for candidate in &self.candidates {
            nonzero(candidate.object_model.digest)?;
            nonzero(candidate.context_digest)?;
            if candidate.object_model.revision.0 == 0
                || candidate.object_model.catalog != self.workflow.catalog
                || !keys.insert(&candidate.key)
                || !variants.insert((&candidate.object_model, candidate.context_digest))
            {
                return Err("candidate identity, revision or duplicate differs".into());
            }
        }
        if self
            .slot_order
            .iter()
            .enumerate()
            .any(|(ordinal, slot)| ordinal != usize::from(*slot))
        {
            return Err("slot order must be the complete zero-based row-major order".into());
        }
        for action in self.templates.values() {
            let intent = action.intent.normalized().map_err(|e| e.to_string())?;
            if canonical::bytes(&intent).map_err(|e| e.to_string())?
                != canonical::bytes(&action.intent).map_err(|e| e.to_string())?
                || intent.kind != Kind::FiniteAction
            {
                return Err("normalized finite Program template required".into());
            }
            let Body::Program(goal) = &intent.body else {
                return Err("Program template required".into());
            };
            reference(&goal.parameter_set, PARAMETER_SCHEMA, MAX_PARAMETER_BYTES)?;
        }
        for contract in self.node_contracts.values() {
            contract.validate()?;
        }
        if canonical::bytes(self).map_err(|e| e.to_string())?.len() > MAX_POLICY_BYTES {
            return Err("execution v2 policy exceeds 128 KiB".into());
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-EXECUTION-POLICY-v2", self).map_err(|e| e.to_string())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let value: Self = decode_canonical(bytes, MAX_POLICY_BYTES)?;
        value.validate()?;
        Ok(value)
    }
}

/// Entries address the exact ordered tables in Policy, not caller-selected IDs.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportIndex {
    pub schema: Name,
    pub entries: Vec<(u8, u16, Digest)>,
}

/// Content verified against one policy. Private fields prevent bypassing decode.
/// This is integrity evidence only; policy admission remains P's responsibility.
pub struct ValidatedIndex {
    policy_digest: Digest,
    candidates: usize,
    slots: usize,
    reports: Vec<Digest>,
}

impl ReportIndex {
    pub fn validate(&self, candidates: usize, slots: usize) -> Result<(), String> {
        if self.schema.as_str() != INDEX_SCHEMA
            || !(1..=MAX_VARIANTS).contains(&candidates)
            || !(1..=MAX_SLOTS).contains(&slots)
            || self.entries.len() != candidates * slots
        {
            return Err("report index schema or complete domain differs".into());
        }
        for (position, (candidate, slot, digest)) in self.entries.iter().enumerate() {
            if usize::from(*candidate) != position / slots || usize::from(*slot) != position % slots
            {
                return Err(format!(
                    "report index entry {position}: duplicate, hole or order differs"
                ));
            }
            nonzero(*digest)?;
        }
        Ok(())
    }

    /// Large artifacts have their own bound; legacy 1 MiB wire decoding is unchanged.
    pub fn decode(bytes: &[u8], policy: &Policy) -> Result<ValidatedIndex, String> {
        policy.validate()?;
        verify_artifact(bytes, &policy.report_index, MAX_INDEX_BYTES)?;
        let value: Self = decode_canonical(bytes, MAX_INDEX_BYTES)?;
        value.validate(policy.candidates.len(), policy.slot_order.len())?;
        Ok(ValidatedIndex {
            policy_digest: policy.digest()?,
            candidates: policy.candidates.len(),
            slots: policy.slot_order.len(),
            reports: value
                .entries
                .into_iter()
                .map(|(_, _, digest)| digest)
                .collect(),
        })
    }
}

impl ValidatedIndex {
    pub fn report(
        &self,
        policy_digest: Digest,
        candidate: u8,
        slot: u16,
    ) -> Result<Digest, String> {
        if policy_digest != self.policy_digest
            || usize::from(candidate) >= self.candidates
            || usize::from(slot) >= self.slots
        {
            return Err("selected candidate/slot outside published domain".into());
        }
        Ok(self.reports[usize::from(candidate) * self.slots + usize::from(slot)])
    }
}

/// Canonical artifact bytes only: round-trip equality also rejects duplicate keys,
/// alternate numeric spellings and reordered members without changing v1 decoding.
fn decode_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
    max: usize,
) -> Result<T, String> {
    if bytes.len() > max {
        return Err("execution v2 artifact exceeds byte bound".into());
    }
    let value: T = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if canonical::bytes(&value).map_err(|e| e.to_string())? != bytes {
        return Err("execution v2 artifact must use canonical JSON bytes".into());
    }
    Ok(value)
}

pub fn verify_artifact(bytes: &[u8], expected: &ArtifactRef, max: usize) -> Result<(), String> {
    if bytes.is_empty()
        || bytes.len() > max
        || bytes.len() as u64 != expected.size_bytes.0
        || Digest::from_bytes(Sha256::digest(bytes).into()) != expected.sha256
    {
        return Err("execution v2 artifact bytes, hash or size differs".into());
    }
    Ok(())
}

/// A durable selection must be read from P's authenticated authority boundary.
/// Deserializing this DTO is not evidence that a selection was authorized.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub schema: Name,
    pub publication: Id,
    pub policy_digest: Digest,
    pub configuration_digest: Digest,
    pub run: Id,
    pub part: Id,
    pub ordinal: Counter,
    pub object: Reference,
    pub object_values_digest: Digest,
    pub candidate: u8,
    pub slot: u16,
    pub report_digest: Digest,
    pub node: Name,
    pub parameter: ArtifactRef,
    pub intent_digest: Digest,
    pub authority_generation: Counter,
}

impl Selection {
    /// Compare a request to the exact selection loaded by the authority owner.
    /// Cross-Run reuse fails even when the approved parameter bytes are identical.
    pub fn verify_request(
        &self,
        submitted: &Self,
        policy: &Policy,
        index: &ValidatedIndex,
        host: &Name,
        intent: &Intent,
        parameter_bytes: &[u8],
    ) -> Result<(), String> {
        if self.schema.as_str() != "rx.execution-selection.v2"
            || self.authority_generation.0 == 0
            || self.object.revision.0 == 0
            || self.ordinal.0 == 0
            || self.ordinal.0 > policy.slot_order.len() as u64
            || policy
                .slot_order
                .get((self.ordinal.0 - 1) as usize)
                .copied()
                != Some(self.slot)
            || self.object.catalog != policy.workflow.catalog
            || canonical::bytes(self).map_err(|e| e.to_string())?
                != canonical::bytes(submitted).map_err(|e| e.to_string())?
            || self.policy_digest != policy.digest()?
            || self.report_digest != index.report(self.policy_digest, self.candidate, self.slot)?
        {
            return Err(
                "execution v2 selection differs from authoritative Run/Part binding".into(),
            );
        }
        nonzero(self.configuration_digest)?;
        nonzero(self.object.digest)?;
        nonzero(self.object_values_digest)?;
        reference(&self.parameter, PARAMETER_SCHEMA, MAX_PARAMETER_BYTES)?;
        verify_artifact(
            parameter_bytes,
            &self.parameter,
            MAX_PARAMETER_BYTES as usize,
        )?;
        let template = policy.templates.get(&self.node).ok_or("unpublished node")?;
        let mut expected = template.intent.clone();
        let Body::Program(goal) = &mut expected.body else {
            return Err("Program required".into());
        };
        goal.parameter_set = self.parameter.clone();
        if host != &template.host
            || expected.digest().map_err(|e| e.to_string())? != self.intent_digest
            || intent.digest().map_err(|e| e.to_string())? != self.intent_digest
        {
            return Err("execution v2 host or non-parameter Intent fields differ".into());
        }
        Ok(())
    }
}
