use rx_domain::{budget::*, canonical, operation::*, types::*};
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}

#[test]
fn restoring_reconciling_operation_preserves_unknown_and_quarantine() {
    let mut operation = Operation::admitted(id(), Digest::from_bytes([1; 32]));
    operation.sent().unwrap();
    operation.lose_continuity().unwrap();
    let bytes = canonical::bytes(&operation).unwrap();
    let mut restored: Operation = canonical::decode_json(&bytes).unwrap();
    assert_eq!(restored, operation);
    assert!(restored.sent().is_err());
    assert_eq!(restored.outcome(), Outcome::None);
    assert_eq!(restored.disposition(), Disposition::Quarantined);
}

#[test]
fn stored_state_cannot_claim_success_without_terminal_evidence() {
    let operation = Operation::admitted(id(), Digest::from_bytes([1; 32]));
    let mut value = serde_json::to_value(operation).unwrap();
    value["outcome"] = "SUCCEEDED".into();
    assert!(canonical::decode_json::<Operation>(&serde_json::to_vec(&value).unwrap()).is_err());
    value["phase"] = "SETTLED".into();
    value["execution_knowledge"] = "ENDED".into();
    assert!(canonical::decode_json::<Operation>(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn historical_dispute_survives_restart_without_rewriting_success() {
    let mut operation = Operation::admitted(id(), Digest::from_bytes([1; 32]));
    operation
        .conclude(Conclusion {
            outcome: Outcome::Succeeded,
            evidence_ids: vec![id()],
        })
        .unwrap();
    operation
        .conclude(Conclusion {
            outcome: Outcome::Failed,
            evidence_ids: vec![id()],
        })
        .unwrap();
    let restored: Operation =
        canonical::decode_json(&canonical::bytes(&operation).unwrap()).unwrap();
    assert_eq!(restored.outcome(), Outcome::Succeeded);
    assert_eq!(restored.integrity(), Integrity::Disputed);
    assert_eq!(restored.disposition(), Disposition::Quarantined);
}

#[test]
fn restoring_run_budget_never_refunds_consumption() {
    let mut budget = RunBudget::new(BudgetUnit::PartAttempt, Counter(2)).unwrap();
    let part = Consumption::PartAttempt(id());
    budget.consume(part.clone()).unwrap();
    let mut restored: RunBudget =
        canonical::decode_json(&canonical::bytes(&budget).unwrap()).unwrap();
    assert_eq!(restored.remaining(), Counter(1));
    assert!(!restored.consume(part).unwrap());
    restored.consume(Consumption::PartAttempt(id())).unwrap();
    assert!(restored.consume(Consumption::PartAttempt(id())).is_err());
}

#[test]
fn duplicate_or_overbudget_history_is_corruption_not_a_refund() {
    let mut budget = RunBudget::new(BudgetUnit::PartAttempt, Counter(2)).unwrap();
    budget.consume(Consumption::PartAttempt(id())).unwrap();
    let original = serde_json::to_value(budget).unwrap();
    let mut value = original.clone();
    let item = value["consumptions"][0].clone();
    value["consumptions"].as_array_mut().unwrap().push(item);
    assert!(canonical::decode_json::<RunBudget>(&serde_json::to_vec(&value).unwrap()).is_err());
    let mut value = original;
    value["revision"] = "1".into();
    assert!(canonical::decode_json::<RunBudget>(&serde_json::to_vec(&value).unwrap()).is_err());
}
