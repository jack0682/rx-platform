use super::*;
use rx_application::requalification as q;
#[path = "requalification_fixture.rs"]
mod qsupport;
fn setup() -> (Fixture, Identity, process_change::Change, qsupport::Fixture) {
    let (f, r, c, q, _p) = setup_sources();
    (f, r, c, q)
}
fn setup_sources() -> (
    Fixture,
    Identity,
    process_change::Change,
    qsupport::Fixture,
    review_support::Fixture,
) {
    setup_sources_count(1)
}
fn setup_sources_count(
    hosts: usize,
) -> (
    Fixture,
    Identity,
    process_change::Change,
    qsupport::Fixture,
    review_support::Fixture,
) {
    let mut source = None;
    let mut f = fixture_configured((hosts, false, false, false, None, false, true), |cfg| {
        let (cfg, bytes) = qsupport::configure(cfg);
        source = Some(bytes);
        cfg
    });
    let (p, job, release, c) = prepared_materials(&mut f);
    authorize_and_observe(&mut f, &release, &c);
    let t = apply_prepared(&mut f, &p, &job, &release, &c, &id());
    let applied = f.app.commit_process_change_apply(t).unwrap();
    let cfg = f
        .app
        .inspect_cell(&f.admin, &c.cell)
        .unwrap()
        .1
        .configuration;
    let proof = qsupport::policy(&cfg, source.unwrap());
    f.app
        .configure_requalification(Some(proof.policy.clone()))
        .unwrap();
    (f, release, applied, proof, p)
}
fn begin_input(f: &mut Fixture, c: &process_change::Change, p: &qsupport::Fixture) -> q::Begin {
    q::Begin {
        runtime_restrictions: BTreeMap::new(),
        id: id(),
        cell: c.cell.clone(),
        change: c.id.clone(),
        expected_change: c.revision,
        expected_cells: c
            .impact
            .cells
            .iter()
            .map(|c| (c.id.clone(), f.app.inspect_cell(&f.admin, &c.id).unwrap().0))
            .collect(),
        policy_digest: p.policy.digest().unwrap(),
    }
}
fn ack(f: &mut Fixture, j: &q::Job) {
    for (i, v) in j.request.fences.iter().enumerate() {
        let who = f.hosts.iter().find(|h| h.principal == v.host).unwrap();
        let registered = f.registrations.iter().find(|h| h.id == v.host).unwrap();
        f.app.plan_delivery(who, &v.message).unwrap();
        f.app
            .finish_fence_delivery(
                who,
                &v.message,
                FenceAcknowledgment {
                    cell: v.cell.clone(),
                    invalidation: v.message.clone(),
                    epoch: v.epoch,
                    scopes: v.scopes.clone(),
                    host_boot: registered.boot_id.clone(),
                    journal: registered.delivery_journal.clone(),
                    sequence: Counter(500 + i as u64),
                },
            )
            .unwrap();
    }
}
fn prepared_report(
    f: &mut Fixture,
    j: &q::Job,
    p: &qsupport::Fixture,
    key: &Id,
    expected: Option<Counter>,
) -> q::Prepared {
    let (r, s, b) = p.report(j);
    let input = q::Submit {
        review: j.request.id.clone(),
        cell: j.request.origin.clone(),
        expected,
        directory: rx_package::PackagePath::new("qualification").unwrap(),
        report_digest: r.digest().unwrap(),
    };
    let q::Preflight::Verify(t) = f
        .app
        .prepare_requalification_report(&f.admin, key, input)
        .unwrap()
    else {
        panic!("report ticket")
    };
    q::Prepared::new(*t, q::Verified::check(j, &p.policy, r, s, b).unwrap()).unwrap()
}
fn decision(v: &q::Version, j: &q::Job) -> q::Decide {
    q::Decide {
        review: j.request.id.clone(),
        cell: j.request.origin.clone(),
        report_revision: v.revision,
        report_digest: v.digest,
        expected: None,
        choice: q::Choice::Approve,
        note: "Independently reviewed all exact-scope evidence, no activation".into(),
    }
}
#[test]
fn requalification_begin_is_atomic_and_same_key_preserves_fences() {
    for mode in [1, 2] {
        let (mut f, release, c, p) = setup();
        let input = begin_input(&mut f, &c, &p);
        let key = id();
        let before = f.app.inspect_cell(&f.admin, &c.cell).unwrap();
        f.failure.store(mode, Ordering::SeqCst);
        assert!(
            f.app
                .begin_requalification(&release, &key, input.clone())
                .is_err()
        );
        if mode == 1 {
            assert_eq!(f.app.inspect_cell(&f.admin, &c.cell).unwrap().0, before.0);
        }
        let j = f
            .app
            .begin_requalification(&release, &key, input.clone())
            .unwrap();
        let again = f.app.begin_requalification(&release, &key, input).unwrap();
        assert_eq!(j.request.digest().unwrap(), again.request.digest().unwrap());
        assert_eq!(j.request.cells[0].epoch.0, before.1.epoch.0 + 1);
        assert!(
            !f.app
                .requalification(&f.admin, &c.cell, &j.request.id)
                .unwrap()
                .fences_confirmed
        );
    }
}
#[test]
fn requalification_rejects_missing_forged_or_wrong_environment_evidence() {
    let (mut f, release, c, p) = setup();
    let input = begin_input(&mut f, &c, &p);
    let j = f.app.begin_requalification(&release, &id(), input).unwrap();
    for mode in 0..6 {
        let (mut r, mut s, mut b) = p.report(&j);
        match mode {
            0 => {
                r.checks.pop();
                s = qsupport::sign(&r);
            }
            1 => {
                r.checks[0].evidence.clear();
                s = qsupport::sign(&r);
            }
            2 => {
                b.values_mut().next().unwrap().push(1);
            }
            3 => {
                s.signature = "00".repeat(64);
            }
            4 => {
                r.validator = Digest::from_bytes([91; 32]);
                s = qsupport::sign(&r);
            }
            _ => {
                r.request.cells[0].profile.environment = Environment::Physical;
                s = qsupport::sign(&r);
            }
        }
        assert!(q::Verified::check(&j, &p.policy, r, s, b).is_err());
    }
}
#[test]
fn requalification_report_commit_loss_recovers_original_bytes_and_never_qualifies() {
    for mode in [1, 2] {
        let (mut f, release, c, p) = setup();
        let input = begin_input(&mut f, &c, &p);
        let j = f.app.begin_requalification(&release, &id(), input).unwrap();
        let key = id();
        let prepared = prepared_report(&mut f, &j, &p, &key, None);
        f.failure.store(mode, Ordering::SeqCst);
        assert!(f.app.commit_requalification_report(prepared).is_err());
        let (r, s, b) = p.report(&j);
        let input = q::Submit {
            review: j.request.id.clone(),
            cell: c.cell.clone(),
            expected: None,
            directory: rx_package::PackagePath::new("qualification").unwrap(),
            report_digest: r.digest().unwrap(),
        };
        let v = match f
            .app
            .prepare_requalification_report(&f.admin, &key, input)
            .unwrap()
        {
            q::Preflight::Recorded(v) => *v,
            q::Preflight::Verify(t) => f
                .app
                .commit_requalification_report(
                    q::Prepared::new(
                        *t,
                        q::Verified::check(&j, &p.policy, r, s, b.clone()).unwrap(),
                    )
                    .unwrap(),
                )
                .unwrap(),
        };
        let a = &v.report.checks[0].evidence[0];
        assert_eq!(
            f.app
                .requalification_artifact(&f.admin, &c.cell, &j.request.id, a)
                .unwrap(),
            b[&a.sha256]
        );
        let cell = f.app.inspect_cell(&f.admin, &c.cell).unwrap().1;
        assert!(cell.qualification.is_none() && !cell.blocks.is_empty());
        assert!(
            !f.app
                .requalification(&f.admin, &c.cell, &j.request.id)
                .unwrap()
                .activation_authorized
        );
    }
}
#[test]
fn requalification_approval_requires_fences_and_independent_reviewer_and_fresh_context() {
    let (mut f, release, c, p) = setup();
    let input = begin_input(&mut f, &c, &p);
    let j = f.app.begin_requalification(&release, &id(), input).unwrap();
    let report = prepared_report(&mut f, &j, &p, &id(), None);
    let v = f.app.commit_requalification_report(report).unwrap();
    let reviewer = add_identity(
        &mut f.app,
        &f.admin,
        "qualification-reviewer",
        &[Role::Verifier],
    );
    assert!(
        f.app
            .prepare_requalification_decision(&reviewer, &id(), decision(&v, &j))
            .is_err()
    );
    ack(&mut f, &j);
    assert!(
        f.app
            .prepare_requalification_decision(&f.admin, &id(), decision(&v, &j))
            .is_err()
    );
    let q::DecisionPreflight::Verify(t) = f
        .app
        .prepare_requalification_decision(&reviewer, &id(), decision(&v, &j))
        .unwrap()
    else {
        panic!("verify")
    };
    let verified = t.verify(&p.policy).unwrap();
    f.app.hold(&f.operator, id().as_str(), &c.cell).unwrap();
    assert!(f.app.commit_requalification_decision(verified).is_err());
    assert!(
        !f.app
            .requalification(&f.admin, &c.cell, &j.request.id)
            .unwrap()
            .approval_current
    );
}
#[test]
fn requalification_approval_is_idempotent_and_new_report_invalidates_its_scope() {
    let (mut f, release, c, p) = setup();
    let input = begin_input(&mut f, &c, &p);
    let j = f.app.begin_requalification(&release, &id(), input).unwrap();
    ack(&mut f, &j);
    let prepared = prepared_report(&mut f, &j, &p, &id(), None);
    let v = f.app.commit_requalification_report(prepared).unwrap();
    let reviewer = add_identity(
        &mut f.app,
        &f.admin,
        "qualification-reviewer",
        &[Role::Verifier],
    );
    let key = id();
    let q::DecisionPreflight::Verify(t) = f
        .app
        .prepare_requalification_decision(&reviewer, &key, decision(&v, &j))
        .unwrap()
    else {
        panic!("ticket")
    };
    let t = t.verify(&p.policy).unwrap();
    f.failure.store(2, Ordering::SeqCst);
    assert!(f.app.commit_requalification_decision(t).is_err());
    let q::DecisionPreflight::Recorded(d) = f
        .app
        .prepare_requalification_decision(&reviewer, &key, decision(&v, &j))
        .unwrap()
    else {
        panic!("cached")
    };
    assert_eq!(d.scope.as_str(), "REQUALIFICATION_EVIDENCE_REVIEW");
    let view = f
        .app
        .requalification(&f.admin, &c.cell, &j.request.id)
        .unwrap();
    assert!(view.approval_current && !view.activation_authorized);
    let prepared = prepared_report(&mut f, &j, &p, &id(), Some(v.revision));
    f.app.commit_requalification_report(prepared).unwrap();
    assert!(
        !f.app
            .requalification(&f.admin, &c.cell, &j.request.id)
            .unwrap()
            .approval_current
    );
}
#[test]
fn requalification_not_run_is_preserved_and_cannot_be_approved() {
    let (mut f, release, c, p) = setup();
    let input = begin_input(&mut f, &c, &p);
    let j = f.app.begin_requalification(&release, &id(), input).unwrap();
    ack(&mut f, &j);
    let (mut r, _, mut b) = p.report(&j);
    r.checks[0].verdict = q::Verdict::NotRun;
    r.checks[0].evidence.clear();
    let refs: BTreeSet<_> = r.references().iter().map(|r| r.sha256).collect();
    b.retain(|h, _| refs.contains(h));
    let input = q::Submit {
        review: j.request.id.clone(),
        cell: c.cell.clone(),
        expected: None,
        directory: rx_package::PackagePath::new("qualification").unwrap(),
        report_digest: r.digest().unwrap(),
    };
    let q::Preflight::Verify(t) = f
        .app
        .prepare_requalification_report(&f.admin, &id(), input)
        .unwrap()
    else {
        panic!("ticket")
    };
    let s = qsupport::sign(&r);
    let v = f
        .app
        .commit_requalification_report(
            q::Prepared::new(*t, q::Verified::check(&j, &p.policy, r, s, b).unwrap()).unwrap(),
        )
        .unwrap();
    assert!(!v.ready_for_review);
    let reviewer = add_identity(
        &mut f.app,
        &f.admin,
        "qualification-reviewer",
        &[Role::Verifier],
    );
    assert!(
        f.app
            .prepare_requalification_decision(&reviewer, &id(), decision(&v, &j))
            .is_err()
    );
}

#[test]
fn requalification_large_evidence_is_chunked_atomically_and_restored_exactly() {
    let (mut f, release, c, p) = setup();
    let input = begin_input(&mut f, &c, &p);
    let j = f.app.begin_requalification(&release, &id(), input).unwrap();
    let (mut r, _, mut b) = p.report(&j);
    let large = vec![37u8; 2 * 1024 * 1024];
    let h = rx_package::content_digest(&large);
    let reference = ArtifactRef {
        sha256: h,
        schema_id: r.checks[0].evidence[0].schema_id.clone(),
        size_bytes: Counter(large.len() as u64),
    };
    r.checks[0].evidence = vec![reference.clone()];
    b.insert(h, large.clone());
    let refs: BTreeSet<_> = r.references().iter().map(|a| a.sha256).collect();
    b.retain(|h, _| refs.contains(h));
    let input = q::Submit {
        review: j.request.id.clone(),
        cell: c.cell.clone(),
        expected: None,
        directory: rx_package::PackagePath::new("qualification").unwrap(),
        report_digest: r.digest().unwrap(),
    };
    let key = id();
    let q::Preflight::Verify(t) = f
        .app
        .prepare_requalification_report(&f.admin, &key, input.clone())
        .unwrap()
    else {
        panic!("ticket")
    };
    let s = qsupport::sign(&r);
    let proof = q::Prepared::new(
        *t,
        q::Verified::check(&j, &p.policy, r.clone(), s.clone(), b.clone()).unwrap(),
    )
    .unwrap();
    f.failure.store(1, Ordering::SeqCst);
    assert!(f.app.commit_requalification_report(proof).is_err());
    assert!(
        f.app
            .requalification_artifact(&f.admin, &c.cell, &j.request.id, &reference)
            .is_err()
    );
    let q::Preflight::Verify(t) = f
        .app
        .prepare_requalification_report(&f.admin, &key, input)
        .unwrap()
    else {
        panic!("retry")
    };
    let proof = q::Prepared::new(*t, q::Verified::check(&j, &p.policy, r, s, b).unwrap()).unwrap();
    f.app.commit_requalification_report(proof).unwrap();
    assert_eq!(
        f.app
            .requalification_artifact(&f.admin, &c.cell, &j.request.id, &reference)
            .unwrap(),
        large
    );
}

#[path = "qualification_activation_tests.rs"]
mod qualification_activation_tests;
