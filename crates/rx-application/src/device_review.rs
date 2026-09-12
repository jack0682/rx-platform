//! Package software review tokens and cryptographic checks; physical commissioning is separate.
pub use crate::process_review::{Choice, Decide, Decision, Submit, VerifierKey};
use crate::{Identity, package_intake::Registration};
use rx_domain::{canonical, types::*};
use rx_package::{SignatureEnvelope, store::StoredPackage};
pub use rx_process_contract::device_review::{Report, Request};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authority {
    pub schema: Name,
    pub keys: Vec<VerifierKey>,
}
impl Authority {
    pub fn digest(&self) -> Result<Digest, String> {
        if self.schema.as_str() != "rx.device-verification-authority.v1"
            || self.keys.is_empty()
            || self.keys.len() > 128
            || self
                .keys
                .iter()
                .map(|k| &k.id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.keys.len()
            || self
                .keys
                .iter()
                .any(|k| k.validators.is_empty() || k.validators.len() > 64)
        {
            return Err("device review authority shape".into());
        }
        let mut a = self.clone();
        a.keys.sort_by(|a, b| a.id.cmp(&b.id));
        canonical::digest("RX-DEVICE-VERIFICATION-AUTHORITY-v1", &a).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub id: Id,
    pub intake: Id,
    pub cell: Name,
    pub configuration_digest: Digest,
    pub policy_generation: Id,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub request: Request,
    pub registration: Registration,
    pub submitted_by: Name,
    pub requested_by: Name,
    pub created_at: TimePoint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Version {
    pub review: Id,
    pub cell: Name,
    pub revision: Counter,
    pub review_digest: Digest,
    pub checker_digest: Digest,
    pub report_digest: Digest,
    pub report: Report,
    pub signature: SignatureEnvelope,
    pub ready_for_software_approval: bool,
    pub recorded_by: Name,
    pub recorded_at: TimePoint,
}
impl Version {
    pub fn digest(&self) -> Result<Digest, String> {
        let mut v = self.clone();
        v.review_digest = Digest::from_bytes([0; 32]);
        canonical::digest("RX-DEVICE-REVIEW-VERSION-v1", &v).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Detail {
    pub job: Job,
    pub version: Option<Version>,
    pub decision: Option<Decision>,
    pub latest_report_revision: Option<Counter>,
    pub is_latest: bool,
    pub context_current: bool,
    pub approval_matches_current_review: bool,
    pub activation_authorized: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub cell: Name,
    pub intake: Id,
    pub reviews: Vec<Summary>,
    pub next: Option<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub id: Id,
    pub requested_by: Name,
    pub created_at: TimePoint,
    pub report_revision: Option<Counter>,
    pub report_digest: Option<Digest>,
    pub ready_for_software_approval: bool,
    pub context_current: bool,
    pub decision_revision: Option<Counter>,
    pub choice: Option<Choice>,
    pub approval_matches_current_review: bool,
}
impl Summary {
    pub(crate) fn from_detail(d: Detail) -> Self {
        Self {
            id: d.job.request.id,
            requested_by: d.job.requested_by,
            created_at: d.job.created_at,
            report_revision: d.version.as_ref().map(|v| v.revision),
            report_digest: d.version.as_ref().map(|v| v.report_digest),
            ready_for_software_approval: d
                .version
                .as_ref()
                .is_some_and(|v| v.ready_for_software_approval),
            context_current: d.context_current,
            decision_revision: d.decision.as_ref().map(|v| v.revision),
            choice: d.decision.map(|v| v.choice),
            approval_matches_current_review: d.approval_matches_current_review,
        }
    }
}
pub struct Ticket {
    pub(crate) job: Job,
    pub(crate) identity: Identity,
    pub(crate) key: Id,
    pub(crate) input: Submit,
    pub(crate) registration: Registration,
    pub(crate) boot: Id,
    pub(crate) issued: TimePoint,
}
impl Ticket {
    pub fn job(&self) -> &Job {
        &self.job
    }
    pub fn registration(&self) -> &Registration {
        &self.registration
    }
    pub fn directory(&self) -> &rx_package::PackagePath {
        &self.input.directory
    }
}
pub enum Preflight {
    Recorded(Box<Version>),
    Verify(Box<Ticket>),
}
pub struct Validated {
    pub(crate) stored: StoredPackage,
    pub(crate) report: Report,
    pub(crate) signature: SignatureEnvelope,
}
impl Validated {
    pub fn check(
        job: &Job,
        stored: StoredPackage,
        authority: &Authority,
        report: Report,
        signature: SignatureEnvelope,
    ) -> Result<Self, String> {
        report.validate()?;
        if report.request.digest()? != job.request.digest()?
            || authority.digest()? != job.request.verification_authority_digest
            || stored.object().manifest != job.request.package_manifest
            || stored.object().signature != job.request.package_signature
            || stored.policy_fingerprint() != job.request.package_policy_fingerprint
        {
            return Err("device report request/package/authority differs".into());
        }
        let key = authority
            .keys
            .iter()
            .find(|k| k.id == signature.key && k.validators.contains(&report.validator_digest))
            .ok_or("untrusted device signer/validator")?;
        rx_package::verify_detached_message(
            &report.signing_message(&signature.key)?,
            &signature.signature,
            key.public_key.as_bytes(),
        )
        .map_err(|e| e.to_string())?;
        let catalog =
            crate::device_catalog::extract(stored.package())?.ok_or("device catalog missing")?;
        let bytes = canonical::bytes(&catalog).map_err(|e| e.to_string())?;
        if rx_package::content_digest(&bytes) != job.request.catalog.sha256
            || bytes.len() as u64 != job.request.catalog.size_bytes.0
            || catalog.installation != job.request.installation
            || catalog.cell != job.request.cell
        {
            return Err("reviewed device catalog differs from verified package".into());
        }
        Ok(Self {
            stored,
            report,
            signature,
        })
    }
}
pub struct Prepared {
    pub(crate) ticket: Ticket,
    pub(crate) validated: Validated,
}
impl Prepared {
    pub fn new(ticket: Ticket, validated: Validated) -> Result<Self, String> {
        if validated.report.digest()? != ticket.input.report_digest
            || validated.stored.owner() != &ticket.registration.store_owner
        {
            return Err("device ticket/verification mismatch".into());
        }
        Ok(Self { ticket, validated })
    }
}
pub struct DecisionTicket {
    pub(crate) job: Job,
    pub(crate) version: Version,
    pub(crate) identity: Identity,
    pub(crate) key: Id,
    pub(crate) input: Decide,
    pub(crate) registration: Option<Registration>,
    pub(crate) boot: Id,
    pub(crate) issued: TimePoint,
}
impl DecisionTicket {
    pub fn job(&self) -> &Job {
        &self.job
    }
    pub fn version(&self) -> &Version {
        &self.version
    }
    pub fn requires_verification(&self) -> bool {
        self.input.choice == Choice::Approve
    }
}
pub enum DecisionPreflight {
    Recorded(Box<Decision>),
    Verify(Box<DecisionTicket>),
}
pub struct PreparedDecision {
    pub(crate) ticket: DecisionTicket,
    pub(crate) validated: Option<Validated>,
}
impl PreparedDecision {
    pub fn approve(ticket: DecisionTicket, validated: Validated) -> Result<Self, String> {
        if ticket.input.choice != Choice::Approve
            || !validated.report.passed()
            || validated.report.digest()? != ticket.version.report_digest
            || canonical::bytes(&validated.signature).map_err(|e| e.to_string())?
                != canonical::bytes(&ticket.version.signature).map_err(|e| e.to_string())?
        {
            return Err("fresh device approval material differs".into());
        }
        Ok(Self {
            ticket,
            validated: Some(validated),
        })
    }
    pub fn reject(ticket: DecisionTicket) -> Result<Self, String> {
        if ticket.input.choice != Choice::Reject {
            return Err("approval requires fresh verification".into());
        }
        Ok(Self {
            ticket,
            validated: None,
        })
    }
}
pub fn checker_digest() -> Digest {
    rx_package::content_digest(
        concat!(
            "RX-DEVICE-CHECKER-v1\0",
            include_str!("device_review.rs"),
            include_str!("device_catalog.rs"),
            include_str!("../../rx-process-contract/src/device_review.rs"),
            include_str!("../../rx-process-contract/src/device_catalog.rs")
        )
        .as_bytes(),
    )
}
