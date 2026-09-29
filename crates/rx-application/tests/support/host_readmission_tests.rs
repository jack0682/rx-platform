use super::*;
use rx_application::{host_link, host_readmission as ra};

fn linked() -> (Fixture, host_link::Prepare, HostRegistration) {
    let (mut f, input) = link_fixture();
    let plan = f.app.prepare_host_link(input.clone()).unwrap();
    let registration = f.app.commit_host_link(link_commit(&f, &plan)).unwrap();
    (f, input, registration)
}
/// The Host process restarts on the same storage: a new boot keeps both journals.
fn restart(
    f: &mut Fixture,
    input: &host_link::Prepare,
    evidence_journal: &Id,
) -> host_link::Prepare {
    let boot = id();
    let session = f
        .app
        .open_evidence_producer(
            &name("host/0"),
            boot.clone(),
            evidence_journal.clone(),
            Digest::from_bytes([81; 32]),
        )
        .unwrap();
    f.hosts[0].session = session.id;
    f.app
        .negotiate_evidence_cell(&f.hosts[0], f.configuration.definition.sha256)
        .unwrap();
    let (_, cell) = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap();
    let mut next = input.clone();
    next.platform_session = id();
    next.snapshot.host_boot = boot;
    next.snapshot.evidence_journal = evidence_journal.clone();
    next.snapshot.epoch = cell.epoch;
    next.snapshot.scopes = cell.scope_epochs.clone();
    next.snapshot.block_ids = cell
        .blocks
        .iter()
        .filter(|b| b.latched)
        .map(|b| b.id.clone())
        .collect();
    next.snapshot.captured_at = f.clock.now();
    next.read_started = f.clock.now();
    next
}
/// The kept delivery journal continues, so a later fence acknowledgement has a later sequence.
fn relink(f: &Fixture, plan: &host_link::Plan, sequence: u64) -> host_link::Commit {
    let mut commit = link_commit(f, plan);
    commit.fence_receipt.sequence = Counter(sequence);
    commit
}
fn approve(r: &HostRegistration, evidence_journal: &Id) -> ra::Approve {
    ra::Approve {
        host: r.id.clone(),
        previous_boot: r.boot_id.clone(),
        delivery_journal: r.delivery_journal.clone(),
        evidence_journal: evidence_journal.clone(),
        binding_intent: None,
    }
}
fn continuity_unproven<T: std::fmt::Debug>(v: Result<T, StoreError>) {
    assert!(
        matches!(v, Err(StoreError::Rejected(Rejection::ContinuityUnproven))),
        "{v:?}"
    );
}

#[test]
fn restarted_host_needs_explicit_readmission_and_its_cell_stays_blocked() {
    let (mut f, input, first) = linked();
    let journal = input.snapshot.evidence_journal.clone();
    let next = restart(&mut f, &input, &journal);
    continuity_unproven(f.app.prepare_host_link(next.clone()));

    let operator = f.operator.clone();
    assert!(
        f.app
            .approve_host_readmission(&operator, &id(), approve(&first, &journal))
            .is_err()
    );
    let release = release_identity(&mut f);
    let key = id();
    let approval = f
        .app
        .approve_host_readmission(&release, &key, approve(&first, &journal))
        .unwrap();
    assert_eq!(
        approval.id,
        f.app
            .approve_host_readmission(&release, &key, approve(&first, &journal))
            .unwrap()
            .id
    );
    assert!(matches!(
        f.app
            .approve_host_readmission(&release, &id(), approve(&first, &journal)),
        Err(StoreError::Rejected(Rejection::Busy))
    ));

    let plan = f.app.prepare_host_link(next.clone()).unwrap();
    assert_eq!(plan.readmission.as_ref(), Some(&approval.id));
    // Reusing a sequence of the kept journal with different content is an integrity conflict.
    assert!(matches!(
        f.app.commit_host_link(link_commit(&f, &plan)),
        Err(StoreError::Integrity(_))
    ));
    let second = f.app.commit_host_link(relink(&f, &plan, 2)).unwrap();
    assert_eq!(second.boot_id, next.snapshot.host_boot);
    assert_eq!(second.delivery_journal, first.delivery_journal);
    assert_ne!(second.session, first.session);
    // The restart block stays latched: re-admission is not a resume.
    let (_, cell) = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap();
    assert!(cell.blocks.iter().any(|b| b.latched));
    assert!(
        f.app
            .pending_deliveries(128)
            .unwrap()
            .iter()
            .all(|p| !matches!(p.payload, Delivery::Arm { .. } | Delivery::Prepare { .. }))
    );

    // The approval is spent: a further restart needs a new approval for the new generation.
    let third = restart(&mut f, &next, &journal);
    continuity_unproven(f.app.prepare_host_link(third.clone()));
    continuity_unproven(
        f.app
            .approve_host_readmission(&release, &id(), approve(&first, &journal)),
    );
    f.app
        .approve_host_readmission(&release, &id(), approve(&second, &journal))
        .unwrap();
    let plan = f.app.prepare_host_link(third.clone()).unwrap();
    assert_eq!(
        f.app
            .commit_host_link(relink(&f, &plan, 3))
            .unwrap()
            .boot_id,
        third.snapshot.host_boot
    );
}

#[test]
fn readmission_requires_the_named_generation_and_both_kept_journals() {
    let (mut f, input, first) = linked();
    let journal = input.snapshot.evidence_journal.clone();
    let release = release_identity(&mut f);
    let mut wrong_boot = approve(&first, &journal);
    wrong_boot.previous_boot = id();
    continuity_unproven(f.app.approve_host_readmission(&release, &id(), wrong_boot));
    let mut wrong_delivery = approve(&first, &journal);
    wrong_delivery.delivery_journal = id();
    continuity_unproven(
        f.app
            .approve_host_readmission(&release, &id(), wrong_delivery),
    );
    continuity_unproven(
        f.app
            .approve_host_readmission(&release, &id(), approve(&first, &id())),
    );
    f.app
        .approve_host_readmission(&release, &id(), approve(&first, &journal))
        .unwrap();

    // Reinstalled storage: a new delivery journal is a different Host, not a restart.
    let mut next = restart(&mut f, &input, &journal);
    next.snapshot.delivery_journal = id();
    continuity_unproven(f.app.prepare_host_link(next));
    // A new evidence journal is rejected the same way.
    let other = id();
    let next = restart(&mut f, &input, &other);
    continuity_unproven(f.app.prepare_host_link(next));
    // The unchanged restart is still accepted afterwards.
    let next = restart(&mut f, &input, &journal);
    let plan = f.app.prepare_host_link(next).unwrap();
    f.app.commit_host_link(relink(&f, &plan, 2)).unwrap();
}

#[test]
fn a_binding_readmission_needs_an_existing_intent_for_the_named_generation() {
    let (mut f, input, first) = linked();
    let journal = input.snapshot.evidence_journal.clone();
    let release = release_identity(&mut f);
    let mut unknown = approve(&first, &journal);
    unknown.binding_intent = Some(id());
    assert!(
        f.app
            .approve_host_readmission(&release, &id(), unknown)
            .is_err()
    );
    // Without a binding transition a new boot must still present the current definition.
    f.app
        .approve_host_readmission(&release, &id(), approve(&first, &journal))
        .unwrap();
    let mut next = restart(&mut f, &input, &journal);
    next.snapshot.definition = Digest::from_bytes([99; 32]);
    assert!(f.app.prepare_host_link(next).is_err());
}
