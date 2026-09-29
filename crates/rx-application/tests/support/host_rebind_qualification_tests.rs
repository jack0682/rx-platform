use super::super::requalification_tests::qsupport as material;
use super::*;
use rx_application::requalification as q;
fn fixture() -> (
    Fixture,
    Identity,
    process_change::Change,
    material::Fixture,
    rx_application::host_rejoin::Restriction,
) {
    let mut artifacts = None;
    let (mut f, mut input) = link_fixture_configured(false, |c| {
        let (c, a) = material::configure(c);
        artifacts = Some(a);
        c
    });
    input.provenance = Some(proof(&input));
    let plan = f.app.prepare_host_link(input.clone()).unwrap();
    let host = f.app.commit_host_link(link_commit(&f, &plan)).unwrap();
    f.registrations.push(host);
    f.app
        .register_host_recovery_transport(input.host.clone(), pin())
        .unwrap();
    let (source, job, mut release, change) = prepared_materials(&mut f);
    release.session = f
        .app
        .authenticated_terminal_user_session(
            &release.principal,
            id(),
            Counter(1_000_000_000_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap()
        .id;
    let tasks = authorize_and_observe(&mut f, &release, &change);
    let apply = apply_prepared(&mut f, &source, &job, &release, &change, &id());
    let change = f.app.commit_process_change_apply(apply).unwrap();
    f.configuration = f
        .app
        .inspect_cell(&f.admin, &change.cell)
        .unwrap()
        .1
        .configuration;
    let policy = material::policy(&f.configuration, artifacts.unwrap());
    f.app
        .configure_requalification(Some(policy.policy.clone()))
        .unwrap();
    let old = f
        .app
        .host_rejoin_context(&release, &input.host, &change.cell)
        .unwrap();
    f.registrations[0] = old.cells[&change.cell].registration.clone().unwrap();
    let who = replace_host(
        &mut f,
        &input,
        id(),
        input.snapshot.evidence_journal.clone(),
        Digest::from_bytes([81; 32]),
    );
    let context = f
        .app
        .host_rejoin_context(&release, &input.host, &change.cell)
        .unwrap();
    assert!(
        context.local_prerequisites_current,
        "{:?}",
        context.blockers
    );
    let cell = &context.cells[&change.cell];
    let reg = cell.registration.as_ref().unwrap();
    let mut snapshot = input.snapshot;
    snapshot.host_boot = context.producer.peer_boot.clone();
    snapshot.definition = f.configuration.definition.sha256;
    snapshot.envelope = f.configuration.envelope.sha256;
    snapshot.epoch = reg.epoch;
    snapshot.scopes = reg.scopes.clone();
    snapshot.block_ids = old.cells[&change.cell]
        .cell
        .blocks
        .iter()
        .map(|b| b.id.clone())
        .collect();
    snapshot.resource_fences = reg
        .grant
        .resources
        .iter()
        .map(|r| (r.clone(), reg.grant.fence))
        .collect();
    let mut configuration = configuration_read(&snapshot);
    configuration.snapshot.cells[0].applied = applied(&tasks[0]).snapshot.cells[0].applied.clone();
    let read = recovery::ReadEvidence {
        platform_session: id(),
        transport: pin(),
        configuration,
        configuration_started: f.clock.now(),
        configuration_finished: f.clock.now(),
        cells: BTreeMap::from([(
            change.cell.clone(),
            recovery::SnapshotRead {
                snapshot,
                started: f.clock.now(),
                finished: f.clock.now(),
            },
        )]),
    };
    let p = f
        .app
        .propose_host_rejoin(
            &release,
            &id(),
            rx_application::host_rejoin::Prepare {
                host: context.host.clone(),
                origin: change.cell.clone(),
                expected_context: context.digest().unwrap(),
                expected_cells: context.expected_cells(),
            },
            verified(read.clone()),
        )
        .unwrap();
    f.app
        .approve_host_rejoin(&release, &id(), rejoin_approval_input(&p))
        .unwrap();
    let task = f
        .app
        .plan_host_rejoin_fence(&p.proposal.id, &change.cell, verified(read.clone()))
        .unwrap();
    f.app
        .record_host_rejoin_fence(
            &p.proposal.id,
            &change.cell,
            FenceAcknowledgment {
                cell: change.cell.clone(),
                invalidation: task.request,
                epoch: task.epoch,
                scopes: task.scopes,
                host_boot: context.producer.peer_boot.clone(),
                journal: reg.delivery_journal.clone(),
                sequence: Counter(900),
            },
        )
        .unwrap();
    let mut read = fresh_rejoin_read(&read, f.clock.now(), &context, true);
    f.app
        .refresh_host_rejoin(&p.proposal.id, verified(read.clone()))
        .unwrap();
    let parent = f.app.host_rejoin_binding(&release, &p.proposal.id).unwrap();
    let rebind = f
        .app
        .approve_host_rebind(&release, &id(), rebind_approval(&parent))
        .unwrap();
    let prepared = f
        .app
        .prepare_host_rebind(
            &rebind.rebind.id,
            verified(read.clone()),
            BTreeMap::from([(change.cell.clone(), Counter(1000))]),
        )
        .unwrap();
    let plan = f
        .app
        .plan_host_rebind_grant(&prepared.id, &change.cell, verified(read.clone()))
        .unwrap();
    let mut commit = link_commit(&f, &plan);
    commit.fence_receipt = prepared.steps[&change.cell].fence.clone();
    f.app
        .record_host_rebind_grant(&prepared.id, &change.cell, commit, &plan.host_boot)
        .unwrap();
    for r in &plan.resources {
        read.cells
            .get_mut(&change.cell)
            .unwrap()
            .snapshot
            .resource_fences
            .insert(r.clone(), plan.fence);
    }
    f.app
        .commit_host_rebind(&prepared.id, verified(read))
        .unwrap();
    f.hosts[0] = who;
    f.registrations[0] = f.app.bound_host_link(&plan.id).unwrap();
    let eligible = f
        .app
        .host_rebind_restrictions(&release, &change.cell)
        .unwrap();
    assert!(!eligible.clearance_authorized);
    assert_eq!(eligible.restrictions.len(), 1);
    (f, release, change, policy, eligible.restrictions[0].clone())
}
fn begin(
    f: &mut Fixture,
    change: &process_change::Change,
    policy: &material::Fixture,
    restriction: &rx_application::host_rejoin::Restriction,
) -> q::Begin {
    q::Begin {
        id: id(),
        cell: change.cell.clone(),
        change: change.id.clone(),
        expected_change: change.revision,
        expected_cells: BTreeMap::from([(
            change.cell.clone(),
            f.app.inspect_cell(&f.admin, &change.cell).unwrap().0,
        )]),
        policy_digest: policy.policy.digest().unwrap(),
        runtime_restrictions: BTreeMap::new(),
        host_rebind_restrictions: BTreeMap::from([(
            restriction.block.id.clone(),
            restriction.digest().unwrap(),
        )]),
    }
}
#[test]
fn host_rebind_restriction_is_selected_signed_and_bound_to_one_review_without_clearing() {
    let (mut f, release, change, policy, restriction) = fixture();
    let input = begin(&mut f, &change, &policy, &restriction);
    let key = id();
    let job = f
        .app
        .begin_requalification(&release, &key, input.clone())
        .unwrap();
    assert_eq!(job.request.schema.as_str(), "rx.requalification-request.v3");
    assert_eq!(job.request.host_rebind_restrictions.len(), 1);
    assert!(job.request.cells[0].blocks.contains(&restriction.block.id));
    assert_eq!(
        job.request.digest().unwrap(),
        f.app
            .begin_requalification(&release, &key, input)
            .unwrap()
            .request
            .digest()
            .unwrap()
    );
    let (mut report, signature, blobs) = policy.report(&job);
    assert!(
        q::Verified::check(
            &job,
            &policy.policy,
            report.clone(),
            signature.clone(),
            blobs.clone()
        )
        .is_ok()
    );
    report.request.host_rebind_restrictions[0].rebind = id();
    assert!(q::Verified::check(&job, &policy.policy, report, signature, blobs).is_err());
    let cell = f.app.inspect_cell(&f.admin, &change.cell).unwrap().1;
    assert!(cell.blocks.iter().any(|b| b.id == restriction.block.id));
    assert!(cell.qualification.is_none());
    let mut repo = f.app.into_repository();
    repo.transact(|tx| {
        let rows = tx.scan("host-rebind-restriction-binding/")?;
        assert_eq!(rows.len(), 1);
        assert!(
            tx.get(&p::key("changeblockowner", &restriction.block.id))?
                .is_none()
        );
        Ok(())
    })
    .unwrap();
}
#[test]
fn host_rebind_restriction_wrong_digest_or_unrelated_block_is_not_adopted() {
    for bad in 0..3 {
        let (mut f, release, change, policy, restriction) = fixture();
        let mut input = begin(&mut f, &change, &policy, &restriction);
        if bad == 0 {
            input
                .host_rebind_restrictions
                .insert(restriction.block.id.clone(), Digest::from_bytes([99; 32]));
        }
        if bad == 1 {
            input.host_rebind_restrictions.clear();
            input
                .host_rebind_restrictions
                .insert(id(), restriction.digest().unwrap());
        }
        if bad == 2 {
            f.app
                .put_principal(
                    &f.admin,
                    principal(release.principal.as_str(), &[Role::Observer]),
                    Some(Counter(1)),
                )
                .unwrap();
        }
        assert!(f.app.begin_requalification(&release, &id(), input).is_err());
        let mut repo = f.app.into_repository();
        assert!(
            repo.transact(|tx| tx.scan("host-rebind-restriction-binding/"))
                .unwrap()
                .is_empty()
        );
    }
}

fn approve_report(
    f: &mut Fixture,
    job: &q::Job,
    policy: &material::Fixture,
) -> (q::Version, q::Decision) {
    for (i, target) in job.request.fences.iter().enumerate() {
        f.app.plan_delivery(&f.hosts[0], &target.message).unwrap();
        f.app
            .finish_fence_delivery(
                &f.hosts[0],
                &target.message,
                FenceAcknowledgment {
                    cell: target.cell.clone(),
                    invalidation: target.message.clone(),
                    epoch: target.epoch,
                    scopes: target.scopes.clone(),
                    host_boot: f.registrations[0].boot_id.clone(),
                    journal: f.registrations[0].delivery_journal.clone(),
                    sequence: Counter(1500 + i as u64),
                },
            )
            .unwrap();
    }
    let (report, signature, blobs) = policy.report(job);
    let input = q::Submit {
        review: job.request.id.clone(),
        cell: job.request.origin.clone(),
        expected: None,
        directory: rx_package::PackagePath::new("qualification").unwrap(),
        report_digest: report.digest().unwrap(),
    };
    let q::Preflight::Verify(ticket) = f
        .app
        .prepare_requalification_report(&f.admin, &id(), input)
        .unwrap()
    else {
        panic!("report ticket")
    };
    let version = f
        .app
        .commit_requalification_report(
            q::Prepared::new(
                *ticket,
                q::Verified::check(job, &policy.policy, report, signature, blobs).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let reviewer = add_identity(&mut f.app, &f.admin, "rebind-reviewer", &[Role::Verifier]);
    let input = q::Decide {
        review: job.request.id.clone(),
        cell: job.request.origin.clone(),
        report_revision: version.revision,
        report_digest: version.digest,
        expected: None,
        choice: q::Choice::Approve,
        note: "Test-only review of exact bound recovery evidence".into(),
    };
    let q::DecisionPreflight::Verify(ticket) = f
        .app
        .prepare_requalification_decision(&reviewer, &id(), input)
        .unwrap()
    else {
        panic!("decision ticket")
    };
    let decision = f
        .app
        .commit_requalification_decision(ticket.verify(&policy.policy).unwrap())
        .unwrap();
    (version, decision)
}
#[test]
fn host_rebind_restriction_clearance_requires_its_own_current_signed_review() {
    use rx_application::qualification_activation as a;
    for explicit in [false, true] {
        let (mut f, release, change, policy, restriction) = fixture();
        // A historical review binding must not authorize a different review.
        if !explicit {
            let input = begin(&mut f, &change, &policy, &restriction);
            f.app.begin_requalification(&release, &id(), input).unwrap();
        }
        let mut input = begin(&mut f, &change, &policy, &restriction);
        if !explicit {
            input.host_rebind_restrictions.clear();
        }
        let job = f.app.begin_requalification(&release, &id(), input).unwrap();
        let (version, decision) = approve_report(&mut f, &job, &policy);
        let (revision, cell) = f.app.inspect_cell(&f.admin, &change.cell).unwrap();
        let input = a::IssueRequest {
            review: job.request.id.clone(),
            cell: change.cell.clone(),
            report_revision: version.revision,
            report_digest: version.digest,
            decision_revision: decision.revision,
            expected_cells: BTreeMap::from([(change.cell.clone(), revision)]),
            clear_blocks: BTreeMap::from([(
                change.cell.clone(),
                cell.blocks.iter().map(|b| b.id.clone()).collect(),
            )]),
        };
        let result = f.app.prepare_qualification_issue(&release, &id(), input);
        if explicit {
            assert!(matches!(result, Ok(a::Preflight::Verify(_))));
        } else {
            assert!(matches!(
                result,
                Err(StoreError::Rejected(Rejection::Forbidden))
            ));
        }
        assert!(
            f.app
                .inspect_cell(&f.admin, &change.cell)
                .unwrap()
                .1
                .blocks
                .iter()
                .any(|b| b.id == restriction.block.id)
        );
    }
}
#[test]
fn host_rebind_restriction_cannot_survive_an_unapproved_next_host_boot() {
    let (mut f, release, change, policy, restriction) = fixture();
    let context = f
        .app
        .host_rejoin_context(&release, &restriction.host, &change.cell)
        .unwrap();
    f.app
        .open_evidence_producer(
            &restriction.host,
            id(),
            context.producer.journal,
            context.producer.authentication_binding,
        )
        .unwrap();
    assert!(
        f.app
            .host_rebind_restrictions(&release, &change.cell)
            .unwrap()
            .restrictions
            .is_empty()
    );
    let input = begin(&mut f, &change, &policy, &restriction);
    assert!(f.app.begin_requalification(&release, &id(), input).is_err());
}

#[test]
fn host_rebind_restriction_review_and_binding_commit_atomically_and_recover_one_request() {
    for failure in [1, 2] {
        let (mut f, release, change, policy, restriction) = fixture();
        let input = begin(&mut f, &change, &policy, &restriction);
        let key = id();
        f.failure.store(failure, Ordering::SeqCst);
        assert!(
            f.app
                .begin_requalification(&release, &key, input.clone())
                .is_err()
        );
        let job = f
            .app
            .begin_requalification(&release, &key, input.clone())
            .unwrap();
        assert_eq!(
            job.request.digest().unwrap(),
            f.app
                .begin_requalification(&release, &key, input)
                .unwrap()
                .request
                .digest()
                .unwrap()
        );
        let mut repo = f.app.into_repository();
        repo.transact(|tx| {
            assert_eq!(tx.scan("host-rebind-restriction-binding/")?.len(), 1);
            assert_eq!(tx.scan("requalificationjob/")?.len(), 1);
            Ok(())
        })
        .unwrap();
    }
}
#[test]
fn host_rebind_restriction_v3_is_explicit_and_empty_legacy_encoding_is_preserved() {
    let (mut f, release, change, policy, restriction) = fixture();
    let mut input = begin(&mut f, &change, &policy, &restriction);
    input.host_rebind_restrictions.clear();
    let value = serde_json::to_value(&input).unwrap();
    assert!(value.get("host_rebind_restrictions").is_none());
    let job = f.app.begin_requalification(&release, &id(), input).unwrap();
    assert_eq!(job.request.schema.as_str(), "rx.requalification-request.v2");
    let legacy = serde_json::to_value(&job.request).unwrap();
    assert!(legacy.get("host_rebind_restrictions").is_none());
    let decoded: q::Request = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(
        canonical::bytes(&decoded).unwrap(),
        canonical::bytes(&legacy).unwrap()
    );
    let mut wrong = job.request;
    wrong.host_rebind_restrictions = vec![restriction];
    assert!(wrong.digest().is_err());
}

#[test]
fn host_rebind_restriction_rejects_registration_journal_drift() {
    let (mut f, release, change, policy, restriction) = fixture();
    let mut registration = f.registrations[0].clone();
    registration.delivery_journal = id();
    f.app.register_host(&f.hosts[0], registration).unwrap();
    assert!(
        f.app
            .host_rebind_restrictions(&release, &change.cell)
            .unwrap()
            .restrictions
            .is_empty()
    );
    let input = begin(&mut f, &change, &policy, &restriction);
    assert!(f.app.begin_requalification(&release, &id(), input).is_err());
}
