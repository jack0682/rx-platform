use super::*;
use rx_application::definition_catalog::{Access, Catalog, CatalogSave, Filter, Prepared, Save};
use rx_domain::definition::{Body, Category, Field, Property, Value, ValueType};

fn standalone() -> (tempfile::TempDir, App, Identity, Arc<AtomicU8>) {
    let directory = tempfile::tempdir().unwrap();
    let failure = Arc::new(AtomicU8::new(0));
    let repository = FaultRepository {
        inner: SqliteRepository::open(directory.path().join("definitions.db")).unwrap(),
        mode: failure.clone(),
    };
    let mut owner = principal("owner", &[Role::Engineer, Role::AccountAdmin]);
    owner.cells.clear();
    let mut app = Engine::open(
        repository,
        ManualClock(Arc::new(AtomicU64::new(1000))),
        SimulationAuthority,
        id(),
        owner,
    )
    .unwrap();
    let session = app
        .authenticated_session(&name("owner"), id(), expiry(100000))
        .unwrap();
    (
        directory,
        app,
        Identity {
            principal: name("owner"),
            session: session.id,
            terminal: None,
        },
        failure,
    )
}
fn catalog_save() -> CatalogSave {
    CatalogSave {
        id: id(),
        expected: None,
        title: "Laser authoring".into(),
        members: Default::default(),
        terminals: Default::default(),
        archived: false,
    }
}
fn revise(c: &Catalog) -> CatalogSave {
    CatalogSave {
        id: c.id.clone(),
        expected: Some(c.revision),
        title: c.title.clone(),
        members: c.members.clone(),
        terminals: c.terminals.clone(),
        archived: c.archived,
    }
}
fn save(catalog: &Id, body: Body) -> Save {
    Save {
        catalog: catalog.clone(),
        id: id(),
        expected: None,
        label: "Definition".into(),
        body,
        archived: false,
    }
}
fn property(category: Category, overridable: bool) -> Body {
    Body::Property {
        specification: Property {
            value_type: ValueType::Number,
            category,
            constraint_scope: (category == Category::Constraint)
                .then_some(rx_domain::definition::ConstraintScope::Object),
            parameter_mapping: Default::default(),
            unit: name("mm"),
            minimum: Some(Real::new(0.0).unwrap()),
            maximum: Some(Real::new(3000.0).unwrap()),
            choices: vec![],
            vector_length: None,
            overridable,
        },
    }
}
fn number(v: f64) -> Value {
    Value::Number(Real::new(v).unwrap())
}
fn put(app: &mut App, owner: &Identity, input: Save) -> rx_application::definition_catalog::View {
    app.save_definition(owner, &id(), Prepared::prepare(input).unwrap())
        .unwrap()
}

#[test]
fn catalog_and_definition_recover_original_commits_without_a_cell() {
    for mode in [1, 2] {
        let (_dir, mut app, owner, failure) = standalone();
        let create = catalog_save();
        let key = id();
        failure.store(mode, Ordering::SeqCst);
        assert!(
            app.save_definition_catalog(&owner, &key, create.clone())
                .is_err()
        );
        if mode == 1 {
            assert!(app.definition_catalog(&owner, &create.id).is_err());
        }
        let catalog = app
            .save_definition_catalog(&owner, &key, create.clone())
            .unwrap();
        assert_eq!(catalog.revision, Counter(1));
        let input = save(&catalog.id, property(Category::Resource, false));
        let request = id();
        failure.store(mode, Ordering::SeqCst);
        assert!(
            app.save_definition(&owner, &request, Prepared::prepare(input.clone()).unwrap())
                .is_err()
        );
        let value = app
            .save_definition(&owner, &request, Prepared::prepare(input.clone()).unwrap())
            .unwrap();
        assert_eq!(value.version.definition.reference.revision, Counter(1));
        assert_eq!(
            app.definition_history(&owner, &catalog.id, &input.id, None)
                .unwrap()
                .versions
                .len(),
            1
        );
        assert_eq!(
            app.definition_catalogs(&owner, None)
                .unwrap()
                .catalogs
                .len(),
            1
        );
        let mut changed = input;
        changed.label = "Different request content".into();
        assert!(matches!(
            app.save_definition(&owner, &request, Prepared::prepare(changed).unwrap()),
            Err(StoreError::KeyConflict)
        ));
        assert!(app.pending_deliveries(128).unwrap().is_empty());
    }
}

#[test]
fn tray_instances_preserve_pinned_model_values_overrides_and_missing_values() {
    let (_dir, mut app, owner, _) = standalone();
    let catalog = app
        .save_definition_catalog(&owner, &id(), catalog_save())
        .unwrap();
    let prop = put(
        &mut app,
        &owner,
        save(&catalog.id, property(Category::Resource, false)),
    );
    let typ = put(
        &mut app,
        &owner,
        save(
            &catalog.id,
            Body::ResourceType {
                parent: None,
                fields: [(
                    name("height"),
                    Field {
                        property: prop.version.definition.reference.clone(),
                        required: true,
                    },
                )]
                .into(),
            },
        ),
    );
    let mut model_input = save(
        &catalog.id,
        Body::ResourceModel {
            resource_type: typ.version.definition.reference.clone(),
            values: [(name("height"), number(900.0))].into(),
        },
    );
    let model = put(&mut app, &owner, model_input.clone());
    let a = put(
        &mut app,
        &owner,
        save(
            &catalog.id,
            Body::ResourceInstance {
                base: model.version.definition.reference.clone(),
                values: [(name("height"), number(760.0))].into(),
            },
        ),
    );
    assert_eq!(a.effective.values[&name("height")].value, number(760.0));
    assert_eq!(
        a.effective.values[&name("height")].declared_by,
        a.version.definition.reference
    );
    assert_eq!(
        a.effective.shadowed[&name("height")][0].value,
        number(900.0)
    );
    assert_eq!(
        a.effective.shadowed[&name("height")][0].declared_by,
        model.version.definition.reference
    );
    let b = put(
        &mut app,
        &owner,
        save(
            &catalog.id,
            Body::ResourceInstance {
                base: model.version.definition.reference.clone(),
                values: Default::default(),
            },
        ),
    );
    model_input.expected = Some(Counter(1));
    if let Body::ResourceModel { values, .. } = &mut model_input.body {
        values.insert(name("height"), number(950.0));
    }
    let updated = put(&mut app, &owner, model_input);
    assert_eq!(updated.version.definition.reference.revision, Counter(2));
    assert_eq!(
        app.definition(
            &owner,
            &catalog.id,
            &b.version.definition.reference.id,
            None
        )
        .unwrap()
        .effective
        .values[&name("height")]
            .value,
        number(900.0)
    );
    let direct = put(
        &mut app,
        &owner,
        save(
            &catalog.id,
            Body::ResourceInstance {
                base: typ.version.definition.reference.clone(),
                values: Default::default(),
            },
        ),
    );
    assert_eq!(direct.effective.missing, vec![name("height")]);
    assert!(direct.effective.values.is_empty());
    let invalid = save(
        &catalog.id,
        Body::ResourceInstance {
            base: typ.version.definition.reference.clone(),
            values: [(name("height"), Value::Text("900".into()))].into(),
        },
    );
    assert!(
        app.save_definition(&owner, &id(), Prepared::prepare(invalid).unwrap())
            .is_err()
    );
    let mut altered = typ.version.definition.reference.clone();
    altered.digest = Digest::from_bytes([55; 32]);
    let wrong = save(
        &catalog.id,
        Body::ResourceModel {
            resource_type: altered,
            values: Default::default(),
        },
    );
    assert!(
        app.save_definition(&owner, &id(), Prepared::prepare(wrong).unwrap())
            .is_err()
    );
}

#[test]
fn membership_revocation_and_catalog_reference_isolation_apply_before_replay() {
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
    assert_eq!(
        app.definition_catalogs(&reader, None)
            .unwrap()
            .catalogs
            .len(),
        1
    );
    let definition = save(&catalog.id, property(Category::Object, false));
    let key = id();
    assert!(
        app.save_definition(
            &reader,
            &key,
            Prepared::prepare(definition.clone()).unwrap()
        )
        .is_err()
    );
    let saved = app
        .save_definition(
            &editor,
            &key,
            Prepared::prepare(definition.clone()).unwrap(),
        )
        .unwrap();
    let mut revoke = revise(&catalog);
    revoke.members.remove(&editor.principal);
    app.save_definition_catalog(&owner, &id(), revoke).unwrap();
    assert!(
        app.save_definition(&editor, &key, Prepared::prepare(definition).unwrap())
            .is_err()
    );
    assert!(
        app.definition_catalogs(&editor, None)
            .unwrap()
            .catalogs
            .is_empty()
    );
    assert!(
        app.definition(
            &editor,
            &catalog.id,
            &saved.version.definition.reference.id,
            None
        )
        .is_err()
    );
    assert!(
        app.definition(
            &reader,
            &catalog.id,
            &saved.version.definition.reference.id,
            None
        )
        .is_ok()
    );
    let other = app
        .save_definition_catalog(&owner, &id(), catalog_save())
        .unwrap();
    let cross = save(
        &other.id,
        Body::ObjectType {
            parent: None,
            fields: [(
                name("width"),
                Field {
                    property: saved.version.definition.reference,
                    required: true,
                },
            )]
            .into(),
        },
    );
    assert!(
        app.save_definition(&owner, &id(), Prepared::prepare(cross).unwrap())
            .is_err()
    );
    assert!(
        app.definitions(&reader, &other.id, None, &Filter::default())
            .is_err()
    );
}

#[test]
fn workflow_overrides_cannot_weaken_constraint_or_locked_property_definitions() {
    let (_dir, mut app, owner, _) = standalone();
    let catalog = app
        .save_definition_catalog(&owner, &id(), catalog_save())
        .unwrap();
    let constraint = put(
        &mut app,
        &owner,
        save(&catalog.id, property(Category::Constraint, false)),
    );
    let set = save(
        &catalog.id,
        Body::PropertySet {
            values: [(
                name("maximum"),
                rx_domain::definition::Assignment {
                    property: constraint.version.definition.reference,
                    value: number(20.0),
                },
            )]
            .into(),
        },
    );
    assert!(
        app.save_definition(&owner, &id(), Prepared::prepare(set).unwrap())
            .is_err()
    );
    assert!(Prepared::prepare(save(&catalog.id, property(Category::Constraint, true))).is_err());
    let prop = put(
        &mut app,
        &owner,
        save(&catalog.id, property(Category::Execution, true)),
    );
    let set = put(
        &mut app,
        &owner,
        save(
            &catalog.id,
            Body::PropertySet {
                values: [(
                    name("speed"),
                    rx_domain::definition::Assignment {
                        property: prop.version.definition.reference,
                        value: number(20.0),
                    },
                )]
                .into(),
            },
        ),
    );
    assert_eq!(set.effective.values[&name("speed")].value, number(20.0));
}

#[test]
fn task_slots_and_property_sources_preserve_authoring_metadata() {
    use rx_domain::definition::{Services, Slot, SlotKind, Source, TaskMetadata, TaskProperty};
    let (_dir, mut app, owner, _failure) = standalone();
    let catalog = app
        .save_definition_catalog(&owner, &id(), catalog_save())
        .unwrap();
    let resource = put(
        &mut app,
        &owner,
        save(
            &catalog.id,
            Body::ResourceType {
                parent: None,
                fields: Default::default(),
            },
        ),
    );
    let setting = put(
        &mut app,
        &owner,
        save(&catalog.id, property(Category::Execution, true)),
    );
    let body = Body::Task {
        metadata: TaskMetadata {
            display_name: "Load aligned blank".into(),
            category: "Manipulation".into(),
            services: Services::Only {
                names: vec!["Laser heat treatment".into()],
            },
            completion_description: "Blank seated and clamped".into(),
            default_timeout_ns: Some(Counter(1_000_000_000)),
        },
        slots: [(
            name("to"),
            Slot {
                label: "Tray targets".into(),
                kind: SlotKind::Resource,
                accepted_types: vec![resource.version.definition.reference.clone()],
                required: true,
                multiple: true,
            },
        )]
        .into(),
        properties: [(
            name("height"),
            TaskProperty {
                property: setting.version.definition.reference.clone(),
                required: true,
                sources: vec![
                    Source::Override,
                    Source::Context {
                        slot: name("to"),
                        field: name("height"),
                    },
                    Source::Default,
                ],
                default: Some(number(100.0)),
            },
        )]
        .into(),
    };
    let stored = put(&mut app, &owner, save(&catalog.id, body.clone()));
    assert_eq!(stored.version.definition.body, body);
    // A default declaration is not an evaluated Task source chain.
    assert!(stored.effective.values.is_empty());
    assert_eq!(stored.effective.missing, vec![name("height")]);
    let reread = app
        .definition(
            &owner,
            &catalog.id,
            &stored.version.definition.reference.id,
            Some(Counter(1)),
        )
        .unwrap();
    assert_eq!(reread.version.definition, stored.version.definition);
    let mut duplicate = body.clone();
    if let Body::Task { properties, .. } = &mut duplicate {
        properties
            .get_mut(&name("height"))
            .unwrap()
            .sources
            .push(Source::Override);
    }
    assert!(Prepared::prepare(save(&catalog.id, duplicate)).is_err());
    let mut absent = body.clone();
    if let Body::Task { slots, .. } = &mut absent {
        slots.clear();
    }
    assert!(Prepared::prepare(save(&catalog.id, absent)).is_err());
    let mut wrong_kind = body;
    if let Body::Task { slots, .. } = &mut wrong_kind {
        let slot = slots.get_mut(&name("to")).unwrap();
        slot.kind = SlotKind::Object;
        assert!(slot.multiple);
    }
    assert!(Prepared::prepare(save(&catalog.id, wrong_kind)).is_err());
}

#[test]
fn object_models_keep_constraint_scope_and_missing_values_explicit() {
    let (_dir, mut app, owner, _failure) = standalone();
    let catalog = app
        .save_definition_catalog(&owner, &id(), catalog_save())
        .unwrap();
    let dimension = put(
        &mut app,
        &owner,
        save(&catalog.id, property(Category::Object, false)),
    );
    let limit = put(
        &mut app,
        &owner,
        save(&catalog.id, property(Category::Constraint, false)),
    );
    let fields: std::collections::BTreeMap<Name, Field> = [
        (
            name("diameter"),
            Field {
                property: dimension.version.definition.reference,
                required: true,
            },
        ),
        (
            name("limit"),
            Field {
                property: limit.version.definition.reference,
                required: true,
            },
        ),
    ]
    .into();
    let typ = put(
        &mut app,
        &owner,
        save(
            &catalog.id,
            Body::ObjectType {
                parent: None,
                fields: fields.clone(),
            },
        ),
    );
    let model = put(
        &mut app,
        &owner,
        save(
            &catalog.id,
            Body::ObjectModel {
                object_type: typ.version.definition.reference,
                values: [(name("diameter"), number(0.0))].into(),
            },
        ),
    );
    assert_eq!(model.effective.values[&name("diameter")].value, number(0.0));
    assert_eq!(model.effective.missing, vec![name("limit")]);
    let wrong = save(
        &catalog.id,
        Body::ResourceType {
            parent: None,
            fields,
        },
    );
    assert!(
        app.save_definition(&owner, &id(), Prepared::prepare(wrong).unwrap())
            .is_err()
    );
}

#[test]
fn point_patterns_are_versioned_data_with_inheritance_and_bounded_paging() {
    use rx_domain::definition::{
        Reference,
        pattern::{Axis, Query},
    };
    let (_dir, mut app, owner, _) = standalone();
    let catalog = app
        .save_definition_catalog(&owner, &id(), catalog_save())
        .unwrap();
    let mut fields = BTreeMap::new();
    for (field, value_type, unit) in [
        ("origin", ValueType::Vector, "mm"),
        ("orientation", ValueType::Vector, "unitless"),
        ("frame", ValueType::Text, "unitless"),
        ("rows", ValueType::Number, "unitless"),
        ("columns", ValueType::Number, "unitless"),
        ("pitch", ValueType::Number, "mm"),
    ] {
        let p =
            Property {
                value_type,
                category: Category::Resource,
                constraint_scope: None,
                unit: name(unit),
                minimum: None,
                maximum: None,
                choices: vec![],
                vector_length: (value_type == ValueType::Vector)
                    .then_some(if field == "orientation" { 4 } else { 3 }),
                overridable: false,
                parameter_mapping: BTreeMap::new(),
            };
        let property = put(
            &mut app,
            &owner,
            save(&catalog.id, Body::Property { specification: p }),
        );
        fields.insert(
            name(field),
            Field {
                property: property.version.definition.reference,
                required: true,
            },
        );
    }
    let typ = put(
        &mut app,
        &owner,
        save(
            &catalog.id,
            Body::ResourceType {
                parent: None,
                fields,
            },
        ),
    );
    let vector =
        |v: [f64; 3]| Value::Vector(v.into_iter().map(|n| Real::new(n).unwrap()).collect());
    let mut model = save(
        &catalog.id,
        Body::ResourceModel {
            resource_type: typ.version.definition.reference.clone(),
            values: [
                (name("origin"), vector([10.0, 20.0, 30.0])),
                (
                    name("orientation"),
                    Value::Vector(
                        [0.0, 0.0, 0.0, 1.0]
                            .into_iter()
                            .map(|v| Real::new(v).unwrap())
                            .collect(),
                    ),
                ),
                (name("frame"), Value::Text("test/frame".into())),
                (name("rows"), number(40.0)),
                (name("columns"), number(60.0)),
                (name("pitch"), number(2.0)),
            ]
            .into(),
        },
    );
    let base = put(&mut app, &owner, model.clone());
    let instance = put(
        &mut app,
        &owner,
        save(
            &catalog.id,
            Body::ResourceInstance {
                base: base.version.definition.reference.clone(),
                values: [(name("origin"), vector([100.0, 200.0, 300.0]))].into(),
            },
        ),
    );
    let mut rule = save(
        &catalog.id,
        Body::PointPattern {
            resource_type: typ.version.definition.reference.clone(),
            origin: name("origin"),
            orientation: Some(name("orientation")),
            frame: name("frame"),
            axes: vec![
                Axis {
                    count: name("rows"),
                    pitch: name("pitch"),
                    direction: vec![
                        Real::new(0.0).unwrap(),
                        Real::new(1.0).unwrap(),
                        Real::new(0.0).unwrap(),
                    ],
                },
                Axis {
                    count: name("columns"),
                    pitch: name("pitch"),
                    direction: vec![
                        Real::new(-1.0).unwrap(),
                        Real::new(0.0).unwrap(),
                        Real::new(0.0).unwrap(),
                    ],
                },
            ],
        },
    );
    let pattern = put(&mut app, &owner, rule.clone());
    let mut query = Query {
        subject: instance.version.definition.reference.clone(),
        rule: pattern.version.definition.reference.clone(),
        offset: Counter(0),
        limit: 100,
    };
    let first = app.definition_points(&owner, &query).unwrap();
    assert!(first.violations.is_empty());
    assert_eq!(first.total, Counter(2400));
    assert_eq!(first.frame.as_deref(), Some("test/frame"));
    assert_eq!(first.unit, Some(name("mm")));
    assert_eq!(
        first.inputs.values[&name("origin")].declared_by,
        instance.version.definition.reference
    );
    assert_eq!(
        first.inputs.values[&name("pitch")].declared_by,
        base.version.definition.reference
    );
    assert_eq!(
        first.inputs.shadowed[&name("origin")][0].value,
        vector([10.0, 20.0, 30.0])
    );
    let mut indices = BTreeSet::new();
    loop {
        let page = app.definition_points(&owner, &query).unwrap();
        for point in &page.points {
            assert!(indices.insert(point.index.0));
        }
        if let Some(next) = page.next {
            query.offset = next;
        } else {
            let last = page.points.last().unwrap();
            assert_eq!(last.indices, vec![Counter(39), Counter(59)]);
            assert_eq!(
                last.position.iter().map(|v| v.get()).collect::<Vec<_>>(),
                vec![-18.0, 278.0, 300.0]
            );
            break;
        }
    }
    assert_eq!(indices.len(), 2400);
    // A new model is only data; the same saved rule generates its 15 points.
    model.id = id();
    if let Body::ResourceModel { values, .. } = &mut model.body {
        values.insert(name("rows"), number(3.0));
        values.insert(name("columns"), number(5.0));
    }
    let new_model = put(&mut app, &owner, model.clone());
    let mut rotated = model.clone();
    rotated.id = id();
    if let Body::ResourceModel { values, .. } = &mut rotated.body {
        let half = std::f64::consts::FRAC_1_SQRT_2;
        values.insert(
            name("orientation"),
            Value::Vector(
                [0.0, 0.0, half, half]
                    .into_iter()
                    .map(|v| Real::new(v).unwrap())
                    .collect(),
            ),
        );
    }
    let rotated = put(&mut app, &owner, rotated);
    query.subject = rotated.version.definition.reference;
    query.offset = Counter(0);
    let points = app.definition_points(&owner, &query).unwrap();
    let last = points.points.last().unwrap();
    for (actual, expected) in last.position.iter().zip([6.0, 12.0, 30.0]) {
        assert!((actual.get() - expected).abs() < 1e-9);
    }
    query.subject = new_model.version.definition.reference.clone();
    query.offset = Counter(0);
    assert_eq!(
        app.definition_points(&owner, &query).unwrap().total,
        Counter(15)
    );
    model.expected = Some(Counter(1));
    if let Body::ResourceModel { values, .. } = &mut model.body {
        values.insert(name("rows"), number(1.5));
    }
    let fractional = put(&mut app, &owner, model.clone());
    assert_eq!(
        app.definition_points(&owner, &query).unwrap().total,
        Counter(15)
    );
    query.subject = fractional.version.definition.reference;
    let bad = app.definition_points(&owner, &query).unwrap();
    assert_eq!(bad.violations[0].code, "AXIS_COUNT");
    assert!(bad.violations[0].location.ends_with("/values/rows"));
    assert!(bad.points.is_empty());
    model.expected = Some(Counter(2));
    if let Body::ResourceModel { values, .. } = &mut model.body {
        values.remove(&name("origin"));
        values.remove(&name("frame"));
    }
    query.subject = put(&mut app, &owner, model.clone())
        .version
        .definition
        .reference;
    let missing = app.definition_points(&owner, &query).unwrap();
    assert!(
        missing
            .violations
            .iter()
            .any(|v| v.code == "MISSING_ORIGIN")
    );
    assert!(missing.violations.iter().any(|v| v.code == "MISSING_FRAME"));
    assert!(missing.points.is_empty());
    query.subject = Reference {
        digest: Digest::from_bytes([9; 32]),
        ..query.subject
    };
    assert!(app.definition_points(&owner, &query).is_err());
    query.subject = instance.version.definition.reference.clone();
    query.limit = 101;
    assert!(app.definition_points(&owner, &query).is_err());
    query.limit = 100;
    query.offset = Counter(2401);
    assert!(app.definition_points(&owner, &query).is_err());
    query.offset = Counter(0);
    query.rule.catalog = id();
    assert!(matches!(
        app.definition_points(&owner, &query),
        Err(StoreError::Rejected(rx_domain::fault::Rejection::Forbidden))
    ));
    // A count property cannot be used as a length, even with numerically plausible values.
    rule.id = id();
    if let Body::PointPattern { axes, .. } = &mut rule.body {
        axes[0].pitch = name("rows");
    }
    assert!(
        app.save_definition(&owner, &id(), Prepared::prepare(rule).unwrap())
            .is_err()
    );
}

#[path = "workflow_model_tests.rs"]
pub(crate) mod workflow_model_tests;

#[test]
fn object_instance_catalog_history_is_pinned_and_original_save_recovers_after_loss() {
    let (directory, mut app, owner, failure) = standalone();
    let catalog = app
        .save_definition_catalog(&owner, &id(), catalog_save())
        .unwrap();
    let field = put(
        &mut app,
        &owner,
        save(&catalog.id, property(Category::Object, false)),
    )
    .version
    .definition
    .reference;
    let typ = put(
        &mut app,
        &owner,
        save(
            &catalog.id,
            Body::ObjectType {
                parent: None,
                fields: [(
                    name("width"),
                    Field {
                        property: field,
                        required: true,
                    },
                )]
                .into(),
            },
        ),
    )
    .version
    .definition
    .reference;
    let model = put(
        &mut app,
        &owner,
        save(
            &catalog.id,
            Body::ObjectModel {
                object_type: typ,
                values: [(name("width"), number(45.0))].into(),
            },
        ),
    )
    .version
    .definition
    .reference;
    let input = save(
        &catalog.id,
        Body::ObjectInstance {
            base: model.clone(),
            values: Default::default(),
        },
    );
    let key = id();
    failure.store(2, Ordering::SeqCst);
    assert!(
        app.save_definition(&owner, &key, Prepared::prepare(input.clone()).unwrap())
            .is_err()
    );
    let original = app
        .save_definition(&owner, &key, Prepared::prepare(input.clone()).unwrap())
        .unwrap();
    assert_eq!(original.effective.values[&name("width")].declared_by, model);
    let mut changed = input.clone();
    changed.expected = Some(Counter(1));
    changed.body = Body::ObjectInstance {
        base: model,
        values: [(name("width"), number(40.0))].into(),
    };
    let current = put(&mut app, &owner, changed);
    assert_eq!(
        current.effective.values[&name("width")].declared_by,
        current.version.definition.reference
    );
    let history = app
        .definition(&owner, &catalog.id, &input.id, Some(Counter(1)))
        .unwrap();
    assert_eq!(
        rx_domain::canonical::bytes(&history).unwrap(),
        rx_domain::canonical::bytes(&original).unwrap()
    );
    let installation = app.installation.id.clone();
    drop(app);
    let mut app = Engine::open(
        FaultRepository {
            inner: SqliteRepository::open(directory.path().join("definitions.db")).unwrap(),
            mode: failure,
        },
        ManualClock(Arc::new(AtomicU64::new(1000))),
        SimulationAuthority,
        installation,
        principal("owner", &[Role::AccountAdmin]),
    )
    .unwrap();
    let session = app
        .authenticated_session(&name("owner"), id(), expiry(100000))
        .unwrap();
    let owner = Identity {
        session: session.id,
        ..owner
    };
    assert_eq!(
        app.definition(&owner, &catalog.id, &input.id, None)
            .unwrap()
            .version
            .definition
            .reference
            .revision,
        Counter(2)
    );
}
