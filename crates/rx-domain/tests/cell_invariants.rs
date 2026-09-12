use rx_domain::{budget::*, epoch::ScopeEpochs, types::*};
fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}

#[test]
fn last_counted_part_can_continue_but_a_new_part_cannot_be_created() {
    let mut budget = RunBudget::new(BudgetUnit::PartAttempt, Counter(1)).unwrap();
    let part = Consumption::PartAttempt(id());
    assert!(budget.consume(part.clone()).unwrap());
    assert_eq!(budget.remaining(), Counter(0));
    let revision = budget.revision();
    for _ in 0..6 {
        assert!(!budget.consume(part.clone()).unwrap());
    }
    assert_eq!(budget.revision(), revision);
    assert!(budget.contains(&part));
    assert!(budget.consume(Consumption::PartAttempt(id())).is_err());
}

#[test]
fn setup_and_material_budgets_are_not_interchangeable() {
    let mut budget = RunBudget::new(BudgetUnit::PartAttempt, Counter(5)).unwrap();
    assert!(budget.consume(Consumption::Operation(id())).is_err());
    assert_eq!(budget.consumed(), Counter(0));
    let mut setup = RunBudget::new(BudgetUnit::OperationCount, Counter(5)).unwrap();
    assert!(setup.consume(Consumption::PartAttempt(id())).is_err());
    assert!(RunBudget::new(BudgetUnit::PartAttempt, Counter(0)).is_err());
}

#[test]
fn stale_vector_does_not_regress_or_partially_update_any_scope() {
    let mut epochs = ScopeEpochs::default();
    epochs
        .install(&[(name("a"), Counter(8)), (name("b"), Counter(6))])
        .unwrap();
    let before = epochs.clone();
    assert!(
        epochs
            .install(&[(name("b"), Counter(7)), (name("a"), Counter(7))])
            .is_err()
    );
    assert_eq!(epochs, before);
    assert!(epochs.install(&[(name("a"), Counter(9))]).unwrap());
    assert_eq!(epochs.get(&name("b")), Some(Counter(6)));
    assert!(!epochs.matches_exact(&[(name("a"), Counter(8))]));
    assert!(epochs.matches_exact(&[(name("a"), Counter(9))]));
}

#[test]
fn duplicate_or_empty_scope_vectors_do_not_vacuously_authorize() {
    let mut epochs = ScopeEpochs::default();
    assert!(epochs.install(&[]).is_err());
    assert!(epochs.install(&[(name("a"), Counter(0))]).is_err());
    assert!(
        epochs
            .install(&[(name("a"), Counter(1)), (name("a"), Counter(2))])
            .is_err()
    );
    assert_eq!(epochs.get(&name("a")), None);
    assert!(!epochs.matches_exact(&[]));
}
