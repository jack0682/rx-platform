//! Host acceptance of process context. This does not assert native driver/PLC reconfiguration.
use crate::{canonical, types::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellTarget {
    pub cell: Name,
    pub expected_context: Option<Digest>,
    pub before_configuration: Digest,
    pub after_configuration: Digest,
    pub recipe: ArtifactRef,
    pub definition: Digest,
    pub envelope: Digest,
    pub environment: Name,
    pub required_intents: Vec<Digest>,
    pub required_conditions: Vec<Name>,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub fence_request: Id,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: Name,
    pub id: Id,
    pub change: Id,
    pub preparation: Counter,
    pub plan_digest: Digest,
    pub host: Name,
    pub expected_host_boot: Id,
    pub expected_delivery_journal: Id,
    pub binding_digest: Digest,
    pub cells: Vec<CellTarget>,
}
impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != "rx.host-process-configuration-request.v1"
            || self.preparation.0 == 0
            || self.cells.is_empty()
            || self.cells.len() > 64
            || self
                .cells
                .iter()
                .map(|c| &c.cell)
                .collect::<BTreeSet<_>>()
                .len()
                != self.cells.len()
        {
            return Err("configuration request shape".into());
        }
        for c in &self.cells {
            if c.epoch.0 == 0
                || c.scopes.is_empty()
                || c.scopes.len() > 128
                || c.scopes.values().any(|v| v.0 == 0)
                || !matches!(c.environment.as_str(), "SIMULATION" | "PHYSICAL")
                || c.recipe.size_bytes.0 == 0
                || c.required_intents.len() > 4096
                || c.required_intents.iter().collect::<BTreeSet<_>>().len()
                    != c.required_intents.len()
                || c.required_conditions.len() > 128
                || c.required_conditions.iter().collect::<BTreeSet<_>>().len()
                    != c.required_conditions.len()
            {
                return Err("configuration target shape".into());
            }
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        let mut v = self.clone();
        v.cells.sort_by(|a, b| a.cell.cmp(&b.cell));
        for c in &mut v.cells {
            c.required_intents.sort();
            c.required_conditions.sort();
        }
        canonical::digest("RX-HOST-PROCESS-CONFIGURATION-v1", &v).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedContext {
    pub cell: Name,
    pub configuration: Digest,
    pub change: Id,
    pub request: Id,
    pub receipt_sequence: Counter,
    pub binding_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellObservation {
    pub cell: Name,
    pub definition: Digest,
    pub envelope: Digest,
    pub environment: Name,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub blocked: Vec<Id>,
    pub applied: Option<AppliedContext>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: Name,
    pub host: Name,
    pub host_boot: Id,
    pub delivery_journal: Id,
    pub binding_digest: Digest,
    pub cells: Vec<CellObservation>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    AppliedUnqualified,
    NotApplied,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Effect {
    Installed,
    AlreadyPresent,
    None,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quiescence {
    pub device_session: Id,
    pub observed_at: TimePoint,
    pub uncertainty_ns: Counter,
    pub resources: Vec<Name>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub schema: Name,
    pub request: Request,
    pub request_digest: Digest,
    pub host_boot: Id,
    pub journal: Id,
    pub sequence: Counter,
    pub status: Status,
    pub effect: Effect,
    pub reason: Option<Name>,
    pub quiescence: Option<Quiescence>,
    pub recorded_at: TimePoint,
}
impl Receipt {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != "rx.host-process-configuration-receipt.v1"
            || self.request.digest()? != self.request_digest
            || self.sequence.0 == 0
            || self.host_boot != self.request.expected_host_boot
            || self.journal != self.request.expected_delivery_journal
            || (self.status == Status::AppliedUnqualified
                && (self.effect == Effect::None
                    || self.reason.is_some()
                    || self.quiescence.is_none()))
            || (self.status == Status::NotApplied
                && (self.effect != Effect::None || self.reason.is_none()))
        {
            return Err("configuration receipt shape/identity".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub schema: Name,
    pub snapshot: Snapshot,
    pub receipt: Option<Receipt>,
    pub context_matches_current_host: bool,
    pub activation_authorized: bool,
}

impl Observation {
    pub fn validate(&self) -> Result<(), String> {
        let s = &self.snapshot;
        if self.schema.as_str() != "rx.host-process-configuration-observation.v1"
            || s.schema.as_str() != "rx.host-process-configuration-snapshot.v1"
            || self.activation_authorized
            || s.cells.is_empty()
            || s.cells.len() > 64
            || s.cells
                .iter()
                .map(|c| &c.cell)
                .collect::<BTreeSet<_>>()
                .len()
                != s.cells.len()
        {
            return Err("Host configuration observation shape".into());
        }
        for c in &s.cells {
            if c.epoch.0 == 0
                || c.scopes.is_empty()
                || c.scopes.values().any(|v| v.0 == 0)
                || c.applied
                    .as_ref()
                    .is_some_and(|a| a.cell != c.cell || a.receipt_sequence.0 == 0)
            {
                return Err("Host context snapshot shape".into());
            }
        }
        if let Some(r) = &self.receipt {
            r.validate()?;
            if r.request.host != s.host {
                return Err("receipt Host differs".into());
            }
        }
        let expected = self.receipt.as_ref().is_some_and(|r| {
            r.status == Status::AppliedUnqualified
                && r.host_boot == s.host_boot
                && r.journal == s.delivery_journal
                && r.request.binding_digest == s.binding_digest
                && r.request.cells.iter().all(|t| {
                    s.cells.iter().any(|c| {
                        c.cell == t.cell
                            && c.epoch == t.epoch
                            && c.scopes == t.scopes
                            && c.applied.as_ref().is_some_and(|a| {
                                a.configuration == t.after_configuration
                                    && a.binding_digest == s.binding_digest
                                    && a.request == r.request.id
                                    && a.receipt_sequence == r.sequence
                                    && a.change == r.request.change
                            })
                    })
                })
        });
        if expected != self.context_matches_current_host {
            return Err("Host context currency claim differs".into());
        }
        Ok(())
    }
}
