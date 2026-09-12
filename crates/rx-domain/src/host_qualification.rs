//! Host-local acceptance of P-reviewed qualification identity. Never a native dispatch permit.
use crate::{canonical, host_configuration as configuration, types::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellTarget {
    pub cell: Name,
    pub configuration: Digest,
    pub context_request: Id,
    pub context_sequence: Counter,
    pub definition: Digest,
    pub envelope: Digest,
    pub environment: Name,
    pub qualification: Id,
    pub qualification_revision: Counter,
    pub dependencies: Vec<Digest>,
    pub limitations: ArtifactRef,
    pub allowed_intents: Vec<Digest>,
    pub purposes: Vec<Name>,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub fence_request: Id,
    pub required_blocks: Vec<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: Name,
    pub id: Id,
    pub host: Name,
    pub expected_host_boot: Id,
    pub delivery_journal: Id,
    pub binding_digest: Digest,
    pub change: Id,
    pub review: Id,
    pub review_revision: Counter,
    pub review_digest: Digest,
    pub decision_revision: Counter,
    pub policy_digest: Digest,
    pub application_digest: Digest,
    pub cells: Vec<CellTarget>,
}
impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != "rx.host-qualification-request.v1"
            || self.review_revision.0 == 0
            || self.decision_revision.0 == 0
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
            return Err("qualification request identity/scope".into());
        }
        for c in &self.cells {
            if c.context_sequence.0 == 0
                || c.qualification_revision.0 == 0
                || c.epoch.0 == 0
                || c.scopes.is_empty()
                || c.scopes.len() > 128
                || c.scopes.values().any(|v| v.0 == 0)
                || c.required_blocks.is_empty()
                || c.required_blocks.len() > 512
                || c.required_blocks.iter().collect::<BTreeSet<_>>().len()
                    != c.required_blocks.len()
                || !matches!(c.environment.as_str(), "SIMULATION" | "PHYSICAL")
                || c.limitations.size_bytes.0 == 0
                || c.allowed_intents.len() > 4096
                || c.allowed_intents.iter().collect::<BTreeSet<_>>().len()
                    != c.allowed_intents.len()
                || c.dependencies.len() > 1024
                || c.dependencies.iter().collect::<BTreeSet<_>>().len() != c.dependencies.len()
                || ![c.configuration, c.definition, c.envelope]
                    .iter()
                    .all(|d| c.dependencies.contains(d))
                || c.purposes.is_empty()
                || c.purposes.len() > 3
                || c.purposes.iter().collect::<BTreeSet<_>>().len() != c.purposes.len()
                || c.purposes
                    .iter()
                    .any(|p| !matches!(p.as_str(), "PRODUCTION" | "SETUP" | "RECOVERY"))
            {
                return Err("qualification target bounds/dependencies".into());
            }
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<Digest, String> {
        self.validate()?;
        let mut r = self.clone();
        r.cells.sort_by(|a, b| a.cell.cmp(&b.cell));
        for c in &mut r.cells {
            c.dependencies.sort();
            c.allowed_intents.sort();
            c.required_blocks.sort();
            c.purposes.sort();
        }
        canonical::digest("RX-HOST-QUALIFICATION-REQUEST-v1", &r).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    Accepted,
    NotAccepted,
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
    pub reason: Option<Name>,
    pub quiescence: Option<configuration::Quiescence>,
    pub recorded_at: TimePoint,
}
impl Receipt {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.as_str() != "rx.host-qualification-receipt.v1"
            || self.request.digest()? != self.request_digest
            || self.host_boot != self.request.expected_host_boot
            || self.journal != self.request.delivery_journal
            || self.sequence.0 == 0
            || (self.status == Status::Accepted
                && (self.reason.is_some() || self.quiescence.is_none()))
            || (self.status == Status::NotAccepted && self.reason.is_none())
        {
            return Err("qualification receipt integrity".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedCell {
    pub cell: Name,
    pub request: Id,
    pub qualification: Id,
    pub qualification_revision: Counter,
    pub acceptance_sequence: Counter,
    pub host_boot: Id,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub configuration: Digest,
    pub context_request: Id,
    pub context_sequence: Counter,
    pub binding_digest: Digest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub schema: Name,
    pub snapshot: configuration::Snapshot,
    pub receipt: Option<Receipt>,
    pub accepted: Vec<AcceptedCell>,
    pub receipt_matches_current_host: bool,
    pub activation_authorized: bool,
}
impl Observation {
    pub fn current(&self) -> bool {
        let Some(r) = &self.receipt else {
            return false;
        };
        if r.status != Status::Accepted
            || r.host_boot != self.snapshot.host_boot
            || r.journal != self.snapshot.delivery_journal
            || r.request.binding_digest != self.snapshot.binding_digest
            || r.request.cells.len() != self.snapshot.cells.len()
        {
            return false;
        }
        r.request.cells.iter().all(|t| {
            let context = self.snapshot.cells.iter().any(|c| {
                c.cell == t.cell
                    && c.epoch == t.epoch
                    && c.scopes == t.scopes
                    && c.definition == t.definition
                    && c.envelope == t.envelope
                    && c.environment == t.environment
                    && c.applied.as_ref().is_some_and(|p| {
                        p.configuration == t.configuration
                            && p.request == t.context_request
                            && p.receipt_sequence == t.context_sequence
                            && p.binding_digest == r.request.binding_digest
                    })
            });
            let accepted = self.accepted.iter().any(|a| {
                a.cell == t.cell
                    && a.request == r.request.id
                    && a.acceptance_sequence == r.sequence
                    && a.qualification == t.qualification
                    && a.qualification_revision == t.qualification_revision
                    && a.host_boot == r.host_boot
                    && a.epoch == t.epoch
                    && a.scopes == t.scopes
                    && a.configuration == t.configuration
                    && a.context_request == t.context_request
                    && a.context_sequence == t.context_sequence
                    && a.binding_digest == r.request.binding_digest
            });
            context && accepted
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        let baseline = configuration::Observation {
            schema: Name::new("rx.host-process-configuration-observation.v1").unwrap(),
            snapshot: self.snapshot.clone(),
            receipt: None,
            context_matches_current_host: false,
            activation_authorized: false,
        };
        baseline.validate()?;
        if self.schema.as_str() != "rx.host-qualification-observation.v1"
            || self.activation_authorized
            || self.accepted.len() > 64
            || self
                .accepted
                .iter()
                .map(|c| &c.cell)
                .collect::<BTreeSet<_>>()
                .len()
                != self.accepted.len()
            || self.accepted.iter().any(|c| {
                c.acceptance_sequence.0 == 0
                    || c.qualification_revision.0 == 0
                    || !self.snapshot.cells.iter().any(|s| s.cell == c.cell)
            })
        {
            return Err("qualification observation shape".into());
        }
        if let Some(r) = &self.receipt {
            r.validate()?;
            if r.request.host != self.snapshot.host {
                return Err("qualification receipt Host differs".into());
            }
        }
        if self.receipt_matches_current_host != self.current() {
            return Err("qualification currency claim differs".into());
        }
        Ok(())
    }
}
