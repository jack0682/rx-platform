//! External decision transport only. These public DTOs are not live permission.
//! Receiver Request, Challenge, Gate and VerifiedDecision remain sealed in the owner.
use crate::SignatureEnvelope;
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use serde_json::Value;
type Result<T> = std::result::Result<T, Failure>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Kind {
    WorkUse,
    InitialBinding,
    ReplacementBinding,
}
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Failure {
    #[error("decision/unconfigured: no author-owned issuer policy for this current target")]
    Unconfigured,
    #[error("decision/policy: invalid author policy")]
    Policy,
    #[error("decision/issuer-scope: key, issuer, role or operating area is not authorized")]
    IssuerScope,
    #[error("decision/context: exact current subject/catalog/generation/binding differs")]
    Context,
    #[error(
        "decision/kind: work use, initial binding and replacement binding are separate decisions"
    )]
    Kind,
    #[error("decision/epoch: receiver was closed/restarted or belongs to another owner")]
    Epoch,
    #[error("decision/ttl: lifetime must be positive and within the authored maximum")]
    Ttl,
    #[error("decision/expired: local monotonic deadline measured from challenge creation elapsed")]
    Expired,
    #[error("decision/revoked: the external decision has been invalidated")]
    Revoked,
    #[error("decision/signature: detached Ed25519 verification failed")]
    Signature,
    #[error("decision/encoding: malformed or oversized decision input")]
    Encoding,
    #[error("decision/identity: decision id already names different signed content")]
    Identity,
    #[error("decision/capacity: receiver decision history is full")]
    Capacity,
    #[error("decision/receiver: receiver ledger unavailable")]
    Receiver,
}
fn message<T: Serialize>(domain: &str, key: &Name, value: &T) -> Result<Vec<u8>> {
    let payload = canonical::bytes(&(key, value)).map_err(|_| Failure::Encoding)?;
    if payload.len() > 65_536 {
        return Err(Failure::Encoding);
    }
    let mut bytes = domain.as_bytes().to_vec();
    bytes.push(0);
    bytes.extend(payload);
    Ok(bytes)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub registration: Id,
    pub revision: Counter,
    pub program: Name,
    pub catalog: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeInfo {
    pub id: Id,
    pub epoch: Id,
    pub kind: Kind,
    pub owner: Owner,
    pub operating_area: Name,
    pub role: Name,
    pub subject: Value,
    pub context_digest: Digest,
    pub policy_digest: Digest,
    pub key: Name,
    pub issuer: Name,
    pub max_ttl_ms: Counter,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Approve,
}

/// Untrusted transport data. Signing bytes are supplied for the external issuer;
/// constructing a claim is not signing it or creating a verified decision.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub schema: Name,
    pub verdict: Verdict,
    pub challenge: ChallengeInfo,
    pub decision: Id,
    pub ttl_ms: Counter,
}
impl Claim {
    pub fn signing_message(&self, key: &Name) -> Result<Vec<u8>> {
        message("RX-EXTERNAL-DECISION-v1", key, self)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedDecision {
    pub claim: Claim,
    pub signature: SignatureEnvelope,
}
impl SignedDecision {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 131_072 {
            return Err(Failure::Encoding);
        }
        canonical::decode_json(bytes).map_err(|_| Failure::Encoding)
    }
}
/// Inert historical reference. No local deadline, live receiver or key capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub decision: Id,
    pub kind: Kind,
    pub owner: Owner,
    pub issuer: Name,
    pub key: Name,
    pub epoch: Id,
    pub challenge: Id,
    pub context_digest: Digest,
    pub policy_digest: Digest,
    pub signed_digest: Digest,
    pub ttl_ms: Counter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevocationClaim {
    pub schema: Name,
    pub target: Reference,
    pub reason: Name,
}
impl RevocationClaim {
    pub fn signing_message(&self, key: &Name) -> Result<Vec<u8>> {
        message("RX-EXTERNAL-DECISION-REVOCATION-v1", key, self)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedRevocation {
    pub claim: RevocationClaim,
    pub signature: SignatureEnvelope,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevocationReference {
    pub target: Reference,
    pub signed_digest: Digest,
    pub reason: Name,
}
