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

#[test]
fn execution_inputs_are_current_server_reads_and_do_not_create_resolution_receipts() {
    let (_dir, mut app, owner, _) = standalone();
    let catalog = app
        .save_definition_catalog(&owner, &id(), catalog_save())
        .unwrap();
    let input = setup(&mut app, &owner, &catalog);
    let model = app
        .save_workflow_model(&owner, &id(), wm::PreparedSave::prepare(input).unwrap())
        .unwrap();
    let request = query(&model);
    let snapshot = app
        .prepare_execution_inputs(&owner, vec![request.clone()], 2400)
        .unwrap();
    let report = snapshot.resolve(0, 0).unwrap();
    assert!(report.valid && report.concrete);
    assert_eq!(report.request.workflow, model.reference);
    assert_eq!(
        snapshot.resolve(0, 2399).unwrap().request.slot_index,
        Counter(2399)
    );
    assert!(snapshot.resolve(1, 0).is_err());
    assert!(snapshot.resolve(0, 2400).is_err());
    assert!(
        app.workflow_resolutions(&owner, &catalog.id, None)
            .unwrap()
            .reports
            .is_empty()
    );

    // An unrelated current definition does not invalidate this closure.
    put(
        &mut app,
        &owner,
        save(&catalog.id, property(Category::Resource, false)),
    );
    assert!(
        app.prepare_execution_inputs(&owner, vec![request.clone()], 1)
            .is_ok()
    );
    let reference = report.definitions[0].reference.clone();
    let original = app
        .definition(&owner, &catalog.id, &reference.id, None)
        .unwrap();
    let mut revised = save(&catalog.id, original.version.definition.body);
    revised.id = reference.id;
    revised.expected = Some(reference.revision);
    revised.label = "New revision with identical values".into();
    put(&mut app, &owner, revised);
    let error = match app.prepare_execution_inputs(&owner, vec![request], 1) {
        Ok(_) => panic!("stale value-bearing definition accepted"),
        Err(e) => e.to_string(),
    };
    assert!(error.contains("STALE_EXECUTION_REFERENCE"), "{error}");
    assert!(
        error.contains("pinned 1") && error.contains("current 2"),
        "{error}"
    );
    // A precomputation snapshot stays reproducible, but is not execution admission.
    assert_eq!(
        canonical::bytes(&snapshot.resolve(0, 0).unwrap()).unwrap(),
        canonical::bytes(&report).unwrap()
    );
    assert!(app.pending_deliveries(128).unwrap().is_empty());
}

#[test]
fn execution_input_cut_rejects_unauthorized_stale_and_unbounded_requests() {
    let (_dir, mut app, owner, _) = standalone();
    let reader = add_identity(&mut app, &owner, "read-v2", &[Role::Verifier]);
    let mut catalog_input = catalog_save();
    catalog_input
        .members
        .insert(reader.principal.clone(), Access::Read);
    let catalog = app
        .save_definition_catalog(&owner, &id(), catalog_input)
        .unwrap();
    let mut input = setup(&mut app, &owner, &catalog);
    let model = app
        .save_workflow_model(
            &owner,
            &id(),
            wm::PreparedSave::prepare(input.clone()).unwrap(),
        )
        .unwrap();
    let request = query(&model);
    assert!(
        app.prepare_execution_inputs(&reader, vec![request.clone()], 1)
            .is_err()
    );
    assert!(app.prepare_execution_inputs(&owner, vec![], 1).is_err());
    assert!(
        app.prepare_execution_inputs(&owner, vec![request.clone(); 9], 1)
            .is_err()
    );
    assert!(
        app.prepare_execution_inputs(&owner, vec![request.clone(); 2], 1)
            .is_err()
    );
    for slots in [0, 2401] {
        assert!(
            app.prepare_execution_inputs(&owner, vec![request.clone()], slots)
                .is_err()
        );
    }
    let mut reordered = request.clone();
    reordered.slot_index = Counter(1);
    assert!(
        app.prepare_execution_inputs(&owner, vec![reordered], 1)
            .is_err()
    );
    input.expected = Some(Counter(1));
    input.label = "Changed workflow".into();
    app.save_workflow_model(&owner, &id(), wm::PreparedSave::prepare(input).unwrap())
        .unwrap();
    assert!(
        app.prepare_execution_inputs(&owner, vec![request], 1)
            .is_err()
    );
}

struct ExecutionFixture {
    directory: tempfile::TempDir,
    app: App,
    owner: Identity,
    failure: Arc<AtomicU8>,
    catalog: Catalog,
    policy: rx_process_contract::execution_v2::Policy,
    requests: Vec<w::Request>,
    snapshot: wm::ExecutionSnapshot,
}
fn execution_fixture() -> ExecutionFixture {
    use rx_domain::intent::{Body as IntentBody, Intent, Kind, ProgramGoal};
    use rx_process_contract::{ActionBinding, execution_v2 as v2};
    let artifact = |schema: &str, bytes: &[u8]| ArtifactRef {
        schema_id: name(schema),
        sha256: rx_package::content_digest(bytes),
        size_bytes: Counter(bytes.len() as u64),
    };
    let (directory, mut app, owner, failure) = standalone();
    let catalog = app
        .save_definition_catalog(&owner, &id(), catalog_save())
        .unwrap();
    let mut input = setup(&mut app, &owner, &catalog);
    let typ = put(
        &mut app,
        &owner,
        save(
            &catalog.id,
            Body::ObjectType {
                parent: None,
                fields: Default::default(),
            },
        ),
    )
    .version
    .definition
    .reference;
    let object = put(
        &mut app,
        &owner,
        save(
            &catalog.id,
            Body::ObjectModel {
                object_type: typ.clone(),
                values: Default::default(),
            },
        ),
    )
    .version
    .definition
    .reference;
    input.spec.contexts.insert(
        name("object"),
        rx_domain::definition::Slot {
            label: "Actual object model".into(),
            kind: rx_domain::definition::SlotKind::Object,
            accepted_types: vec![typ],
            required: true,
            multiple: false,
        },
    );
    input
        .spec
        .defaults
        .insert(name("object"), vec![object.clone()]);
    let model = app
        .save_workflow_model(&owner, &id(), wm::PreparedSave::prepare(input).unwrap())
        .unwrap();
    let a = query(&model);
    let mut b = a.clone();
    b.overrides.insert(name("node"),[(name("target"),serde_json::from_value(serde_json::json!({"unit":"mm","data":{"kind":"NUMBER","range":{"min":40,"max":40}}})).unwrap())].into());
    let snapshot = app
        .prepare_execution_inputs(&owner, vec![a.clone(), b.clone()], 2)
        .unwrap();
    let inputs = snapshot.input_closure();
    let p = v2::Policy {
        schema: name(v2::POLICY_SCHEMA),
        workflow: model.reference,
        definition_closure: inputs.artifact().unwrap(),
        resolver_digest: snapshot.resolve(0, 0).unwrap().resolver_digest,
        compiler_digest: v2::compiler_digest(),
        candidates: [&a, &b]
            .iter()
            .enumerate()
            .map(|(i, r)| v2::Candidate {
                key: name(&format!("candidate/{i}")),
                object_model: object.clone(),
                context_digest: v2::context_digest(r).unwrap(),
            })
            .collect(),
        slot_order: vec![0, 1],
        templates: [(
            name("node"),
            ActionBinding {
                host: name("sim-host"),
                intent: Intent {
                    kind: Kind::FiniteAction,
                    target: name("device"),
                    profile_digest: Digest::from_bytes([1; 32]),
                    site_config_digest: Digest::from_bytes([2; 32]),
                    calibration_digests: vec![],
                    resource_set: vec![name("resource")],
                    execution_timeout_ms: Counter(30000),
                    prepare_validity_ms: Counter(1000),
                    completion_rule: name("done"),
                    cancel_rule: name("stop"),
                    body: IntentBody::Program(ProgramGoal {
                        program: artifact("fixture.program.v1", b"program"),
                        parameter_set: artifact(v2::PARAMETER_SCHEMA, b"template"),
                    }),
                },
            },
        )]
        .into(),
        node_contracts: [(
            name("node"),
            v2::NodeContract {
                implementation: "simulated".into(),
                version: "1".into(),
                primitive: name("set"),
                parameters: [(
                    name("value"),
                    v2::ParameterContract {
                        unit: name("mm"),
                        value_type: ValueType::Number,
                        frame: None,
                    },
                )]
                .into(),
            },
        )]
        .into(),
        report_index: artifact(v2::INDEX_SCHEMA, b"pending-not-approved"),
    };
    ExecutionFixture {
        directory,
        app,
        owner,
        failure,
        catalog,
        policy: p,
        requests: vec![a, b],
        snapshot,
    }
}
#[test]
fn execution_materialization_recomputes_stored_values_and_matches_preapproved_reports() {
    use rx_process_contract::execution_v2 as v2;
    let artifact = |schema: &str, bytes: &[u8]| ArtifactRef {
        schema_id: name(schema),
        sha256: rx_package::content_digest(bytes),
        size_bytes: Counter(bytes.len() as u64),
    };
    let ExecutionFixture {
        directory: _dir,
        mut app,
        owner,
        policy: mut p,
        requests,
        snapshot,
        ..
    } = execution_fixture();
    let [a, b]: [w::Request; 2] = requests.try_into().unwrap();
    let inputs = snapshot.input_closure();
    let mut index = v2::ReportIndex {
        schema: name(v2::INDEX_SCHEMA),
        entries: vec![],
    };
    for candidate in 0..2 {
        for slot in 0..2 {
            let generated = snapshot.materialize(&p, candidate, slot).unwrap();
            let parameter: serde_json::Value =
                serde_json::from_slice(&generated.parameters()[&name("node")]).unwrap();
            assert_eq!(
                parameter["values"]["value"]["value"]["data"]["range"]["min"],
                if candidate == 0 { 20 } else { 40 }
            );
            assert_eq!(parameter["slot"], slot);
            assert_eq!(
                generated.actions()[&name("node")]
                    .intent
                    .execution_timeout_ms,
                Counter(30000),
                "fixed Intent timeout must not change to derived 10000 ms"
            );
            index
                .entries
                .push((candidate, slot, generated.report_digest()));
        }
    }
    let bytes = canonical::bytes(&index).unwrap();
    p.report_index = artifact(v2::INDEX_SCHEMA, &bytes);
    let approved = v2::ReportIndex::decode(&bytes, &p).unwrap();
    let generated = snapshot.materialize(&p, 0, 0).unwrap();
    generated.verify_index(&p, &approved, 0, 0).unwrap();
    assert!(generated.verify_index(&p, &approved, 0, 1).is_err());
    assert!(generated.verify_index(&p, &approved, 1, 0).is_err());
    let reread = app
        .prepare_execution_inputs(&owner, vec![a.clone(), b], 2)
        .unwrap();
    assert_eq!(
        reread.materialize(&p, 0, 0).unwrap().report(),
        generated.report()
    );

    for (unit, min, max) in [("mm", 60, 60), ("mm", 20, 40), ("kg", 20, 20)] {
        let mut changed = a.clone();
        changed.overrides.insert(name("node"),[(name("target"),serde_json::from_value(serde_json::json!({"unit":unit,"data":{"kind":"NUMBER","range":{"min":min,"max":max}}})).unwrap())].into());
        let changed = app
            .prepare_execution_inputs(&owner, vec![changed], 2)
            .unwrap();
        let closure = changed.input_closure();
        let mut policy = p.clone();
        policy.candidates.truncate(1);
        policy.candidates[0].context_digest = v2::context_digest(&closure.requests[0]).unwrap();
        policy.definition_closure = closure.artifact().unwrap();
        assert!(
            changed.materialize(&policy, 0, 0).is_err(),
            "{unit} {min}..{max}"
        );
    }
    let mut bad = p.clone();
    bad.node_contracts
        .get_mut(&name("node"))
        .unwrap()
        .parameters
        .get_mut(&name("value"))
        .unwrap()
        .frame = Some("WRONG_FRAME".into());
    assert!(snapshot.materialize(&bad, 0, 0).is_err());
    let mut bad = p.clone();
    bad.compiler_digest = Digest::from_bytes([9; 32]);
    assert!(snapshot.materialize(&bad, 0, 0).is_err());
    let mut bad = p.clone();
    bad.resolver_digest = Digest::from_bytes([9; 32]);
    assert!(snapshot.materialize(&bad, 0, 0).is_err());
    let mut missing = inputs;
    missing.definitions.pop();
    let mut bad = p;
    bad.definition_closure = missing.artifact().unwrap();
    assert!(v2::materialize(&bad, &missing, 0, 0).is_err());
}

impl ExecutionFixture {
    fn preview_input(&self) -> rx_application::workflow_publication::PreviewInput {
        use rx_application::workflow_publication::{CandidateInput, PreviewInput};
        PreviewInput {
            id: id(),
            slots: 2,
            candidates: self
                .requests
                .iter()
                .zip(&self.policy.candidates)
                .map(|(request, c)| CandidateInput {
                    key: c.key.clone(),
                    object_model: c.object_model.clone(),
                    request: request.clone(),
                })
                .collect(),
            templates: self.policy.templates.clone(),
            node_contracts: self.policy.node_contracts.clone(),
        }
    }
    fn prepare_preview(
        &mut self,
        key: &Id,
        input: rx_application::workflow_publication::PreviewInput,
    ) -> rx_application::workflow_publication::PreparedPreview {
        use rx_application::workflow_publication::{Preparation, PreparedPreview};
        match self
            .app
            .prepare_execution_preview(&self.owner, key, input)
            .unwrap()
        {
            Preparation::Pending(work) => PreparedPreview::prepare(*work).unwrap(),
            Preparation::Recorded(_) => panic!("expected new preparation"),
        }
    }
    fn revise_dependency(&mut self) {
        let definition = self.snapshot.input_closure().definitions[0].clone();
        let mut changed = save(&self.catalog.id, definition.body);
        changed.id = definition.reference.id;
        changed.expected = Some(definition.reference.revision);
        changed.label = "Revision changed while values stayed the same".into();
        put(&mut self.app, &self.owner, changed);
    }
}

#[test]
fn execution_preview_and_publication_recover_commit_loss_and_reopen_immutable_bytes() {
    use rx_application::workflow_publication::{Preparation, Publish};
    for mode in [1, 2] {
        let mut f = execution_fixture();
        let input = f.preview_input();
        let key = id();
        let prepared = f.prepare_preview(&key, input.clone());
        f.failure.store(mode, Ordering::SeqCst);
        assert!(
            f.app
                .save_execution_preview(&f.owner, &key, prepared)
                .is_err()
        );
        let preview = match f
            .app
            .prepare_execution_preview(&f.owner, &key, input.clone())
            .unwrap()
        {
            Preparation::Recorded(record) => {
                assert_eq!(mode, 2);
                *record
            }
            Preparation::Pending(work) => {
                assert_eq!(mode, 1);
                let prepared =
                    rx_application::workflow_publication::PreparedPreview::prepare(*work).unwrap();
                f.app
                    .save_execution_preview(&f.owner, &key, prepared)
                    .unwrap()
            }
        };
        let original = f
            .app
            .execution_preview(&f.owner, &preview.reference)
            .unwrap()
            .report(1, 1)
            .unwrap()
            .report()
            .to_vec();
        let publish = Publish {
            id: id(),
            preview: preview.reference.clone(),
        };
        let publish_key = id();
        f.failure.store(mode, Ordering::SeqCst);
        assert!(
            f.app
                .publish_workflow_execution(&f.owner, &publish_key, publish.clone())
                .is_err()
        );
        let publication = f
            .app
            .publish_workflow_execution(&f.owner, &publish_key, publish.clone())
            .unwrap();
        assert_eq!(publication.policy, preview.policy);
        assert_eq!(publication.preview, preview.reference);
        assert_eq!(publication.reference.revision, Counter(1));
        assert!(
            f.app
                .publish_workflow_execution(&f.owner, &id(), publish.clone())
                .is_err(),
            "immutable ID must not be overwritten"
        );
        let mut conflict = input.clone();
        conflict.slots = 1;
        assert!(
            f.app
                .prepare_execution_preview(&f.owner, &key, conflict)
                .is_err(),
            "same request key with changed input"
        );

        let installation = f.app.installation.id.clone();
        let owner_id = f.owner.principal.clone();
        drop(f.app);
        let repository = FaultRepository {
            inner: SqliteRepository::open(f.directory.path().join("definitions.db")).unwrap(),
            mode: f.failure,
        };
        let mut bootstrap = principal("owner", &[Role::Engineer, Role::AccountAdmin]);
        bootstrap.cells.clear();
        let mut app = Engine::open(
            repository,
            ManualClock(Arc::new(AtomicU64::new(1000))),
            SimulationAuthority,
            installation,
            bootstrap,
        )
        .unwrap();
        let session = app
            .authenticated_session(&owner_id, id(), expiry(100000))
            .unwrap();
        let owner = Identity {
            principal: owner_id,
            session: session.id,
            terminal: None,
        };
        let read = app
            .workflow_publication(&owner, &publication.reference)
            .unwrap();
        assert_eq!(
            canonical::bytes(&read).unwrap(),
            canonical::bytes(&publication).unwrap()
        );
        assert_eq!(
            app.execution_preview(&owner, &preview.reference)
                .unwrap()
                .report(1, 1)
                .unwrap()
                .report(),
            original
        );
        assert!(
            app.pending_deliveries(128).unwrap().is_empty(),
            "publication must not enqueue a device effect"
        );
    }
}

#[test]
fn execution_publication_rechecks_currentness_and_access_without_rewriting_old_preview() {
    use rx_application::workflow_publication::{Preparation, Publish};
    let mut f = execution_fixture();
    let input = f.preview_input();
    let key = id();
    let prepared = f.prepare_preview(&key, input.clone());
    f.revise_dependency();
    assert!(
        f.app
            .save_execution_preview(&f.owner, &key, prepared)
            .is_err(),
        "change during CPU work must block commit"
    );

    let mut f = execution_fixture();
    let input = f.preview_input();
    let key = id();
    let prepared = f.prepare_preview(&key, input.clone());
    let preview = f
        .app
        .save_execution_preview(&f.owner, &key, prepared)
        .unwrap();
    let before = f
        .app
        .execution_preview(&f.owner, &preview.reference)
        .unwrap()
        .report(0, 0)
        .unwrap()
        .report()
        .to_vec();
    f.revise_dependency();
    assert!(
        f.app
            .publish_workflow_execution(
                &f.owner,
                &id(),
                Publish {
                    id: id(),
                    preview: preview.reference.clone()
                }
            )
            .is_err()
    );
    assert_eq!(
        f.app
            .execution_preview(&f.owner, &preview.reference)
            .unwrap()
            .report(0, 0)
            .unwrap()
            .report(),
        before
    );
    assert!(
        matches!(
            f.app
                .prepare_execution_preview(&f.owner, &key, input)
                .unwrap(),
            Preparation::Recorded(_)
        ),
        "recover original result even after revision changes"
    );
    let mut forged = preview.reference;
    forged.digest = Digest::from_bytes([7; 32]);
    assert!(f.app.execution_preview(&f.owner, &forged).is_err());

    let mut f = execution_fixture();
    let editor = add_identity(&mut f.app, &f.owner, "preview-author", &[Role::Engineer]);
    let mut membership = revise(&f.catalog);
    membership
        .members
        .insert(editor.principal.clone(), Access::Edit);
    let catalog = f
        .app
        .save_definition_catalog(&f.owner, &id(), membership)
        .unwrap();
    let input = f.preview_input();
    let key = id();
    let Preparation::Pending(work) = f
        .app
        .prepare_execution_preview(&editor, &key, input)
        .unwrap()
    else {
        panic!("new")
    };
    let prepared = rx_application::workflow_publication::PreparedPreview::prepare(*work).unwrap();
    let mut membership = revise(&catalog);
    membership.members.remove(&editor.principal);
    f.app
        .save_definition_catalog(&f.owner, &id(), membership)
        .unwrap();
    assert!(
        f.app
            .save_execution_preview(&editor, &key, prepared)
            .is_err()
    );
}
