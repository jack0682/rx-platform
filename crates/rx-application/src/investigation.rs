//! Human investigation attestation and conservative T5 conclusion. Never resource release.
use crate::{Identity, Permit, Work};
use rx_domain::{canonical, types::*};
use rx_package::SignatureEnvelope;
pub use rx_process_contract::investigation::{Action, Procedure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const POLICY_SCHEMA: &str = "rx.investigation-policy.v1";
pub const ATTESTATION_SCHEMA: &str = "rx.investigation-attestation.v1";
pub const DISPOSITION_SCHEMA: &str = "rx.investigation-disposition.v1";
/// Exhaustion is explicit; history is never truncated or deleted to make a partial scan pass.
pub const MAX_SCAN: usize = 4096;
pub const MAX_EVIDENCE: usize = 128;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub id: Name,
    pub public_key: Digest,
    pub cells: BTreeSet<Name>,
    pub environments: BTreeSet<Name>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema: Name,
    pub keys: Vec<Key>,
    pub procedures: Vec<ArtifactRef>,
}
impl Policy {
    pub fn digest(&self) -> Result<Digest, String> {
        if self.schema.as_str() != POLICY_SCHEMA
            || self.keys.is_empty()
            || self.keys.len() > 128
            || self
                .keys
                .iter()
                .map(|k| &k.id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.keys.len()
            || self.keys.iter().any(|k| {
                k.cells.is_empty()
                    || k.cells.len() > 64
                    || k.environments.is_empty()
                    || k.environments
                        .iter()
                        .any(|e| !matches!(e.as_str(), "SIMULATION" | "PHYSICAL"))
            })
            || self.procedures.is_empty()
            || self.procedures.len() > 128
            || self
                .procedures
                .iter()
                .map(|p| p.sha256)
                .collect::<BTreeSet<_>>()
                .len()
                != self.procedures.len()
            || self.procedures.iter().any(|p| {
                p.schema_id.as_str() != rx_process_contract::investigation::SCHEMA
                    || p.size_bytes.0 == 0
                    || p.size_bytes.0 > rx_process_contract::investigation::MAX_BYTES as u64
            })
        {
            return Err("investigation policy scope/allowlist differs".into());
        }
        canonical::digest("RX-INVESTIGATION-POLICY-v1", self).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyGeneration {
    pub id: Id,
    pub runtime_boot: Id,
    pub digest: Digest,
    pub file_digest: Digest,
}
/// Attribution is row data, not proof of current authentication.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub principal: Name,
    pub session: Id,
    pub terminal: Option<(Name, Digest)>,
}
impl From<&Identity> for Actor {
    fn from(i: &Identity) -> Self {
        Self {
            principal: i.principal.clone(),
            session: i.session.clone(),
            terminal: i.terminal.clone(),
        }
    }
}
impl Actor {
    pub fn identity(&self) -> Identity {
        Identity {
            principal: self.principal.clone(),
            session: self.session.clone(),
            terminal: self.terminal.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellCut {
    pub revision: Counter,
    pub epoch: Counter,
    pub scopes: BTreeMap<Name, Counter>,
    pub configuration_digest: Digest,
    pub definition: Digest,
    pub environment: Name,
    pub blocks: Vec<crate::Block>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceKind {
    Native,
    Receipt,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub id: Id,
    pub kind: EvidenceKind,
    pub reference: ArtifactRef,
    pub document: rx_ports::Document,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub installation: Id,
    pub store_generation: Id,
    pub runtime_boot: Id,
    pub work_revision: Counter,
    pub work: Work,
    pub permit: Permit,
    pub resources: BTreeMap<Name, crate::Resource>,
    pub cells: BTreeMap<Name, CellCut>,
    pub evidence: BTreeMap<Id, Evidence>,
    pub policy: PolicyGeneration,
    pub procedures: Vec<ArtifactRef>,
    pub operation_authorized: bool,
    pub resource_release_authorized: bool,
}
impl Context {
    pub fn digest(&self) -> Result<Digest, String> {
        canonical::digest("RX-INVESTIGATION-CONTEXT-v1", self).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Assertion {
    ResultRemainsUnknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttestSubmit {
    pub id: Id,
    pub operation: Id,
    pub expected_operation_revision: Counter,
    pub context_digest: Digest,
    pub procedure_digest: Digest,
    pub evidence_ids: Vec<Id>,
    pub assertion: Assertion,
    pub note: String,
    pub occurred_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attestation {
    pub schema: Name,
    pub id: Id,
    pub request: AttestSubmit,
    pub actor: Actor,
    pub recorded_at: TimePoint,
    pub context: Context,
    pub procedure: Procedure,
    pub procedure_reference: ArtifactRef,
    pub signature: SignatureEnvelope,
    pub operation_authorized: bool,
    pub resource_release_authorized: bool,
}
impl Attestation {
    pub fn reference(&self) -> Result<ArtifactRef, String> {
        reference(ATTESTATION_SCHEMA, self)
    }
}
pub enum Preflight {
    Recorded(Box<Attestation>),
    Verify(Box<Ticket>),
}
/// No Deserialize: only a current preflight can create the verification ticket.
pub struct Ticket {
    pub(crate) identity: Identity,
    pub(crate) key: Id,
    pub(crate) request: AttestSubmit,
    pub(crate) context: Context,
}
impl Ticket {
    pub fn request(&self) -> &AttestSubmit {
        &self.request
    }
    pub fn context(&self) -> &Context {
        &self.context
    }
    pub fn policy_generation(&self) -> &PolicyGeneration {
        &self.context.policy
    }
    pub fn procedure_digest(&self) -> Digest {
        self.request.procedure_digest
    }
    pub fn policy_digest(&self) -> Digest {
        self.context.policy.digest
    }
    pub fn policy_file_digest(&self) -> Digest {
        self.context.policy.file_digest
    }
    pub fn procedure_reference(&self) -> &ArtifactRef {
        self.context
            .procedures
            .iter()
            .find(|r| r.sha256 == self.request.procedure_digest)
            .expect("validated investigation ticket")
    }
}
pub struct ValidatedProcedure {
    procedure: Procedure,
    reference: ArtifactRef,
    signature: SignatureEnvelope,
    policy_digest: Digest,
}
impl ValidatedProcedure {
    pub fn check(
        policy: &Policy,
        expected: &ArtifactRef,
        bytes: &[u8],
        signature: SignatureEnvelope,
    ) -> Result<Self, String> {
        let policy_digest = policy.digest()?;
        if bytes.len() > rx_process_contract::investigation::MAX_BYTES
            || !policy.procedures.contains(expected)
        {
            return Err("investigation artifact is not allowed".into());
        }
        let procedure: Procedure = canonical::decode_json(bytes).map_err(|e| e.to_string())?;
        procedure.validate()?;
        let reference = procedure.reference()?;
        if canonical::bytes(&procedure).map_err(|e| e.to_string())? != bytes
            || reference != *expected
            || rx_package::content_digest(bytes) != reference.sha256
        {
            return Err("canonical investigation reference differs".into());
        }
        let key = policy
            .keys
            .iter()
            .find(|k| {
                k.id == signature.key
                    && k.cells.contains(&procedure.cell)
                    && k.environments.contains(&procedure.environment)
            })
            .ok_or("investigation signer scope missing")?;
        rx_package::verify_detached_message(
            &procedure.signing_message(&signature.key)?,
            &signature.signature,
            key.public_key.as_bytes(),
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            procedure,
            reference,
            signature,
            policy_digest,
        })
    }
}
/// Verified off the writer from exact artifact bytes and the currently pinned investigation policy.
pub struct Prepared {
    pub(crate) ticket: Ticket,
    pub(crate) procedure: Procedure,
    pub(crate) reference: ArtifactRef,
    pub(crate) signature: SignatureEnvelope,
}
impl Prepared {
    pub fn new(ticket: Ticket, validated: ValidatedProcedure) -> Result<Self, String> {
        let ValidatedProcedure {
            procedure,
            reference,
            signature,
            policy_digest,
        } = validated;
        let cell = &ticket.context.work.cell;
        let cut = ticket
            .context
            .cells
            .get(cell)
            .ok_or("investigation cell missing")?;
        if policy_digest != ticket.context.policy.digest
            || reference.sha256 != ticket.request.procedure_digest
            || procedure.cell != *cell
            || procedure.environment != cut.environment
            || procedure.definition != cut.definition
            || !procedure
                .profiles
                .contains(&ticket.context.work.intent.profile_digest)
        {
            return Err("investigation procedure not allowed for original operation".into());
        }
        Ok(Self {
            ticket,
            procedure,
            reference,
            signature,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Disposition {
    Quarantined,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Reason {
    UnknownOutcome,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordDisposition {
    pub operation: Id,
    pub expected_revision: Counter,
    pub evidence_ids: Vec<Id>,
    pub procedure_digest: Digest,
    pub disposition: Disposition,
    pub reason: Reason,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub schema: Name,
    pub id: Id,
    pub request: RecordDisposition,
    pub actor: Actor,
    pub recorded_at: TimePoint,
    pub attestation: Attestation,
    pub attestation_reference: ArtifactRef,
    pub before: Work,
    pub after: Work,
    pub operation_authorized: bool,
    pub resource_release_authorized: bool,
    pub resources_released: bool,
}
impl Receipt {
    pub fn reference(&self) -> Result<ArtifactRef, String> {
        reference(DISPOSITION_SCHEMA, self)
    }
}
pub enum DispositionPreflight {
    Recorded(Box<Receipt>),
    Verify(Box<DispositionTicket>),
}
pub struct DispositionTicket {
    pub(crate) identity: Identity,
    pub(crate) key: Id,
    pub(crate) request: RecordDisposition,
    pub(crate) attestation: Attestation,
    pub(crate) context: Context,
}
impl DispositionTicket {
    pub fn request(&self) -> &RecordDisposition {
        &self.request
    }
    pub fn attestation(&self) -> &Attestation {
        &self.attestation
    }
    pub fn context(&self) -> &Context {
        &self.context
    }
    pub fn procedure_reference(&self) -> &ArtifactRef {
        &self.attestation.procedure_reference
    }
    pub fn procedure_digest(&self) -> Digest {
        self.request.procedure_digest
    }
    pub fn policy_digest(&self) -> Digest {
        self.context.policy.digest
    }
    pub fn policy_file_digest(&self) -> Digest {
        self.context.policy.file_digest
    }
}
pub struct PreparedDisposition {
    pub(crate) ticket: DispositionTicket,
}
impl PreparedDisposition {
    pub fn new(ticket: DispositionTicket, validated: ValidatedProcedure) -> Result<Self, String> {
        if validated.policy_digest != ticket.context.policy.digest
            || validated.reference != ticket.attestation.procedure_reference
            || canonical::bytes(&validated.procedure).map_err(|e| e.to_string())?
                != canonical::bytes(&ticket.attestation.procedure).map_err(|e| e.to_string())?
        {
            return Err("disposition procedure or policy changed".into());
        }
        Ok(Self { ticket })
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct AttestationView {
    pub attestation: Attestation,
    pub current: bool,
    pub operation_authorized: bool,
    pub resource_release_authorized: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct AttestationPage {
    pub items: Vec<AttestationView>,
    pub next: Option<Id>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ReceiptView {
    pub receipt: Receipt,
    pub current: bool,
    pub operation_authorized: bool,
    pub resource_release_authorized: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct ReceiptPage {
    pub items: Vec<ReceiptView>,
    pub next: Option<Id>,
}
pub(crate) fn reference<T: Serialize>(schema: &str, value: &T) -> Result<ArtifactRef, String> {
    let bytes = canonical::bytes(value).map_err(|e| e.to_string())?;
    Ok(ArtifactRef {
        sha256: rx_package::content_digest(&bytes),
        schema_id: Name::new(schema).map_err(|e| e.to_string())?,
        size_bytes: Counter(bytes.len() as u64),
    })
}
