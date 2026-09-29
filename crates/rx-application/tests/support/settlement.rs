use super::*;
use rx_application::settlement::Approve;
use rx_domain::operation::Disposition;
fn prepared(fence: bool) -> (Fixture, Identity, Work, NativeEvidence) {
    let mut f = fixture_complete(1, true, false, true);
    let (work, evidence) = completed_work(&mut f);
    f.app
        .put_principal(
            &f.admin,
            principal("release", &[Role::ReleaseManager, Role::Observer]),
            None,
        )
        .unwrap();
    let session = f
        .app
        .authenticated_terminal_user_session(
            &name("release"),
            id(),
            Counter(99_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap();
    let lead = Identity {
        principal: name("release"),
        session: session.id,
        terminal: Some((name("panel/main"), Digest::from_bytes([77; 32]))),
    };
    let new = f
        .app
        .open_executor_peer(&name("executor"), id(), Digest::from_bytes([41; 32]))
        .unwrap();
    f.executor = Identity {
        principal: name("executor"),
        session: new.id,
        terminal: None,
    };
    f.app
        .negotiate_executor_cell(&f.executor, f.configuration.definition.sha256)
        .unwrap();
    if fence {
        confirm_case_fences_from(&mut f, 30);
    }
    let work = f
        .app
        .inspect_work(&f.operator, work.operation.id())
        .unwrap();
    assert_eq!(work.operation.disposition(), Disposition::Quarantined);
    (f, lead, work, evidence)
}
fn approval(f: &mut Fixture, work: &Work) -> Approve {
    Approve {
        operation: work.operation.id().clone(),
        expected_operation: work.operation.revision(),
        expected_cell: f.app.inspect_cell(&f.operator, &work.cell).unwrap().0,
        justification: "Known result; current non-actuating handover only".into(),
    }
}
#[test]
fn current_settlement_closes_confirmed_work_without_restoring_execution_rights() {
    let (mut f, lead, work, evidence) = prepared(true);
    let cell_before = f.app.inspect_cell(&f.operator, &work.cell).unwrap().1;
    let command = approval(&mut f, &work);
    let key = id();
    assert!(
        f.app
            .approve_settlement(&f.executor, id().as_str(), command.clone())
            .is_err()
    );
    let auth = f
        .app
        .approve_settlement(&lead, key.as_str(), command.clone())
        .unwrap();
    assert_eq!(
        f.app
            .approve_settlement(&lead, key.as_str(), command)
            .unwrap()
            .id,
        auth.id
    );
    let proof = handover_proof(&mut f, &work, &evidence);
    assert!(
        f.app
            .release_resources(&f.hosts[0], id().as_str(), proof.clone())
            .is_err()
    );
    assert!(
        f.app
            .settle_resources(&f.executor, &auth.id, proof.clone())
            .is_err()
    );
    let result = f
        .app
        .settle_resources(&f.hosts[0], &auth.id, proof.clone())
        .unwrap();
    assert_eq!(result.operation.disposition(), Disposition::Released);
    assert_eq!(
        result.operation.outcome(),
        rx_domain::operation::Outcome::Succeeded
    );
    assert_eq!(
        f.app
            .settle_resources(&f.hosts[0], &auth.id, proof)
            .unwrap()
            .operation
            .id(),
        work.operation.id()
    );
    let run = f.app.inspect_run(&f.operator, &work.run).unwrap().1;
    assert_eq!(run.state, RunState::Completed);
    let cell = f.app.inspect_cell(&f.operator, &work.cell).unwrap().1;
    assert_eq!(
        rx_domain::canonical::bytes(&cell).unwrap(),
        rx_domain::canonical::bytes(&cell_before).unwrap()
    );
    assert!(!cell.blocks.is_empty());
    let permit = f.app.inspect_permit(&f.operator, &work.permit).unwrap();
    let mut repository = f.app.into_repository();
    let mandate: Mandate = repository
        .transact(|tx| {
            let row = tx
                .get(&rx_application::persistence::key(
                    "mandate",
                    &permit.mandate,
                ))?
                .unwrap();
            rx_application::persistence::decode(&row, "rx.internal.mandate.v1")
        })
        .unwrap();
    assert_eq!(mandate.state, MandateState::Revoked);
}
#[test]
fn settlement_requires_current_fence_and_fresh_correlated_handover() {
    let (mut f, lead, work, evidence) = prepared(false);
    let command = approval(&mut f, &work);
    assert!(
        f.app
            .approve_settlement(&lead, id().as_str(), command)
            .is_err()
    );
    confirm_case_fences_from(&mut f, 30);
    let command = approval(&mut f, &work);
    let auth = f
        .app
        .approve_settlement(&lead, id().as_str(), command)
        .unwrap();
    for variant in 0..4 {
        let mut proof = handover_proof(&mut f, &work, &evidence);
        match variant {
            0 => proof.observations[0].value = false,
            1 => proof.observations[0].invocation = id(),
            2 => proof.observations[0].device_session = id(),
            _ => proof.observations[0].observed_at.clock_id = "foreign".into(),
        }
        assert!(
            f.app
                .settle_resources(&f.hosts[0], &auth.id, proof)
                .is_err()
        );
        assert_eq!(
            f.app
                .inspect_work(&f.operator, work.operation.id())
                .unwrap()
                .operation
                .disposition(),
            Disposition::Quarantined
        );
    }
}
#[test]
fn settlement_context_change_and_actor_revocation_invalidate_the_grant() {
    for change in [0, 1] {
        let (mut f, lead, work, evidence) = prepared(true);
        let command = approval(&mut f, &work);
        let auth = f
            .app
            .approve_settlement(&lead, id().as_str(), command)
            .unwrap();
        let proof = handover_proof(&mut f, &work, &evidence);
        if change == 0 {
            f.app
                .open_executor_peer(&name("executor"), id(), Digest::from_bytes([41; 32]))
                .unwrap();
        } else {
            let mut p = principal("release", &[Role::ReleaseManager, Role::Observer]);
            p.active = false;
            f.app.put_principal(&f.admin, p, Some(Counter(1))).unwrap();
        }
        assert!(
            f.app
                .settle_resources(&f.hosts[0], &auth.id, proof)
                .is_err()
        );
        assert_eq!(
            f.app
                .inspect_work(&f.operator, work.operation.id())
                .unwrap()
                .operation
                .disposition(),
            Disposition::Quarantined
        );
    }
}
#[test]
fn settlement_resource_work_and_run_commit_atomically_and_recover_reply_loss() {
    for mode in [1, 2] {
        let (mut f, lead, work, evidence) = prepared(true);
        let command = approval(&mut f, &work);
        let auth = f
            .app
            .approve_settlement(&lead, id().as_str(), command)
            .unwrap();
        let proof = handover_proof(&mut f, &work, &evidence);
        f.failure.store(mode, Ordering::SeqCst);
        assert!(
            f.app
                .settle_resources(&f.hosts[0], &auth.id, proof.clone())
                .is_err()
        );
        let before = f
            .app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap();
        assert_eq!(
            before.operation.disposition(),
            if mode == 1 {
                Disposition::Quarantined
            } else {
                Disposition::Released
            }
        );
        let result = f
            .app
            .settle_resources(&f.hosts[0], &auth.id, proof)
            .unwrap();
        assert_eq!(result.operation.disposition(), Disposition::Released);
        assert_eq!(
            f.app.inspect_run(&f.operator, &work.run).unwrap().1.state,
            RunState::Completed
        );
    }
}

#[test]
fn superseded_settlement_approval_cannot_be_used() {
    let (mut f, lead, work, evidence) = prepared(true);
    let command = approval(&mut f, &work);
    let old = f
        .app
        .approve_settlement(&lead, id().as_str(), command.clone())
        .unwrap();
    let current = f
        .app
        .approve_settlement(&lead, id().as_str(), command)
        .unwrap();
    let proof = handover_proof(&mut f, &work, &evidence);
    assert!(
        f.app
            .settle_resources(&f.hosts[0], &old.id, proof.clone())
            .is_err()
    );
    assert_eq!(
        f.app
            .settle_resources(&f.hosts[0], &current.id, proof)
            .unwrap()
            .operation
            .disposition(),
        Disposition::Released
    );
}
