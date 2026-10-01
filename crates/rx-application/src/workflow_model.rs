//! Workflow authoring commands. Resolution does not qualify, publish or run work.
use rx_domain::{
    canonical,
    definition::{Definition, Reference},
    types::*,
    workflow,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Save {
    pub catalog: Id,
    pub id: Id,
    pub expected: Option<Counter>,
    pub label: String,
    pub spec: workflow::Spec,
}
pub struct PreparedSave {
    pub(crate) input: Save,
}
impl PreparedSave {
    pub fn prepare(input: Save) -> Result<Self, String> {
        if input.label.trim().is_empty() || input.label.chars().count() > 120 {
            return Err("workflow label".into());
        }
        input.spec.validate_shape()?;
        Ok(Self { input })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Version {
    pub reference: Reference,
    pub label: String,
    pub spec: workflow::Spec,
    pub created_by: Name,
    pub updated_by: Name,
    pub updated_at: TimePoint,
}
impl Version {
    pub fn digest(
        catalog: &Id,
        id: &Id,
        revision: Counter,
        label: &str,
        spec: &workflow::Spec,
    ) -> Result<Digest, String> {
        canonical::digest(
            "RX-WORKFLOW-MODEL-v1",
            &(catalog, id, revision, label, spec),
        )
        .map_err(|e| e.to_string())
    }
    pub fn verify(&self) -> Result<(), String> {
        self.spec.validate_shape()?;
        if self.reference.revision.0 == 0
            || Self::digest(
                &self.reference.catalog,
                &self.reference.id,
                self.reference.revision,
                &self.label,
                &self.spec,
            )? != self.reference.digest
        {
            return Err("workflow reference digest differs".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub reference: Reference,
    pub label: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub catalog: Id,
    pub workflows: Vec<Summary>,
    pub next: Option<Name>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub reference: Reference,
    pub report: workflow::Report,
    pub created_by: Name,
    pub created_at: TimePoint,
}

// Deliberately not Deserialize: only an authorized snapshot can enter the CPU worker.
pub struct Snapshot {
    pub(crate) model: Version,
    pub(crate) request: workflow::Request,
    pub(crate) definitions: BTreeMap<Reference, Definition>,
}
pub enum Preparation {
    Recorded(Box<Receipt>),
    Pending(Box<Snapshot>),
}
pub struct PreparedResolution {
    pub(crate) report: workflow::Report,
}
impl PreparedResolution {
    pub fn prepare(snapshot: Snapshot) -> Result<Self, String> {
        Ok(Self {
            report: workflow::resolve(
                &snapshot.model.spec,
                snapshot.request,
                &snapshot.definitions,
            )?,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportSummary {
    pub reference: Reference,
    pub workflow: Reference,
    pub slot_index: Counter,
    pub status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reports {
    pub catalog: Id,
    pub reports: Vec<ReportSummary>,
    pub next: Option<Name>,
}
