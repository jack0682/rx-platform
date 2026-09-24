use rx_domain::types::*;
use rx_package::{external_decision::*, operating_area::*};
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new("00000000-0000-4000-8000-000000000001").unwrap()
}
fn request() -> ChallengeInfo {
    let owner = Owner {
        registration: id(),
        revision: Counter(1),
        program: n(PROGRAM),
        catalog: Digest::from_bytes([1; 32]),
    };
    ChallengeInfo {
        id: id(),
        epoch: id(),
        kind: Kind::WorkUse,
        owner: owner.clone(),
        operating_area: n(AREA),
        role: n(ROLE),
        subject: serde_json::json!({"operation":"support-gap-report.v1","task":{"operation":id(),"selection":"status","operating_area":AREA,"required_native_packages":"1000","required_support_profiles":"6"},"subject":{"registration":id(),"registration_revision":"1","program":PROGRAM,"catalog_digest":owner.catalog},"configuration":owner.catalog,"input_digest":owner.catalog,"readiness_semantics":owner.catalog}),
        context_digest: owner.catalog,
        policy_digest: owner.catalog,
        key: n(KEY_ID),
        issuer: n(ISSUER),
        max_ttl_ms: Counter(MAX_TTL_MS),
    }
}
#[test]
fn judge_approves_only_its_bounded_nonactuating_report_rule() {
    let good = request();
    assert_eq!(judge(&good, id(), Counter(1000)).unwrap().challenge, good);
    let mut bad = good.clone();
    bad.subject["task"]["required_support_profiles"] = "65".into();
    assert_eq!(
        judge(&bad, id(), Counter(1000)).unwrap_err().reason,
        "judge/bounded-report-rule"
    );
    let mut bad = good.clone();
    bad.subject["operation"] = "move-robot".into();
    assert!(judge(&bad, id(), Counter(1000)).is_err());
    let mut bad = good.clone();
    bad.kind = Kind::ReplacementBinding;
    assert!(judge(&bad, id(), Counter(1000)).is_err());
    let mut bad = good.clone();
    bad.operating_area = n("other/area");
    assert!(judge(&bad, id(), Counter(1000)).is_err());
    let mut bad = good.clone();
    bad.owner.program = n("rx/status-http");
    assert!(judge(&bad, id(), Counter(1000)).is_err());
    assert!(judge(&good, id(), Counter(MAX_TTL_MS + 1)).is_err());
}
