//! Focused invariant scenario tests (see docs/invariant-traceability.json).
use super::*;
use rx_application::{persistence as p, store_restore};
use rx_domain::operation::Outcome;
use rx_ports::OutboxState;

fn receipt(
    work: &Work,
    journal: &Id,
    invocation: &Id,
    sequence: u64,
    state: ReceiptState,
) -> HostReceipt {
    HostReceipt {
        operation: work.operation.id().clone(),
        digest: work.intent.digest().unwrap(),
        invocation: Some(invocation.clone()),
        journal: journal.clone(),
        sequence: Counter(sequence),
        state,
    }
}

fn pending(app: &mut App, message: &Id) -> Option<OutboxState> {
    app.pending_deliveries(128)
        .unwrap()
        .into_iter()
        .find(|d| &d.id == message)
        .map(|d| d.state)
}

/// OI06: a permit names its operation, intent digest, Host and Host boot, cell epoch vector,
/// mandate and purpose; P refuses to emit an authorization for it once it is consumed or expired,
/// and an epoch change voids an unconsumed one. The Host's own native gate is not exercised.
#[test]
fn permit_binds_operation_content_host_epoch_and_purpose_and_is_not_emitted_after_consumption_or_expiry()
 {
    let mut f = fixture_complete(2, true, false, true);
    let run = start(&mut f, 2);
    let journal = f.registrations[0].delivery_journal.clone();
    let part = f
        .app
        .begin_part(
            &f.executor,
            id().as_str(),
            &run.id,
            run.budget.as_ref().unwrap().revision(),
        )
        .unwrap();
    let rev = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
    let first = f
        .app
        .resolve_activation(&f.executor, &run.id, &name("step/0"), part.ordinal, rev)
        .unwrap();
    let rev = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
    let second = f
        .app
        .resolve_activation(&f.executor, &run.id, &name("step/1"), part.ordinal, rev)
        .unwrap();
    let work = submit(&mut f, &first, id().as_str()).unwrap();
    let op = work.operation.id().clone();

    // Binding: operation, content, Host/boot, epoch vector, mandate and purpose.
    let (_, cell) = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    let permit = f.app.inspect_permit(&f.operator, &work.permit).unwrap();
    assert_eq!(permit.operation, op);
    assert_eq!(permit.intent_digest, work.intent.digest().unwrap());
    assert_eq!(permit.host, name("host/0"));
    assert_eq!(permit.host_boot, f.registrations[0].boot_id);
    assert_eq!(permit.epoch, cell.epoch);
    assert_eq!(permit.scopes, cell.scope_epochs);
    assert_eq!(Some(&permit.mandate), run.mandate.as_ref());
    assert_eq!(permit.purpose, Purpose::Production);
    assert_eq!(permit.state, PermitState::Issued);
    // now=1000 plus permit_ttl_ns=5000; the observation and grant both outlive it.
    assert_eq!(permit.expires_at.ticks_ns, Counter(6000));

    // Another Host cannot take the delivery or report against this operation, and a
    // receipt for different content is refused.
    assert!(matches!(
        f.app.plan_delivery(&f.hosts[1], &op),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    assert_eq!(pending(&mut f.app, &op), Some(OutboxState::New));
    assert!(
        f.app
            .plan_delivery(&f.hosts[0], &op)
            .unwrap()
            .first_emission
    );
    let invocation = id();
    let foreign = HostReceipt {
        journal: f.registrations[1].delivery_journal.clone(),
        ..receipt(&work, &journal, &invocation, 1, ReceiptState::Prepared)
    };
    assert!(matches!(
        f.app.record_host_receipt(&f.hosts[1], &op, foreign),
        Err(StoreError::Rejected(Rejection::InvalidInput))
    ));
    let mut changed = work.intent.clone();
    changed.execution_timeout_ms = Counter(6000);
    let altered = HostReceipt {
        digest: changed.digest().unwrap(),
        ..receipt(&work, &journal, &invocation, 1, ReceiptState::Prepared)
    };
    assert!(matches!(
        f.app.record_host_receipt(&f.hosts[0], &op, altered),
        Err(StoreError::Rejected(Rejection::InvalidInput))
    ));

    // Consumption: the Prepared receipt queues one Authorize for this permit. Once the Host
    // reports that it entered the send, the permit is Consumed and that queued Authorize can no
    // longer be emitted.
    f.app
        .record_host_receipt(
            &f.hosts[0],
            &op,
            receipt(&work, &journal, &invocation, 1, ReceiptState::Prepared),
        )
        .unwrap();
    let authorization = rx_application::engine::authorization_delivery_id(&op);
    assert!(f.app.pending_deliveries(128).unwrap().iter().any(|d| {
        d.id == authorization
            && d.state == OutboxState::New
            && matches!(&d.payload, Delivery::Authorize { operation, host, permit, .. }
                if operation == &op && host == &name("host/0") && permit == &work.permit)
    }));
    f.app
        .record_host_receipt(
            &f.hosts[0],
            &op,
            receipt(&work, &journal, &invocation, 2, ReceiptState::SendEntered),
        )
        .unwrap();
    assert_eq!(
        f.app
            .inspect_permit(&f.operator, &work.permit)
            .unwrap()
            .state,
        PermitState::Consumed
    );
    assert!(matches!(
        f.app.begin_delivery(&authorization),
        Err(StoreError::Rejected(Rejection::StaleEpoch))
    ));
    assert!(matches!(
        f.app.plan_delivery(&f.hosts[0], &authorization),
        Err(StoreError::Rejected(Rejection::StaleEpoch))
    ));
    assert_eq!(pending(&mut f.app, &authorization), Some(OutboxState::New));
    // A new request key for the same slot returns the same operation and permit.
    let again = submit(&mut f, &first, id().as_str()).unwrap();
    assert_eq!(again.operation.id(), &op);
    assert_eq!(again.permit, work.permit);

    // Expiry: a prepared operation whose permit lapses cannot have its authorization emitted.
    let second_journal = f.registrations[1].delivery_journal.clone();
    let cr = f
        .app
        .inspect_cell(&f.executor, &f.configuration.id)
        .unwrap()
        .0;
    let rr = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
    let late = f
        .app
        .submit(
            &f.executor,
            id().as_str(),
            SubmitWork {
                intent: f.configuration.steps[1].intent.clone(),
                ..work_command(&f, &second, cr, rr)
            },
        )
        .unwrap();
    let late_op = late.operation.id().clone();
    assert_eq!(
        f.app
            .inspect_permit(&f.operator, &late.permit)
            .unwrap()
            .expires_at
            .ticks_ns,
        Counter(6000)
    );
    assert!(
        f.app
            .plan_delivery(&f.hosts[1], &late_op)
            .unwrap()
            .first_emission
    );
    f.app
        .record_host_receipt(
            &f.hosts[1],
            &late_op,
            receipt(&late, &second_journal, &id(), 1, ReceiptState::Prepared),
        )
        .unwrap();
    let late_authorization = rx_application::engine::authorization_delivery_id(&late_op);
    assert_eq!(
        pending(&mut f.app, &late_authorization),
        Some(OutboxState::New)
    );
    f.clock.0.store(6000, Ordering::SeqCst);
    assert!(matches!(
        f.app.begin_delivery(&late_authorization),
        Err(StoreError::Rejected(Rejection::StaleEpoch))
    ));
    assert!(matches!(
        f.app.plan_delivery(&f.hosts[1], &late_authorization),
        Err(StoreError::Rejected(Rejection::StaleEpoch))
    ));
    assert_eq!(
        pending(&mut f.app, &late_authorization),
        Some(OutboxState::New)
    );
    assert_eq!(
        f.app
            .inspect_permit(&f.operator, &late.permit)
            .unwrap()
            .state,
        PermitState::Issued
    );

    // Epoch binding: a new cell epoch voids the unconsumed permit and its queued authorization,
    // and the never-sent operation is concluded NotExecuted rather than left executable.
    f.app
        .hold(&f.operator, id().as_str(), &f.configuration.id)
        .unwrap();
    assert_eq!(
        f.app
            .inspect_permit(&f.operator, &late.permit)
            .unwrap()
            .state,
        PermitState::Voided
    );
    assert_eq!(pending(&mut f.app, &late_authorization), None);
    assert_eq!(
        f.app
            .inspect_work(&f.operator, &late_op)
            .unwrap()
            .operation
            .outcome(),
        Outcome::NotExecuted
    );
    assert_eq!(
        f.app
            .inspect_permit(&f.operator, &work.permit)
            .unwrap()
            .state,
        PermitState::Consumed
    );
}

/// Every mandate and permit of this cell, the admitted work, and the Prepare outbox state.
fn authority(
    repository: &mut FaultRepository,
    cell: &Name,
    operation: &Id,
) -> (
    Vec<MandateState>,
    Vec<PermitState>,
    Outcome,
    Option<OutboxState>,
) {
    repository
        .transact(|tx| {
            let mut mandates = vec![];
            for row in tx.scan("mandate/")? {
                let m: Mandate = p::decode(&row, "rx.internal.mandate.v1")?;
                if &m.cell == cell {
                    mandates.push(m.state);
                }
            }
            let mut permits = vec![];
            for row in tx.scan("permit/")? {
                let permit: Permit = p::decode(&row, "rx.internal.permit.v1")?;
                if &permit.cell == cell {
                    permits.push(permit.state);
                }
            }
            let (_, work): (_, Work) = p::load(tx, "work", operation, "rx.internal.work.v1")?;
            let outbox = tx.outbox(operation)?.map(|r| r.state);
            Ok((mandates, permits, work.operation.outcome(), outbox))
        })
        .unwrap()
}

fn open(repository: FaultRepository, clock: &ManualClock, installation: &Id) -> App {
    Engine::open(
        repository,
        clock.clone(),
        SimulationAuthority,
        installation.clone(),
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap()
}

/// OI11: after a P restart, and again after a backup taken while authority was live is
/// restored, the restored mandate is Revoked, the unsent permit Voided, the admitted work
/// NotExecuted and its Prepare not emittable; old sessions are refused and the restore rotates
/// the store generation. Host, executor and controller restarts are not exercised here, and
/// the rollback epoch is not checked against Host maxima.
#[test]
fn p_restart_and_backup_rollback_leave_no_old_mandate_or_permit_executable() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 2);
    let a = activation(&mut f, &run);
    let work = submit(&mut f, &a, id().as_str()).unwrap();
    let op = work.operation.id().clone();
    let cell = f.configuration.id.clone();
    let installation = f.app.installation.clone();
    let epoch = f.app.inspect_cell(&f.operator, &cell).unwrap().1.epoch;
    let resubmission = SubmitWork {
        run: a.run.clone(),
        activation: a.id.clone(),
        part: a.part.clone(),
        slot: name("main"),
        intent: work.intent.clone(),
        expected_cell: Counter(1),
        expected_run: Counter(1),
    };
    let mut repository = f.app.into_repository();
    let before = authority(&mut repository, &cell, &op);
    assert_eq!(before.0, vec![MandateState::Active]);
    assert_eq!(before.1, vec![PermitState::Issued]);
    assert_eq!(before.2, Outcome::None);
    assert_eq!(before.3, Some(OutboxState::New));
    let directory = tempfile::tempdir().unwrap();
    let backup = directory.path().join("backup.db");
    repository.inner.backup(&backup).unwrap();

    let expect_retired = |app: &mut App, repository_epoch: Counter| {
        let operator = Identity {
            session: app
                .authenticated_terminal_user_session(
                    &name("operator"),
                    id(),
                    Counter(99_000),
                    Digest::from_bytes([77; 32]),
                )
                .unwrap()
                .id,
            ..f.operator.clone()
        };
        let restored = app.inspect_run(&operator, &run.id).unwrap().1;
        assert_eq!(restored.state, RunState::RecoveryRequired);
        assert!(restored.executor_session.is_none());
        let (_, current) = app.inspect_cell(&operator, &cell).unwrap();
        assert!(current.epoch > repository_epoch);
        assert!(
            current
                .blocks
                .iter()
                .any(|b| b.latched && b.reason == BlockReason::RuntimeRestart)
        );
        assert_eq!(pending(app, &op), None);
        assert!(app.begin_delivery(&op).is_err());
        // The old executor session cannot reuse the slot or submit again.
        assert!(matches!(
            app.submit(&f.executor, id().as_str(), resubmission.clone()),
            Err(StoreError::Rejected(Rejection::Unauthenticated))
        ));
        assert!(matches!(
            app.inspect_run(&f.operator, &run.id),
            Err(StoreError::Rejected(Rejection::Unauthenticated))
        ));
    };

    // P restart on the live store.
    let mut app = open(repository, &f.clock, &installation.id);
    assert_eq!(
        app.installation.store_generation,
        installation.store_generation
    );
    expect_retired(&mut app, epoch);
    let mut live = app.into_repository();
    let after_restart = authority(&mut live, &cell, &op);
    assert_eq!(after_restart.0, vec![MandateState::Revoked]);
    assert_eq!(after_restart.1, vec![PermitState::Voided]);
    assert_eq!(after_restart.2, Outcome::NotExecuted);
    assert_eq!(after_restart.3, Some(OutboxState::Voided));
    drop(live);

    // Backup rollback: the cut still holds the Active mandate and Issued permit.
    let mut restored = FaultRepository {
        inner: SqliteRepository::open(&backup).unwrap(),
        mode: Arc::new(AtomicU8::new(0)),
    };
    assert_eq!(authority(&mut restored, &cell, &op), before);
    let record = store_restore::restore_store(
        &mut restored,
        &installation.id,
        Digest::from_bytes([8; 32]),
        f.clock.now(),
    )
    .unwrap();
    assert_eq!(record.previous_generation, installation.store_generation);
    let mut app = open(restored, &f.clock, &installation.id);
    assert_eq!(app.installation.store_generation, record.generation);
    assert_ne!(
        app.installation.store_generation,
        installation.store_generation
    );
    expect_retired(&mut app, epoch);
    let mut rolled_back = app.into_repository();
    let after_rollback = authority(&mut rolled_back, &cell, &op);
    assert_eq!(after_rollback.0, vec![MandateState::Revoked]);
    assert_eq!(after_rollback.1, vec![PermitState::Voided]);
    assert_eq!(after_rollback.2, Outcome::NotExecuted);
    assert_eq!(after_rollback.3, Some(OutboxState::Voided));
}
