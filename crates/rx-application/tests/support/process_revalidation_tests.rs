use super::*;
use process_change::{Create, Mode, Preflight, Prepared, ReviewRef, State};

// Install the exact signed compiled configuration as bootstrap, without a Change root.
fn compiled_bootstrap() -> (Fixture, review_support::Fixture) {
    let mut package = None;
    let f = fixture_configured((1, false, false, false, None, false, true), |mut cfg| {
        let first = review_support::fixture(&cfg, Digest::from_bytes([71; 32]));
        cfg.steps[0].id = first.resolved.root.id.clone();
        let source = review_support::fixture(&cfg, Digest::from_bytes([71; 32]));
        cfg.recipe = ArtifactRef {
            sha256: rx_process_contract::frontier::resolved_digest(&source.resolved).unwrap(),
            schema_id: source.resolved.schema.clone(),
            size_bytes: Counter(canonical::bytes(&source.resolved).unwrap().len() as u64),
        };
        cfg.process = Some(Box::new(source.resolved.clone()));
        package = Some(source);
        cfg
    });
    (f, package.unwrap())
}
fn input(
    job: &process_review::Job,
    v: &process_review::Version,
    d: &process_review::Decision,
    mode: Mode,
) -> Create {
    Create {
        mode,
        id: id(),
        cell: job.request.cell.clone(),
        review: ReviewRef {
            id: job.request.id.clone(),
            revision: v.revision,
            review_digest: v.review_digest,
            decision_revision: d.revision,
        },
        reason: "Establish the reviewed current configuration root without replacing it".into(),
    }
}
fn verified(
    f: &mut Fixture,
    source: &review_support::Fixture,
    request: Create,
    key: &Id,
) -> Result<Prepared, String> {
    let Preflight::Verify(ticket) = f
        .app
        .prepare_process_change(&f.admin, key, request)
        .map_err(|e| e.to_string())?
    else {
        return Err("expected a new verification ticket".into());
    };
    let (report, signature, bytes) = source.report(ticket.job());
    let checked = process_review::Validated::check(
        ticket.job(),
        source
            .store
            .verify_owned(&source.object, &source.policy)
            .unwrap(),
        &source.authority,
        report,
        signature,
        Some(&bytes),
    )?;
    Prepared::new(*ticket, checked)
}
fn old_plan_digest(c: &process_change::Change) -> Digest {
    canonical::digest(
        "RX-PROCESS-CHANGE-PLAN-v1",
        &(
            &c.id,
            &c.cell,
            &c.review,
            &c.before,
            &c.after,
            &c.step_origins,
            &c.impact,
            &c.reason,
            &c.proposed_by,
            c.builder_digest,
        ),
    )
    .unwrap()
}

#[test]
fn replace_default_preserves_legacy_request_change_shape_and_plan_digest() {
    let mut f = fixture(1, false);
    let source = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let (job, v, d, _) = approved_change_review(&mut f, &source);
    let request = input(&job, &v, &d, Mode::Replace);
    let bytes = canonical::bytes(&request).unwrap();
    assert!(
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .unwrap()
            .get("mode")
            .is_none()
    );
    assert_eq!(
        canonical::decode_json::<Create>(&bytes).unwrap().mode,
        Mode::Replace
    );
    let p = verified(&mut f, &source, request, &id()).unwrap();
    let c = f.app.commit_process_change(p).unwrap();
    assert_eq!(c.plan_digest, old_plan_digest(&c));
    let bytes = canonical::bytes(&c).unwrap();
    assert!(
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .unwrap()
            .get("mode")
            .is_none()
    );
    assert_eq!(
        canonical::decode_json::<process_change::Change>(&bytes)
            .unwrap()
            .mode,
        Mode::Replace
    );
}

#[test]
fn current_revalidation_requires_explicit_mode_exact_target_and_existing_compiled_process() {
    let (mut f, source) = compiled_bootstrap();
    let (job, v, d, _) = approved_change_review(&mut f, &source);
    assert!(verified(&mut f, &source, input(&job, &v, &d, Mode::Replace), &id()).is_err());
    let p = verified(
        &mut f,
        &source,
        input(&job, &v, &d, Mode::RevalidateCurrent),
        &id(),
    )
    .unwrap();
    let c = f.app.commit_process_change(p).unwrap();
    assert_eq!(c.mode, Mode::RevalidateCurrent);
    assert_eq!(c.before, c.after);
    assert_ne!(c.plan_digest, old_plan_digest(&c));
    assert_eq!(
        c.plan_digest,
        canonical::digest(
            "RX-PROCESS-CHANGE-MODE-v1",
            &(old_plan_digest(&c), Mode::RevalidateCurrent)
        )
        .unwrap()
    );
    assert_eq!(
        serde_json::to_value(&c).unwrap()["mode"],
        "REVALIDATE_CURRENT"
    );

    let mut f = fixture_with_process(1, false, false, true, Some(TestProcess::Branch));
    let source = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let (job, v, d, _) = approved_change_review(&mut f, &source);
    assert!(f.configuration.process.is_some());
    assert!(
        verified(
            &mut f,
            &source,
            input(&job, &v, &d, Mode::RevalidateCurrent),
            &id()
        )
        .is_err()
    );
    assert!(verified(&mut f, &source, input(&job, &v, &d, Mode::Replace), &id()).is_ok());

    let mut f = fixture(1, false);
    let source = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let (job, v, d, _) = approved_change_review(&mut f, &source);
    assert!(
        f.app
            .prepare_process_change(
                &f.admin,
                &id(),
                input(&job, &v, &d, Mode::RevalidateCurrent)
            )
            .is_err()
    );
}

#[test]
fn current_revalidation_keeps_review_fence_host_ack_and_unqualified_apply_gates_and_reuses_applied_root()
 {
    let (mut f, source) = compiled_bootstrap();
    let (job, v, d, reviewer) = approved_change_review(&mut f, &source);
    let release = release_identity(&mut f);
    let request = input(&job, &v, &d, Mode::RevalidateCurrent);
    let key = id();
    let before = f.app.inspect_cell(&f.admin, &request.cell).unwrap();
    let p = verified(&mut f, &source, request.clone(), &key).unwrap();
    let c = f.app.commit_process_change(p).unwrap();
    let delayed = verified(
        &mut f,
        &source,
        input(&job, &v, &d, Mode::RevalidateCurrent),
        &id(),
    )
    .unwrap();
    let p = verified(
        &mut f,
        &source,
        input(&job, &v, &d, Mode::RevalidateCurrent),
        &id(),
    )
    .unwrap();
    let pending = f.app.commit_process_change(p).unwrap();
    assert!(
        f.app
            .prepare_process_change_stage(&release, &id(), change_target(&c))
            .is_err()
    );
    assert!(
        f.app
            .review_process_change_impact(
                &f.admin,
                &id(),
                process_change::ReviewImpact {
                    target: change_target(&c),
                    note: "Self approval must fail".into(),
                }
            )
            .is_err()
    );
    let mut different_mode = request.clone();
    different_mode.mode = Mode::Replace;
    assert!(matches!(
        f.app.prepare_process_change(&f.admin, &key, different_mode),
        Err(StoreError::KeyConflict)
    ));
    let staged = stage_change(&mut f, &source, &job, &c, &reviewer, &release);
    assert_eq!(f.app.inspect_cell(&f.admin, &c.cell).unwrap().0, before.0);
    assert!(f.app.pending_deliveries(128).unwrap().is_empty());
    let c = f
        .app
        .begin_process_change_preparation(
            &release,
            &id(),
            process_change::BeginPreparation {
                target: change_target(&staged),
                refresh: false,
            },
        )
        .unwrap();
    assert!(
        f.app
            .authorize_host_configuration(&release, &id(), change_target(&c))
            .is_err()
    );
    assert!(
        f.app
            .prepare_process_change_apply(&release, &id(), change_target(&c))
            .is_err()
    );
    fences(&mut f, &c);
    assert!(
        f.app
            .prepare_process_change_apply(&release, &id(), change_target(&c))
            .is_err()
    );
    f.app
        .authorize_host_configuration(&release, &id(), change_target(&c))
        .unwrap();
    let t = bind(&mut f, 0);
    let host_request = t.request.as_ref().unwrap();
    assert!(
        host_request
            .cells
            .iter()
            .all(|c| c.before_configuration == c.after_configuration)
    );
    f.app
        .enter_host_configuration_send(&f.hosts[0], &t.id, false)
        .unwrap();
    f.app
        .record_host_configuration_read(&f.hosts[0], &t.id, applied(&t), f.clock.now())
        .unwrap();
    let p = apply_prepared(&mut f, &source, &job, &release, &c, &id());
    let done = f.app.commit_process_change_apply(p).unwrap();
    assert_eq!(done.state, State::AppliedUnqualified);
    assert_eq!(done.before, done.after);
    assert!(
        done.application
            .as_ref()
            .unwrap()
            .cells
            .iter()
            .all(|c| c.before == c.after)
    );
    let current = f.app.inspect_cell(&f.admin, &c.cell).unwrap().1;
    assert_eq!(
        canonical::bytes(&current.configuration).unwrap(),
        canonical::bytes(&before.1.configuration).unwrap()
    );
    assert!(current.qualification.is_none());
    assert!(
        current
            .blocks
            .iter()
            .any(|b| b.reason == BlockReason::ConfigurationChange && b.latched)
    );
    assert!(
        before
            .1
            .blocks
            .iter()
            .all(|b| current.blocks.iter().any(|retained| retained.id == b.id))
    );
    let view = f
        .app
        .process_change(&f.admin, &done.cell, &done.id)
        .unwrap();
    assert!(view.applied && !view.activation_authorized);
    assert!(
        view.blockers
            .iter()
            .any(|b| matches!(b, process_change::Blocker::RequalificationRequired))
    );
    assert!(matches!(
        f.app.commit_process_change(delayed),
        Err(StoreError::Rejected(Rejection::Busy))
    ));
    assert!(matches!(
        f.app.prepare_process_change(
            &f.admin,
            &id(),
            input(&job, &v, &d, Mode::RevalidateCurrent)
        ),
        Err(StoreError::Rejected(Rejection::Busy))
    ));
    assert!(matches!(
        f.app.review_process_change_impact(
            &reviewer,
            &id(),
            process_change::ReviewImpact {
                target: change_target(&pending),
                note: "Existing root must be reused".into(),
            }
        ),
        Err(StoreError::Rejected(Rejection::Busy))
    ));
    let Preflight::Recorded(original) = f
        .app
        .prepare_process_change(&f.admin, &key, request)
        .unwrap()
    else {
        panic!("same request recovery")
    };
    assert_eq!(original.id, done.id);
    assert_eq!(original.mode, Mode::RevalidateCurrent);
}

#[test]
fn current_revalidation_proposal_commit_loss_is_atomic_and_recoverable() {
    for failure in [1, 2] {
        let (mut f, source) = compiled_bootstrap();
        let (job, v, d, _) = approved_change_review(&mut f, &source);
        let request = input(&job, &v, &d, Mode::RevalidateCurrent);
        let key = id();
        let before = f.app.inspect_cell(&f.admin, &request.cell).unwrap();
        let p = verified(&mut f, &source, request.clone(), &key).unwrap();
        f.failure.store(failure, Ordering::SeqCst);
        assert!(f.app.commit_process_change(p).is_err());
        assert_eq!(
            f.app
                .process_change(&f.admin, &request.cell, &request.id)
                .is_ok(),
            failure == 2
        );
        let c = match f
            .app
            .prepare_process_change(&f.admin, &key, request.clone())
            .unwrap()
        {
            Preflight::Recorded(c) => *c,
            Preflight::Verify(_) => {
                let p = verified(&mut f, &source, request.clone(), &key).unwrap();
                f.app.commit_process_change(p).unwrap()
            }
        };
        assert_eq!(c.id, request.id);
        assert_eq!(c.revision, Counter(1));
        assert_eq!(c.before, c.after);
        assert_eq!(
            f.app.inspect_cell(&f.admin, &request.cell).unwrap().0,
            before.0
        );
    }
}
