//! Device package software verification. No physical validation or activation claim is representable.
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: Name,
    pub id: Id,
    pub intake: Id,
    pub installation: Id,
    pub cell: Name,
    pub package_manifest: Digest,
    pub package_signature: Digest,
    pub catalog: ArtifactRef,
    pub configuration_digest: Digest,
    pub package_policy_fingerprint: Digest,
    pub package_policy_file_digest: Digest,
    pub verification_authority_digest: Digest,
}
impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != "rx.device-review-request.v1"
            || self.catalog.schema_id.as_str() != "rx.device-operation-catalog.v1"
            || !(1..=131_072).contains(&self.catalog.size_bytes.0)
        {
            return Err("device review request shape".into());
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-DEVICE-REVIEW-REQUEST-v1", self).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Check {
    ContentSignature,
    DeviceSourceConsistency,
    CatalogRequestBinding,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResultKind {
    Passed,
    Failed,
    NotPerformed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Scope {
    DevicePackageSoftware,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: Name,
    pub request: Request,
    pub validator_digest: Digest,
    pub validator_policy_file_digest: Digest,
    pub scope: Scope,
    pub checks: BTreeMap<Check, ResultKind>,
    pub issues: Vec<crate::package_review::Issue>,
}
impl Report {
    pub fn validate(&self) -> Result<(), String> {
        self.request.validate()?;
        if self.schema.as_str() != "rx.device-verification-report.v1"
            || self.checks.len() != 3
            || self.issues.len() > 32
            || self
                .issues
                .iter()
                .any(|i| i.location.len() > 512 || i.detail.len() > 2048)
            || (self.checks.values().any(|v| *v != ResultKind::Passed) && self.issues.is_empty())
            || canonical::bytes(self).map_err(|e| e.to_string())?.len() > 131_072
        {
            return Err("device report shape/check coverage differs".into());
        }
        Ok(())
    }
    pub fn passed(&self) -> bool {
        self.validate().is_ok()
            && self.issues.is_empty()
            && self.checks.values().all(|v| *v == ResultKind::Passed)
    }
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        canonical::digest("RX-DEVICE-VERIFICATION-REPORT-v1", self).map_err(|e| e.to_string())
    }
    pub fn signing_message(&self, key: &Name) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut bytes = b"RX-DEVICE-VERIFICATION-REPORT-v1\0".to_vec();
        bytes.extend(canonical::bytes(key).map_err(|e| e.to_string())?);
        bytes.push(0);
        bytes.extend(canonical::bytes(self).map_err(|e| e.to_string())?);
        Ok(bytes)
    }
}
