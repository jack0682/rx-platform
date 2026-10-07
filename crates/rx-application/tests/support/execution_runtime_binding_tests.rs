use super::*;
use rx_process_contract::execution_v2 as v2;

fn property_ref(
    f: &mut ExecutionFixture,
    kind: ValueType,
    category: Category,
    unit: &str,
) -> rx_domain::definition::Reference {
    put(
        &mut f.app,
        &f.owner,
        save(
            &f.catalog.id,
            Body::Property {
                specification: Property {
                    value_type: kind,
                    category,
                    constraint_scope: None,
                    unit: name(unit),
                    minimum: None,
                    maximum: None,
                    choices: vec![],
                    vector_length: (kind == ValueType::Vector).then_some(3),
                    overridable: false,
                    parameter_mapping: BTreeMap::new(),
                },
            },
        ),
    )
    .version
    .definition
    .reference
}
fn prepared_inputs(f: &mut ExecutionFixture, spec: w::Spec) -> (v2::InputClosure, v2::Policy) {
    let old = f.snapshot.input_closure();
    let version = f
        .app
        .save_workflow_model(
            &f.owner,
            &id(),
            wm::PreparedSave::prepare(wm::Save {
                catalog: f.catalog.id.clone(),
                id: old.workflow.id,
                expected: Some(old.workflow.revision),
                label: old.label,
                spec,
            })
            .unwrap(),
        )
        .unwrap();
    for request in &mut f.requests {
        request.workflow = version.reference.clone();
    }
    f.snapshot = f
        .app
        .prepare_execution_inputs(&f.owner, f.requests.clone(), 2)
        .unwrap();
    let report = f.snapshot.resolve(0, 0).unwrap();
    assert!(report.valid, "{:?}", report.violations);
    let input = f.preview_input();
    let key = id();
    let prepared = f.prepare_preview(&key, input);
    let preview = f
        .app
        .save_execution_preview(&f.owner, &key, prepared)
        .unwrap();
    let saved = f
        .app
        .execution_preview(&f.owner, &preview.reference)
        .unwrap();
    (f.snapshot.input_closure(), saved.policy().clone())
}
#[test]
fn actual_instance_projection_preserves_provenance_but_cannot_expand_approved_values() {
    let mut f = execution_fixture();
    let width = property_ref(&mut f, ValueType::Number, Category::Object, "mm");
    let typ = put(
        &mut f.app,
        &f.owner,
        save(
            &f.catalog.id,
            Body::ObjectType {
                parent: None,
                fields: [(
                    name("width"),
                    Field {
                        property: width,
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
        &mut f.app,
        &f.owner,
        save(
            &f.catalog.id,
            Body::ObjectModel {
                object_type: typ.clone(),
                values: [(name("width"), number(45.0))].into(),
            },
        ),
    )
    .version
    .definition
    .reference;
    let mut spec = f.snapshot.input_closure().spec;
    spec.contexts
        .get_mut(&name("object"))
        .unwrap()
        .accepted_types = vec![typ];
    spec.defaults.insert(name("object"), vec![model.clone()]);
    f.requests.truncate(1);
    f.policy.candidates.truncate(1);
    f.policy.candidates[0].object_model = model.clone();
    let (inputs, policy) = prepared_inputs(&mut f, spec);
    let create = |f: &mut ExecutionFixture, values| {
        put(
            &mut f.app,
            &f.owner,
            save(
                &f.catalog.id,
                Body::ObjectInstance {
                    base: model.clone(),
                    values,
                },
            ),
        )
        .version
        .definition
    };
    let inherited = create(&mut f, BTreeMap::new());
    let same = create(&mut f, [(name("width"), number(45.0))].into());
    let checked = inputs.object_projection(&policy, &inherited).unwrap();
    let overridden = inputs.object_projection(&policy, &same).unwrap();
    assert_eq!(checked.values_digest, overridden.values_digest);
    assert_ne!(checked.instance, overridden.instance);
    assert_eq!(checked.effective.values[&name("width")].declared_by, model);
    assert_eq!(
        overridden.effective.values[&name("width")].declared_by,
        same.reference
    );
    let different = create(&mut f, [(name("width"), number(44.0))].into());
    assert!(
        inputs.object_projection(&policy, &different).is_err(),
        "even an unused declared value is part of the approved model"
    );
    let model_definition = f
        .app
        .definition(&f.owner, &f.catalog.id, &model.id, Some(model.revision))
        .unwrap()
        .version
        .definition;
    assert!(
        inputs
            .object_projection(&policy, &model_definition)
            .is_err(),
        "model identity is not an actual object"
    );
    let mut ambiguous_inputs = inputs.clone();
    let mut alternative = inputs.requests[0].clone();
    alternative.overrides.insert(name("node"), [(name("target"), serde_json::from_value(serde_json::json!({"unit":"mm","data":{"kind":"NUMBER","range":{"min":40,"max":40}}})).unwrap())].into());
    ambiguous_inputs.requests.push(alternative);
    let mut ambiguous = policy;
    let mut candidate = ambiguous.candidates[0].clone();
    candidate.key = name("another-variant");
    candidate.context_digest = v2::context_digest(&ambiguous_inputs.requests[1]).unwrap();
    ambiguous.candidates.push(candidate);
    ambiguous.definition_closure = ambiguous_inputs.artifact().unwrap();
    assert!(
        ambiguous_inputs
            .object_projection(&ambiguous, &inherited)
            .unwrap_err()
            .contains("ambiguous")
    );
}

pub(super) fn attach_stock(
    f: &mut ExecutionFixture,
) -> (
    v2::InputClosure,
    v2::Policy,
    rx_domain::definition::Reference,
    rx_domain::definition::Reference,
) {
    let origin = property_ref(f, ValueType::Vector, Category::Resource, "mm");
    let orientation = put(
        &mut f.app,
        &f.owner,
        save(
            &f.catalog.id,
            Body::Property {
                specification: Property {
                    value_type: ValueType::Vector,
                    category: Category::Resource,
                    constraint_scope: None,
                    unit: name("unitless"),
                    minimum: None,
                    maximum: None,
                    choices: vec![],
                    vector_length: Some(4),
                    overridable: false,
                    parameter_mapping: BTreeMap::new(),
                },
            },
        ),
    )
    .version
    .definition
    .reference;
    let frame = property_ref(f, ValueType::Text, Category::Resource, "unitless");
    let count = property_ref(f, ValueType::Number, Category::Resource, "unitless");
    let pitch = property_ref(f, ValueType::Number, Category::Resource, "mm");
    let typ = put(
        &mut f.app,
        &f.owner,
        save(
            &f.catalog.id,
            Body::ResourceType {
                parent: None,
                fields: [
                    ("origin", origin),
                    ("orientation", orientation),
                    ("frame", frame),
                    ("count", count),
                    ("pitch", pitch),
                ]
                .into_iter()
                .map(|(key, property)| {
                    (
                        name(key),
                        Field {
                            property,
                            required: true,
                        },
                    )
                })
                .collect(),
            },
        ),
    )
    .version
    .definition
    .reference;
    let model = put(
        &mut f.app,
        &f.owner,
        save(
            &f.catalog.id,
            Body::ResourceModel {
                resource_type: typ.clone(),
                values: [
                    (
                        name("origin"),
                        Value::Vector(vec![Real::new(0.0).unwrap(); 3]),
                    ),
                    (
                        name("orientation"),
                        Value::Vector(
                            [0.0, 0.0, 0.0, 1.0]
                                .into_iter()
                                .map(|v| Real::new(v).unwrap())
                                .collect(),
                        ),
                    ),
                    (name("frame"), Value::Text("SIMULATION/world".into())),
                    (name("count"), number(4.0)),
                    (name("pitch"), number(10.0)),
                ]
                .into(),
            },
        ),
    )
    .version
    .definition
    .reference;
    let instance = put(
        &mut f.app,
        &f.owner,
        save(
            &f.catalog.id,
            Body::ResourceInstance {
                base: model.clone(),
                values: BTreeMap::new(),
            },
        ),
    )
    .version
    .definition
    .reference;
    let rule = put(
        &mut f.app,
        &f.owner,
        save(
            &f.catalog.id,
            Body::PointPattern {
                resource_type: typ.clone(),
                origin: name("origin"),
                orientation: Some(name("orientation")),
                frame: name("frame"),
                axes: vec![rx_domain::definition::pattern::Axis {
                    count: name("count"),
                    pitch: name("pitch"),
                    direction: [1.0, 0.0, 0.0]
                        .into_iter()
                        .map(|v| Real::new(v).unwrap())
                        .collect(),
                }],
            },
        ),
    )
    .version
    .definition
    .reference;
    let position = property_ref(f, ValueType::Vector, Category::Execution, "mm");
    let mut spec = f.snapshot.input_closure().spec;
    for key in ["stock", "same-stock"] {
        spec.contexts.insert(
            name(key),
            rx_domain::definition::Slot {
                label: key.into(),
                kind: rx_domain::definition::SlotKind::Resource,
                accepted_types: vec![typ.clone()],
                required: true,
                multiple: false,
            },
        );
        spec.defaults.insert(name(key), vec![instance.clone()]);
        let task = spec.tasks.get_mut(&name("act")).unwrap();
        task.contexts.push(name(key));
        task.properties.insert(
            name(key),
            w::Usage {
                property: position.clone(),
                sources: vec![w::Source::Pattern {
                    slot: name(key),
                    rule: rule.clone(),
                    component: w::PatternComponent::Position,
                }],
                default: None,
            },
        );
    }
    let (inputs, policy) = prepared_inputs(f, spec);
    (inputs, policy, model, instance)
}
#[test]
fn slot_resources_come_from_all_active_pattern_contexts_and_stable_instance_identity() {
    let mut f = execution_fixture();
    let (inputs, policy, model, instance) = attach_stock(&mut f);
    let pools = inputs.slot_resources(&policy).unwrap();
    assert_eq!(pools.len(), 1);
    assert_eq!(pools[0].resource, instance);
    assert_eq!(pools[0].count, Counter(4));
    assert_eq!(pools[0].contexts, vec![name("same-stock"), name("stock")]);
    let other = put(
        &mut f.app,
        &f.owner,
        save(
            &f.catalog.id,
            Body::ResourceInstance {
                base: model,
                values: BTreeMap::new(),
            },
        ),
    )
    .version
    .definition;
    let mut changed = inputs.clone();
    changed.requests[1]
        .contexts
        .insert(name("stock"), vec![other.reference.clone()]);
    changed.definitions.push(other);
    let mut changed_policy = policy;
    changed_policy.definition_closure = changed.artifact().unwrap();
    changed_policy.candidates[1].context_digest = v2::context_digest(&changed.requests[1]).unwrap();
    assert!(
        changed
            .slot_resources(&changed_policy)
            .unwrap_err()
            .contains("across candidates")
    );
}

pub(super) fn attach_distinct_stock(f: &mut ExecutionFixture) {
    let (_, _, model, _) = attach_stock(f);
    let other = put(
        &mut f.app,
        &f.owner,
        save(
            &f.catalog.id,
            Body::ResourceInstance {
                base: model,
                values: [(
                    name("origin"),
                    Value::Vector(
                        [100.0, 0.0, 0.0]
                            .into_iter()
                            .map(|v| Real::new(v).unwrap())
                            .collect(),
                    ),
                )]
                .into(),
            },
        ),
    )
    .version
    .definition
    .reference;
    let mut spec = f.snapshot.input_closure().spec;
    spec.defaults.insert(name("same-stock"), vec![other]);
    prepared_inputs(f, spec);
}
