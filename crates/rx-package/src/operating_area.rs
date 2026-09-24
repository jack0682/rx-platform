//! Independently authored development judgment, not physical or production authority.
//! An unsigned Claim is not a verified permission; only the external issuer signs.
use crate::external_decision::*;
use rx_domain::types::*;
use serde::{Deserialize, Serialize};
pub const KEY_ID: &str = "rx/development-work-judge-v1";
pub const ISSUER: &str = "development/support-gap-judge-v1";
pub const AREA: &str = "development/support-area";
pub const ROLE: &str = "work/support-gap-report";
pub const PROGRAM: &str = "rx/status-work-http";
pub const MAX_TTL_MS: u64 = 30_000;
pub const RULE: &str = "development/bounded-support-gap-report-v1";
pub const PUBLIC_KEY: [u8; 32] = [
    0x7a, 0x65, 0x6f, 0x9a, 0x14, 0x50, 0x8a, 0xc4, 0x40, 0xff, 0x26, 0x64, 0x6e, 0xce, 0x22, 0x47,
    0x27, 0xfe, 0xd7, 0x31, 0x51, 0xa0, 0x06, 0x70, 0x22, 0x7d, 0xaf, 0x10, 0xe1, 0x5f, 0x14, 0xe9,
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub operation: Id,
    pub selection: Name,
    pub operating_area: Name,
    pub required_native_packages: Counter,
    pub required_support_profiles: Counter,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    operation: String,
    task: Task,
    subject: serde_json::Value,
    configuration: Digest,
    input_digest: Digest,
    readiness_semantics: Digest,
}
/// A refusal of the request as presented, not a verified identity statement.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Denial {
    pub challenge: Id,
    pub rule: String,
    pub reason: String,
}
/// Evaluate the issuer's own bounded report-generation rule. It deliberately
/// permits reporting a shortfall, not operating deficient physical equipment.
pub fn judge(info: &ChallengeInfo, decision: Id, ttl: Counter) -> Result<Claim, Denial> {
    let deny = |reason: &str| Denial {
        challenge: info.id.clone(),
        rule: RULE.into(),
        reason: reason.into(),
    };
    if info.key.as_str() != KEY_ID || info.issuer.as_str() != ISSUER {
        return Err(deny("judge/issuer-scope"));
    }
    if info.operating_area.as_str() != AREA {
        return Err(deny("judge/operating-area"));
    }
    if info.role.as_str() != ROLE
        || info.kind != Kind::WorkUse
        || info.owner.program.as_str() != PROGRAM
    {
        return Err(deny("judge/role-kind-program"));
    }
    if ttl.0 == 0 || ttl.0 > MAX_TTL_MS || ttl > info.max_ttl_ms {
        return Err(deny("judge/ttl"));
    }
    let intent: Intent =
        serde_json::from_value(info.subject.clone()).map_err(|_| deny("judge/intent-shape"))?;
    if intent.operation != "support-gap-report.v1"
        || intent.task.operating_area.as_str() != AREA
        || intent.task.required_native_packages.0 > 10_000
        || intent.task.required_support_profiles.0 > 64
    {
        return Err(deny("judge/bounded-report-rule"));
    }
    if intent.subject["registration"] != serde_json::json!(info.owner.registration)
        || intent.subject["registration_revision"] != serde_json::json!(info.owner.revision)
        || intent.subject["program"] != serde_json::json!(info.owner.program)
        || intent.subject["catalog_digest"] != serde_json::json!(info.owner.catalog)
    {
        return Err(deny("judge/owner-context"));
    }
    // These pins are opaque owner-authored context, not independent measurements.
    let _pins = (
        intent.configuration,
        intent.input_digest,
        intent.readiness_semantics,
    );
    Ok(Claim {
        schema: Name::new("rx.external-decision.v1").expect("literal"),
        verdict: Verdict::Approve,
        challenge: info.clone(),
        decision,
        ttl_ms: ttl,
    })
}
