use super::*;
use rx_application::operator_start::{AttemptRequest, ContextRequest, DeadlineStatus};
use rx_domain::canonical;

fn prepared(f: &mut Fixture) -> Run {
    let revision = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .0;
    f.app
        .create_run(&f.operator, id().as_str(), create_command(f, revision))
        .unwrap()
}
fn input(f: &Fixture, run: &Run, limit: u64) -> ContextRequest {
    ContextRequest {
        cell: f.configuration.id.clone(),
        run: run.id.clone(),
        purpose: Purpose::Production,
        budget_limit: Counter(limit),
    }
}
#[test]
fn context_is_read_only_and_the_exact_candidate_starts_only_after_explicit_mutation() {
    let mut f = fixture(2, true);
    let run = prepared(&mut f);
    let before_cell = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    let before_run = f.app.inspect_run(&f.operator, &run.id).unwrap();
    let context = f
        .app
        .operator_start_context(&f.operator, input(&f, &run, 2))
        .unwrap();
    assert!(context.can_request && context.blocking_reason.is_none());
    assert_eq!(context.maximum_budget, f.configuration.maximum_budget);
    assert_eq!(context.request.expected_cell, before_cell.0);
    assert_eq!(context.request.expected_run, before_run.0);
    assert_eq!(
        context.request.envelope_digest,
        f.configuration.envelope.sha256
    );
    assert_eq!(context.request.budget_limit, Counter(2));
    assert!(context.run.budget.is_none() && context.run.pending_attempt.is_none());
    assert_eq!(
        canonical::bytes(
            &f.app
                .inspect_cell(&f.operator, &f.configuration.id)
                .unwrap()
        )
        .unwrap(),
        canonical::bytes(&before_cell).unwrap()
    );
    assert_eq!(
        canonical::bytes(&f.app.inspect_run(&f.operator, &run.id).unwrap()).unwrap(),
        canonical::bytes(&before_run).unwrap()
    );
    let attempt = f
        .app
        .start_run(&f.operator, id().as_str(), context.request)
        .unwrap();
    assert_eq!(attempt.status, StartStatus::Arming);
    let query = AttemptRequest {
        cell: f.configuration.id.clone(),
        run: run.id.clone(),
        id: attempt.id.clone(),
    };
    let view = f
        .app
        .operator_start_attempt(&f.operator, query.clone())
        .unwrap();
    assert_eq!(view.attempt.id, attempt.id);
    assert_eq!(view.run.state, RunState::Prepared);
    for (index, (host, registration)) in f.hosts.iter().zip(&f.registrations).enumerate() {
        f.app
            .acknowledge_arm(
                host,
                ArmAcknowledgment {
                    attempt: attempt.id.clone(),
                    host_boot: registration.boot_id.clone(),
                    delivery_journal: registration.delivery_journal.clone(),
                    sequence: Counter(1),
                    epoch: registration.epoch,
                    scopes: registration.scopes.clone(),
                },
            )
            .unwrap();
        let view = f
            .app
            .operator_start_attempt(&f.operator, query.clone())
            .unwrap();
        if index == 0 {
            assert_eq!(view.attempt.status, StartStatus::Arming);
        } else {
            assert_eq!(view.attempt.status, StartStatus::Started);
            assert_eq!(view.run.state, RunState::Executing);
        }
    }
}

#[test]
fn candidate_rejections_match_start_and_old_context_cannot_override_a_new_revision() {
    for (qualified, limit) in [(false, 2), (true, 0), (true, 11)] {
        let mut f = fixture(1, qualified);
        let run = prepared(&mut f);
        let context = f
            .app
            .operator_start_context(&f.operator, input(&f, &run, limit))
            .unwrap();
        assert!(!context.can_request);
        let reason = context.blocking_reason.unwrap();
        assert!(
            matches!(f.app.start_run(&f.operator, id().as_str(), context.request), Err(StoreError::Rejected(actual)) if actual == reason)
        );
        assert!(
            f.app
                .inspect_run(&f.operator, &run.id)
                .unwrap()
                .1
                .budget
                .is_none()
        );
    }
    let mut f = fixture(1, true);
    let run = prepared(&mut f);
    let context = f
        .app
        .operator_start_context(&f.operator, input(&f, &run, 2))
        .unwrap();
    for change in 0..4 {
        let mut request = context.request.clone();
        match change {
            0 => request.envelope_digest = Digest::from_bytes([0; 32]),
            1 => request.budget_unit = rx_domain::budget::BudgetUnit::OperationCount,
            2 => request.expected_cell = Counter(0),
            _ => request.expected_run = Counter(0),
        }
        assert!(
            f.app
                .start_run(&f.operator, id().as_str(), request)
                .is_err()
        );
    }
    f.app
        .hold(&f.operator, id().as_str(), &f.configuration.id)
        .unwrap();
    assert!(matches!(
        f.app.start_run(&f.operator, id().as_str(), context.request),
        Err(StoreError::Rejected(Rejection::StaleRevision))
    ));
}

#[test]
fn a_valid_operator_without_terminal_gets_a_blocked_candidate_and_scope_stays_private() {
    let mut f = fixture(1, true);
    let run = prepared(&mut f);
    let session = f
        .app
        .authenticated_session(&f.operator.principal, id(), expiry(99_000))
        .unwrap();
    let actor = Identity {
        principal: f.operator.principal.clone(),
        session: session.id,
        terminal: None,
    };
    let context = f
        .app
        .operator_start_context(&actor, input(&f, &run, 2))
        .unwrap();
    assert!(!context.can_request);
    assert_eq!(context.blocking_reason, Some(Rejection::Forbidden));
    let reader = add_identity(&mut f.app, &f.admin, "start-reader", &[Role::Observer]);
    assert!(matches!(
        f.app.operator_start_context(&reader, input(&f, &run, 2)),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    let mut wrong = input(&f, &run, 2);
    wrong.cell = name("cell/b");
    assert!(f.app.operator_start_context(&actor, wrong).is_err());
    let original = f.app.installation.clone();
    f.app.installation.id = id();
    assert!(
        f.app
            .operator_start_context(&actor, input(&f, &run, 2))
            .is_err()
    );
    f.app.installation = original;
}

#[test]
fn lost_start_response_uses_the_original_attempt_and_deadline_read_does_not_mutate_status() {
    let mut f = fixture(1, true);
    let run = prepared(&mut f);
    let context = f
        .app
        .operator_start_context(&f.operator, input(&f, &run, 2))
        .unwrap();
    let key = id();
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .start_run(&f.operator, key.as_str(), context.request.clone())
            .is_err()
    );
    let attempt = f
        .app
        .start_run(&f.operator, key.as_str(), context.request.clone())
        .unwrap();
    let query = AttemptRequest {
        cell: f.configuration.id.clone(),
        run: run.id.clone(),
        id: attempt.id.clone(),
    };
    let current = f
        .app
        .operator_start_attempt(&f.operator, query.clone())
        .unwrap();
    assert_eq!(current.deadline_status, DeadlineStatus::WithinDeadline);
    assert_eq!(current.run.pending_attempt, Some(attempt.id.clone()));
    let pending = f
        .app
        .operator_start_context(&f.operator, input(&f, &run, 2))
        .unwrap();
    assert_eq!(pending.blocking_reason, Some(Rejection::Busy));
    let mut conflict = context.request;
    conflict.budget_limit = Counter(3);
    assert!(matches!(
        f.app.start_run(&f.operator, key.as_str(), conflict),
        Err(StoreError::KeyConflict)
    ));
    let mut wrong = query.clone();
    wrong.run = id();
    assert!(f.app.operator_start_attempt(&f.operator, wrong).is_err());
    f.clock
        .0
        .store(attempt.valid_until.ticks_ns.0 + 1, Ordering::SeqCst);
    let expired = f.app.operator_start_attempt(&f.operator, query).unwrap();
    assert_eq!(expired.deadline_status, DeadlineStatus::Elapsed);
    assert_eq!(expired.attempt.status, StartStatus::Arming);
    assert_eq!(expired.run.pending_attempt, Some(attempt.id));
    assert_eq!(
        canonical::bytes(&expired.run).unwrap(),
        canonical::bytes(&current.run).unwrap()
    );
}
