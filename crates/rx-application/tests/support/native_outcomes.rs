use super::*;
use rx_domain::operation::{Disposition, Integrity, Knowledge, Outcome};
use rx_process_contract::native_outcome::{
    NativeConclusion, NativeOutcomeCase, NativeOutcomeTable,
};

fn table(profile: Digest, rule: Name) -> NativeOutcomeTable {
    NativeOutcomeTable {
        schema: name("rx.native-outcome-table.v1"),
        profile_digest: profile,
        completion_rule: rule,
        cases: [
            ("driver/done", vec![Integer(0)], NativeConclusion::Succeeded),
            (
                "driver/aborted",
                vec![Integer(0), Integer(-4)],
                NativeConclusion::Failed,
            ),
            (
                "driver/canceled",
                vec![Integer(0)],
                NativeConclusion::Canceled,
            ),
        ]
        .into_iter()
        .map(|(schema, statuses, conclusion)| NativeOutcomeCase {
            status_schema: name(schema),
            statuses,
            conclusion,
        })
        .collect(),
    }
}
fn configured(postconditions: bool) -> Fixture {
    fixture_configured((1, true, false, true, None, false, true), |mut c| {
        let s = &mut c.steps[0];
        s.completion = CompletionRule::NativeOutcomes {
            table: table(s.intent.profile_digest, s.intent.completion_rule.clone()),
            postconditions: if postconditions {
                s.conditions.clone()
            } else {
                vec![]
            },
        };
        c
    })
}
fn batch(e: NativeEvidence) -> EvidenceBatch {
    EvidenceBatch {
        journal: id(),
        first: Counter(1),
        records: vec![e],
    }
}

#[test]
fn native_outcome_pairs_separate_terminal_meanings_and_leave_unknown_unsettled() {
    for (schema, code, expected) in [
        ("driver/done", 0, Outcome::Succeeded),
        ("driver/aborted", 0, Outcome::Failed),
        ("driver/aborted", -4, Outcome::Failed),
        ("driver/canceled", 0, Outcome::Canceled),
        ("driver/done", -4, Outcome::None),
        ("driver/accepted", 0, Outcome::None),
        ("driver/unknown", 0, Outcome::None),
        ("driver/canceled", 987, Outcome::None),
    ] {
        let mut f = configured(false);
        let (work, mut e) = native_started(&mut f);
        e.status_schema = name(schema);
        e.status = Integer(code);
        let b = batch(e.clone());
        f.app.ingest_evidence(&f.hosts[0], b.clone()).unwrap();
        let done = f
            .app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap();
        assert_eq!(done.operation.outcome(), expected, "{schema}/{code}");
        assert_ne!(done.operation.disposition(), Disposition::Released);
        assert_eq!(done.operation.integrity(), Integrity::Valid);
        if expected == Outcome::None {
            assert_eq!(done.operation.knowledge(), Knowledge::Unknown);
        }
        let revision = done.operation.revision();
        f.app.ingest_evidence(&f.hosts[0], b).unwrap();
        assert_eq!(
            f.app
                .inspect_work(&f.operator, work.operation.id())
                .unwrap()
                .operation
                .revision(),
            revision
        );
        assert_eq!(
            f.app
                .inspect_native_evidence(&f.hosts[0], &e.id)
                .unwrap()
                .status_schema,
            e.status_schema
        );
    }
}

#[test]
fn native_outcome_table_rejects_ambiguous_unbound_and_nonterminal_policy() {
    let mut f = configured(false);
    for variant in 0..7 {
        let mut c = f.configuration.clone();
        c.id = name("cell/b");
        let CompletionRule::NativeOutcomes { table: t, .. } = &mut c.steps[0].completion else {
            panic!()
        };
        match variant {
            0 => t.profile_digest = Digest::from_bytes([91; 32]),
            1 => t.completion_rule = name("other/rule"),
            2 => t.cases.push(t.cases[0].clone()),
            3 => t.cases[0].statuses.clear(),
            4 => t.schema = name("rx.native-outcome-table.v99"),
            5 => t.cases.clear(),
            _ => {
                let mut conflicting = t.cases[0].clone();
                conflicting.conclusion = NativeConclusion::Failed;
                t.cases.push(conflicting);
            }
        }
        assert!(f.app.install_cell(&f.admin, c).is_err());
    }
    let mut valid = f.configuration.clone();
    valid.id = name("cell/b");
    f.app.install_cell(&f.admin, valid).unwrap();
    let t = table(Digest::from_bytes([1; 32]), name("rule"));
    let mut v = serde_json::to_value(t).unwrap();
    v["cases"][0]["conclusion"] = serde_json::json!("UNRESOLVED");
    assert!(serde_json::from_value::<NativeOutcomeTable>(v.clone()).is_err());
    v["cases"][0]["conclusion"] = serde_json::json!("NOT_EXECUTED");
    assert!(serde_json::from_value::<NativeOutcomeTable>(v).is_err());
}

#[test]
fn native_outcome_postconditions_require_current_evidence_and_no_hold() {
    for case in 0..3 {
        let mut f = configured(true);
        let (work, mut e) = native_started(&mut f);
        if case == 1 {
            f.clock.0.store(30_000, Ordering::SeqCst);
        }
        if case == 2 {
            f.app
                .hold(&f.operator, id().as_str(), &name("cell/a"))
                .unwrap();
        }
        e.status_schema = name("driver/done");
        f.app.ingest_evidence(&f.hosts[0], batch(e)).unwrap();
        let done = f
            .app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap();
        assert_eq!(
            done.operation.outcome(),
            if case == 0 {
                Outcome::Succeeded
            } else {
                Outcome::None
            }
        );
        assert_ne!(done.operation.disposition(), Disposition::Released);
    }
}

#[test]
fn native_outcome_atomic_ack_loss_and_late_contradiction_preserve_first_conclusion() {
    for fault in [1, 2] {
        let mut f = configured(false);
        let (work, mut e) = native_started(&mut f);
        e.status_schema = name("driver/canceled");
        let b = batch(e.clone());
        f.failure.store(fault, Ordering::SeqCst);
        assert!(f.app.ingest_evidence(&f.hosts[0], b.clone()).is_err());
        f.app.ingest_evidence(&f.hosts[0], b.clone()).unwrap();
        let done = f
            .app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap();
        assert_eq!(done.operation.outcome(), Outcome::Canceled);
        e.id = id();
        e.status_schema = name("driver/done");
        f.app
            .ingest_evidence(
                &f.hosts[0],
                EvidenceBatch {
                    journal: b.journal,
                    first: Counter(2),
                    records: vec![e],
                },
            )
            .unwrap();
        let disputed = f
            .app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap();
        assert_eq!(disputed.operation.outcome(), Outcome::Canceled);
        assert_eq!(disputed.operation.integrity(), Integrity::Disputed);
        assert_eq!(disputed.operation.disposition(), Disposition::Quarantined);
    }
}
