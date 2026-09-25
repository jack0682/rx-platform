//! Independently authored development judgment, not physical or production authority.
//! An unsigned Claim is not a verified permission; only the external issuer signs.
use crate::external_decision::*;
use rx_domain::types::*;
use serde::{Deserialize, Serialize};
/// Authored code, never deserialized from site configuration. Validation is
/// required before any map or selector is constructed (duplicates cannot overwrite).
#[derive(Clone, Copy, Debug)]
pub struct Area {
    pub key_id: &'static str,
    pub issuer: &'static str,
    pub area: &'static str,
    pub role: &'static str,
    pub program: &'static str,
    pub max_ttl_ms: u64,
    pub rule: &'static str,
    pub public_key: [u8; 32],
    pub max_native_packages: u64,
    pub max_support_profiles: u64,
}
pub const SUPPORT: Area = Area {
    key_id: "rx/development-work-judge-v1",
    issuer: "development/support-gap-judge-v1",
    area: "development/support-area",
    role: "work/support-gap-report",
    program: "rx/status-work-http",
    max_ttl_ms: 30_000,
    rule: "development/bounded-support-gap-report-v1",
    public_key: [
        0x7a, 0x65, 0x6f, 0x9a, 0x14, 0x50, 0x8a, 0xc4, 0x40, 0xff, 0x26, 0x64, 0x6e, 0xce, 0x22,
        0x47, 0x27, 0xfe, 0xd7, 0x31, 0x51, 0xa0, 0x06, 0x70, 0x22, 0x7d, 0xaf, 0x10, 0xe1, 0x5f,
        0x14, 0xe9,
    ],
    max_native_packages: 10_000,
    max_support_profiles: 64,
};
pub const COMPACT: Area = Area {
    key_id: "rx/development-compact-judge-v1",
    issuer: "development/compact-support-judge-v1",
    area: "development/compact-support-area",
    role: "work/compact-support-gap-report",
    program: "rx/status-compact-work-http",
    max_ttl_ms: 15_000,
    rule: "development/compact-support-gap-report-v1",
    public_key: [
        134, 9, 150, 127, 4, 174, 197, 79, 224, 11, 241, 69, 202, 5, 49, 75, 132, 92, 83, 3, 36,
        120, 219, 246, 204, 122, 80, 233, 253, 89, 100, 163,
    ],
    max_native_packages: 2_000,
    max_support_profiles: 8,
};
const AREAS: [Area; 2] = [SUPPORT, COMPACT];
/// G3 source compatibility only. Catalog programs always select their own record.
pub const KEY_ID: &str = SUPPORT.key_id;
pub const ISSUER: &str = SUPPORT.issuer;
pub const AREA: &str = SUPPORT.area;
pub const ROLE: &str = SUPPORT.role;
pub const PROGRAM: &str = SUPPORT.program;
pub const MAX_TTL_MS: u64 = SUPPORT.max_ttl_ms;
pub const RULE: &str = SUPPORT.rule;
pub const PUBLIC_KEY: [u8; 32] = SUPPORT.public_key;

/// This validates declarations, not an authority enrollment API. No returned
/// value can extend the compiled catalog or construct a live receiving proof.
pub fn validate_catalog(areas: &[Area]) -> Result<(), &'static str> {
    if areas.is_empty() || areas.len() > 8 {
        return Err("area-catalog/count: requires 1..8 declarations");
    }
    for (i, a) in areas.iter().enumerate() {
        if a.public_key == [0; 32]
            || !(1..=60_000).contains(&a.max_ttl_ms)
            || a.max_native_packages == 0
            || a.max_support_profiles == 0
            || [a.key_id, a.issuer, a.area, a.role, a.program, a.rule]
                .iter()
                .any(|v| Name::new(*v).is_err())
        {
            return Err("area-catalog/invalid-declaration");
        }
        for b in &areas[..i] {
            if a.area == b.area {
                return Err("area-catalog/duplicate-area");
            }
            if a.issuer == b.issuer {
                return Err("area-catalog/duplicate-issuer");
            }
            if a.key_id == b.key_id {
                return Err("area-catalog/duplicate-key-id");
            }
            if a.public_key == b.public_key {
                return Err("area-catalog/duplicate-public-key");
            }
            if a.program == b.program {
                return Err("area-catalog/duplicate-program");
            }
        }
    }
    Ok(())
}
pub fn catalog() -> Result<&'static [Area], &'static str> {
    validate_catalog(&AREAS)?;
    Ok(&AREAS)
}
pub fn for_program(program: &str) -> Result<Option<&'static Area>, &'static str> {
    Ok(catalog()?.iter().find(|a| a.program == program))
}

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
    let entries = catalog().map_err(|reason| Denial {
        challenge: info.id.clone(),
        rule: "area-catalog".into(),
        reason: reason.into(),
    })?;
    let area = entries
        .iter()
        .find(|a| a.key_id == info.key.as_str())
        .ok_or_else(|| Denial {
            challenge: info.id.clone(),
            rule: "area-catalog".into(),
            reason: "judge/issuer-scope".into(),
        })?;
    let deny = |reason: &str| Denial {
        challenge: info.id.clone(),
        rule: area.rule.into(),
        reason: reason.into(),
    };
    if info.key.as_str() != area.key_id || info.issuer.as_str() != area.issuer {
        return Err(deny("judge/issuer-scope"));
    }
    if info.operating_area.as_str() != area.area {
        return Err(deny("judge/operating-area"));
    }
    if info.role.as_str() != area.role
        || info.kind != Kind::WorkUse
        || info.owner.program.as_str() != area.program
    {
        return Err(deny("judge/role-kind-program"));
    }
    if ttl.0 == 0 || ttl.0 > area.max_ttl_ms || ttl > info.max_ttl_ms {
        return Err(deny("judge/ttl"));
    }
    let intent: Intent =
        serde_json::from_value(info.subject.clone()).map_err(|_| deny("judge/intent-shape"))?;
    if intent.operation != "support-gap-report.v1"
        || intent.task.operating_area.as_str() != area.area
        || intent.task.required_native_packages.0 > area.max_native_packages
        || intent.task.required_support_profiles.0 > area.max_support_profiles
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
