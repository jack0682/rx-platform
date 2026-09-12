use super::*;
use configuration_dispatch::{Emission, Issue, Phase, Task};
use rx_domain::{canonical, host_configuration as wire};

fn prepared(f: &mut Fixture) -> (Identity, process_change::Change) {
    let (_, _, identity, change) = prepared_materials(f);
    (identity, change)
}
fn prepared_materials(
    f: &mut Fixture,
) -> (
    review_support::Fixture,
    process_review::Job,
    Identity,
    process_change::Change,
) {
    let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let (job, v, d, reviewer) = approved_change_review(f, &p);
    let release = release_identity(f);
    let proposal = change_proposal(f, &p, &job, &v, &d, &id(), id());
    let c = f.app.commit_process_change(proposal).unwrap();
    let staged = stage_change(f, &p, &job, &c, &reviewer, &release);
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
    (p, job, release, c)
}
fn fences(f: &mut Fixture, c: &process_change::Change) {
    for (i, fence) in c.preparation.as_ref().unwrap().fences.iter().enumerate() {
        let host = f.hosts.iter().find(|h| h.principal == fence.host).unwrap();
        let reg = f.registrations.iter().find(|h| h.id == fence.host).unwrap();
        f.app.plan_delivery(host, &fence.message).unwrap();
        f.app
            .finish_fence_delivery(
                host,
                &fence.message,
                FenceAcknowledgment {
                    cell: fence.cell.clone(),
                    invalidation: fence.message.clone(),
                    epoch: fence.epoch,
                    scopes: fence.scopes.clone(),
                    host_boot: reg.boot_id.clone(),
                    journal: reg.delivery_journal.clone(),
                    sequence: Counter(
                        100 + i as u64 + c.preparation.as_ref().unwrap().attempt.0 * 64,
                    ),
                },
            )
            .unwrap();
    }
}
fn task(f: &mut Fixture, index: usize) -> Task {
    f.app
        .host_configuration_tasks(&f.hosts[index], None)
        .unwrap()
        .remove(0)
}
fn inspect(t: &Task) -> wire::Observation {
    wire::Observation {
        schema: name("rx.host-process-configuration-observation.v1"),
        snapshot: wire::Snapshot {
            schema: name("rx.host-process-configuration-snapshot.v1"),
            host: t.host.clone(),
            host_boot: t.host_boot.clone(),
            delivery_journal: t.delivery_journal.clone(),
            binding_digest: Digest::from_bytes([85; 32]),
            cells: t
                .cells
                .iter()
                .map(|c| wire::CellObservation {
                    cell: c.cell.clone(),
                    definition: c.definition,
                    envelope: c.envelope,
                    environment: c.environment.clone(),
                    epoch: c.epoch,
                    scopes: c.scopes.clone(),
                    blocked: c.change_blocks.clone(),
                    applied: None,
                })
                .collect(),
        },
        receipt: None,
        context_matches_current_host: false,
        activation_authorized: false,
    }
}
fn bind(f: &mut Fixture, index: usize) -> Task {
    let t = task(f, index);
    f.app
        .bind_host_configuration(&f.hosts[index], &t.id, inspect(&t), f.clock.now())
        .unwrap()
}
fn applied(t: &Task) -> wire::Observation {
    let mut o = inspect(t);
    let r = t.request.clone().unwrap();
    for cell in &mut o.snapshot.cells {
        let target = r.cells.iter().find(|c| c.cell == cell.cell).unwrap();
        cell.applied = Some(wire::AppliedContext {
            cell: cell.cell.clone(),
            configuration: target.after_configuration,
            change: t.change.clone(),
            request: t.id.clone(),
            receipt_sequence: Counter(17),
            binding_digest: r.binding_digest,
        });
    }
    o.receipt = Some(wire::Receipt {
        schema: name("rx.host-process-configuration-receipt.v1"),
        request_digest: r.digest().unwrap(),
        request: r,
        host_boot: t.host_boot.clone(),
        journal: t.delivery_journal.clone(),
        sequence: Counter(17),
        status: wire::Status::AppliedUnqualified,
        effect: wire::Effect::Installed,
        reason: None,
        quiescence: Some(wire::Quiescence {
            device_session: id(),
            observed_at: expiry(1000),
            uncertainty_ns: Counter(0),
            resources: vec![name("controller/0")],
        }),
        recorded_at: expiry(1000),
    });
    o.context_matches_current_host = true;
    o.validate().unwrap();
    o
}
#[test]
fn configuration_dispatch_requires_fences_terminal_and_retains_one_task_after_commit_loss() {
    for mode in [1, 2] {
        let mut f = fixture(1, false);
        let (release, c) = prepared(&mut f);
        assert!(matches!(
            f.app
                .authorize_host_configuration(&release, &id(), change_target(&c)),
            Err(StoreError::Rejected(Rejection::HostNotPrepared))
        ));
        fences(&mut f, &c);
        let unbound = Identity {
            terminal: None,
            ..release.clone()
        };
        assert!(
            f.app
                .authorize_host_configuration(&unbound, &id(), change_target(&c))
                .is_err()
        );
        let key = id();
        f.failure.store(mode, Ordering::SeqCst);
        assert!(
            f.app
                .authorize_host_configuration(&release, &key, change_target(&c))
                .is_err()
        );
        assert_eq!(
            f.app
                .host_configuration_tasks(&f.hosts[0], None)
                .unwrap()
                .len(),
            usize::from(mode == 2)
        );
        let b = f
            .app
            .authorize_host_configuration(&release, &key, change_target(&c))
            .unwrap();
        let again = f
            .app
            .authorize_host_configuration(&release, &id(), change_target(&c))
            .unwrap();
        assert_eq!(b.tasks, again.tasks);
        assert_eq!(
            f.app
                .host_configuration_tasks(&f.hosts[0], None)
                .unwrap()
                .len(),
            1
        );
        let mut changed = change_target(&c);
        changed.plan_digest = Digest::from_bytes([99; 32]);
        assert!(matches!(
            f.app.authorize_host_configuration(&release, &key, changed),
            Err(StoreError::KeyConflict)
        ));
    }
}
#[test]
fn configuration_dispatch_send_entry_is_durable_and_missing_retry_uses_exact_request() {
    for mode in [1, 2] {
        let mut f = fixture(1, false);
        let (release, c) = prepared(&mut f);
        fences(&mut f, &c);
        f.app
            .authorize_host_configuration(&release, &id(), change_target(&c))
            .unwrap();
        let t = bind(&mut f, 0);
        let bytes = canonical::bytes(t.request.as_ref().unwrap()).unwrap();
        f.failure.store(mode, Ordering::SeqCst);
        assert!(
            f.app
                .enter_host_configuration_send(&f.hosts[0], &t.id, false)
                .is_err()
        );
        let actual = task(&mut f, 0);
        assert_eq!(
            actual.phase,
            if mode == 1 {
                Phase::Prepared
            } else {
                Phase::SendEntered
            }
        );
        let emission = f
            .app
            .enter_host_configuration_send(&f.hosts[0], &t.id, false)
            .unwrap();
        if mode == 2 {
            assert!(matches!(emission, Emission::Lookup { .. }));
        } else {
            assert!(matches!(emission, Emission::Send { .. }));
        }
        let view = f.app.process_change(&f.admin, &c.cell, &c.id).unwrap();
        assert!(view.host_configuration.outcome_unknown);
        assert!(
            f.app
                .enter_host_configuration_send(&f.hosts[0], &t.id, true)
                .is_err()
        );
        let recorded = f
            .app
            .record_host_configuration_observation(&f.hosts[0], &t.id, inspect(&t))
            .unwrap();
        assert_eq!(recorded.issue, Some(Issue::ReceiptMissing));
        let Emission::Send { request } = f
            .app
            .enter_host_configuration_send(&f.hosts[0], &t.id, true)
            .unwrap()
        else {
            panic!("exact retry")
        };
        assert_eq!(canonical::bytes(&*request).unwrap(), bytes);
        let receipt = applied(&t);
        f.app
            .record_host_configuration_observation(&f.hosts[0], &t.id, receipt.clone())
            .unwrap();
        let actual = task(&mut f, 0);
        assert_eq!(
            canonical::bytes(&actual.receipt).unwrap(),
            canonical::bytes(&receipt.receipt).unwrap()
        );
    }
}
#[test]
fn configuration_dispatch_receipt_survives_transport_failure_and_disappearance_latches_dispute() {
    let mut f = fixture(1, false);
    let (release, c) = prepared(&mut f);
    fences(&mut f, &c);
    f.app
        .authorize_host_configuration(&release, &id(), change_target(&c))
        .unwrap();
    let t = bind(&mut f, 0);
    f.app
        .enter_host_configuration_send(&f.hosts[0], &t.id, false)
        .unwrap();
    let observation = applied(&t);
    f.app
        .record_host_configuration_observation(&f.hosts[0], &t.id, observation.clone())
        .unwrap();
    let before = f.app.process_change(&f.admin, &c.cell, &c.id).unwrap();
    assert!(before.host_configuration.all_hosts_acknowledged);
    assert!(!before.applied && !before.activation_authorized);
    f.app
        .note_host_configuration_issue(&f.hosts[0], &t.id, Issue::TransportUnavailable)
        .unwrap();
    let kept = task(&mut f, 0);
    assert!(kept.receipt.is_some());
    assert!(
        !f.app
            .process_change(&f.admin, &c.cell, &c.id)
            .unwrap()
            .host_configuration
            .all_hosts_acknowledged
    );
    f.app
        .record_host_configuration_observation(&f.hosts[0], &t.id, observation.clone())
        .unwrap();
    let kept = f
        .app
        .record_host_configuration_observation(&f.hosts[0], &t.id, inspect(&t))
        .unwrap();
    assert!(kept.integrity_disputed);
    assert_eq!(
        canonical::bytes(&kept.receipt).unwrap(),
        canonical::bytes(&observation.receipt).unwrap()
    );
    let later = f
        .app
        .record_host_configuration_observation(&f.hosts[0], &t.id, observation)
        .unwrap();
    assert!(later.integrity_disputed);
    assert!(
        !f.app
            .process_change(&f.admin, &c.cell, &c.id)
            .unwrap()
            .host_configuration
            .all_hosts_acknowledged
    );
    assert!(
        f.app
            .enter_host_configuration_send(&f.hosts[0], &t.id, true)
            .is_err()
    );
}
#[test]
fn configuration_dispatch_revoked_sender_blocks_resend_but_late_fact_is_retained() {
    let mut f = fixture(1, false);
    let (release, c) = prepared(&mut f);
    fences(&mut f, &c);
    f.app
        .authorize_host_configuration(&release, &id(), change_target(&c))
        .unwrap();
    let t = bind(&mut f, 0);
    f.app
        .enter_host_configuration_send(&f.hosts[0], &t.id, false)
        .unwrap();
    f.app
        .record_host_configuration_observation(&f.hosts[0], &t.id, inspect(&t))
        .unwrap();
    let mut p = principal("release-manager", &[Role::ReleaseManager]);
    p.active = false;
    f.app.put_principal(&f.admin, p, Some(Counter(1))).unwrap();
    assert!(
        f.app
            .enter_host_configuration_send(&f.hosts[0], &t.id, true)
            .is_err()
    );
    let actual = f
        .app
        .record_host_configuration_observation(&f.hosts[0], &t.id, applied(&t))
        .unwrap();
    assert!(actual.receipt.is_some());
    assert!(
        !f.app
            .process_change(&f.admin, &c.cell, &c.id)
            .unwrap()
            .applied
    );
}
#[test]
fn configuration_dispatch_partial_host_confirmation_is_mixed_without_platform_swap() {
    let mut f = fixture(2, false);
    let (release, c) = prepared(&mut f);
    fences(&mut f, &c);
    f.app
        .authorize_host_configuration(&release, &id(), change_target(&c))
        .unwrap();
    let t = bind(&mut f, 0);
    f.app
        .enter_host_configuration_send(&f.hosts[0], &t.id, false)
        .unwrap();
    f.app
        .record_host_configuration_observation(&f.hosts[0], &t.id, applied(&t))
        .unwrap();
    let view = f.app.process_change(&f.admin, &c.cell, &c.id).unwrap();
    assert!(view.host_configuration.mixed_configuration);
    assert!(!view.host_configuration.all_hosts_acknowledged);
    let t = bind(&mut f, 1);
    f.app
        .enter_host_configuration_send(&f.hosts[1], &t.id, false)
        .unwrap();
    f.app
        .record_host_configuration_observation(&f.hosts[1], &t.id, applied(&t))
        .unwrap();
    let view = f.app.process_change(&f.admin, &c.cell, &c.id).unwrap();
    assert!(!view.host_configuration.mixed_configuration);
    assert!(view.host_configuration.all_hosts_acknowledged);
    assert!(
        !view.host_configuration.platform_configuration_applied
            && view
                .host_configuration
                .revalidation_required_before_platform_apply
    );
    let cell = f.app.inspect_cell(&f.admin, &c.cell).unwrap().1;
    assert_eq!(
        canonical::bytes(&cell.configuration).unwrap(),
        canonical::bytes(&f.configuration).unwrap()
    );
    assert!(
        cell.blocks
            .iter()
            .any(|b| b.reason == BlockReason::ConfigurationChange)
    );
}
#[test]
fn configuration_dispatch_changed_snapshot_cannot_authorize_missing_receipt_retry() {
    for kind in 0..4 {
        let mut f = fixture(1, false);
        let (release, c) = prepared(&mut f);
        fences(&mut f, &c);
        f.app
            .authorize_host_configuration(&release, &id(), change_target(&c))
            .unwrap();
        let t = bind(&mut f, 0);
        f.app
            .enter_host_configuration_send(&f.hosts[0], &t.id, false)
            .unwrap();
        let mut o = inspect(&t);
        match kind {
            0 => o.snapshot.host_boot = id(),
            1 => o.snapshot.binding_digest = Digest::from_bytes([92; 32]),
            2 => o.snapshot.cells[0].blocked.clear(),
            _ => o.snapshot.cells[0].environment = name("PHYSICAL"),
        }
        let actual = f
            .app
            .record_host_configuration_observation(&f.hosts[0], &t.id, o)
            .unwrap();
        assert_ne!(actual.issue, Some(Issue::ReceiptMissing));
        assert!(actual.receipt.is_none());
        assert!(
            f.app
                .enter_host_configuration_send(&f.hosts[0], &t.id, true)
                .is_err()
        );
    }
}
#[test]
fn configuration_dispatch_conflicting_receipt_never_overwrites_first_fact() {
    let mut f = fixture(1, false);
    let (release, c) = prepared(&mut f);
    fences(&mut f, &c);
    f.app
        .authorize_host_configuration(&release, &id(), change_target(&c))
        .unwrap();
    let t = bind(&mut f, 0);
    f.app
        .enter_host_configuration_send(&f.hosts[0], &t.id, false)
        .unwrap();
    let first = applied(&t);
    f.app
        .record_host_configuration_observation(&f.hosts[0], &t.id, first.clone())
        .unwrap();
    let mut conflicting = first.clone();
    conflicting.receipt.as_mut().unwrap().recorded_at = expiry(1001);
    let actual = f
        .app
        .record_host_configuration_observation(&f.hosts[0], &t.id, conflicting)
        .unwrap();
    assert!(actual.integrity_disputed);
    assert_eq!(
        canonical::bytes(&actual.receipt).unwrap(),
        canonical::bytes(&first.receipt).unwrap()
    );
}
#[test]
fn configuration_dispatch_new_preparation_cannot_bypass_older_unknown_send() {
    let mut f = fixture(1, false);
    let (release, c) = prepared(&mut f);
    fences(&mut f, &c);
    f.app
        .authorize_host_configuration(&release, &id(), change_target(&c))
        .unwrap();
    let t = bind(&mut f, 0);
    f.app
        .enter_host_configuration_send(&f.hosts[0], &t.id, false)
        .unwrap();
    let next = f
        .app
        .begin_process_change_preparation(
            &release,
            &id(),
            process_change::BeginPreparation {
                target: change_target(&c),
                refresh: true,
            },
        )
        .unwrap();
    fences(&mut f, &next);
    let view = f.app.process_change(&f.admin, &c.cell, &c.id).unwrap();
    assert!(view.host_configuration.outcome_unknown);
    assert_eq!(
        view.host_configuration.hosts[0].preparation,
        Some(t.preparation)
    );
    assert!(!view.host_configuration.all_hosts_acknowledged);
    assert!(matches!(
        f.app
            .authorize_host_configuration(&release, &id(), change_target(&next)),
        Err(StoreError::Rejected(Rejection::ContinuityUnproven))
    ));
    assert!(
        f.app
            .enter_host_configuration_send(&f.hosts[0], &t.id, true)
            .is_err()
    );
    assert_eq!(task(&mut f, 0).id, t.id);
}

fn authorize_and_observe(
    f: &mut Fixture,
    release: &Identity,
    c: &process_change::Change,
) -> Vec<Task> {
    fences(f, c);
    f.app
        .authorize_host_configuration(release, &id(), change_target(c))
        .unwrap();
    let mut tasks = Vec::new();
    for index in 0..f.hosts.len() {
        let t = bind(f, index);
        f.app
            .enter_host_configuration_send(&f.hosts[index], &t.id, false)
            .unwrap();
        f.app
            .record_host_configuration_read(&f.hosts[index], &t.id, applied(&t), f.clock.now())
            .unwrap();
        tasks.push(t);
    }
    tasks
}
fn apply_prepared(
    f: &mut Fixture,
    p: &review_support::Fixture,
    job: &process_review::Job,
    release: &Identity,
    c: &process_change::Change,
    key: &Id,
) -> process_change::Prepared {
    let process_change::Preflight::Verify(ticket) = f
        .app
        .prepare_process_change_apply(release, key, change_target(c))
        .unwrap()
    else {
        panic!("apply ticket")
    };
    let (r, s, b) = p.report(job);
    let v = process_review::Validated::check(
        job,
        p.store.verify_owned(&p.object, &p.policy).unwrap(),
        &p.authority,
        r,
        s,
        Some(&b),
    )
    .unwrap();
    process_change::Prepared::new(*ticket, v).unwrap()
}
#[test]
fn configuration_apply_is_atomic_and_lost_reply_recovers_selection_epoch_and_history() {
    for mode in [1, 2] {
        let mut f = fixture(1, true);
        let (p, job, release, c) = prepared_materials(&mut f);
        authorize_and_observe(&mut f, &release, &c);
        let before = f.app.inspect_cell(&f.admin, &c.cell).unwrap().1;
        let key = id();
        let prepared = apply_prepared(&mut f, &p, &job, &release, &c, &key);
        f.failure.store(mode, Ordering::SeqCst);
        assert!(f.app.commit_process_change_apply(prepared).is_err());
        let interim = f.app.inspect_cell(&f.admin, &c.cell).unwrap().1;
        if mode == 1 {
            assert_eq!(
                canonical::bytes(&interim).unwrap(),
                canonical::bytes(&before).unwrap()
            );
        }
        let done = match f
            .app
            .prepare_process_change_apply(&release, &key, change_target(&c))
            .unwrap()
        {
            process_change::Preflight::Recorded(c) => *c,
            process_change::Preflight::Verify(t) => {
                let (r, s, b) = p.report(&job);
                let v = process_review::Validated::check(
                    &job,
                    p.store.verify_owned(&p.object, &p.policy).unwrap(),
                    &p.authority,
                    r,
                    s,
                    Some(&b),
                )
                .unwrap();
                f.app
                    .commit_process_change_apply(process_change::Prepared::new(*t, v).unwrap())
                    .unwrap()
            }
        };
        assert_eq!(done.state, process_change::State::AppliedUnqualified);
        let applied = done.application.as_ref().unwrap();
        assert_eq!(applied.host_proofs.len(), 1);
        assert_eq!(
            applied.cells[0].previous_qualification.as_ref().unwrap().id,
            before.qualification.as_ref().unwrap().id
        );
        let current = f.app.inspect_cell(&f.admin, &c.cell).unwrap().1;
        assert_eq!(current.epoch.0, before.epoch.0 + 1);
        assert!(current.qualification.is_none());
        assert_eq!(
            current.commissioning,
            Some(Commissioning::RevalidationRequired)
        );
        assert_ne!(current.configuration.recipe, before.configuration.recipe);
        assert!(
            current
                .blocks
                .iter()
                .any(|b| b.reason == BlockReason::ConfigurationChange)
        );
        let view = f.app.process_change(&f.admin, &c.cell, &c.id).unwrap();
        assert!(view.applied);
        assert!(!view.activation_authorized);
        assert!(
            view.blockers
                .iter()
                .any(|b| matches!(b, process_change::Blocker::RequalificationRequired))
        );
        let process_change::Preflight::Recorded(again) = f
            .app
            .prepare_process_change_apply(&release, &key, change_target(&c))
            .unwrap()
        else {
            panic!("cached apply")
        };
        assert_eq!(again.revision, done.revision);
        assert_eq!(
            again.application.as_ref().unwrap().fences[0].message,
            applied.fences[0].message
        );
    }
}
#[test]
fn configuration_apply_requires_volatile_fresh_observations_at_both_prepare_and_commit() {
    let mut f = fixture(1, false);
    let (p, job, release, c) = prepared_materials(&mut f);
    fences(&mut f, &c);
    f.app
        .authorize_host_configuration(&release, &id(), change_target(&c))
        .unwrap();
    let t = bind(&mut f, 0);
    f.app
        .enter_host_configuration_send(&f.hosts[0], &t.id, false)
        .unwrap();
    let o = applied(&t);
    f.app
        .record_host_configuration_observation(&f.hosts[0], &t.id, o.clone())
        .unwrap();
    assert!(matches!(
        f.app
            .prepare_process_change_apply(&release, &id(), change_target(&c)),
        Err(StoreError::Rejected(Rejection::Expired))
    ));
    f.app
        .record_host_configuration_read(&f.hosts[0], &t.id, o, f.clock.now())
        .unwrap();
    let prepared = apply_prepared(&mut f, &p, &job, &release, &c, &id());
    f.app
        .note_host_configuration_issue(&f.hosts[0], &t.id, Issue::TransportUnavailable)
        .unwrap();
    assert!(f.app.commit_process_change_apply(prepared).is_err());
    assert_eq!(
        canonical::bytes(
            &f.app
                .inspect_cell(&f.admin, &c.cell)
                .unwrap()
                .1
                .configuration
        )
        .unwrap(),
        canonical::bytes(&f.configuration).unwrap()
    );
}
#[test]
fn configuration_apply_cannot_use_partial_host_results_or_revoked_terminal() {
    let mut f = fixture(2, false);
    let (p, job, release, c) = prepared_materials(&mut f);
    fences(&mut f, &c);
    f.app
        .authorize_host_configuration(&release, &id(), change_target(&c))
        .unwrap();
    let t = bind(&mut f, 0);
    f.app
        .enter_host_configuration_send(&f.hosts[0], &t.id, false)
        .unwrap();
    f.app
        .record_host_configuration_read(&f.hosts[0], &t.id, applied(&t), f.clock.now())
        .unwrap();
    assert!(
        f.app
            .prepare_process_change_apply(&release, &id(), change_target(&c))
            .is_err()
    );
    let t = bind(&mut f, 1);
    f.app
        .enter_host_configuration_send(&f.hosts[1], &t.id, false)
        .unwrap();
    f.app
        .record_host_configuration_read(&f.hosts[1], &t.id, applied(&t), f.clock.now())
        .unwrap();
    let prepared = apply_prepared(&mut f, &p, &job, &release, &c, &id());
    f.app
        .put_terminal(
            &f.admin,
            Terminal {
                id: name("panel/main"),
                certificate_digest: Digest::from_bytes([77; 32]),
                cells: [c.cell.clone()].into_iter().collect(),
                active: false,
            },
            Some(Counter(1)),
        )
        .unwrap();
    assert!(f.app.commit_process_change_apply(prepared).is_err());
    assert!(
        !f.app
            .process_change(&f.admin, &c.cell, &c.id)
            .unwrap()
            .applied
    );
}
#[test]
fn configuration_apply_keeps_completed_process_artifacts_and_new_runs_bind_new_configuration() {
    let (mut f, old_run) = completed_branch_fixture();
    let old_snapshot = f
        .app
        .execution_snapshot(&f.executor, &old_run.id, Counter(1))
        .unwrap();
    let old_bytes = f
        .app
        .executor_artifact(&f.executor, &old_run.id, &old_snapshot.resolved)
        .unwrap();
    let (p, job, release, c) = prepared_materials(&mut f);
    authorize_and_observe(&mut f, &release, &c);
    let prepared = apply_prepared(&mut f, &p, &job, &release, &c, &id());
    f.app.commit_process_change_apply(prepared).unwrap();
    let now = f
        .app
        .execution_snapshot(&f.executor, &old_run.id, Counter(1))
        .unwrap();
    assert_eq!(now.resolved, old_snapshot.resolved);
    assert!(!now.request_admission_allowed);
    assert_eq!(
        canonical::bytes(&now.run.run).unwrap(),
        canonical::bytes(&old_snapshot.run.run).unwrap()
    );
    assert_eq!(
        f.app
            .executor_artifact(&f.executor, &old_run.id, &old_snapshot.resolved)
            .unwrap(),
        old_bytes
    );
    let production = f.app.production_view(&f.executor, &old_run.id).unwrap();
    assert_eq!(production.resolved, old_snapshot.resolved);
    assert!(!production.admission_allowed);
    let (revision, cell) = f.app.inspect_cell(&f.admin, &c.cell).unwrap();
    let new = f
        .app
        .create_run(
            &f.operator,
            id().as_str(),
            CreateRun {
                cell: c.cell.clone(),
                expected_cell: revision,
                recipe_digest: cell.configuration.recipe.sha256,
                site_config_digest: cell.configuration.site_config_digest,
            },
        )
        .unwrap();
    assert_ne!(new.recipe_digest, old_run.recipe_digest);
    assert!(
        f.app
            .start_run(
                &f.operator,
                id().as_str(),
                start_command(&f, &new, revision, 1)
            )
            .is_err()
    );
}

#[test]
fn configuration_apply_rejects_expired_host_read_even_with_a_fresh_release_session() {
    let mut f = fixture(1, false);
    let (_p, _job, release, c) = prepared_materials(&mut f);
    authorize_and_observe(&mut f, &release, &c);
    f.clock.0.store(3_000_002_000, Ordering::SeqCst);
    let session = f
        .app
        .authenticated_terminal_user_session(
            &release.principal,
            id(),
            Counter(99_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap();
    let current = Identity {
        session: session.id,
        ..release
    };
    assert!(matches!(
        f.app
            .prepare_process_change_apply(&current, &id(), change_target(&c)),
        Err(StoreError::Rejected(Rejection::Expired))
    ));
}

#[path = "requalification_tests.rs"]
mod requalification_tests;

#[path = "process_revalidation_tests.rs"]
mod process_revalidation_tests;
