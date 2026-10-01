use super::*;
use rx_application::workflow_model as wm;
use rx_domain::canonical;
use rx_domain::workflow as w;

fn setup(app: &mut App, owner: &Identity, catalog: &Catalog) -> wm::Save {
    fn prop(
        app: &mut App,
        owner: &Identity,
        catalog: &Id,
        kind: ValueType,
        unit: &str,
        category: Category,
        over: bool,
    ) -> rx_domain::definition::Reference {
        let p = Property {
            value_type: kind,
            category,
            constraint_scope: None,
            unit: name(unit),
            minimum: matches!(kind, ValueType::Number).then_some(Real::new(0.0).unwrap()),
            maximum: matches!(kind, ValueType::Number).then_some(Real::new(1000.0).unwrap()),
            choices: vec![],
            vector_length: None,
            overridable: over,
            parameter_mapping: Default::default(),
        };
        put(
            app,
            owner,
            save(catalog, Body::Property { specification: p }),
        )
        .version
        .definition
        .reference
    }
    let target = prop(
        app,
        owner,
        &catalog.id,
        ValueType::Number,
        "mm",
        Category::Execution,
        true,
    );
    let timeout = prop(
        app,
        owner,
        &catalog.id,
        ValueType::Number,
        "s",
        Category::Execution,
        true,
    );
    let flag = prop(
        app,
        owner,
        &catalog.id,
        ValueType::Boolean,
        "unitless",
        Category::Resource,
        false,
    );
    let text = prop(
        app,
        owner,
        &catalog.id,
        ValueType::Text,
        "unitless",
        Category::Resource,
        false,
    );
    let typ = put(
        app,
        owner,
        save(
            &catalog.id,
            Body::ResourceType {
                parent: None,
                fields: [
                    (
                        name("can"),
                        Field {
                            property: flag.clone(),
                            required: true,
                        },
                    ),
                    (
                        name("implementation"),
                        Field {
                            property: text.clone(),
                            required: true,
                        },
                    ),
                    (
                        name("version"),
                        Field {
                            property: text,
                            required: true,
                        },
                    ),
                ]
                .into(),
            },
        ),
    )
    .version
    .definition
    .reference;
    let actor = put(
        app,
        owner,
        save(
            &catalog.id,
            Body::ResourceModel {
                resource_type: typ.clone(),
                values: [
                    (name("can"), Value::Boolean(true)),
                    (name("implementation"), Value::Text("simulated".into())),
                    (name("version"), Value::Text("1".into())),
                ]
                .into(),
            },
        ),
    )
    .version
    .definition
    .reference;
    let number = |v, unit| serde_json::json!({"unit":unit,"data":{"kind":"NUMBER","range":{"min":v,"max":v}}});
    let spec=serde_json::from_value(serde_json::json!({
        "schema":"rx.workflow-model.v1",
        "contexts":{"actor":{"label":"Actor","kind":"RESOURCE","accepted_types":[typ],"required":true,"multiple":false}},
        "defaults":{"actor":[actor]},"property_sets":[],"rules":{},
        "constraints":{"limit":{"left":{"kind":"PROPERTY","name":"target"},"right":{"kind":"LITERAL","value":number(50,"mm")},"relation":"LE","property":"target","message":"maximum target"}},
        "tasks":{"act":{"label":"Act","contexts":["actor"],"properties":{
            "target":{"property":target,"sources":[{"kind":"OVERRIDE"},{"kind":"DEFAULT"}],"default":number(20,"mm")},
            "timeout":{"property":timeout,"sources":[{"kind":"DEFAULT"}],"default":number(10,"s")}},
            "constraints":["limit"],"capabilities":[{"name":"act","slot":"actor","field":"can"}],
            "skills":[{"capability":"act","slot":"actor","implementation_field":"implementation","version_field":"version","primitive":"set","parameters":{"value":"target"}}],
            "timeout_property":"timeout","done":{"observation":"done","property":flag,"equals":{"unit":"unitless","data":{"kind":"BOOLEAN","value":true}}},"on_failure":"STOP","on_unknown":"HOLD_AND_RECONCILE"}},
        "steps":[{"id":"node","task":"act"}]
    })).unwrap();
    wm::Save {
        catalog: catalog.id.clone(),
        id: id(),
        expected: None,
        label: "Workflow".into(),
        spec,
    }
}
fn query(v: &wm::Version) -> w::Request {
    w::Request {
        workflow: v.reference.clone(),
        contexts: Default::default(),
        property_sets: vec![],
        overrides: Default::default(),
        inputs: Default::default(),
        slot_index: Counter(0),
    }
}
fn prepare(app: &mut App, actor: &Identity, key: &Id, input: w::Request) -> wm::PreparedResolution {
    match app.prepare_workflow_resolution(actor, key, input).unwrap() {
        wm::Preparation::Pending(snapshot) => wm::PreparedResolution::prepare(*snapshot).unwrap(),
        wm::Preparation::Recorded(_) => panic!("expected new snapshot"),
    }
}
#[test]
fn workflow_and_resolution_recover_real_transaction_boundaries_and_keep_history() {
    for mode in [1, 2] {
        let (_dir, mut app, owner, failure) = standalone();
        let catalog = app
            .save_definition_catalog(&owner, &id(), catalog_save())
            .unwrap();
        let input = setup(&mut app, &owner, &catalog);
        let key = id();
        let old_definitions = app
            .definitions(&owner, &catalog.id, None, &Filter::default())
            .unwrap()
            .definitions
            .len();
        failure.store(mode, Ordering::SeqCst);
        assert!(
            app.save_workflow_model(
                &owner,
                &key,
                wm::PreparedSave::prepare(input.clone()).unwrap()
            )
            .is_err()
        );
        let model = app
            .save_workflow_model(
                &owner,
                &key,
                wm::PreparedSave::prepare(input.clone()).unwrap(),
            )
            .unwrap();
        assert_eq!(model.reference.revision, Counter(1));
        assert_eq!(
            app.definitions(&owner, &catalog.id, None, &Filter::default())
                .unwrap()
                .definitions
                .len(),
            old_definitions
        );
        let request = query(&model);
        let resolve_key = id();
        let prepared = prepare(&mut app, &owner, &resolve_key, request.clone());
        failure.store(mode, Ordering::SeqCst);
        assert!(
            app.save_workflow_resolution(&owner, &resolve_key, prepared)
                .is_err()
        );
        let receipt = match app
            .prepare_workflow_resolution(&owner, &resolve_key, request.clone())
            .unwrap()
        {
            wm::Preparation::Recorded(v) => {
                assert_eq!(mode, 2);
                *v
            }
            wm::Preparation::Pending(s) => {
                assert_eq!(mode, 1);
                app.save_workflow_resolution(
                    &owner,
                    &resolve_key,
                    wm::PreparedResolution::prepare(*s).unwrap(),
                )
                .unwrap()
            }
        };
        assert!(receipt.report.valid && receipt.report.concrete);
        let reports = app.workflow_resolutions(&owner, &catalog.id, None).unwrap();
        assert_eq!(reports.reports.len(), 1);
        assert_eq!(reports.reports[0].reference, receipt.reference);

        let read = app
            .workflow_resolution(&owner, &catalog.id, &receipt.reference.id)
            .unwrap();
        assert_eq!(
            canonical::bytes(&read).unwrap(),
            canonical::bytes(&receipt).unwrap()
        );
        let mut updated = input.clone();
        updated.expected = Some(Counter(1));
        updated.label = "Renamed workflow".into();
        let revised = app
            .save_workflow_model(&owner, &id(), wm::PreparedSave::prepare(updated).unwrap())
            .unwrap();
        assert_eq!(revised.reference.revision, Counter(2));
        assert_eq!(
            app.workflow_model(&owner, &catalog.id, &input.id, Some(Counter(1)))
                .unwrap()
                .label,
            "Workflow"
        );
        match app
            .prepare_workflow_resolution(&owner, &resolve_key, request)
            .unwrap()
        {
            wm::Preparation::Recorded(v) => assert_eq!(v.reference, receipt.reference),
            _ => panic!("original result missing"),
        }
        assert!(app.pending_deliveries(128).unwrap().is_empty());
    }
}
#[test]
fn resolution_rechecks_access_after_cpu_work_and_rejects_forged_references() {
    let (_dir, mut app, owner, _) = standalone();
    let editor = add_identity(&mut app, &owner, "editor", &[Role::Engineer]);
    let reader = add_identity(&mut app, &owner, "reader", &[Role::Verifier]);
    let mut input = catalog_save();
    input.members = [
        (editor.principal.clone(), Access::Edit),
        (reader.principal.clone(), Access::Read),
    ]
    .into();
    let catalog = app.save_definition_catalog(&owner, &id(), input).unwrap();
    let input = setup(&mut app, &owner, &catalog);
    let model = app
        .save_workflow_model(
            &editor,
            &id(),
            wm::PreparedSave::prepare(input.clone()).unwrap(),
        )
        .unwrap();
    let request = query(&model);
    assert!(
        app.prepare_workflow_resolution(&reader, &id(), request.clone())
            .is_err()
    );
    let key = id();
    let prepared = prepare(&mut app, &editor, &key, request.clone());
    let mut revoke = revise(&catalog);
    revoke.members.remove(&editor.principal);
    app.save_definition_catalog(&owner, &id(), revoke).unwrap();
    assert!(
        app.save_workflow_resolution(&editor, &key, prepared)
            .is_err()
    );
    assert!(
        app.prepare_workflow_resolution(&editor, &key, request.clone())
            .is_err()
    );
    let mut tampered = request.clone();
    tampered.workflow.digest = Digest::from_bytes([44; 32]);
    assert!(
        app.prepare_workflow_resolution(&owner, &id(), tampered)
            .is_err()
    );
    let other = app
        .save_definition_catalog(&owner, &id(), catalog_save())
        .unwrap();
    let mut crossed = request;
    crossed.workflow.catalog = other.id;
    assert!(
        app.prepare_workflow_resolution(&owner, &id(), crossed)
            .is_err()
    );
    assert!(
        app.workflow_model(&reader, &catalog.id, &input.id, None)
            .is_ok()
    );
}
