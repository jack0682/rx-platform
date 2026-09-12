use crate::{DomainError, Result, canonical, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Kind {
    FiniteAction,
    EnsureState,
    ModeTransition,
    ControlSession,
    LifecycleTransition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleVerb {
    Prepare,
    Activate,
    Deactivate,
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub kind: Kind,
    pub target: Name,
    pub profile_digest: Digest,
    pub site_config_digest: Digest,
    pub calibration_digests: Vec<Digest>,
    pub resource_set: Vec<Name>,
    pub execution_timeout_ms: Counter,
    pub prepare_validity_ms: Counter,
    pub completion_rule: Name,
    pub cancel_rule: Name,
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Body {
    Trajectory(TrajectoryGoal),
    Program(ProgramGoal),
    Predicate(PredicateGoal),
    Mode(ModeGoal),
    Control(ControlGoal),
    Lifecycle(LifecycleGoal),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrajectoryGoal {
    pub trajectory: ArtifactRef,
    pub joint_group: Name,
    pub tool_digest: Digest,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramGoal {
    pub program: ArtifactRef,
    pub parameter_set: ArtifactRef,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredicateGoal {
    pub predicate_id: Name,
    pub target: TypedValue,
    pub settle_ms: Counter,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeGoal {
    pub mode_id: Name,
    pub transition_profile: Digest,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlGoal {
    pub source_id: Name,
    pub sample_schema: Name,
    pub stream_profile: Digest,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleGoal {
    pub transition: LifecycleVerb,
    pub target_level: Name,
    pub support_evidence: Vec<Id>,
}

fn unique<T: Ord>(values: &[T], field: &str) -> Result<()> {
    if values.iter().collect::<BTreeSet<_>>().len() != values.len() {
        return Err(DomainError::InvalidInput(format!("duplicate {field}")));
    }
    Ok(())
}
impl Intent {
    /// Pure validation; profile admission and physical conditions belong to the application.
    pub fn normalized(&self) -> Result<Self> {
        self.execution_timeout_ms.nonzero("execution_timeout_ms")?;
        self.prepare_validity_ms.nonzero("prepare_validity_ms")?;
        if self.resource_set.is_empty() {
            return Err(DomainError::InvalidInput("empty resource set".into()));
        }
        unique(&self.resource_set, "resource_set")?;
        unique(&self.calibration_digests, "calibration_digests")?;
        let kind_matches = matches!(
            (&self.kind, &self.body),
            (Kind::FiniteAction, Body::Trajectory(_) | Body::Program(_))
                | (Kind::EnsureState, Body::Predicate(_))
                | (Kind::ModeTransition, Body::Mode(_))
                | (Kind::ControlSession, Body::Control(_))
                | (Kind::LifecycleTransition, Body::Lifecycle(_))
        );
        if !kind_matches {
            return Err(DomainError::InvalidInput("kind/body mismatch".into()));
        }
        let mut result = self.clone();
        result.resource_set.sort();
        result.calibration_digests.sort();
        if let Body::Lifecycle(goal) = &mut result.body {
            unique(&goal.support_evidence, "support_evidence")?;
            goal.support_evidence.sort();
        }
        Ok(result)
    }
    pub fn digest(&self) -> Result<Digest> {
        canonical::digest("RX-INTENT-v1", &self.normalized()?)
    }
}
