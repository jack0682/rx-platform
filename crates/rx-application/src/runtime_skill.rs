//! Skill-facing projections of installed P processes. Never execution authority.
use crate::projection::Versioned;
use crate::{Block, Commissioning, Environment, Installation, OperatingMode, PartAttempt, Run};
use rx_domain::{operation::Operation, types::*};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Binding {
    pub binding_digest: Digest,
    pub name: Name,
    pub cell: Name,
    pub environment: Environment,
    pub recipe: ArtifactRef,
    pub envelope: ArtifactRef,
    pub source_digest: Digest,
    pub package_digest: Option<Digest>,
    pub site_config_digest: Digest,
    pub maximum_budget: Counter,
    pub input_mode: &'static str,
}
#[derive(Clone, Debug, Serialize)]
pub struct Installed {
    pub binding: Binding,
    pub cell_revision: Counter,
    pub epoch: Counter,
    pub commissioning: Option<Commissioning>,
    pub mode: Option<OperatingMode>,
    pub blocks: Vec<Block>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Catalog {
    pub schema: &'static str,
    pub snapshot_id: Id,
    pub installation: Installation,
    pub observed_at: TimePoint,
    pub bindings: Vec<Installed>,
    pub truncated: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Work {
    /// Current P-owned resource state, observed in the same read transaction.
    pub resources: Vec<Versioned<crate::Resource>>,
    pub reconciliation: Option<crate::ReconciliationRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution: Option<Box<rx_process_contract::execution_v2::OperationBinding>>,
    pub operation: Operation,
    pub part: Option<Id>,
    pub slot: Name,
    pub host: Name,
    pub activation: Id,
    pub invocation: Option<Id>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ResultView {
    pub schema: &'static str,
    pub snapshot_id: Id,
    pub installation: Installation,
    pub observed_at: TimePoint,
    pub binding: Binding,
    pub run: Versioned<Run>,
    pub parts: Vec<Versioned<PartAttempt>>,
    pub work: Vec<Work>,
    pub details_truncated: bool,
    pub current_cell_revision: Counter,
    pub current_binding_matches: bool,
    pub result_owner: &'static str,
}
