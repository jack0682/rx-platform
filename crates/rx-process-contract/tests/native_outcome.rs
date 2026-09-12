use rx_domain::{canonical, types::*};
use rx_process_contract::native_outcome::*;
fn table() -> NativeOutcomeTable {
    NativeOutcomeTable {
        schema: Name::new("rx.native-outcome-table.v1").unwrap(),
        profile_digest: Digest::from_bytes([1; 32]),
        completion_rule: Name::new("test/done").unwrap(),
        cases: vec![NativeOutcomeCase {
            status_schema: Name::new("test/result").unwrap(),
            statuses: vec![Integer(0)],
            conclusion: NativeConclusion::Succeeded,
        }],
    }
}
#[test]
fn table_rejects_duplicates_conflicts_and_oversized_code_sets() {
    for variant in 0..5 {
        let mut t = table();
        match variant {
            0 => t.cases[0].statuses.push(Integer(0)),
            1 => {
                let mut second = t.cases[0].clone();
                second.conclusion = NativeConclusion::Failed;
                t.cases.push(second);
            }
            2 => t.cases[0].statuses = (0..65).map(Integer).collect(),
            3 => {
                t.cases = (0..17)
                    .map(|i| {
                        let mut c = t.cases[0].clone();
                        c.statuses = vec![Integer(i)];
                        c
                    })
                    .collect()
            }
            _ => {
                t.cases = (0..3)
                    .map(|i| {
                        let mut c = t.cases[0].clone();
                        c.statuses = (i * 64..(i + 1) * 64).map(Integer).collect();
                        c
                    })
                    .collect()
            }
        }
        assert!(t.validate().is_err());
        assert!(
            t.resolve(&Name::new("test/result").unwrap(), Integer(0))
                .is_err()
        );
    }
}
#[test]
fn table_is_strict_data_and_missing_pair_cannot_resolve() {
    let t = table();
    let bytes = canonical::bytes(&t).unwrap();
    let decoded: NativeOutcomeTable = canonical::decode_json(&bytes).unwrap();
    assert_eq!(
        decoded
            .resolve(&Name::new("test/result").unwrap(), Integer(0))
            .unwrap(),
        Some(NativeConclusion::Succeeded)
    );
    assert_eq!(
        decoded
            .resolve(&Name::new("test/result").unwrap(), Integer(-1))
            .unwrap(),
        None
    );
    assert_eq!(
        decoded
            .resolve(&Name::new("test/unknown").unwrap(), Integer(0))
            .unwrap(),
        None
    );
    let mut value = serde_json::to_value(t).unwrap();
    value["fallback"] = serde_json::json!("SUCCEEDED");
    assert!(serde_json::from_value::<NativeOutcomeTable>(value).is_err());
}
