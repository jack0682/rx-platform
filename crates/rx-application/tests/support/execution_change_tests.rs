//! Real P review/fence/apply transactions with signed packages and simulated Host receipts.
//! This is not native Host execution, qualification activation, or frozen-v1 evidence.
use super::*;
use crate::configuration_dispatch_tests as dispatch;
use rx_application::{
    configuration_dispatch::{Emission, Request},
    package_intake as intake, workflow_publication as publication,
};
use rx_process_contract::execution_v2 as v2;

fn verified(
    ticket: Box<process_change::Ticket>,
    p: &review_support::Fixture,
) -> process_change::Prepared {
    let (report, signature, bytes) = p.report(ticket.job());
    let checked = process_review::Validated::check(
        ticket.job(),
        p.store.verify_owned(&p.object, &p.policy).unwrap(),
        &p.authority,
        report,
        signature,
        Some(&bytes),
    )
    .unwrap();
    process_change::Prepared::new(*ticket, checked).unwrap()
}

#[test]
fn reviewed_v2_change_requires_matching_host_policy_and_preserves_unqualified_apply() {
    run_reviewed_change(0);
}
#[test]
fn reviewed_v2_change_refuses_current_definition_drift_after_host_acceptance() {
    run_reviewed_change(1);
}
#[test]
fn reviewed_v2_change_rechecks_definition_drift_after_apply_preflight() {
    run_reviewed_change(2);
}
fn run_reviewed_change(stale_definition: u8) {
    let mut action = execution_fixture().policy.templates[&name("node")].clone();
    action.host = name("host/0");
    action.intent.resource_set = vec![name("controller/0")];
    let mut f = fixture_configured((1, false, false, true, None, false, true), |mut cfg| {
        cfg.site_config_digest = action.intent.site_config_digest;
        cfg.steps[0].intent = action.intent.clone();
        cfg
    });
    let mut e = execution_fixture_in((f._directory, f.app, f.admin.clone(), f.failure.clone()));
    e.policy.templates.insert(name("node"), action);
    let mut p = review_support::fixture_sequence(&f.configuration, Digest::from_bytes([71; 32]));
    let mut contracts = p.policy.contracts.clone();
    contracts.package_abi = name("rx.package-abi.v2");
    let device = signed_package::fixture_contracts(
        |catalog| {
            catalog.installation = e.app.installation.id.clone();
            catalog.cell = f.configuration.id.clone();
            catalog.templates = [(
                name("node"),
                v2::TemplateDeclaration {
                    action: e.policy.templates[&name("node")].clone(),
                    contract: e.policy.node_contracts[&name("node")].clone(),
                },
            )]
            .into();
        },
        false,
        Some(contracts),
    );
    // A single current store/policy serves both process review and template intake.
    let mut policy_doc: rx_package::policy::Policy =
        serde_json::from_slice(&p.policy_bytes).unwrap();
    let device_doc: rx_package::policy::Policy = serde_json::from_slice(
        &std::fs::read(device._directory.path().join("policy.json")).unwrap(),
    )
    .unwrap();
    policy_doc.schema = name("rx.package-verification-policy.v2");
    policy_doc
        .additional_package_abis
        .push(name("rx.package-abi.v2"));
    policy_doc.keys.extend(device_doc.keys);
    policy_doc.assets.extend(device_doc.assets);
    p.policy = policy_doc.load().unwrap();
    p.policy_bytes = canonical::bytes(&policy_doc).unwrap();
    std::fs::write(&p.policy_path, &p.policy_bytes).unwrap();
    let package = rx_package::verify_package(
        &device.manifest,
        &device.signature,
        device.files.clone(),
        &p.policy,
    )
    .unwrap();
    let object = p.store.put(&package).unwrap();
    e.app
        .configure_package_intake(Some((
            p.store.owner().clone(),
            p.policy.fingerprint().unwrap(),
            rx_package::content_digest(&p.policy_bytes),
        )))
        .unwrap();
    let context = e
        .app
        .package_intake_context(&e.owner, &f.configuration.id)
        .unwrap();
    let intake_id = id();
    let intake::Preflight::Verify(ticket) = e
        .app
        .prepare_package_intake(
            &e.owner,
            &id(),
            intake::Submit {
                id: intake_id.clone(),
                cell: f.configuration.id.clone(),
                title: "Execution templates".into(),
                relative_path: rx_package::PackagePath::new("templates").unwrap(),
                object: object.clone(),
                configuration_digest: context.configuration_digest,
                policy_generation: context.registration.unwrap().generation,
            },
        )
        .unwrap()
    else {
        panic!("new template intake")
    };
    e.app
        .commit_package_intake(
            intake::Prepared::new(*ticket, p.store.verify_owned(&object, &p.policy).unwrap())
                .unwrap(),
        )
        .unwrap();
    let preview_input = e.preview_input();
    let key = id();
    let prepared = e.prepare_preview(&key, preview_input);
    let preview = e
        .app
        .save_execution_preview(&e.owner, &key, prepared)
        .unwrap();
    let input = e.publish_input(preview.reference, intake_id.clone());
    let publication::PublishPreparation::Verify(ticket) = e
        .app
        .prepare_workflow_publication(&e.owner, &id(), input)
        .unwrap()
    else {
        panic!("new publication")
    };
    let prepared = publication::PreparedPublication::verify(
        *ticket,
        [(intake_id, p.store.verify_owned(&object, &p.policy).unwrap())].into(),
    )
    .unwrap();
    let published = e.app.commit_workflow_publication(prepared).unwrap();
    let dependency = e.snapshot.input_closure().definitions[0].clone();
    let catalog = e.catalog;
    f.app = e.app;
    f._directory = e.directory;
    let revise = |f: &mut Fixture, label: &str| {
        let mut update = save(&catalog.id, dependency.body.clone());
        update.id = dependency.reference.id.clone();
        update.expected = Some(dependency.reference.revision);
        update.label = label.into();
        put(&mut f.app, &f.admin, update);
    };

    let (job, version, decision, reviewer) = approved_change_review(&mut f, &p);
    let child = rx_process_contract::validation::nodes(&p.resolved)
        .into_iter()
        .find(|n| matches!(n.body, rx_process_contract::CompiledBody::Operation { .. }))
        .unwrap();
    let plan = v2::Plan {
        schema: name(v2::PLAN_SCHEMA),
        binding: v2::Binding {
            schema: name(v2::BINDING_SCHEMA),
            publication: published.reference.clone(),
            policy: published.policy.clone(),
            nodes: [(child.id.clone(), name("node"))].into(),
        },
        process: p.resolved.clone(),
    };
    let mut target = f.configuration.clone();
    target.steps[0].id = child.id.clone();
    target.process = Some(Box::new(plan.process.clone()));
    target.execution = Some(Box::new(plan.binding.clone()));
    target.recipe = plan.reference().unwrap();
    let configuration = f
        .app
        .prepare_workflow_configuration(&f.admin, target.clone())
        .unwrap();
    let input = process_change::Create {
        execution_configuration: Some(configuration),
        mode: process_change::Mode::Replace,
        id: id(),
        cell: f.configuration.id.clone(),
        review: process_change::ReviewRef {
            id: job.request.id.clone(),
            revision: version.revision,
            review_digest: version.review_digest,
            decision_revision: decision.revision,
        },
        reason: "Apply exact reviewed template with its published input domain".into(),
    };
    let process_change::Preflight::Verify(ticket) = f
        .app
        .prepare_process_change(&f.admin, &id(), input)
        .unwrap()
    else {
        panic!("change ticket")
    };
    let change = f.app.commit_process_change(verified(ticket, &p)).unwrap();
    let release = release_identity(&mut f);
    let staged = stage_change(&mut f, &p, &job, &change, &reviewer, &release);
    let change = f
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
            .authorize_host_configuration(&release, &id(), change_target(&change))
            .is_err()
    );
    dispatch::fences(&mut f, &change);
    f.app
        .authorize_host_configuration(&release, &id(), change_target(&change))
        .unwrap();
    let task = f
        .app
        .host_configuration_tasks(&f.hosts[0], None)
        .unwrap()
        .remove(0);
    assert_eq!(
        task.execution_policies[&f.configuration.id].publication,
        published.reference
    );
    assert!(
        f.app
            .bind_host_configuration(
                &f.hosts[0],
                &task.id,
                dispatch::inspect(&task),
                f.clock.now()
            )
            .is_err(),
        "v1 cannot bind a v2 task"
    );
    let observation = v2::host_configuration::Observation {
        schema: name(v2::host_configuration::OBSERVATION_SCHEMA),
        snapshot: dispatch::inspect(&task).snapshot,
        policies: BTreeMap::new(),
        receipt: None,
        context_matches_current_host: false,
        activation_authorized: false,
    };
    let task = f
        .app
        .bind_host_configuration(&f.hosts[0], &task.id, observation, f.clock.now())
        .unwrap();
    let Emission::Send { request } = f
        .app
        .enter_host_configuration_send(&f.hosts[0], &task.id, false)
        .unwrap()
    else {
        panic!("send")
    };
    let Request::V2(request) = *request else {
        panic!("no v1 fallback")
    };
    assert_eq!(request.digest().unwrap(), task.request_digest.unwrap());
    assert!(
        matches!(f.app.enter_host_configuration_send(&f.hosts[0], &task.id, false).unwrap(), Emission::Lookup { execution_v2: true, request } if request == task.id)
    );
    assert!(
        f.app
            .prepare_process_change_apply(&release, &id(), change_target(&change))
            .is_err(),
        "missing receipt cannot apply"
    );
    let legacy = dispatch::applied(&task);
    assert!(
        f.app
            .record_host_configuration_observation(&f.hosts[0], &task.id, legacy.clone())
            .is_err(),
        "v1 cannot acknowledge a v2 task"
    );
    let context = legacy.receipt.unwrap();
    let policies: BTreeMap<_, _> = request
        .policies
        .iter()
        .map(|(cell, policy)| {
            (
                cell.clone(),
                v2::host_configuration::AppliedPolicy {
                    publication: policy.publication.clone(),
                    policy: policy.reference.clone(),
                    request: task.id.clone(),
                    receipt_sequence: context.sequence,
                    configuration: request
                        .context
                        .cells
                        .iter()
                        .find(|c| &c.cell == cell)
                        .unwrap()
                        .after_configuration,
                },
            )
        })
        .collect();
    let accepted = v2::host_configuration::Observation {
        schema: name(v2::host_configuration::OBSERVATION_SCHEMA),
        snapshot: legacy.snapshot,
        policies: policies.clone(),
        receipt: Some(v2::host_configuration::Receipt {
            schema: name(v2::host_configuration::RECEIPT_SCHEMA),
            request_digest: request.digest().unwrap(),
            request: *request,
            context,
            policies,
        }),
        context_matches_current_host: true,
        activation_authorized: false,
    };
    let mut wrong = accepted.clone();
    wrong
        .policies
        .get_mut(&f.configuration.id)
        .unwrap()
        .policy
        .sha256 = Digest::from_bytes([99; 32]);
    assert!(
        f.app
            .record_host_configuration_observation(&f.hosts[0], &task.id, wrong)
            .is_err()
    );
    f.app
        .record_host_configuration_read(&f.hosts[0], &task.id, accepted, f.clock.now())
        .unwrap();
    if stale_definition == 1 {
        revise(&mut f, "Definition revised after Host acceptance");
        assert!(
            f.app
                .prepare_process_change_apply(&release, &id(), change_target(&change))
                .is_err()
        );
        assert!(
            matches!(f.app.enter_host_configuration_send(&f.hosts[0], &task.id, false).unwrap(),
            Emission::Lookup { execution_v2: true, request } if request == task.id)
        );
        let cell = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap().1;
        assert!(cell.configuration.execution.is_none());
        return;
    }
    let apply_key = id();
    let process_change::Preflight::Verify(ticket) = f
        .app
        .prepare_process_change_apply(&release, &apply_key, change_target(&change))
        .unwrap()
    else {
        panic!("apply ticket")
    };
    let prepared = verified(ticket, &p);
    if stale_definition == 2 {
        revise(&mut f, "Definition revised during apply verification");
        assert!(f.app.commit_process_change_apply(prepared).is_err());
        assert!(
            f.app
                .inspect_cell(&f.admin, &f.configuration.id)
                .unwrap()
                .1
                .configuration
                .execution
                .is_none()
        );
        return;
    }
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app.commit_process_change_apply(prepared).is_err(),
        "lost response injected after commit"
    );
    let process_change::Preflight::Recorded(done) = f
        .app
        .prepare_process_change_apply(&release, &apply_key, change_target(&change))
        .unwrap()
    else {
        panic!("original apply recovery")
    };
    assert_eq!(done.state, process_change::State::AppliedUnqualified);
    let cell = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap().1;
    assert_eq!(
        canonical::bytes(&cell.configuration).unwrap(),
        canonical::bytes(&target).unwrap()
    );
    assert!(cell.qualification.is_none());
    assert_eq!(cell.commissioning, Some(Commissioning::NotCommissioned));
    assert!(
        !f.app
            .process_change(&f.admin, &change.cell, &change.id)
            .unwrap()
            .activation_authorized
    );
    // A changed current definition blocks new effects but never original-key apply recovery.
    revise(&mut f, "Changed after original apply");
    assert!(matches!(
        f.app
            .prepare_process_change_apply(&release, &apply_key, change_target(&change))
            .unwrap(),
        process_change::Preflight::Recorded(_)
    ));
}
