use super::device_review_tests as review;
use super::*;
use rx_application::{device_binding as binding, device_review as dr};
fn approved(
    new_resource: bool,
) -> (
    Fixture,
    intake_support::Fixture,
    dr::Authority,
    dr::Job,
    dr::Version,
    dr::Decision,
    Identity,
) {
    let (mut f, p, a, j, who) = review::setup_for(|c| {
        if new_resource {
            c.steps[0].intent.resource_set = vec![name("controller/b")];
        }
    });
    let v = review::submit(&mut f, &p, &a, &j, None, true);
    let input = review::decision(&v, None);
    let dr::DecisionPreflight::Verify(t) =
        f.app.prepare_device_decision(&who, &id(), input).unwrap()
    else {
        panic!()
    };
    let d = f
        .app
        .commit_device_decision(
            dr::PreparedDecision::approve(*t, review::verified(&j, &p, &a, v.report.clone()))
                .unwrap(),
        )
        .unwrap();
    (f, p, a, j, v, d, who)
}
fn input(f: &Fixture, v: &dr::Version, d: &dr::Decision) -> binding::Propose {
    binding::Propose {
        id: id(),
        cell: f.configuration.id.clone(),
        review: binding::ReviewRef {
            id: v.review.clone(),
            revision: v.revision,
            review_digest: v.review_digest,
            decision_revision: d.revision,
        },
        bindings: [(
            name("step/0"),
            binding::Selection {
                action: name("load"),
                host: name("host/0"),
                conditions: [(
                    name("ready"),
                    f.configuration.steps[0].conditions[0].clone(),
                )]
                .into(),
                completion_postconditions: vec![],
                handover_max_age_ns: Counter(500),
            },
        )]
        .into(),
        reason: "Bind reviewed device action without applying it".into(),
    }
}
fn prepare_plan(
    f: &mut Fixture,
    p: &intake_support::Fixture,
    a: &dr::Authority,
    j: &dr::Job,
    v: &dr::Version,
    key: &Id,
    input: binding::Propose,
) -> binding::Prepared {
    let binding::Preflight::Verify(t) = f.app.prepare_device_binding(&f.admin, key, input).unwrap()
    else {
        panic!()
    };
    binding::Prepared::new(*t, review::verified(j, p, a, v.report.clone())).unwrap()
}
fn add_independent_b(f: &mut Fixture) {
    let mut c = f.configuration.clone();
    c.id = name("cell/b");
    c.hosts = vec![name("host/b")];
    c.scopes = vec![name("zone/b")];
    c.steps[0].host = name("host/b");
    c.steps[0].intent.resource_set = vec![name("controller/b")];
    c.fact_specs[0].host = name("host/b");
    f.app.install_cell(&f.admin, c).unwrap();
}
#[test]
fn approved_action_produces_exact_nonexecuting_candidate_and_independent_impact_review() {
    let (mut f, p, a, j, v, d, who) = approved(false);
    let before = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap();
    let deliveries = f.app.pending_deliveries(128).unwrap().len();
    let input = input(&f, &v, &d);
    let prepared = prepare_plan(&mut f, &p, &a, &j, &v, &id(), input);
    let plan = f.app.commit_device_binding(prepared).unwrap();
    let candidate = &plan.definition.candidates[&name("step/0")];
    assert_eq!(candidate.action, name("load"));
    assert_eq!(candidate.step.condition_revision, Counter(2));
    assert!(candidate.previous_step_digest.is_some());
    assert!(candidate.step.predecessors.is_empty());
    assert_eq!(
        candidate.step.intent.digest().unwrap(),
        f.configuration.steps[0].intent.digest().unwrap()
    );
    assert!(
        plan.definition.requires_process_review
            && plan.definition.requires_host_binding
            && plan.definition.requires_operating_envelope_review
    );
    let change = binding::ReviewImpact {
        plan: plan.id.clone(),
        cell: plan.cell.clone(),
        expected: plan.revision,
        plan_digest: plan.plan_digest,
        note: "Review process, host and envelope before application".into(),
    };
    assert!(
        f.app
            .prepare_device_binding_review(&f.admin, &id(), change.clone())
            .is_err()
    );
    let binding::Preflight::Verify(t) = f
        .app
        .prepare_device_binding_review(&who, &id(), change)
        .unwrap()
    else {
        panic!()
    };
    let reviewed = f
        .app
        .commit_device_binding_review(
            binding::Prepared::new(*t, review::verified(&j, &p, &a, v.report)).unwrap(),
        )
        .unwrap();
    assert_eq!(reviewed.state, binding::State::ImpactReviewed);
    assert_eq!(reviewed.revision, Counter(2));
    assert_eq!(reviewed.plan_digest, plan.plan_digest);
    let detail = f
        .app
        .device_binding_plan(&who, &plan.cell, &plan.id)
        .unwrap();
    assert!(detail.context_current && detail.device_approval_current);
    assert!(
        !detail.activation_authorized
            && !detail.configuration_changed
            && !detail.application_supported
    );
    assert_eq!(
        rx_domain::canonical::bytes(&before).unwrap(),
        rx_domain::canonical::bytes(&f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap())
            .unwrap()
    );
    assert_eq!(deliveries, f.app.pending_deliveries(128).unwrap().len());
}
#[test]
fn prospective_resources_expand_impact_and_uncovered_cell_scope_rejects_commit() {
    let (mut f, p, a, j, v, d, _) = approved(true);
    add_independent_b(&mut f);
    let input = input(&f, &v, &d);
    let prepared = prepare_plan(&mut f, &p, &a, &j, &v, &id(), input.clone());
    let plan = f.app.commit_device_binding(prepared).unwrap();
    assert_eq!(
        plan.definition
            .impact
            .cells
            .iter()
            .map(|c| c.id.clone())
            .collect::<Vec<_>>(),
        vec![name("cell/a"), name("cell/b")]
    );
    assert!(
        plan.definition.impact.cells[0]
            .resources
            .contains(&name("controller/b"))
    );
    let mut other = input;
    other.id = id();
    let prepared = prepare_plan(&mut f, &p, &a, &j, &v, &id(), other);
    let mut actor = principal(
        "admin",
        &[
            Role::AccountAdmin,
            Role::Engineer,
            Role::Verifier,
            Role::Observer,
        ],
    );
    actor.cells = [name("cell/a")].into();
    let admin = f.admin.clone();
    f.app
        .put_principal(&admin, actor, Some(Counter(1)))
        .unwrap();
    assert!(f.app.commit_device_binding(prepared).is_err());
    assert!(
        f.app
            .device_binding_plan(&admin, &plan.cell, &plan.id)
            .is_err()
    );
}
#[test]
fn new_shared_cell_or_new_report_invalidates_impact_review() {
    for new_cell in [false, true] {
        let (mut f, p, a, j, v, d, who) = approved(true);
        let input = input(&f, &v, &d);
        let prepared = prepare_plan(&mut f, &p, &a, &j, &v, &id(), input);
        let plan = f.app.commit_device_binding(prepared).unwrap();
        if new_cell {
            add_independent_b(&mut f);
        } else {
            review::submit(&mut f, &p, &a, &j, Some(v.revision), true);
        }
        let detail = f
            .app
            .device_binding_plan(&who, &plan.cell, &plan.id)
            .unwrap();
        assert!(!detail.context_current || !detail.device_approval_current);
        assert!(
            f.app
                .prepare_device_binding_review(
                    &who,
                    &id(),
                    binding::ReviewImpact {
                        plan: plan.id,
                        cell: plan.cell,
                        expected: plan.revision,
                        plan_digest: plan.plan_digest,
                        note: "stale impact".into()
                    }
                )
                .is_err()
        );
    }
}
#[test]
fn missing_mandatory_conditions_and_malformed_conditions_never_get_default_success() {
    let (mut f, p, a, j, v, d, _) = approved(false);
    for variant in 0..4 {
        let mut input = input(&f, &v, &d);
        let s = input.bindings.get_mut(&name("step/0")).unwrap();
        match variant {
            0 => s.conditions.clear(),
            1 => {
                s.conditions
                    .insert(name("ready"), Condition::All { children: vec![] });
            }
            2 => s.handover_max_age_ns = Counter(0),
            _ => s.action = name("not-in-package"),
        };
        let binding::Preflight::Verify(t) = f
            .app
            .prepare_device_binding(&f.admin, &id(), input)
            .unwrap()
        else {
            panic!()
        };
        assert!(
            binding::Prepared::new(*t, review::verified(&j, &p, &a, v.report.clone())).is_err()
        );
    }
    let mut input = input(&f, &v, &d);
    input
        .bindings
        .get_mut(&name("step/0"))
        .unwrap()
        .conditions
        .insert(
            name("ready"),
            Condition::Eq {
                fact: name("unmapped"),
                schema: name("boolean/v1"),
                unit: name("unitless"),
                expected: TypedValue::Boolean(true),
            },
        );
    let prepared = prepare_plan(&mut f, &p, &a, &j, &v, &id(), input);
    let plan = f.app.commit_device_binding(prepared).unwrap();
    assert!(
        plan.definition
            .issues
            .iter()
            .any(|i| i.code.as_str() == "FACT_SPEC_REVIEW_REQUIRED")
    );
    assert!(plan.impact_review.is_none());
}
#[test]
fn proposal_and_impact_review_recover_original_records_after_commit_faults() {
    for fault in [1, 2] {
        let (mut f, p, a, j, v, d, who) = approved(false);
        let input = input(&f, &v, &d);
        let key = id();
        let value = prepare_plan(&mut f, &p, &a, &j, &v, &key, input.clone());
        f.failure.store(fault, Ordering::SeqCst);
        assert!(f.app.commit_device_binding(value).is_err());
        let plan = match f.app.prepare_device_binding(&f.admin, &key, input).unwrap() {
            binding::Preflight::Recorded(p) => *p,
            binding::Preflight::Verify(t) => f
                .app
                .commit_device_binding(
                    binding::Prepared::new(*t, review::verified(&j, &p, &a, v.report.clone()))
                        .unwrap(),
                )
                .unwrap(),
        };
        assert_eq!(plan.revision, Counter(1));
        let request = binding::ReviewImpact {
            plan: plan.id.clone(),
            cell: plan.cell.clone(),
            expected: plan.revision,
            plan_digest: plan.plan_digest,
            note: "independent impact record".into(),
        };
        let key = id();
        let binding::Preflight::Verify(t) = f
            .app
            .prepare_device_binding_review(&who, &key, request.clone())
            .unwrap()
        else {
            panic!()
        };
        let value =
            binding::Prepared::new(*t, review::verified(&j, &p, &a, v.report.clone())).unwrap();
        f.failure.store(fault, Ordering::SeqCst);
        assert!(f.app.commit_device_binding_review(value).is_err());
        let reviewed = match f
            .app
            .prepare_device_binding_review(&who, &key, request)
            .unwrap()
        {
            binding::Preflight::Recorded(p) => *p,
            binding::Preflight::Verify(t) => f
                .app
                .commit_device_binding_review(
                    binding::Prepared::new(*t, review::verified(&j, &p, &a, v.report.clone()))
                        .unwrap(),
                )
                .unwrap(),
        };
        assert_eq!(reviewed.revision, Counter(2));
    }
}

fn authoring_plan() -> (
    Fixture,
    intake_support::Fixture,
    dr::Authority,
    dr::Job,
    dr::Version,
    Identity,
    binding::Plan,
) {
    let (mut f, p, a, j, v, d, who) = approved(false);
    let input = input(&f, &v, &d);
    let prepared = prepare_plan(&mut f, &p, &a, &j, &v, &id(), input);
    let plan = f.app.commit_device_binding(prepared).unwrap();
    let request = binding::ReviewImpact {
        plan: plan.id.clone(),
        cell: plan.cell.clone(),
        expected: plan.revision,
        plan_digest: plan.plan_digest,
        note: "independent authoring impact review".into(),
    };
    let binding::Preflight::Verify(t) = f
        .app
        .prepare_device_binding_review(&who, &id(), request)
        .unwrap()
    else {
        panic!()
    };
    let plan = f
        .app
        .commit_device_binding_review(
            binding::Prepared::new(*t, review::verified(&j, &p, &a, v.report.clone())).unwrap(),
        )
        .unwrap();
    (f, p, a, j, v, who, plan)
}
#[test]
fn device_plan_authoring_exports_exact_provenance_without_changing_active_steps() {
    use rx_application::draft_bindings::BindingPlanRef;
    for fault in [0, 1, 2] {
        let (mut f, _, _, _, _, _, plan) = authoring_plan();
        let before = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap();
        let draft = binding_draft(&mut f);
        let reference = BindingPlanRef {
            id: plan.id.clone(),
            revision: plan.revision,
            plan_digest: plan.plan_digest,
        };
        let catalog = f
            .app
            .draft_device_binding_catalog(&f.admin, &f.configuration.id, vec![reference.clone()])
            .unwrap();
        assert_eq!(catalog.device_plans, vec![reference.clone()]);
        assert_eq!(catalog.candidates[0].device_plan, Some(reference.clone()));
        let command = rx_application::draft_bindings::Save {
            device_plans: vec![reference.clone()],
            draft: draft.version.id.clone(),
            cell: f.configuration.id.clone(),
            source_revision: draft.version.revision,
            expected: None,
            catalog_digest: catalog.catalog_digest,
            selections: [(name("load"), name("step/0"))].into(),
        };
        let key = id();
        if fault > 0 {
            f.failure.store(fault, Ordering::SeqCst);
            assert!(
                f.app
                    .save_draft_bindings(&f.admin, &key, command.clone())
                    .is_err()
            );
        }
        let saved = f.app.save_draft_bindings(&f.admin, &key, command).unwrap();
        assert_eq!(saved.revision, Counter(1));
        assert!(saved.complete);
        let exported = f
            .app
            .draft_compile_input(
                &f.admin,
                &f.configuration.id,
                &draft.version.id,
                draft.version.revision,
                saved.revision,
            )
            .unwrap();
        exported.validate().unwrap();
        assert_eq!(exported.schema.as_str(), "rx.process-compile-input.v2");
        let origin = &exported.device_sources[&name("load")];
        assert_eq!(origin.plan, reference);
        assert_eq!(origin.binding, name("step/0"));
        assert_eq!(origin.step_digest, saved.origins[&name("load")]);
        assert_eq!(
            rx_domain::canonical::bytes(&before).unwrap(),
            rx_domain::canonical::bytes(
                &f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap()
            )
            .unwrap()
        );
    }
}
#[test]
fn stale_or_incomplete_device_plans_do_not_export_or_silently_fall_back() {
    use rx_application::draft_bindings::{BindingPlanRef, StaleReason};
    let (mut f, p, a, j, v, _, plan) = authoring_plan();
    let draft = binding_draft(&mut f);
    let reference = BindingPlanRef {
        id: plan.id,
        revision: plan.revision,
        plan_digest: plan.plan_digest,
    };
    assert!(
        f.app
            .draft_device_binding_catalog(
                &f.admin,
                &f.configuration.id,
                vec![reference.clone(), reference.clone()]
            )
            .is_err()
    );
    let catalog = f
        .app
        .draft_device_binding_catalog(&f.admin, &f.configuration.id, vec![reference.clone()])
        .unwrap();
    let saved = f
        .app
        .save_draft_bindings(
            &f.admin,
            &id(),
            rx_application::draft_bindings::Save {
                device_plans: vec![reference],
                draft: draft.version.id.clone(),
                cell: f.configuration.id.clone(),
                source_revision: draft.version.revision,
                expected: None,
                catalog_digest: catalog.catalog_digest,
                selections: [(name("load"), name("step/0"))].into(),
            },
        )
        .unwrap();
    review::submit(&mut f, &p, &a, &j, Some(v.revision), true);
    let view = f
        .app
        .draft_bindings(&f.admin, &f.configuration.id, &draft.version.id, None)
        .unwrap();
    assert!(view.stale.contains(&StaleReason::DevicePlanChanged));
    assert!(view.binding.is_some());
    assert!(
        f.app
            .draft_compile_input(
                &f.admin,
                &f.configuration.id,
                &draft.version.id,
                draft.version.revision,
                saved.revision
            )
            .is_err()
    );
}
