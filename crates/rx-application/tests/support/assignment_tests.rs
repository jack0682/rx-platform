use super::*;
use rx_process_contract::assignment::{self, Cardinality};
fn fresh() -> Fixture {
    fixture_with_peer(1, true, false, true, None, true)
}
#[test]
fn assignment_read_discovers_arming_and_ambiguity_without_creating_or_selecting_work() {
    let mut f = fresh();
    let cell = f.configuration.id.clone();
    let empty = f.app.executor_assignment(&f.executor, &cell).unwrap();
    assert_eq!(empty.cardinality, Cardinality::None);
    let revision = f.app.inspect_cell(&f.operator, &cell).unwrap().0;
    let run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, revision))
        .unwrap();
    assert_eq!(
        f.app
            .executor_assignment(&f.executor, &cell)
            .unwrap()
            .cardinality,
        Cardinality::None
    );
    let a = f
        .app
        .start_run(
            &f.operator,
            id().as_str(),
            start_command(&f, &run, revision, 2),
        )
        .unwrap();
    let before = f.app.inspect_run(&f.operator, &run.id).unwrap();
    let one = f.app.executor_assignment(&f.executor, &cell).unwrap();
    assert_eq!(one.cardinality, Cardinality::Single);
    assert_eq!(one.candidates[0].run, run.id);
    assert_eq!(one.candidates[0].state, RunState::Prepared);
    assert_eq!(one.candidates[0].pending_attempt.as_ref().unwrap().id, a.id);
    assert!(one.candidates[0].configuration_current);
    assert!(one.candidates[0].mandate.is_none());
    assert_eq!(
        rx_domain::canonical::bytes(&before).unwrap(),
        rx_domain::canonical::bytes(&f.app.inspect_run(&f.operator, &run.id).unwrap()).unwrap()
    );
    assert_eq!(
        f.app
            .executor_assignment(&f.executor, &cell)
            .unwrap()
            .sequence,
        one.sequence
    );
    let b = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, revision))
        .unwrap();
    f.app
        .start_run(
            &f.operator,
            id().as_str(),
            start_command(&f, &b, revision, 2),
        )
        .unwrap();
    let two = f.app.executor_assignment(&f.executor, &cell).unwrap();
    assert_eq!(two.cardinality, Cardinality::Ambiguous);
    assert_eq!(
        two.candidates
            .iter()
            .map(|v| &v.run)
            .collect::<BTreeSet<_>>(),
        [&run.id, &b.id].into_iter().collect()
    );
    assert!(two.candidates.iter().all(|v| v.mandate.is_none()));
    // Expiry of an Arm does not make unfinished work disappear from discovery.
    f.clock.0.store(40_000, Ordering::SeqCst);
    let expired = f.app.executor_assignment(&f.executor, &cell).unwrap();
    assert_eq!(expired.cardinality, Cardinality::Ambiguous);
    assert!(
        expired
            .candidates
            .iter()
            .all(|v| v.pending_attempt.as_ref().unwrap().valid_until.ticks_ns
                < expired.checked_at.ticks_ns)
    );
}
#[test]
fn assignment_read_keeps_recovery_work_visible_after_its_previous_owner_is_revoked() {
    let mut f = fresh();
    let run = start(&mut f, 2);
    let cell = f.configuration.id.clone();
    let view = f.app.executor_assignment(&f.executor, &cell).unwrap();
    assert_eq!(view.candidates[0].state, RunState::Executing);
    let session = f
        .app
        .open_executor_peer(&f.executor.principal, id(), Digest::from_bytes([71; 32]))
        .unwrap();
    let next = Identity {
        session: session.id,
        ..f.executor.clone()
    };
    f.app
        .negotiate_executor_cell(&next, f.configuration.definition.sha256)
        .unwrap();
    assert!(f.app.executor_assignment(&f.executor, &cell).is_err());
    let current = f.app.executor_assignment(&next, &cell).unwrap();
    assert_eq!(current.cardinality, Cardinality::Single);
    assert_eq!(current.candidates[0].run, run.id);
    assert_eq!(current.candidates[0].state, RunState::RecoveryRequired);
    // The authoritative invalidation clears ownership; discovery must not replace it
    // with the new reader session or hide the still-unfinished Run.
    assert!(current.candidates[0].executor_session.is_none());
    assert_eq!(current.candidates[0].mandate, run.mandate);
    assert!(
        !f.app
            .production_view(&next, &run.id)
            .unwrap()
            .admission_allowed
    );
    assert_ne!(
        current.candidates[0].executor_session.as_ref(),
        Some(&current.caller_session)
    );
}
#[test]
fn assignment_read_requires_current_role_cell_and_configured_executor() {
    let mut f = fresh();
    let cell = f.configuration.id.clone();
    assert!(f.app.executor_assignment(&f.operator, &cell).is_err());
    assert!(
        f.app
            .executor_assignment(&f.executor, &name("cell/missing"))
            .is_err()
    );
    let peer = add_identity(&mut f.app, &f.admin, "foreign-executor", &[Role::Executor]);
    assert!(f.app.executor_assignment(&peer, &cell).is_err());
    let mut principal = principal("executor", &[Role::Executor, Role::Observer]);
    principal.active = false;
    f.app
        .put_principal(&f.admin, principal, Some(Counter(1)))
        .unwrap();
    assert!(f.app.executor_assignment(&f.executor, &cell).is_err());
}
#[test]
fn assignment_contract_rejects_false_cardinality_duplicate_witness_and_changed_cut() {
    let mut f = fresh();
    start(&mut f, 2);
    let view = f
        .app
        .executor_assignment(&f.executor, &f.configuration.id)
        .unwrap();
    let mut changed = view.clone();
    changed.cardinality = Cardinality::None;
    assert!(assignment::validate(&changed).is_err());
    let mut changed = view.clone();
    changed.cardinality = Cardinality::Ambiguous;
    changed.candidates.push(changed.candidates[0].clone());
    assert!(assignment::validate(&changed).is_err());
    let mut changed = view.clone();
    changed.sequence = Counter(0);
    assert!(assignment::validate(&changed).is_err());
    let mut changed = view.clone();
    changed.valid_until.ticks_ns = Counter(changed.checked_at.ticks_ns.0 + 100_000_001);
    assert!(assignment::validate(&changed).is_err());
    let mut changed = view.clone();
    changed.candidates[0].state = RunState::Completed;
    assert!(assignment::validate(&changed).is_err());
    let mut changed = view.clone();
    changed.definition = artifact(88, "rx.cell-definition.v1");
    assert!(assignment::validate(&changed).is_err());
    let mut changed = view.clone();
    changed.candidates[0].pending_attempt = Some(assignment::PendingAttempt {
        id: id(),
        status: assignment::AttemptStatus::Arming,
        executor_session: view.caller_session.clone(),
        valid_until: view.valid_until.clone(),
    });
    assert!(assignment::validate(&changed).is_err());
}

#[test]
fn ambiguous_discovery_does_not_revoke_an_already_bound_independent_run() {
    let mut f = fresh();
    let first = start(&mut f, 2);
    let second = start(&mut f, 2);
    let discovery = f
        .app
        .executor_assignment(&f.executor, &f.configuration.id)
        .unwrap();
    assert_eq!(discovery.cardinality, Cardinality::Ambiguous);
    assert_eq!(
        discovery
            .candidates
            .iter()
            .map(|c| &c.run)
            .collect::<BTreeSet<_>>(),
        [&first.id, &second.id].into_iter().collect()
    );
    assert!(
        f.app
            .production_view(&f.executor, &first.id)
            .unwrap()
            .admission_allowed
    );
    assert!(
        f.app
            .production_view(&f.executor, &second.id)
            .unwrap()
            .admission_allowed
    );
    assert!(
        discovery
            .candidates
            .iter()
            .all(|c| c.state == RunState::Executing)
    );
}
