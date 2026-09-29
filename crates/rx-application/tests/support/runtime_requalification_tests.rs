use super::*;

// Model the real init -> first runtime opening before any Host is registered.
fn bootstrap_applied() -> (
    Fixture,
    Identity,
    process_change::Change,
    qsupport::Fixture,
    review_support::Fixture,
) {
    bootstrap_applied_after(Stop::Crash, Executor::Local)
}
/// How the first runtime ends before the next one opens.
#[derive(Clone, Copy, PartialEq)]
enum Stop {
    /// The process just ends.
    Crash,
    /// The lifecycle stop request `rx-platformd` makes on SIGTERM.
    Graceful,
}
/// How the executor authenticates to the next runtime.
#[derive(Clone, Copy, PartialEq)]
enum Executor {
    /// A local session, as most fixtures use.
    Local,
    /// The same executor peer process reconnects (same boot and authentication binding).
    PeerContinues,
    /// A restarted executor peer process reconnects with a new boot.
    PeerRestarted,
}
fn bootstrap_applied_after(
    stop: Stop,
    executor: Executor,
) -> (
    Fixture,
    Identity,
    process_change::Change,
    qsupport::Fixture,
    review_support::Fixture,
) {
    let mut artifacts = None;
    let peer = executor != Executor::Local;
    let mut f = fixture_configured((1, false, false, false, None, peer, false), |cfg| {
        let (cfg, bytes) = qsupport::configure(cfg);
        artifacts = Some(bytes);
        cfg
    });
    let peer_boot = match f.app.inspect_service_peer(&f.executor) {
        Ok(ServicePeer::Executor(p)) => Some(p.peer_boot),
        _ => None,
    };
    if stop == Stop::Graceful {
        f.app.request_runtime_stop().unwrap();
        f.app.commit_runtime_process_stop().unwrap();
    }
    let installation = f.app.installation.id.clone();
    let repository = f.app.into_repository();
    f.app = Engine::open(
        repository,
        f.clock.clone(),
        SimulationAuthority,
        installation,
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    f.admin.session = f
        .app
        .authenticated_session(&f.admin.principal, id(), expiry(99_000))
        .unwrap()
        .id;
    f.executor.session = match executor {
        Executor::Local => {
            f.app
                .authenticated_session(&f.executor.principal, id(), expiry(99_000))
                .unwrap()
                .id
        }
        Executor::PeerContinues | Executor::PeerRestarted => {
            let boot = if executor == Executor::PeerContinues {
                peer_boot.clone().unwrap()
            } else {
                id()
            };
            f.app
                .open_executor_peer(&f.executor.principal, boot, Digest::from_bytes([71; 32]))
                .unwrap()
                .id
        }
    };
    if peer {
        f.app
            .negotiate_executor_cell(&f.executor, f.configuration.definition.sha256)
            .unwrap();
    }
    f.operator.session = f
        .app
        .authenticated_terminal_user_session(
            &f.operator.principal,
            id(),
            Counter(99_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap()
        .id;
    for h in &mut f.hosts {
        h.session = f
            .app
            .authenticated_session(&h.principal, id(), expiry(99_000))
            .unwrap()
            .id;
    }
    f.registrations = register_hosts(&mut f.app, &f.hosts, &f.configuration);
    report_ready(
        &mut f.app,
        &f.hosts[0],
        &f.configuration,
        &f.registrations[0],
    );
    let (source, job, release, c) = prepared_materials(&mut f);
    authorize_and_observe(&mut f, &release, &c);
    let prepared = apply_prepared(&mut f, &source, &job, &release, &c, &id());
    let c = f.app.commit_process_change_apply(prepared).unwrap();
    let current = f
        .app
        .inspect_cell(&f.admin, &c.cell)
        .unwrap()
        .1
        .configuration;
    let proof = qsupport::policy(&current, artifacts.unwrap());
    f.app
        .configure_requalification(Some(proof.policy.clone()))
        .unwrap();
    (f, release, c, proof, source)
}
fn adopting_input(f: &mut Fixture, c: &process_change::Change, p: &qsupport::Fixture) -> q::Begin {
    let mut input = begin_input(f, c, p);
    let view = f.app.runtime_restrictions(&f.admin, &c.cell).unwrap();
    assert_eq!(view.restrictions.len(), 1);
    input.runtime_restrictions = view
        .restrictions
        .into_iter()
        .map(|r| (r.block.id, r.origin_digest.unwrap()))
        .collect();
    input
}
#[test]
fn initial_runtime_restriction_is_signed_into_requalification_and_cleared_only_at_activation() {
    let (mut f, release, c, p, source) = bootstrap_applied();
    let input = adopting_input(&mut f, &c, &p);
    let original = input.runtime_restrictions.keys().next().unwrap().clone();
    let j = f.app.begin_requalification(&release, &id(), input).unwrap();
    assert_eq!(j.request.schema, name("rx.requalification-request.v2"));
    assert_eq!(j.request.runtime_restrictions.len(), 1);
    assert!(j.request.cells[0].blocks.contains(&original));
    assert!(
        f.app
            .inspect_cell(&f.admin, &c.cell)
            .unwrap()
            .1
            .blocks
            .iter()
            .any(|b| b.id == original)
    );
    let (mut wrong, signature, blobs) = p.report(&j);
    wrong.request.runtime_restrictions.clear();
    wrong.request.schema = name("rx.requalification-request.v1");
    wrong.request.impact_digest = None;
    assert!(q::Verified::check(&j, &p.policy, wrong, signature, blobs).is_err());
    ack(&mut f, &j);
    let prepared = prepared_report(&mut f, &j, &p, &id(), None);
    let v = f.app.commit_requalification_report(prepared).unwrap();
    let reviewer = add_identity(&mut f.app, &f.admin, "restart-reviewer", &[Role::Verifier]);
    let q::DecisionPreflight::Verify(t) = f
        .app
        .prepare_requalification_decision(&reviewer, &id(), decision(&v, &j))
        .unwrap()
    else {
        panic!()
    };
    let d = f
        .app
        .commit_requalification_decision(t.verify(&p.policy).unwrap())
        .unwrap();
    let batch = issue(&mut f, &release, &j, &v, &d, &p, &source);
    assert!(
        f.app
            .inspect_cell(&f.admin, &c.cell)
            .unwrap()
            .1
            .blocks
            .iter()
            .any(|b| b.id == original)
    );
    confirm(&mut f, &batch);
    activate(&mut f, &release, &batch, &p, &source);
    let after = f.app.inspect_cell(&f.admin, &c.cell).unwrap().1;
    assert!(!after.blocks.iter().any(|b| b.id == original));
    assert!(after.qualification.is_some());
    assert!(
        f.app
            .overview(&f.admin)
            .unwrap()
            .cells
            .iter()
            .all(|c| c.runs.is_empty())
    );
}
fn reasons(f: &mut Fixture, cell: &Name) -> Vec<BlockReason> {
    let (_, cell) = f.app.inspect_cell(&f.admin, cell).unwrap();
    cell.blocks.iter().map(|b| b.reason).collect()
}
/// Begin a requalification selecting `selected` and take it through report and independent
/// decision; returns the job, version and decision ready for issuance.
fn approved(
    f: &mut Fixture,
    release: &Identity,
    c: &process_change::Change,
    p: &qsupport::Fixture,
    selected: BTreeMap<Id, Digest>,
) -> (q::Job, q::Version, q::Decision) {
    let mut input = begin_input(f, c, p);
    input.runtime_restrictions = selected;
    let j = f.app.begin_requalification(release, &id(), input).unwrap();
    ack(f, &j);
    let prepared = prepared_report(f, &j, p, &id(), None);
    let v = f.app.commit_requalification_report(prepared).unwrap();
    let reviewer = add_identity(&mut f.app, &f.admin, "return-reviewer", &[Role::Verifier]);
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
    (j, v, d)
}
#[test]
fn restarted_cells_return_to_service_after_a_crash_a_graceful_stop_or_a_continuing_executor_peer() {
    for (stop, executor) in [
        (Stop::Crash, Executor::Local),
        (Stop::Graceful, Executor::Local),
        (Stop::Crash, Executor::PeerContinues),
        (Stop::Graceful, Executor::PeerContinues),
    ] {
        let (mut f, release, c, p, source) = bootstrap_applied_after(stop, executor);
        // A graceful stop leaves one AuthorityRevoked block with provenance; a continuing
        // executor peer adds none, because the new runtime already restricted its cells.
        let revoked = reasons(&mut f, &c.cell)
            .into_iter()
            .filter(|r| *r == BlockReason::AuthorityRevoked)
            .count();
        assert_eq!(revoked, usize::from(stop == Stop::Graceful));
        let view = f.app.runtime_restrictions(&f.admin, &c.cell).unwrap();
        assert_eq!(view.restrictions.len(), 1 + revoked);
        let selected: BTreeMap<_, _> = view
            .restrictions
            .iter()
            .map(|r| (r.block.id.clone(), r.origin_digest.unwrap()))
            .collect();
        let (j, v, d) = approved(&mut f, &release, &c, &p, selected);
        let batch = issue(&mut f, &release, &j, &v, &d, &p, &source);
        confirm(&mut f, &batch);
        activate(&mut f, &release, &batch, &p, &source);
        let cell = f.app.inspect_cell(&f.admin, &c.cell).unwrap().1;
        assert!(cell.blocks.is_empty(), "{:?}", reasons(&mut f, &c.cell));
        assert!(cell.qualification.is_some());
    }
}
#[test]
fn a_graceful_stop_restriction_is_released_only_when_a_review_selects_it() {
    let (mut f, release, c, p, _) = bootstrap_applied_after(Stop::Graceful, Executor::Local);
    let view = f.app.runtime_restrictions(&f.admin, &c.cell).unwrap();
    let stop = view
        .restrictions
        .iter()
        .find(|r| r.block.reason == BlockReason::AuthorityRevoked)
        .unwrap();
    let origin = stop.origin.as_ref().unwrap();
    // It names the stopped runtime, which is not the current one.
    assert!(origin.stop.is_some());
    assert_eq!(origin.previous_runtime_boot, origin.runtime_boot);
    assert_ne!(origin.runtime_boot, f.app.installation.runtime_boot);
    let restart_only: BTreeMap<_, _> = view
        .restrictions
        .iter()
        .filter(|r| r.block.reason == BlockReason::RuntimeRestart)
        .map(|r| (r.block.id.clone(), r.origin_digest.unwrap()))
        .collect();
    let (j, v, d) = approved(&mut f, &release, &c, &p, restart_only);
    let input = issue_input(&mut f, &j, &v, &d);
    assert!(matches!(
        f.app.prepare_qualification_issue(&release, &id(), input),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
}
#[test]
fn a_restarted_executor_peer_still_revokes_its_cells_without_provenance() {
    let (mut f, _, c, _, _) = bootstrap_applied_after(Stop::Crash, Executor::PeerRestarted);
    assert!(reasons(&mut f, &c.cell).contains(&BlockReason::AuthorityRevoked));
    // Not a runtime restriction: nothing selects it, so it has no release path here.
    let view = f.app.runtime_restrictions(&f.admin, &c.cell).unwrap();
    assert!(
        view.restrictions
            .iter()
            .all(|r| r.block.reason == BlockReason::RuntimeRestart)
    );
}
#[test]
fn restriction_binding_is_atomic_and_missing_or_wrong_origins_never_generate_authority() {
    for failure in [1, 2] {
        let (mut f, release, c, p, _) = bootstrap_applied();
        let input = adopting_input(&mut f, &c, &p);
        let key = id();
        let mut wrong = input.clone();
        wrong
            .runtime_restrictions
            .insert(id(), Digest::from_bytes([1; 32]));
        assert!(f.app.begin_requalification(&release, &id(), wrong).is_err());
        let mut wrong = input.clone();
        *wrong.runtime_restrictions.values_mut().next().unwrap() = Digest::from_bytes([1; 32]);
        assert!(f.app.begin_requalification(&release, &id(), wrong).is_err());
        let before = f.app.inspect_cell(&f.admin, &c.cell).unwrap();
        f.failure.store(failure, Ordering::SeqCst);
        assert!(
            f.app
                .begin_requalification(&release, &key, input.clone())
                .is_err()
        );
        if failure == 1 {
            assert_eq!(f.app.inspect_cell(&f.admin, &c.cell).unwrap().0, before.0);
        }
        let j = f
            .app
            .begin_requalification(&release, &key, input.clone())
            .unwrap();
        let same = f.app.begin_requalification(&release, &key, input).unwrap();
        assert_eq!(j.request.digest().unwrap(), same.request.digest().unwrap());
        assert!(
            f.app
                .inspect_cell(&f.admin, &c.cell)
                .unwrap()
                .1
                .qualification
                .is_none()
        );
    }
}
#[test]
fn review_without_explicit_runtime_restriction_cannot_issue_a_clear_for_it() {
    let (mut f, release, c, p, _) = bootstrap_applied();
    let input = begin_input(&mut f, &c, &p);
    let j = f.app.begin_requalification(&release, &id(), input).unwrap();
    assert!(j.request.runtime_restrictions.is_empty());
    ack(&mut f, &j);
    let prepared = prepared_report(&mut f, &j, &p, &id(), None);
    let v = f.app.commit_requalification_report(prepared).unwrap();
    let reviewer = add_identity(
        &mut f.app,
        &f.admin,
        "non-adopting-reviewer",
        &[Role::Verifier],
    );
    let q::DecisionPreflight::Verify(t) = f
        .app
        .prepare_requalification_decision(&reviewer, &id(), decision(&v, &j))
        .unwrap()
    else {
        panic!()
    };
    let d = f
        .app
        .commit_requalification_decision(t.verify(&p.policy).unwrap())
        .unwrap();
    let input = issue_input(&mut f, &j, &v, &d);
    assert!(matches!(
        f.app.prepare_qualification_issue(&release, &id(), input),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
}

#[test]
fn activated_root_after_restart_keeps_exact_origin_owner_and_can_begin_explicit_revalidation() {
    let (mut f, release, c, p, source, j, v, d) = approved_setup();
    let batch = issue(&mut f, &release, &j, &v, &d, &p, &source);
    confirm(&mut f, &batch);
    activate(&mut f, &release, &batch, &p, &source);
    let installation = f.app.installation.id.clone();
    let repository = f.app.into_repository();
    f.app = Engine::open(
        repository,
        f.clock.clone(),
        SimulationAuthority,
        installation,
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    f.admin.session = f
        .app
        .authenticated_session(&f.admin.principal, id(), expiry(99_000))
        .unwrap()
        .id;
    let mut release = release;
    release.session = f
        .app
        .authenticated_terminal_user_session(
            &release.principal,
            id(),
            Counter(99_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap()
        .id;
    f.app
        .configure_requalification(Some(p.policy.clone()))
        .unwrap();
    let current = f
        .app
        .process_change(&f.admin, &c.cell, &c.id)
        .unwrap()
        .change;
    assert_eq!(current.state, process_change::State::AppliedUnqualified);
    let input = adopting_input(&mut f, &current, &p);
    let next = f.app.begin_requalification(&release, &id(), input).unwrap();
    assert_eq!(next.request.change, c.id);
    assert_eq!(next.request.runtime_restrictions.len(), 1);
    assert!(next.request.impact_digest.is_some());
    assert!(
        f.app
            .inspect_cell(&f.admin, &c.cell)
            .unwrap()
            .1
            .qualification
            .is_none()
    );
    assert!(
        !f.app
            .requalification(&f.admin, &c.cell, &next.request.id)
            .unwrap()
            .fences_confirmed
    );
}
#[test]
fn a_later_review_without_selection_cannot_reuse_an_earlier_runtime_adoption() {
    let (mut f, release, c, p, _) = bootstrap_applied();
    let selected = adopting_input(&mut f, &c, &p);
    let _first = f
        .app
        .begin_requalification(&release, &id(), selected)
        .unwrap();
    let input = begin_input(&mut f, &c, &p);
    let j = f.app.begin_requalification(&release, &id(), input).unwrap();
    assert!(j.request.runtime_restrictions.is_empty());
    ack(&mut f, &j);
    let prepared = prepared_report(&mut f, &j, &p, &id(), None);
    let v = f.app.commit_requalification_report(prepared).unwrap();
    let reviewer = add_identity(&mut f.app, &f.admin, "later-reviewer", &[Role::Verifier]);
    let q::DecisionPreflight::Verify(t) = f
        .app
        .prepare_requalification_decision(&reviewer, &id(), decision(&v, &j))
        .unwrap()
    else {
        panic!()
    };
    let d = f
        .app
        .commit_requalification_decision(t.verify(&p.policy).unwrap())
        .unwrap();
    let input = issue_input(&mut f, &j, &v, &d);
    assert!(matches!(
        f.app.prepare_qualification_issue(&release, &id(), input),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
}
#[test]
fn a_new_shared_cell_invalidates_pending_and_active_requalification_without_changing_the_original_cell()
 {
    for active in [false, true] {
        let (mut f, release, c, p, source, j, v, d) = approved_setup();
        let batch = if active {
            let b = issue(&mut f, &release, &j, &v, &d, &p, &source);
            confirm(&mut f, &b);
            activate(&mut f, &release, &b, &p, &source);
            Some(b)
        } else {
            None
        };
        let before = f.app.inspect_cell(&f.admin, &c.cell).unwrap();
        let mut other = before.1.configuration.clone();
        other.id = name("cell/b");
        other.process = None;
        other.hosts = vec![name("host/b")];
        for s in &mut other.steps {
            s.host = name("host/b");
        }
        for spec in &mut other.fact_specs {
            spec.host = name("host/b");
        }
        f.app.install_cell(&f.admin, other).unwrap();
        assert_eq!(f.app.inspect_cell(&f.admin, &c.cell).unwrap().0, before.0);
        if let Some(batch) = batch {
            assert!(
                !f.app
                    .qualification_batch(&f.admin, &c.cell, &batch.id)
                    .unwrap()
                    .current
            );
        } else {
            let input = issue_input(&mut f, &j, &v, &d);
            assert!(
                f.app
                    .prepare_qualification_issue(&release, &id(), input)
                    .is_err()
            );
        }
    }
}
