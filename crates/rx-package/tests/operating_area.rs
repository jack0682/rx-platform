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

#[test]
fn catalog_rejects_each_ambiguous_declaration_before_map_construction() {
    assert!(validate_catalog(&[SUPPORT]).is_ok());
    assert!(validate_catalog(catalog().unwrap()).is_ok());
    for (label, entry, reason) in [
        (
            "area",
            Area {
                area: SUPPORT.area,
                ..COMPACT
            },
            "duplicate-area",
        ),
        (
            "issuer",
            Area {
                issuer: SUPPORT.issuer,
                ..COMPACT
            },
            "duplicate-issuer",
        ),
        (
            "key-id",
            Area {
                key_id: SUPPORT.key_id,
                ..COMPACT
            },
            "duplicate-key-id",
        ),
        (
            "public-key",
            Area {
                public_key: SUPPORT.public_key,
                ..COMPACT
            },
            "duplicate-public-key",
        ),
        (
            "program",
            Area {
                program: SUPPORT.program,
                ..COMPACT
            },
            "duplicate-program",
        ),
    ] {
        let error = validate_catalog(&[SUPPORT, entry]).unwrap_err();
        assert!(error.contains(reason));
        println!("catalog-{label}: {error}");
    }
    let mut entries = Vec::new();
    for i in 0..9 {
        let name = Box::leak(format!("test/area-{i}").into_boxed_str());
        entries.push(Area {
            key_id: name,
            issuer: name,
            area: name,
            program: name,
            public_key: [i + 1; 32],
            ..SUPPORT
        });
    }
    assert!(validate_catalog(&entries[..8]).is_ok());
    for bad in [&entries[..0], &entries[..9]] {
        let error = validate_catalog(bad).unwrap_err();
        assert!(error.contains("count"));
        println!("catalog-count-{}: {error}", bad.len());
    }
}

#[test]
fn each_area_checks_its_own_limits_role_and_program() {
    for a in catalog().unwrap() {
        let mut info = request();
        info.key = n(a.key_id);
        info.issuer = n(a.issuer);
        info.operating_area = n(a.area);
        info.role = n(a.role);
        info.owner.program = n(a.program);
        info.max_ttl_ms = Counter(a.max_ttl_ms);
        info.subject["task"]["operating_area"] = a.area.into();
        info.subject["subject"]["program"] = a.program.into();
        assert!(judge(&info, id(), Counter(1000)).is_ok());
        for (field, value) in [
            ("required_native_packages", 5000),
            ("required_support_profiles", 32),
        ] {
            let mut candidate = info.clone();
            candidate.subject["task"][field] = value.to_string().into();
            assert_eq!(
                judge(&candidate, id(), Counter(1000)).is_ok(),
                a.area == SUPPORT.area
            );
        }
        assert_eq!(
            judge(&info, id(), Counter(25000)).is_ok(),
            a.area == SUPPORT.area
        );
        let other = if a.area == SUPPORT.area {
            COMPACT
        } else {
            SUPPORT
        };
        let mut wrong = info.clone();
        wrong.role = n(other.role);
        assert_eq!(
            judge(&wrong, id(), Counter(1000)).unwrap_err().reason,
            "judge/role-kind-program"
        );
        let mut wrong = info.clone();
        wrong.owner.program = n(other.program);
        assert_eq!(
            judge(&wrong, id(), Counter(1000)).unwrap_err().reason,
            "judge/role-kind-program"
        );
        let mut wrong = info.clone();
        wrong.operating_area = n(other.area);
        assert_eq!(
            judge(&wrong, id(), Counter(1000)).unwrap_err().reason,
            "judge/operating-area"
        );
        println!(
            "{}: own approval; own rule, foreign role/program/area refusals",
            a.area
        );
    }
}
