//! Candidate bounded Program input selection. This pure check grants no authority.
//! Runtime admission still uses its existing exact-intent path until the versioned
//! package/configuration/Host integration is implemented and reviewed.
use rx_domain::{
    canonical,
    intent::{Body, Intent, Kind},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SCHEMA: &str = "rx.program-input-policy.v1";
pub const MAX_CHOICES: usize = 64;
pub const MAX_PARAMETER_BYTES: u64 = 1_048_576;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema: Name,
    pub template_digest: Digest,
    /// Complete preapproved set, including the template's original parameter set.
    pub parameter_sets: Vec<ArtifactRef>,
}
impl Policy {
    pub fn validate(&self, template: &Intent) -> Result<(), String> {
        let template = template.normalized().map_err(|e| e.to_string())?;
        let Body::Program(goal) = &template.body else {
            return Err("program input policy requires Program body".into());
        };
        if template.kind != Kind::FiniteAction
            || self.schema.as_str() != SCHEMA
            || self.template_digest != template.digest().map_err(|e| e.to_string())?
            || self.parameter_sets.is_empty()
            || self.parameter_sets.len() > MAX_CHOICES
        {
            return Err("program input policy schema, template or bound differs".into());
        }
        let mut hashes = BTreeSet::new();
        for reference in &self.parameter_sets {
            if reference.schema_id != goal.parameter_set.schema_id
                || reference.size_bytes.0 == 0
                || reference.size_bytes.0 > MAX_PARAMETER_BYTES
                || !hashes.insert(reference.sha256)
            {
                return Err("parameter schema, size or duplicate content identity".into());
            }
        }
        if !self.parameter_sets.contains(&goal.parameter_set) {
            return Err("template parameter set must be explicitly approved".into());
        }
        Ok(())
    }
    pub fn digest(&self, template: &Intent) -> Result<Digest, String> {
        self.validate(template)?;
        let mut sorted = self.clone();
        sorted.parameter_sets.sort_by_key(|r| r.sha256);
        canonical::digest("RX-PROGRAM-INPUT-POLICY-v1", &sorted).map_err(|e| e.to_string())
    }
    pub fn variants(&self, template: &Intent) -> Result<Vec<Intent>, String> {
        self.validate(template)?;
        let template = template.normalized().map_err(|e| e.to_string())?;
        self.parameter_sets
            .iter()
            .map(|reference| {
                let mut candidate = template.clone();
                let Body::Program(goal) = &mut candidate.body else {
                    unreachable!("validated Program");
                };
                goal.parameter_set = reference.clone();
                Ok(candidate)
            })
            .collect()
    }
}

/// None preserves existing exact-intent semantics. Some is only meaningful when
/// obtained from a separately admitted/qualified immutable configuration.
pub fn accepts(
    template: &Intent,
    policy: Option<&Policy>,
    submitted: &Intent,
) -> Result<bool, String> {
    let submitted = submitted.normalized().map_err(|e| e.to_string())?;
    match policy {
        None => Ok(template.normalized().map_err(|e| e.to_string())? == submitted),
        Some(policy) => Ok(policy.variants(template)?.contains(&submitted)),
    }
}

/// Concrete variants used consistently by admission, Host projection and reads.
pub fn variants(template: &Intent, policy: Option<&Policy>) -> Result<Vec<Intent>, String> {
    match policy {
        Some(p) => p.variants(template),
        None => Ok(vec![template.normalized().map_err(|e| e.to_string())?]),
    }
}
pub fn accepts_digest(
    template: &Intent,
    policy: Option<&Policy>,
    digest: Digest,
) -> Result<bool, String> {
    for intent in variants(template, policy)? {
        if intent.digest().map_err(|e| e.to_string())? == digest {
            return Ok(true);
        }
    }
    Ok(false)
}
pub fn same(template: &Intent, a: Option<&Policy>, b: Option<&Policy>) -> Result<bool, String> {
    match (a, b) {
        (None, None) => Ok(true),
        (Some(a), Some(b)) => Ok(a.digest(template)? == b.digest(template)?),
        _ => Ok(false),
    }
}
