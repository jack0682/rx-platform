use super::*;
use rx_application::qualification_activation as a;
use rx_domain::{host_configuration as cfg, host_qualification as h};
fn approved_setup() -> (
    Fixture,
    Identity,
    process_change::Change,
    qsupport::Fixture,
    review_support::Fixture,
    q::Job,
    q::Version,
    q::Decision,
) {
    let (mut f, release, c, p, source) = setup_sources();
    let input = begin_input(&mut f, &c, &p);
    let j = f.app.begin_requalification(&release, &id(), input).unwrap();
    ack(&mut f, &j);
    let report = prepared_report(&mut f, &j, &p, &id(), None);
    let v = f.app.commit_requalification_report(report).unwrap();
    let reviewer = add_identity(
        &mut f.app,
        &f.admin,
        "activation-reviewer",
        &[Role::Verifier],
    );
    let q::DecisionPreflight::Verify(t) = f
        .app
        .prepare_requalification_decision(&reviewer, &id(), decision(&v, &j))
        .unwrap()
    else {
        panic!("decision")
    };
    let d = f
        .app
        .commit_requalification_decision(t.verify(&p.policy).unwrap())
        .unwrap();
    (f, release, c, p, source, j, v, d)
}
fn issue_input(f: &mut Fixture, j: &q::Job, v: &q::Version, d: &q::Decision) -> a::IssueRequest {
    a::IssueRequest {
        review: j.request.id.clone(),
        cell: j.request.origin.clone(),
        report_revision: v.revision,
        report_digest: v.digest,
        decision_revision: d.revision,
        expected_cells: j
            .request
            .cells
            .iter()
            .map(|t| {
                (
                    t.profile.cell.clone(),
                    f.app.inspect_cell(&f.admin, &t.profile.cell).unwrap().0,
                )
            })
            .collect(),
        clear_blocks: j
            .request
            .cells
            .iter()
            .map(|t| {
                (
                    t.profile.cell.clone(),
                    f.app
                        .inspect_cell(&f.admin, &t.profile.cell)
                        .unwrap()
                        .1
                        .blocks
                        .iter()
                        .map(|b| b.id.clone())
                        .collect(),
                )
            })
            .collect(),
    }
}
fn verify(t: a::Ticket, p: &qsupport::Fixture, source: &review_support::Fixture) -> a::Prepared {
    a::Prepared::verify(
        t,
        &p.policy,
        source
            .store
            .verify_owned(&source.object, &source.policy)
            .unwrap(),
    )
    .unwrap()
}
fn issue(
    f: &mut Fixture,
    r: &Identity,
    j: &q::Job,
    v: &q::Version,
    d: &q::Decision,
    p: &qsupport::Fixture,
    source: &review_support::Fixture,
) -> a::Batch {
    let input = issue_input(f, j, v, d);
    let a::Preflight::Verify(t) = f.app.prepare_qualification_issue(r, &id(), input).unwrap()
    else {
        panic!("ticket")
    };
    f.app
        .commit_qualification_issue(verify(*t, p, source))
        .unwrap()
}
fn inspect(f: &mut Fixture, b: &a::Batch, t: &a::Task) -> h::Observation {
    let c = f
        .app
        .process_change(&f.admin, &b.origin, &b.change)
        .unwrap()
        .change;
    let origin = c
        .application
        .as_ref()
        .unwrap()
        .host_proofs
        .iter()
        .find(|p| p.host == t.host)
        .unwrap();
    h::Observation {
        schema: name("rx.host-qualification-observation.v1"),
        snapshot: cfg::Snapshot {
            schema: name("rx.host-process-configuration-snapshot.v1"),
            host: t.host.clone(),
            host_boot: t.host_boot.clone(),
            delivery_journal: t.journal.clone(),
            binding_digest: Digest::from_bytes([85; 32]),
            cells: t
                .cells
                .iter()
                .map(|id| {
                    let cell = f.app.inspect_cell(&f.admin, id).unwrap().1;
                    let target = b.cells.iter().find(|c| &c.cell == id).unwrap();
                    cfg::CellObservation {
                        cell: id.clone(),
                        definition: cell.configuration.definition.sha256,
                        envelope: cell.configuration.envelope.sha256,
                        environment: name("SIMULATION"),
                        epoch: cell.epoch,
                        scopes: cell.scope_epochs,
                        blocked: cell.blocks.iter().map(|b| b.id.clone()).collect(),
                        applied: Some(cfg::AppliedContext {
                            cell: id.clone(),
                            configuration: target.configuration.sha256,
                            change: b.change.clone(),
                            request: origin.task.clone(),
                            receipt_sequence: Counter(17),
                            binding_digest: Digest::from_bytes([85; 32]),
                        }),
                    }
                })
                .collect(),
        },
        receipt: None,
        accepted: vec![],
        receipt_matches_current_host: false,
        activation_authorized: false,
    }
}
fn accepted(mut o: h::Observation, t: &a::Task) -> h::Observation {
    let request = t.request.as_ref().unwrap();
    let receipt = h::Receipt {
        schema: name("rx.host-qualification-receipt.v1"),
        request: request.clone(),
        request_digest: request.digest().unwrap(),
        host_boot: t.host_boot.clone(),
        journal: t.journal.clone(),
        sequence: Counter(900),
        status: h::Status::Accepted,
        reason: None,
        quiescence: Some(cfg::Quiescence {
            device_session: id(),
            observed_at: expiry(1000),
            uncertainty_ns: Counter(0),
            resources: vec![name("controller/0")],
        }),
        recorded_at: expiry(1000),
    };
    o.accepted = request
        .cells
        .iter()
        .map(|c| h::AcceptedCell {
            cell: c.cell.clone(),
            request: t.id.clone(),
            qualification: c.qualification.clone(),
            qualification_revision: c.qualification_revision,
            acceptance_sequence: receipt.sequence,
            host_boot: t.host_boot.clone(),
            epoch: c.epoch,
            scopes: c.scopes.clone(),
            configuration: c.configuration,
            context_request: c.context_request.clone(),
            context_sequence: c.context_sequence,
            binding_digest: request.binding_digest,
        })
        .collect();
    o.receipt = Some(receipt);
    o.receipt_matches_current_host = true;
    o.validate().unwrap();
    o
}
fn confirm(f: &mut Fixture, b: &a::Batch) -> (a::Task, h::Observation) {
    confirm_host(f, b, 0)
}
fn confirm_host(f: &mut Fixture, b: &a::Batch, host: usize) -> (a::Task, h::Observation) {
    let t = f
        .app
        .qualification_tasks(&f.hosts[host], None)
        .unwrap()
        .into_iter()
        .find(|t| t.batch == b.id)
        .unwrap();
    let snapshot = inspect(f, b, &t);
    let t = f
        .app
        .bind_qualification_request(&f.hosts[host], &t.id, snapshot.clone(), f.clock.now())
        .unwrap();
    f.app
        .enter_qualification_send(&f.hosts[host], &t.id, false)
        .unwrap();
    let observation = accepted(snapshot, &t);
    let t = f
        .app
        .record_qualification_observation(&f.hosts[host], &t.id, observation.clone(), f.clock.now())
        .unwrap();
    (t, observation)
}
fn finalize(b: &a::Batch) -> a::Finalize {
    a::Finalize {
        batch: b.id.clone(),
        cell: b.origin.clone(),
        expected: b.revision,
        expected_cells: b
            .cells
            .iter()
            .map(|c| (c.cell.clone(), c.expected_revision))
            .collect(),
    }
}
fn activate(
    f: &mut Fixture,
    r: &Identity,
    b: &a::Batch,
    p: &qsupport::Fixture,
    source: &review_support::Fixture,
) -> a::Batch {
    let a::Preflight::Verify(t) = f
        .app
        .prepare_qualification_activation(r, &id(), finalize(b))
        .unwrap()
    else {
        panic!("activation")
    };
    f.app
        .commit_qualification_activation(verify(*t, p, source))
        .unwrap()
}
#[test]
fn qualification_issuance_is_atomic_and_request_loss_preserves_ids_and_clear_plan() {
    for mode in [1, 2] {
        let (mut f, r, _, p, source, j, v, d) = approved_setup();
        let input = issue_input(&mut f, &j, &v, &d);
        let key = id();
        let a::Preflight::Verify(t) = f
            .app
            .prepare_qualification_issue(&r, &key, input.clone())
            .unwrap()
        else {
            panic!("ticket")
        };
        let proof = verify(*t, &p, &source);
        f.failure.store(mode, Ordering::SeqCst);
        assert!(f.app.commit_qualification_issue(proof).is_err());
        let b = match f
            .app
            .prepare_qualification_issue(&r, &key, input.clone())
            .unwrap()
        {
            a::Preflight::Recorded(b) => *b,
            a::Preflight::Verify(t) => f
                .app
                .commit_qualification_issue(verify(*t, &p, &source))
                .unwrap(),
        };
        let a::Preflight::Recorded(again) =
            f.app.prepare_qualification_issue(&r, &id(), input).unwrap()
        else {
            panic!("same slot")
        };
        assert_eq!(b.tasks, again.tasks);
        assert_eq!(b.cells[0].qualification.id, again.cells[0].qualification.id);
        assert!(
            f.app
                .inspect_cell(&f.admin, &b.origin)
                .unwrap()
                .1
                .qualification
                .is_none()
        );
    }
}
#[test]
fn qualification_send_journal_does_not_infer_rejection_from_missing_reply() {
    let (mut f, r, _, p, source, j, v, d) = approved_setup();
    let b = issue(&mut f, &r, &j, &v, &d, &p, &source);
    let t = f
        .app
        .qualification_tasks(&f.hosts[0], None)
        .unwrap()
        .remove(0);
    let o = inspect(&mut f, &b, &t);
    let t = f
        .app
        .bind_qualification_request(&f.hosts[0], &t.id, o.clone(), f.clock.now())
        .unwrap();
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .enter_qualification_send(&f.hosts[0], &t.id, false)
            .is_err()
    );
    assert!(matches!(
        f.app
            .enter_qualification_send(&f.hosts[0], &t.id, false)
            .unwrap(),
        a::Emission::Lookup { .. }
    ));
    assert!(
        f.app
            .prepare_qualification_activation(&r, &id(), finalize(&b))
            .is_err()
    );
    f.app
        .record_qualification_observation(&f.hosts[0], &t.id, o, f.clock.now())
        .unwrap();
    let a::Emission::Send { request } = f
        .app
        .enter_qualification_send(&f.hosts[0], &t.id, true)
        .unwrap()
    else {
        panic!("same resend")
    };
    assert_eq!(request.digest().unwrap(), t.digest.unwrap());
}
#[test]
fn qualification_activation_is_atomic_clears_only_owned_blocks_and_requires_separate_start() {
    for mode in [1, 2] {
        let (mut f, r, c, p, source, j, v, d) = approved_setup();
        let b = issue(&mut f, &r, &j, &v, &d, &p, &source);
        confirm(&mut f, &b);
        let before = f.app.inspect_cell(&f.admin, &c.cell).unwrap();
        let key = id();
        let a::Preflight::Verify(t) = f
            .app
            .prepare_qualification_activation(&r, &key, finalize(&b))
            .unwrap()
        else {
            panic!("ticket")
        };
        f.failure.store(mode, Ordering::SeqCst);
        assert!(
            f.app
                .commit_qualification_activation(verify(*t, &p, &source))
                .is_err()
        );
        if mode == 1 {
            assert_eq!(f.app.inspect_cell(&f.admin, &c.cell).unwrap().0, before.0);
        }
        let active = match f
            .app
            .prepare_qualification_activation(&r, &key, finalize(&b))
            .unwrap()
        {
            a::Preflight::Recorded(b) => *b,
            a::Preflight::Verify(t) => f
                .app
                .commit_qualification_activation(verify(*t, &p, &source))
                .unwrap(),
        };
        assert_eq!(active.state, a::State::Active);
        let (rev, cell) = f.app.inspect_cell(&f.admin, &c.cell).unwrap();
        assert_eq!(cell.epoch, before.1.epoch);
        assert!(cell.blocks.is_empty());
        assert_eq!(
            cell.qualification.as_ref().unwrap().id,
            b.cells[0].qualification.id
        );
        assert_eq!(
            f.app
                .process_change(&f.admin, &c.cell, &c.id)
                .unwrap()
                .change
                .state,
            process_change::State::QualifiedActive
        );
        assert!(
            f.app
                .overview(&f.admin)
                .unwrap()
                .cells
                .iter()
                .all(|c| c.runs.is_empty())
        );
        f.configuration = cell.configuration.clone();
        for reg in &mut f.registrations {
            reg.epoch = cell.epoch;
            reg.scopes = cell.scope_epochs.clone();
        }
        report_ready(
            &mut f.app,
            &f.hosts[0],
            &f.configuration,
            &f.registrations[0],
        );
        let run = f
            .app
            .create_run(&f.operator, id().as_str(), create_command(&f, rev))
            .unwrap();
        let attempt = f
            .app
            .start_run(&f.operator, id().as_str(), start_command(&f, &run, rev, 1))
            .unwrap();
        assert_eq!(attempt.status, StartStatus::Arming);
    }
}
#[test]
fn qualification_disappearing_receipt_suspends_active_cohort_without_overwriting_fact() {
    let (mut f, r, _, p, source, j, v, d) = approved_setup();
    let b = issue(&mut f, &r, &j, &v, &d, &p, &source);
    let (t, observed) = confirm(&mut f, &b);
    activate(&mut f, &r, &b, &p, &source);
    let mut missing = observed.clone();
    missing.receipt = None;
    missing.receipt_matches_current_host = false;
    let recorded = f
        .app
        .record_qualification_observation(&f.hosts[0], &t.id, missing, f.clock.now())
        .unwrap();
    assert!(recorded.disputed && recorded.receipt.is_some());
    let view = f
        .app
        .qualification_batch(&f.admin, &b.origin, &b.id)
        .unwrap();
    assert_eq!(view.batch.state, a::State::Suspended);
    assert!(!view.current);
    assert!(
        f.app
            .inspect_cell(&f.admin, &b.origin)
            .unwrap()
            .1
            .qualification
            .is_none()
    );
}
#[test]
fn qualification_policy_replacement_revokes_active_grade_but_preserves_history() {
    let (mut f, r, _, p, source, j, v, d) = approved_setup();
    let b = issue(&mut f, &r, &j, &v, &d, &p, &source);
    confirm(&mut f, &b);
    activate(&mut f, &r, &b, &p, &source);
    f.app.configure_requalification(None).unwrap();
    let cell = f.app.inspect_cell(&f.admin, &b.origin).unwrap().1;
    assert!(cell.qualification.is_none() && !cell.blocks.is_empty());
    assert_eq!(
        f.app
            .qualification_batch(&f.admin, &b.origin, &b.id)
            .unwrap()
            .batch
            .state,
        a::State::Suspended
    );
}
#[test]
fn qualification_activation_rechecks_context_after_off_writer_verification() {
    let (mut f, r, _, p, source, j, v, d) = approved_setup();
    let b = issue(&mut f, &r, &j, &v, &d, &p, &source);
    confirm(&mut f, &b);
    let a::Preflight::Verify(t) = f
        .app
        .prepare_qualification_activation(&r, &id(), finalize(&b))
        .unwrap()
    else {
        panic!("ticket")
    };
    let proof = verify(*t, &p, &source);
    f.app.hold(&f.operator, id().as_str(), &b.origin).unwrap();
    assert!(f.app.commit_qualification_activation(proof).is_err());
    assert!(
        f.app
            .inspect_cell(&f.admin, &b.origin)
            .unwrap()
            .1
            .qualification
            .is_none()
    );
}

#[test]
fn qualification_partial_host_receipts_never_activate_the_cell() {
    let (mut f, r, c, p, source) = setup_sources_count(2);
    let input = begin_input(&mut f, &c, &p);
    let j = f.app.begin_requalification(&r, &id(), input).unwrap();
    ack(&mut f, &j);
    let prepared = prepared_report(&mut f, &j, &p, &id(), None);
    let v = f.app.commit_requalification_report(prepared).unwrap();
    let reviewer = add_identity(
        &mut f.app,
        &f.admin,
        "multi-host-reviewer",
        &[Role::Verifier],
    );
    let q::DecisionPreflight::Verify(t) = f
        .app
        .prepare_requalification_decision(&reviewer, &id(), decision(&v, &j))
        .unwrap()
    else {
        panic!("ticket")
    };
    let d = f
        .app
        .commit_requalification_decision(t.verify(&p.policy).unwrap())
        .unwrap();
    let b = issue(&mut f, &r, &j, &v, &d, &p, &source);
    confirm_host(&mut f, &b, 0);
    let view = f
        .app
        .qualification_batch(&f.admin, &b.origin, &b.id)
        .unwrap();
    assert!(view.mixed);
    assert!(
        f.app
            .prepare_qualification_activation(&r, &id(), finalize(&b))
            .is_err()
    );
    assert!(
        f.app
            .inspect_cell(&f.admin, &b.origin)
            .unwrap()
            .1
            .qualification
            .is_none()
    );
    confirm_host(&mut f, &b, 1);
    activate(&mut f, &r, &b, &p, &source);
    assert!(
        f.app
            .qualification_batch(&f.admin, &b.origin, &b.id)
            .unwrap()
            .current
    );
}
#[test]
fn qualification_never_clears_a_manual_hold_without_owned_provenance() {
    let (mut f, r, c, p, source) = setup_sources();
    f.app.hold(&f.operator, id().as_str(), &c.cell).unwrap();
    let input = begin_input(&mut f, &c, &p);
    let j = f.app.begin_requalification(&r, &id(), input).unwrap();
    ack(&mut f, &j);
    let prepared = prepared_report(&mut f, &j, &p, &id(), None);
    let v = f.app.commit_requalification_report(prepared).unwrap();
    let reviewer = add_identity(&mut f.app, &f.admin, "hold-reviewer", &[Role::Verifier]);
    let q::DecisionPreflight::Verify(t) = f
        .app
        .prepare_requalification_decision(&reviewer, &id(), decision(&v, &j))
        .unwrap()
    else {
        panic!("ticket")
    };
    let d = f
        .app
        .commit_requalification_decision(t.verify(&p.policy).unwrap())
        .unwrap();
    let mut input = issue_input(&mut f, &j, &v, &d);
    assert!(matches!(
        f.app.prepare_qualification_issue(&r, &id(), input.clone()),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    let cell = f.app.inspect_cell(&f.admin, &c.cell).unwrap().1;
    input.clear_blocks.insert(
        c.cell.clone(),
        cell.blocks
            .iter()
            .filter(|b| b.reason == BlockReason::ConfigurationChange)
            .map(|b| b.id.clone())
            .collect(),
    );
    let a::Preflight::Verify(t) = f.app.prepare_qualification_issue(&r, &id(), input).unwrap()
    else {
        panic!("ticket")
    };
    let b = f
        .app
        .commit_qualification_issue(verify(*t, &p, &source))
        .unwrap();
    confirm(&mut f, &b);
    activate(&mut f, &r, &b, &p, &source);
    let cell = f.app.inspect_cell(&f.admin, &c.cell).unwrap().1;
    assert!(cell.qualification.is_some());
    assert_eq!(cell.blocks.len(), 1);
    assert_eq!(cell.blocks[0].reason, BlockReason::OperatorHold);
}
#[test]
fn qualification_restart_retires_active_authority_and_preserves_completed_batch() {
    let (mut f, r, c, p, source, j, v, d) = approved_setup();
    let b = issue(&mut f, &r, &j, &v, &d, &p, &source);
    confirm(&mut f, &b);
    activate(&mut f, &r, &b, &p, &source);
    let installation = f.app.installation.id.clone();
    let repository = f.app.into_repository();
    let mut app = Engine::open(
        repository,
        f.clock.clone(),
        SimulationAuthority,
        installation,
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    let login = app
        .authenticated_session(&f.admin.principal, id(), expiry(99_000))
        .unwrap();
    let admin = Identity {
        session: login.id,
        ..f.admin
    };
    let current = app.inspect_cell(&admin, &c.cell).unwrap().1;
    assert!(current.qualification.is_none() && !current.blocks.is_empty());
    let view = app.qualification_batch(&admin, &c.cell, &b.id).unwrap();
    assert_eq!(view.batch.state, a::State::Suspended);
    assert!(view.batch.activated_at.is_some());
    assert!(!view.current);
    assert_eq!(
        app.process_change(&admin, &c.cell, &c.id)
            .unwrap()
            .change
            .state,
        process_change::State::AppliedUnqualified
    );
}
#[test]
fn qualification_legacy_policy_can_be_reviewed_but_cannot_issue_execution_purposes() {
    let (mut f, r, c, mut p, source) = setup_sources();
    p.policy.schema = name("rx.requalification-policy.v1");
    for p in &mut p.policy.profiles {
        p.purposes.clear();
    }
    assert!(
        !serde_json::to_value(&p.policy).unwrap()["profiles"][0]
            .as_object()
            .unwrap()
            .contains_key("purposes")
    );
    f.app
        .configure_requalification(Some(p.policy.clone()))
        .unwrap();
    let input = begin_input(&mut f, &c, &p);
    let j = f.app.begin_requalification(&r, &id(), input).unwrap();
    ack(&mut f, &j);
    let prepared = prepared_report(&mut f, &j, &p, &id(), None);
    let v = f.app.commit_requalification_report(prepared).unwrap();
    let reviewer = add_identity(&mut f.app, &f.admin, "legacy-reviewer", &[Role::Verifier]);
    let q::DecisionPreflight::Verify(t) = f
        .app
        .prepare_requalification_decision(&reviewer, &id(), decision(&v, &j))
        .unwrap()
    else {
        panic!("ticket")
    };
    let d = f
        .app
        .commit_requalification_decision(t.verify(&p.policy).unwrap())
        .unwrap();
    let input = issue_input(&mut f, &j, &v, &d);
    let a::Preflight::Verify(t) = f.app.prepare_qualification_issue(&r, &id(), input).unwrap()
    else {
        panic!("ticket")
    };
    let result = a::Prepared::verify(
        *t,
        &p.policy,
        source
            .store
            .verify_owned(&source.object, &source.policy)
            .unwrap(),
    );
    assert!(result.err().unwrap().contains("v2"));
    assert!(
        f.app
            .qualification_tasks(&f.hosts[0], None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn qualification_new_session_reauthorizes_same_pending_request_without_new_ids() {
    let (mut f, r, _, p, source, j, v, d) = approved_setup();
    let b = issue(&mut f, &r, &j, &v, &d, &p, &source);
    let t = f
        .app
        .qualification_tasks(&f.hosts[0], None)
        .unwrap()
        .remove(0);
    let o = inspect(&mut f, &b, &t);
    let t = f
        .app
        .bind_qualification_request(&f.hosts[0], &t.id, o.clone(), f.clock.now())
        .unwrap();
    f.app
        .enter_qualification_send(&f.hosts[0], &t.id, false)
        .unwrap();
    f.app
        .record_qualification_observation(&f.hosts[0], &t.id, o, f.clock.now())
        .unwrap();
    f.app.end_user_session(&r).unwrap();
    assert!(
        f.app
            .enter_qualification_send(&f.hosts[0], &t.id, true)
            .is_err()
    );
    let session = f
        .app
        .authenticated_terminal_user_session(
            &r.principal,
            id(),
            Counter(99_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap();
    let newer = Identity {
        session: session.id,
        ..r
    };
    let a::Preflight::Verify(ticket) = f
        .app
        .prepare_qualification_issue(&newer, &id(), b.issuance.clone())
        .unwrap()
    else {
        panic!("fresh reauthorization proof")
    };
    let refreshed = f
        .app
        .commit_qualification_issue(verify(*ticket, &p, &source))
        .unwrap();
    assert_eq!(refreshed.id, b.id);
    assert_eq!(refreshed.tasks, b.tasks);
    assert_eq!(
        refreshed.cells[0].qualification.id,
        b.cells[0].qualification.id
    );
    let a::Emission::Send { request } = f
        .app
        .enter_qualification_send(&f.hosts[0], &t.id, true)
        .unwrap()
    else {
        panic!("same request")
    };
    assert_eq!(request.digest().unwrap(), t.digest.unwrap());
}

#[test]
fn qualification_permits_only_the_reviewed_operating_purposes() {
    let (mut f, r, c, mut p, source) = setup_sources();
    p.policy.profiles[0].purposes = [name("PRODUCTION")].into_iter().collect();
    f.app
        .configure_requalification(Some(p.policy.clone()))
        .unwrap();
    let input = begin_input(&mut f, &c, &p);
    let j = f.app.begin_requalification(&r, &id(), input).unwrap();
    ack(&mut f, &j);
    let prepared = prepared_report(&mut f, &j, &p, &id(), None);
    let v = f.app.commit_requalification_report(prepared).unwrap();
    let reviewer = add_identity(&mut f.app, &f.admin, "purpose-reviewer", &[Role::Verifier]);
    let q::DecisionPreflight::Verify(t) = f
        .app
        .prepare_requalification_decision(&reviewer, &id(), decision(&v, &j))
        .unwrap()
    else {
        panic!("review")
    };
    let d = f
        .app
        .commit_requalification_decision(t.verify(&p.policy).unwrap())
        .unwrap();
    let b = issue(&mut f, &r, &j, &v, &d, &p, &source);
    confirm(&mut f, &b);
    activate(&mut f, &r, &b, &p, &source);
    let (rev, cell) = f.app.inspect_cell(&f.admin, &c.cell).unwrap();
    f.configuration = cell.configuration.clone();
    for reg in &mut f.registrations {
        reg.epoch = cell.epoch;
        reg.scopes = cell.scope_epochs.clone();
    }
    report_ready(
        &mut f.app,
        &f.hosts[0],
        &f.configuration,
        &f.registrations[0],
    );
    let run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, rev))
        .unwrap();
    let mut request = start_command(&f, &run, rev, 1);
    request.purpose = Purpose::Setup;
    request.budget_unit = rx_domain::budget::BudgetUnit::OperationCount;
    assert!(matches!(
        f.app.start_run(&f.operator, id().as_str(), request),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
}

#[path = "runtime_requalification_tests.rs"]
mod runtime_requalification_tests;
