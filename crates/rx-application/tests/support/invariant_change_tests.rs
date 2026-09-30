//! Focused invariant scenario tests (see docs/invariant-traceability.json).
use super::*;

fn stored_cell(f: &mut Fixture) -> (Counter, Cell) {
    f.app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
}

fn every_scope_advanced(before: &Cell, after: &Cell) -> bool {
    before.scope_epochs.keys().eq(after.scope_epochs.keys())
        && after
            .scope_epochs
            .iter()
            .all(|(scope, epoch)| *epoch > before.scope_epochs[scope])
}

fn fence_ack(
    f: &Fixture,
    message: &Id,
    epoch: Counter,
    scopes: &BTreeMap<Name, Counter>,
    sequence: u64,
) -> FenceAcknowledgment {
    FenceAcknowledgment {
        cell: f.configuration.id.clone(),
        invalidation: message.clone(),
        epoch,
        scopes: scopes.clone(),
        host_boot: f.registrations[0].boot_id.clone(),
        journal: f.registrations[0].delivery_journal.clone(),
        sequence: Counter(sequence),
    }
}

/// OI09: a notification acknowledgement, completed work and a reset observation are recorded as
/// separate facts. None of them revalidates the cell or starts production.
#[test]
fn acknowledgement_work_completion_and_reset_observation_never_become_a_start() {
    use rx_application::{intervention::*, procedure::*};
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let lead = add_identity(
        &mut f.app,
        &f.admin,
        "lead",
        &[Role::RecoveryLead, Role::Observer],
    );
    let policy = Policy {
        external_procedure: artifact(92, "rx.test.external-procedure.v1"),
        dependencies: vec![artifact(93, "rx.test.entry-function.v1")],
        schema: name("rx.procedure-policy.v1"),
        cell: f.configuration.id.clone(),
        definition: f.configuration.definition.sha256,
        envelope: f.configuration.envelope.sha256,
        case_types: vec![CaseType::PlannedAccess],
        entry_conditions: f.configuration.start_conditions.clone(),
        steps: [
            Action::EntryConditionsReported,
            Action::WorkStarted,
            Action::WorkFinished,
            Action::PersonnelAccounted,
            Action::HandoverAccepted,
            Action::ResetObserved,
        ]
        .into_iter()
        .map(|action| Step {
            id: name(&format!("step/{action:?}")),
            action,
            actors: vec![lead.principal.clone(), f.operator.principal.clone()],
            required_evidence: vec![],
            conditions: vec![],
        })
        .collect(),
        maximum_report_age_ns: Counter(20000),
    };
    let reference = content_ref("rx.procedure-policy.v1", &policy);
    f.app
        .admit_procedure(&f.admin, policy, reference.clone())
        .unwrap();
    let mut command = case_request(&f, CaseType::PlannedAccess);
    command.procedure = reference;
    let opened = f
        .app
        .open_case(&f.operator, id().as_str(), command)
        .unwrap();
    assert_eq!(opened.case.state, CaseState::ContainmentPending);
    assert_eq!(
        f.app.inspect_run(&f.operator, &run.id).unwrap().1.state,
        RunState::RecoveryRequired
    );

    // Acknowledgement records reading only: the case stays contained and the cell record is
    // byte-identical, at the same revision.
    let (revision, contained) = stored_cell(&mut f);
    let acknowledged = f
        .app
        .acknowledge_case(
            &f.operator,
            id().as_str(),
            AcknowledgeCase {
                cell: f.configuration.id.clone(),
                case: opened.case.id.clone(),
                expected_case: opened.revision,
                occurred_at: "2026-09-11T01:02:03Z".into(),
            },
        )
        .unwrap();
    assert_eq!(acknowledged.acknowledgments.len(), 1);
    assert_eq!(
        acknowledged.snapshot.case.state,
        CaseState::ContainmentPending
    );
    assert!(acknowledged.procedure_progress.entry_record.is_none());
    let (after_revision, after) = stored_cell(&mut f);
    assert_eq!(after_revision, revision);
    assert_eq!(
        rx_domain::canonical::bytes(&after).unwrap(),
        rx_domain::canonical::bytes(&contained).unwrap()
    );

    // Finished work is not revalidation: the case stays active until personnel and handover are
    // reported, and even then only reaches REVALIDATING with the cell still restricted.
    confirm_case_fences(&mut f);
    let mut case = acknowledged.snapshot;
    for action in [
        Action::EntryConditionsReported,
        Action::WorkStarted,
        Action::WorkFinished,
    ] {
        let actor = if action == Action::EntryConditionsReported {
            lead.clone()
        } else {
            f.operator.clone()
        };
        let people = if action == Action::EntryConditionsReported {
            vec![]
        } else {
            vec![f.operator.principal.clone()]
        };
        let mut report = procedure_report(&f, &case, &actor, action, people);
        if action == Action::EntryConditionsReported {
            let fact = f
                .app
                .inspect_fact(&lead, &f.configuration.id, &name("ready"))
                .unwrap();
            set_report_evidence(&mut report, vec![fact.evidence_id]);
        }
        let receipt = f
            .app
            .record_procedure(&actor, id().as_str(), report)
            .unwrap();
        assert_eq!(receipt.transition_error, None, "{action:?}");
        case = receipt.case;
    }
    assert_eq!(case.case.state, CaseState::ProcedureActive);
    for action in [Action::PersonnelAccounted, Action::HandoverAccepted] {
        let report = procedure_report(&f, &case, &lead, action, vec![f.operator.principal.clone()]);
        let receipt = f
            .app
            .record_procedure(&lead, id().as_str(), report)
            .unwrap();
        assert_eq!(receipt.transition_error, None, "{action:?}");
        case = receipt.case;
    }
    assert_eq!(case.case.state, CaseState::Revalidating);
    let (revision, revalidating) = stored_cell(&mut f);
    assert!(revalidating.open_cases.contains(&case.case.id));
    assert!(revalidating.blocks.iter().any(|b| b.latched));
    assert_eq!(
        revalidating.commissioning,
        Some(Commissioning::RevalidationRequired)
    );

    // None of the above is a start: the old run is not restartable and a new run is refused.
    let run_revision = f.app.inspect_run(&f.operator, &run.id).unwrap().0;
    assert!(matches!(
        f.app.start_run(
            &f.operator,
            id().as_str(),
            StartRun {
                expected_run: run_revision,
                ..start_command(&f, &run, revision, 1)
            }
        ),
        Err(StoreError::Rejected(Rejection::MandateRevoked))
    ));
    let new_run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, revision))
        .unwrap();
    assert!(matches!(
        f.app.start_run(
            &f.operator,
            id().as_str(),
            start_command(&f, &new_run, revision, 1)
        ),
        Err(StoreError::Rejected(Rejection::QualificationRequired))
    ));

    // A reset observation is a new physical fact: it returns the case to containment, discards
    // the personnel and handover conclusions and latches a new restriction at a new epoch.
    let reset = f
        .app
        .record_procedure(
            &f.operator,
            id().as_str(),
            procedure_report(&f, &case, &f.operator, Action::ResetObserved, vec![]),
        )
        .unwrap();
    assert!(reset.facts_recorded);
    assert_eq!(reset.transition_error, None);
    assert_eq!(reset.case.case.state, CaseState::ContainmentPending);
    let progress = f
        .app
        .inspect_case(&lead, &f.configuration.id, &case.case.id)
        .unwrap()
        .procedure_progress;
    assert!(progress.entry_record.is_none());
    assert!(progress.personnel_record.is_none() && progress.handover_record.is_none());
    assert!(
        progress
            .people
            .values()
            .all(|p| !p.accounted && !p.handover)
    );
    let (revision, reset_cell) = stored_cell(&mut f);
    assert!(reset_cell.epoch > revalidating.epoch);
    assert!(
        reset_cell
            .blocks
            .iter()
            .any(|b| b.reason == BlockReason::ProcedureReported && b.latched)
    );
    // The new epoch also retires the prepared, never-started run; it still has no attempt or
    // mandate and cannot be started.
    let (run_revision, stopped) = f.app.inspect_run(&f.operator, &new_run.id).unwrap();
    assert_eq!(stopped.state, RunState::RecoveryRequired);
    assert!(stopped.pending_attempt.is_none() && stopped.mandate.is_none());
    assert!(matches!(
        f.app.start_run(
            &f.operator,
            id().as_str(),
            StartRun {
                expected_run: run_revision,
                ..start_command(&f, &new_run, revision, 1)
            }
        ),
        Err(StoreError::Rejected(Rejection::MandateRevoked))
    ));
}

/// OI10: a reviewed change's preparation invalidates the authority issued under the previous
/// context and fences a new epoch that each Host must acknowledge exactly.
#[test]
fn a_prepared_change_voids_prior_authority_and_waits_for_the_new_epoch_fence() {
    let mut f = fixture(1, true);
    let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let (job, v, d, reviewer) = approved_change_review(&mut f, &p);
    let release = release_identity(&mut f);
    let proposal = change_proposal(&mut f, &p, &job, &v, &d, &id(), id());
    let c = f.app.commit_process_change(proposal).unwrap();
    let staged = stage_change(&mut f, &p, &job, &c, &reviewer, &release);
    let run = start(&mut f, 1);
    let mandate = run.mandate.clone().unwrap();
    let a = activation(&mut f, &run);
    let work = submit(&mut f, &a, id().as_str()).unwrap();
    assert_eq!(
        f.app
            .inspect_permit(&f.operator, &work.permit)
            .unwrap()
            .state,
        PermitState::Issued
    );
    let (_, before) = stored_cell(&mut f);
    assert_eq!(before.commissioning, Some(Commissioning::Commissioned));

    let started = f
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
    let (_, after) = stored_cell(&mut f);
    assert_eq!(after.epoch.0, before.epoch.0 + 1);
    assert!(every_scope_advanced(&before, &after));
    assert_eq!(
        after.commissioning,
        Some(Commissioning::RevalidationRequired)
    );
    assert!(
        after
            .blocks
            .iter()
            .any(|b| b.reason == BlockReason::ConfigurationChange && b.latched)
    );

    // Authority derived from the previous context is gone; the unsent work cannot be emitted.
    let stopped = f.app.inspect_run(&f.operator, &run.id).unwrap().1;
    assert_eq!(stopped.state, RunState::RecoveryRequired);
    assert!(stopped.executor_session.is_none());
    assert_eq!(
        f.app
            .inspect_permit(&f.operator, &work.permit)
            .unwrap()
            .state,
        PermitState::Voided
    );
    assert_eq!(
        f.app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap()
            .operation
            .outcome(),
        rx_domain::operation::Outcome::NotExecuted
    );
    assert!(f.app.begin_delivery(work.operation.id()).is_err());

    // Every Host receives a fence carrying the new epoch vector.
    let preparation = started.preparation.as_ref().unwrap();
    assert_eq!(preparation.fences.len(), f.configuration.hosts.len());
    let fence = preparation.fences[0].clone();
    assert_eq!(fence.epoch, after.epoch);
    assert_eq!(fence.scopes, after.scope_epochs);
    assert!(f.app.pending_deliveries(128).unwrap().iter().any(|d| {
        d.id == fence.message
            && matches!(&d.payload, Delivery::Fence { epoch, scopes, .. }
                if *epoch == after.epoch && *scopes == after.scope_epochs)
    }));
    let unconfirmed = |f: &mut Fixture| {
        f.app
            .process_change(&f.admin, &c.cell, &c.id)
            .unwrap()
            .blockers
            .iter()
            .any(|b| matches!(b, process_change::Blocker::HostFenceUnconfirmed { .. }))
    };
    assert!(unconfirmed(&mut f));

    // An acknowledgement labelled with the previous epoch cannot confirm the new fence.
    f.app.plan_delivery(&f.hosts[0], &fence.message).unwrap();
    let stale = fence_ack(&f, &fence.message, before.epoch, &before.scope_epochs, 31);
    assert!(matches!(
        f.app
            .finish_fence_delivery(&f.hosts[0], &fence.message, stale),
        Err(StoreError::Rejected(Rejection::InvalidInput))
    ));
    assert!(unconfirmed(&mut f));
    let current = fence_ack(&f, &fence.message, fence.epoch, &fence.scopes, 32);
    f.app
        .finish_fence_delivery(&f.hosts[0], &fence.message, current)
        .unwrap();
    assert!(!unconfirmed(&mut f));
    assert!(
        !f.app
            .process_change(&f.admin, &c.cell, &c.id)
            .unwrap()
            .applied
    );

    let rows = f.app.into_repository().snapshot().unwrap().1;
    let revoked = rows
        .iter()
        .filter(|r| r.document.schema.as_str() == "rx.internal.mandate.v1")
        .map(|r| {
            rx_application::persistence::decode::<Mandate>(r, "rx.internal.mandate.v1").unwrap()
        })
        .find(|m| m.id == mandate)
        .unwrap();
    assert_eq!(revoked.state, MandateState::Revoked);
}

/// OI17 (platform half): P's cell and scope epochs only advance, and a pre-invalidation Arm or
/// Fence neither reaches authority nor removes a restriction or regresses P's Host registration.
#[test]
fn platform_epochs_only_advance_and_stale_arm_or_fence_replies_clear_nothing() {
    let mut f = fixture(1, true);
    let host = f.hosts[0].clone();
    let registration = f.registrations[0].clone();
    let (revision, initial) = stored_cell(&mut f);
    let run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, revision))
        .unwrap();
    let attempt = f
        .app
        .start_run(
            &f.operator,
            id().as_str(),
            start_command(&f, &run, revision, 1),
        )
        .unwrap();
    assert_eq!(attempt.status, StartStatus::Arming);
    let arm = f
        .app
        .pending_deliveries(128)
        .unwrap()
        .into_iter()
        .find(|d| matches!(&d.payload, Delivery::Arm { attempt: a, .. } if *a == attempt.id))
        .unwrap();

    // Two invalidations: each advances the cell epoch and every scope epoch and fences the Host.
    let mut previous = initial;
    let mut fences = Vec::new();
    for _ in 0..2 {
        f.app
            .hold(&f.operator, id().as_str(), &f.configuration.id)
            .unwrap();
        let (_, cell) = stored_cell(&mut f);
        assert!(cell.epoch > previous.epoch);
        assert!(every_scope_advanced(&previous, &cell));
        let fence = f
            .app
            .pending_deliveries(128)
            .unwrap()
            .into_iter()
            .find(|d| matches!(&d.payload, Delivery::Fence { epoch, .. } if *epoch == cell.epoch))
            .unwrap();
        fences.push((fence.id, cell.epoch, cell.scope_epochs.clone()));
        previous = cell;
    }
    let (revision, held) = stored_cell(&mut f);
    let unchanged = |f: &mut Fixture| {
        let (r, cell) = stored_cell(f);
        r == revision
            && rx_domain::canonical::bytes(&cell).unwrap()
                == rx_domain::canonical::bytes(&held).unwrap()
    };

    // The pre-invalidation Arm is not emitted and its acknowledgement creates no authority.
    assert!(f.app.plan_delivery(&host, &arm.id).is_err());
    assert!(
        f.app
            .pending_deliveries(128)
            .unwrap()
            .iter()
            .any(|d| d.id == arm.id && d.state == rx_ports::OutboxState::New)
    );
    let old_arm = ArmAcknowledgment {
        attempt: attempt.id.clone(),
        host_boot: registration.boot_id.clone(),
        delivery_journal: registration.delivery_journal.clone(),
        sequence: Counter(1),
        epoch: attempt.epoch,
        scopes: attempt.scopes.clone(),
    };
    assert!(matches!(
        f.app.acknowledge_arm(&host, old_arm.clone()),
        Err(StoreError::Rejected(Rejection::StaleRevision))
    ));
    let relabelled = ArmAcknowledgment {
        epoch: held.epoch,
        scopes: held.scope_epochs.clone(),
        ..old_arm
    };
    assert!(matches!(
        f.app.acknowledge_arm(&host, relabelled),
        Err(StoreError::Rejected(Rejection::StaleEpoch))
    ));
    let stopped = f.app.inspect_run(&f.operator, &run.id).unwrap().1;
    assert!(stopped.mandate.is_none());
    assert_ne!(stopped.state, RunState::Executing);
    assert_eq!(
        f.app
            .inspect_attempt(&f.operator, &attempt.id)
            .unwrap()
            .status,
        StartStatus::Rejected
    );
    assert!(unchanged(&mut f));

    // An older fence cannot be relabelled as the current one. Acknowledging the newest fence
    // first and the older one late removes no restriction and leaves every epoch unchanged.
    for (message, _, _) in &fences {
        f.app.plan_delivery(&host, message).unwrap();
    }
    let (old_message, old_epoch, old_scopes) = fences[0].clone();
    let (new_message, new_epoch, new_scopes) = fences[1].clone();
    let relabelled = fence_ack(&f, &old_message, new_epoch, &new_scopes, 41);
    assert!(matches!(
        f.app.finish_fence_delivery(&host, &old_message, relabelled),
        Err(StoreError::Rejected(Rejection::InvalidInput))
    ));
    let current = fence_ack(&f, &new_message, new_epoch, &new_scopes, 42);
    f.app
        .finish_fence_delivery(&host, &new_message, current)
        .unwrap();
    let late = fence_ack(&f, &old_message, old_epoch, &old_scopes, 43);
    f.app
        .finish_fence_delivery(&host, &old_message, late)
        .unwrap();
    assert!(unchanged(&mut f));

    // P's record of the Host epoch kept the newest acknowledged fence, not the late older one.
    f.app
        .hold(&f.operator, id().as_str(), &f.configuration.id)
        .unwrap();
    let (_, next) = stored_cell(&mut f);
    assert!(next.epoch > held.epoch);
    let fence = f
        .app
        .pending_deliveries(128)
        .unwrap()
        .into_iter()
        .find(|d| matches!(&d.payload, Delivery::Fence { epoch, .. } if *epoch == next.epoch))
        .unwrap();
    let plan = f.app.plan_delivery(&host, &fence.id).unwrap();
    assert_eq!(plan.registration.epoch, new_epoch);
    assert_eq!(plan.registration.scopes, new_scopes);
}
