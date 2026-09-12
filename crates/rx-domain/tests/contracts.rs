use rx_domain::{
    canonical::{self, decode_json},
    condition::*,
    intent::{Intent, Kind},
    operation::*,
    types::*,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const CV01: &str = r#"{"body":{"predicate":{"predicate_id":"fixture.closed","settle_ms":"0","target":{"boolean":true}}},"calibration_digests":[],"cancel_rule":"example/stop","completion_rule":"example/closed","execution_timeout_ms":"5000","kind":"ENSURE_STATE","prepare_validity_ms":"1000","profile_digest":"0000000000000000000000000000000000000000000000000000000000000000","resource_set":["controller/example"],"site_config_digest":"1111111111111111111111111111111111111111111111111111111111111111","target":"example/fixture"}"#;

fn new_id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn intent() -> Intent {
    decode_json(CV01.as_bytes()).unwrap()
}
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn operation() -> Operation {
    Operation::admitted(new_id(), intent().digest().unwrap())
}

#[test]
fn frozen_cv01_canonical_bytes_and_digest() {
    let value = intent();
    assert_eq!(
        canonical::bytes(&value.normalized().unwrap()).unwrap(),
        CV01.as_bytes()
    );
    assert_eq!(
        value.digest().unwrap().to_string(),
        "e94f6df366963718e61e68018e10b1fb0d453e36143119c1e1b59f07d53ddf92"
    );
}

#[test]
fn frozen_cv02_negative_zero_and_cv03_false() {
    let a = TypedValue::Real(Real::new(-0.0).unwrap());
    let b = TypedValue::Real(Real::new(0.0).unwrap());
    assert_eq!(canonical::bytes(&a).unwrap(), br#"{"real":0}"#);
    assert_eq!(canonical::bytes(&a).unwrap(), canonical::bytes(&b).unwrap());
    assert_eq!(
        canonical::digest("RX-INTENT-v1", &a).unwrap().to_string(),
        "d5dc68a13b5d1ebd2b2ecea05fe2b14022fea142152458f74e0311d21b0306c8"
    );
    assert_eq!(
        canonical::digest("RX-INTENT-v1", &TypedValue::Boolean(false))
            .unwrap()
            .to_string(),
        "f378924571af64a661c8a74c7f675d6c9d64416276deefd8b3cd74297a806e2e"
    );
    assert!(Real::new(f64::NAN).is_err());
    assert!(Real::new(f64::INFINITY).is_err());
}

#[test]
fn resource_order_is_not_semantic_but_duplicates_are_invalid() {
    let mut a = intent();
    a.resource_set.push(name("controller/another"));
    let mut b = a.clone();
    b.resource_set.reverse();
    assert_eq!(a.digest().unwrap(), b.digest().unwrap());
    b.resource_set.push(b.resource_set[0].clone());
    assert!(b.digest().is_err());
}

#[test]
fn profile_calibration_and_rule_change_identity() {
    let a = intent();
    let mut b = a.clone();
    b.calibration_digests.push(Digest::from_bytes([2; 32]));
    assert_ne!(a.digest().unwrap(), b.digest().unwrap());
    b = a.clone();
    b.profile_digest = Digest::from_bytes([3; 32]);
    assert_ne!(a.digest().unwrap(), b.digest().unwrap());
    b = a.clone();
    b.completion_rule = name("example/other");
    assert_ne!(a.digest().unwrap(), b.digest().unwrap());
}

#[test]
fn input_rejects_unknown_duplicate_and_incompatible_fields() {
    let bad = CV01.replace(
        r#""kind":"ENSURE_STATE""#,
        r#""kind":"ENSURE_STATE","kind":"FINITE_ACTION""#,
    );
    assert!(decode_json::<Intent>(bad.as_bytes()).is_err());
    let bad = CV01.replace(
        r#""target":{"boolean":true}"#,
        r#""target":{"boolean":true,"boolean":false}"#,
    );
    assert!(decode_json::<Intent>(bad.as_bytes()).is_err());
    let bad = CV01.replace(
        r#""settle_ms":"0""#,
        r#""settle_ms":"0","ignore_safety":true"#,
    );
    assert!(decode_json::<Intent>(bad.as_bytes()).is_err());
    let bad = CV01.replace("ENSURE_STATE", "FUTURE_ACTION");
    assert!(decode_json::<Intent>(bad.as_bytes()).is_err());
    let mut bad = intent();
    bad.kind = Kind::FiniteAction;
    assert!(bad.digest().is_err());
}

#[test]
fn oneof_presence_and_false_are_not_missing_or_null() {
    assert_eq!(
        decode_json::<TypedValue>(br#"{"boolean":false}"#).unwrap(),
        TypedValue::Boolean(false)
    );
    for raw in [
        r#"{}"#,
        r#"{"boolean":null}"#,
        r#"{"boolean":false,"real":0}"#,
        r#"{"future":1}"#,
    ] {
        assert!(decode_json::<TypedValue>(raw.as_bytes()).is_err(), "{raw}");
    }
}

#[test]
fn counters_preserve_full_width_and_reject_json_numbers() {
    let c: Counter = decode_json(br#""18446744073709551615""#).unwrap();
    assert_eq!(c.0, u64::MAX);
    assert_eq!(canonical::bytes(&c).unwrap(), br#""18446744073709551615""#);
    for raw in [
        r#"1"#,
        r#""01""#,
        r#""-1""#,
        r#""+1""#,
        r#""18446744073709551616""#,
    ] {
        assert!(decode_json::<Counter>(raw.as_bytes()).is_err(), "{raw}");
    }
    assert!(c.increment().is_err());
}

#[test]
fn strict_json_rejects_trailing_bytes_and_duplicate_escaped_keys() {
    assert!(decode_json::<Value>(br#"{"x":1} {}"#).is_err());
    assert!(decode_json::<Value>(br#"{"a":1,"\u0061":2}"#).is_err());
}

#[test]
fn response_loss_and_running_poll_never_create_a_result() {
    let mut op = operation();
    op.sent().unwrap();
    for _ in 0..10 {
        op.lose_continuity().unwrap();
        op.observed_running(new_id()).unwrap();
        assert_eq!(op.phase(), Phase::Reconciling);
        assert_eq!(op.outcome(), Outcome::None);
        assert_eq!(op.disposition(), Disposition::Quarantined);
        assert!(op.sent().is_err());
    }
}

#[test]
fn terminal_success_does_not_release_and_conflicting_result_is_preserved() {
    let mut op = operation();
    op.sent().unwrap();
    op.conclude(Conclusion {
        outcome: Outcome::Succeeded,
        evidence_ids: vec![new_id()],
    })
    .unwrap();
    assert_eq!(op.disposition(), Disposition::Held);
    let incomplete = ReleaseConditions {
        no_residual_native: true,
        control_handover_confirmed: true,
        support_handover_confirmed: false,
    };
    assert!(op.release(incomplete, vec![new_id()]).is_err());
    op.conclude(Conclusion {
        outcome: Outcome::Failed,
        evidence_ids: vec![new_id()],
    })
    .unwrap();
    assert_eq!(op.outcome(), Outcome::Succeeded);
    assert_eq!(op.integrity(), Integrity::Disputed);
    assert_eq!(op.disposition(), Disposition::Quarantined);
}

#[test]
fn duplicate_conclusion_is_idempotent_and_empty_proof_rejected() {
    let mut op = operation();
    assert!(
        op.conclude(Conclusion {
            outcome: Outcome::Succeeded,
            evidence_ids: vec![]
        })
        .is_err()
    );
    let c = Conclusion {
        outcome: Outcome::Succeeded,
        evidence_ids: vec![new_id()],
    };
    op.conclude(c.clone()).unwrap();
    let revision = op.revision();
    op.conclude(c).unwrap();
    assert_eq!(op.revision(), revision);
}

fn fixture() -> (
    TimePoint,
    BTreeMap<Name, Fact>,
    BTreeMap<Name, Id>,
    Condition,
) {
    let id = name("door/closed");
    let generation = new_id();
    let now = TimePoint {
        clock_id: "boot/boottime".into(),
        ticks_ns: Counter(1000),
    };
    let fact = Fact {
        schema: name("boolean/v1"),
        unit: name("unitless"),
        source_generation: generation.clone(),
        acquired_at: TimePoint {
            clock_id: now.clock_id.clone(),
            ticks_ns: Counter(900),
        },
        maximum_age_ns: Counter(100),
        acquisition_uncertainty_ns: Counter(0),
        quality_good: true,
        origin_age_bounded: true,
        disputed: false,
        value: FactValue::Scalar(TypedValue::Boolean(true)),
        evidence_id: new_id(),
    };
    let condition = Condition::Eq {
        fact: id.clone(),
        schema: fact.schema.clone(),
        unit: fact.unit.clone(),
        expected: TypedValue::Boolean(true),
    };
    (
        now,
        BTreeMap::from([(id.clone(), fact)]),
        BTreeMap::from([(id, generation)]),
        condition,
    )
}

#[test]
fn source_age_not_callback_time_controls_freshness() {
    let (mut now, facts, generations, condition) = fixture();
    assert_eq!(
        condition
            .evaluate(&Context {
                now: &now,
                facts: &facts,
                generations: &generations
            })
            .unwrap()
            .verdict,
        Verdict::Pass
    );
    now.ticks_ns = Counter(1001);
    for _ in 0..4 {
        assert_eq!(
            condition
                .evaluate(&Context {
                    now: &now,
                    facts: &facts,
                    generations: &generations
                })
                .unwrap()
                .verdict,
            Verdict::Unknown
        );
    }
}

#[test]
fn quality_alone_never_proves_freshness_generation_or_unit() {
    let (now, original, generations, condition) = fixture();
    for kind in 0..7 {
        let mut facts = original.clone();
        let fact = facts.values_mut().next().unwrap();
        match kind {
            0 => fact.origin_age_bounded = false,
            1 => fact.source_generation = new_id(),
            2 => fact.unit = name("metre"),
            3 => fact.acquired_at.clock_id = "other-boot/boottime".into(),
            4 => fact.acquired_at.ticks_ns = Counter(1001),
            5 => fact.acquisition_uncertainty_ns = Counter(1),
            _ => fact.disputed = true,
        };
        assert_eq!(
            condition
                .evaluate(&Context {
                    now: &now,
                    facts: &facts,
                    generations: &generations
                })
                .unwrap()
                .verdict,
            Verdict::Unknown,
            "case {kind}"
        );
    }
}

#[test]
fn three_valued_groups_and_invalid_hidden_branch() {
    let (now, facts, generations, pass) = fixture();
    let ctx = Context {
        now: &now,
        facts: &facts,
        generations: &generations,
    };
    let mut fail = pass.clone();
    if let Condition::Eq { expected, .. } = &mut fail {
        *expected = TypedValue::Boolean(false);
    }
    let mut unknown = pass.clone();
    if let Condition::Eq { fact, .. } = &mut unknown {
        *fact = name("missing");
    }
    assert_eq!(
        Condition::All {
            children: vec![pass.clone(), unknown.clone()]
        }
        .evaluate(&ctx)
        .unwrap()
        .verdict,
        Verdict::Unknown
    );
    assert_eq!(
        Condition::Any {
            children: vec![fail.clone(), unknown.clone()]
        }
        .evaluate(&ctx)
        .unwrap()
        .verdict,
        Verdict::Unknown
    );
    assert_eq!(
        Condition::All {
            children: vec![fail, unknown.clone()]
        }
        .evaluate(&ctx)
        .unwrap()
        .verdict,
        Verdict::Fail
    );
    assert_eq!(
        Condition::Any {
            children: vec![pass.clone(), unknown]
        }
        .evaluate(&ctx)
        .unwrap()
        .verdict,
        Verdict::Pass
    );
    assert!(
        Condition::Any {
            children: vec![pass, Condition::All { children: vec![] }]
        }
        .evaluate(&ctx)
        .is_err()
    );
}

#[test]
fn condition_schema_is_closed() {
    let raw = json!({"op":"EQ", "fact":"x", "schema":"boolean/v1", "unit":"unitless", "expected":{"boolean":true}, "skip":true});
    assert!(decode_json::<Condition>(&serde_json::to_vec(&raw).unwrap()).is_err());
}

#[test]
fn any_uses_a_valid_witness_deadline_and_all_uses_the_earliest_deadline() {
    let (now, mut facts, mut generations, first) = fixture();
    let mut second_fact = facts.values().next().unwrap().clone();
    second_fact.evidence_id = new_id();
    second_fact.acquired_at.ticks_ns = Counter(950);
    second_fact.maximum_age_ns = Counter(300);
    let evidence = second_fact.evidence_id.clone();
    generations.insert(name("redundant"), second_fact.source_generation.clone());
    facts.insert(name("redundant"), second_fact);
    let mut second = first.clone();
    if let Condition::Eq { fact, .. } = &mut second {
        *fact = name("redundant");
    }
    let ctx = Context {
        now: &now,
        facts: &facts,
        generations: &generations,
    };
    let any = Condition::Any {
        children: vec![first.clone(), second.clone()],
    }
    .evaluate(&ctx)
    .unwrap();
    assert_eq!(any.verdict, Verdict::Pass);
    assert_eq!(any.valid_until.unwrap().ticks_ns, Counter(1250));
    assert_eq!(any.evidence_ids, vec![evidence]);
    let all = Condition::All {
        children: vec![first, second],
    }
    .evaluate(&ctx)
    .unwrap();
    assert_eq!(all.valid_until.unwrap().ticks_ns, Counter(1000));
}
