//! Engineer-owned source drafts. They never modify the installed cell or create an executable package.
use rx_domain::{canonical, types::*};
use rx_process_contract::source_validation;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Save {
    pub id: Id,
    pub cell: Name,
    pub expected: Option<Counter>,
    pub title: String,
    pub document: serde_json::Value,
}
pub struct PreparedSave {
    pub(crate) input: Save,
    pub(crate) digest: Digest,
    pub(crate) report: source_validation::Report,
}
impl PreparedSave {
    /// CPU validation belongs on a worker before entering the state writer.
    pub fn prepare(input: Save) -> Result<Self, String> {
        if input.title.trim().is_empty() || input.title.chars().count() > 120 {
            return Err("draft title must have 1–120 characters".into());
        }
        let bytes = canonical::bytes(&input.document).map_err(|e| e.to_string())?;
        if bytes.len() > 524_288 {
            return Err("draft document exceeds 512 KiB".into());
        }
        let digest = canonical::digest("RX-PROCESS-DRAFT-DOCUMENT-v1", &input.document)
            .map_err(|e| e.to_string())?;
        let report = source_validation::document(&input.document);
        Ok(Self {
            input,
            digest,
            report,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Version {
    pub id: Id,
    pub cell: Name,
    pub revision: Counter,
    pub title: String,
    pub document_digest: Digest,
    pub validation: source_validation::Report,
    pub created_by: Name,
    pub updated_by: Name,
    pub updated_at: TimePoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detail {
    pub version: Version,
    pub document: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub cell: Name,
    pub drafts: Vec<Summary>,
    pub next: Option<Id>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub id: Id,
    pub cell: Name,
    pub revision: Counter,
    pub title: String,
    pub document_digest: Digest,
    pub structurally_valid: bool,
    pub issue_count: Counter,
    pub updated_by: Name,
}
impl From<&Version> for Summary {
    fn from(v: &Version) -> Self {
        Self {
            id: v.id.clone(),
            cell: v.cell.clone(),
            revision: v.revision,
            title: v.title.clone(),
            document_digest: v.document_digest,
            structurally_valid: v.validation.structurally_valid,
            issue_count: Counter(v.validation.issues.len() as u64),
            updated_by: v.updated_by.clone(),
        }
    }
}
