//! Versioned authoring bindings. They are compile inputs, never operation permissions.
use rx_domain::{intent::Kind, types::*};
pub use rx_process_contract::compile_input::{BindingPlanRef, DeviceSource};
use rx_process_contract::model::ActionBinding;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectCatalog {
    pub cell: Name,
    pub device_plans: Vec<BindingPlanRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Save {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub device_plans: Vec<BindingPlanRef>,
    pub draft: Id,
    pub cell: Name,
    pub source_revision: Counter,
    pub expected: Option<Counter>,
    pub catalog_digest: Digest,
    pub selections: BTreeMap<Name, Name>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_plan: Option<BindingPlanRef>,
    pub step: Name,
    pub host: Name,
    pub target: Name,
    pub kind: Kind,
    pub resources: Vec<Name>,
    pub intent_digest: Digest,
    pub step_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub device_plans: Vec<BindingPlanRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_cells: Vec<Name>,
    pub cell: Name,
    pub definition: Digest,
    pub envelope: Digest,
    pub catalog_digest: Digest,
    pub candidates: Vec<Candidate>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Version {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub device_plans: Vec<BindingPlanRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_cells: Vec<Name>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub device_sources: BTreeMap<Name, DeviceSource>,
    pub draft: Id,
    pub cell: Name,
    pub revision: Counter,
    pub source_revision: Counter,
    pub source_digest: Digest,
    pub catalog_digest: Digest,
    pub selections: BTreeMap<Name, Name>,
    pub origins: BTreeMap<Name, Digest>,
    pub resolved: BTreeMap<Name, ActionBinding>,
    pub missing: Vec<Name>,
    pub complete: bool,
    pub updated_by: Name,
    pub updated_at: TimePoint,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StaleReason {
    SourceChanged,
    CatalogChanged,
    DevicePlanChanged,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub cell: Name,
    pub draft: Id,
    pub current_source_revision: Counter,
    pub current_catalog_digest: Digest,
    pub binding: Option<Version>,
    pub stale: Vec<StaleReason>,
}
