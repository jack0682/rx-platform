//! Human-facing read models, never an authority or a filtered control-journal cursor.
use crate::model::*;
use rx_domain::{operation::Operation, types::*};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize)]
pub struct UserProfile {
    pub principal: Name,
    pub terminal: Option<Name>,
    pub roles: BTreeSet<Role>,
    pub cells: BTreeSet<Name>,
    pub expires_at: TimePoint,
}

#[derive(Clone, Debug, Serialize)]
pub struct Versioned<T> {
    pub revision: Counter,
    pub value: T,
}

#[derive(Clone, Debug, Serialize)]
pub struct CellSummary {
    pub id: Name,
    pub mode: Option<OperatingMode>,
    pub commissioning: Option<Commissioning>,
    pub environment: Environment,
    pub epoch: Counter,
    pub definition: ArtifactRef,
    pub envelope: ArtifactRef,
    pub recipe: ArtifactRef,
    pub site_config_digest: Digest,
    pub hosts: Vec<Name>,
    /// A verified envelope alone does not assert current readiness to run.
    pub qualification: Option<Qualification>,
    pub blocks: Vec<Block>,
    pub open_cases: Vec<Id>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkSummary {
    pub cell: Name,
    pub run: Id,
    pub part: Option<Id>,
    pub host: Name,
    pub operation: Operation,
}

#[derive(Clone, Debug, Serialize)]
pub struct CellOverview {
    pub diagnostics: crate::diagnostics::CellDiagnostics,
    pub cell: Versioned<CellSummary>,
    pub runs: Vec<Versioned<Run>>,
    pub work: Vec<WorkSummary>,
    pub runs_truncated: bool,
    pub work_truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Overview {
    /// Opaque identity of this read transaction. Not an event sequence or resumable cursor.
    pub snapshot_id: Id,
    pub installation: Installation,
    pub observed_at: TimePoint,
    pub user: UserProfile,
    pub cells: Vec<CellOverview>,
}
