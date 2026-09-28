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
/// Revision-2 metadata observation of the binding generation used by the running Host.
/// It does not assert process application, native completion, qualification or permission.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingCommit {
    pub schema: Name,
    pub request: Id,
    pub plan_digest: Digest,
    pub cell: Name,
    pub before_configuration: Digest,
    pub after_configuration: Digest,
    pub before_installation_identity: Digest,
    pub after_installation_identity: Digest,
    pub delivery_journal: Id,
    pub evidence_journal: Id,
    pub binding_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation_identity: Option<Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_commit: Option<BindingCommit>,
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
        if let Some(commit) = &s.binding_commit
            && (commit.schema.as_str() != "rx.host-binding-commit-observation.v1"
                || s.installation_identity != Some(commit.after_installation_identity)
                || commit.delivery_journal != s.delivery_journal
                || commit.binding_digest != s.binding_digest
                || !s.cells.iter().any(|c| c.cell == commit.cell))
        {
            return Err("Host binding commit observation correlation differs".into());
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

#[cfg(test)]
mod binding_commit_tests {
    use super::*;
    fn n(v: &str) -> Name {
        Name::new(v).unwrap()
    }
    fn id(v: u8) -> Id {
        Id::new(format!("00000000-0000-4000-8000-{v:012}")).unwrap()
    }
    fn observation() -> Observation {
        Observation {
            schema: n("rx.host-process-configuration-observation.v1"),
            snapshot: Snapshot {
                installation_identity: None,
                binding_commit: None,
                schema: n("rx.host-process-configuration-snapshot.v1"),
                host: n("host/a"),
                host_boot: id(1),
                delivery_journal: id(2),
                binding_digest: Digest::from_bytes([3; 32]),
                cells: vec![CellObservation {
                    cell: n("cell/a"),
                    definition: Digest::from_bytes([1; 32]),
                    envelope: Digest::from_bytes([2; 32]),
                    environment: n("SIMULATION"),
                    epoch: Counter(1),
                    scopes: BTreeMap::from([(n("scope/a"), Counter(1))]),
                    blocked: vec![],
                    applied: None,
                }],
            },
            receipt: None,
            context_matches_current_host: false,
            activation_authorized: false,
        }
    }
    #[test]
    fn absence_is_decodable_but_commit_requires_matching_current_metadata() {
        let mut value = observation();
        value.validate().unwrap();
        let legacy = serde_json::to_value(&value).unwrap();
        assert!(legacy["snapshot"].get("binding_commit").is_none());
        assert!(legacy["snapshot"].get("installation_identity").is_none());
        let decoded: Observation = serde_json::from_value(legacy).unwrap();
        assert!(decoded.snapshot.binding_commit.is_none());
        value.snapshot.installation_identity = Some(Digest::from_bytes([8; 32]));
        value.snapshot.binding_commit = Some(BindingCommit {
            schema: n("rx.host-binding-commit-observation.v1"),
            request: id(3),
            plan_digest: Digest::from_bytes([4; 32]),
            cell: n("cell/a"),
            before_configuration: Digest::from_bytes([5; 32]),
            after_configuration: Digest::from_bytes([6; 32]),
            before_installation_identity: Digest::from_bytes([7; 32]),
            after_installation_identity: Digest::from_bytes([8; 32]),
            delivery_journal: id(2),
            evidence_journal: id(4),
            binding_digest: Digest::from_bytes([3; 32]),
        });
        value.validate().unwrap();
        let valid = value.clone();
        value.snapshot.installation_identity = None;
        assert!(value.validate().is_err());
        value = valid.clone();
        value
            .snapshot
            .binding_commit
            .as_mut()
            .unwrap()
            .delivery_journal = id(9);
        assert!(value.validate().is_err());
        value = valid.clone();
        value
            .snapshot
            .binding_commit
            .as_mut()
            .unwrap()
            .binding_digest = Digest::from_bytes([9; 32]);
        assert!(value.validate().is_err());
        value = valid;
        value.snapshot.binding_commit.as_mut().unwrap().cell = n("cell/other");
        assert!(value.validate().is_err());
    }
}
