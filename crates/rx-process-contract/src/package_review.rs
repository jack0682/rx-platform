//! Immutable process verification artifacts; neither report nor reviewer decision is a run permit.
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_context_digest: Option<Digest>,
    pub schema: Name,
    pub id: Id,
    pub intake: Id,
    pub cell: Name,
    pub package_manifest: Digest,
    pub package_signature: Digest,
    pub configuration_digest: Digest,
    pub package_policy_fingerprint: Digest,
    pub package_policy_file_digest: Digest,
    pub verification_authority_digest: Digest,
    pub binding_selections: BTreeMap<Name, Name>,
}
impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(
            (self.schema.as_str(), self.device_context_digest.is_some()),
            ("rx.process-review-request.v1", false) | ("rx.process-review-request.v2", true)
        ) || self.binding_selections.len() > 128
        {
            return Err("process review request shape".into());
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-PROCESS-REVIEW-REQUEST-v1", self).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Issue {
    pub code: Name,
    pub location: String,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: Name,
    pub request: Request,
    pub validator_digest: Digest,
    pub validator_policy_file_digest: Digest,
    pub resolved: Option<ArtifactRef>,
    pub issues: Vec<Issue>,
}
impl Report {
    pub fn validate(&self) -> Result<(), String> {
        self.request.validate()?;
        if self.schema.as_str() != "rx.process-verification-report.v1"
            || self.issues.len() > 128
            || self
                .issues
                .iter()
                .any(|v| v.location.len() > 512 || v.detail.len() > 2048)
            || (self.issues.is_empty() && self.resolved.is_none())
            || self.resolved.as_ref().is_some_and(|r| {
                r.schema_id.as_str() != "rx.resolved-process.v1"
                    || r.size_bytes.0 == 0
                    || r.size_bytes.0 > 1_048_576
            })
        {
            return Err("process verification report shape".into());
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-PROCESS-VERIFICATION-REPORT-v1", self).map_err(|e| e.to_string())
    }
    pub fn signing_message(&self, key: &Name) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut message = b"RX-PROCESS-VERIFICATION-REPORT-v1\0".to_vec();
        message.extend(canonical::bytes(key).map_err(|e| e.to_string())?);
        message.push(0);
        message.extend(canonical::bytes(self).map_err(|e| e.to_string())?);
        Ok(message)
    }
}
