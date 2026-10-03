use rx_domain::{
    canonical,
    definition::{
        Assignment, Body, Category, Definition, Field, Property, Reference, Slot, SlotKind, Value,
        ValueType,
    },
    types::*,
    workflow::*,
};
use std::collections::BTreeMap;
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id(i: u32) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{i:012}")).unwrap()
}
fn r(v: f64) -> Real {
    Real::new(v).unwrap()
}
fn q(min: f64, max: f64, unit: &str) -> Quantity {
    Quantity {
        unit: n(unit),
        data: Data::Number {
            range: Span {
                min: r(min),
                max: r(max),
            },
        },
    }
}
fn ctx(slot: &str, field: &str) -> Source {
    Source::Context {
        slot: n(slot),
        field: n(field),
        index: 0,
        frame_field: None,
    }
}
fn op(name: &str) -> Operand {
    Operand::Property { name: n(name) }
}
struct Fixture {
    spec: Spec,
    request: Request,
    all: BTreeMap<Reference, Definition>,
    refs: BTreeMap<String, Reference>,
    counter: u32,
}
impl Fixture {
    fn add(&mut self, key: &str, body: Body) -> Reference {
        self.counter += 1;
        let d = Definition::new(id(1), id(self.counter), Counter(1), key.into(), body).unwrap();
        let reference = d.reference.clone();
        self.all.insert(reference.clone(), d);
        self.refs.insert(key.into(), reference.clone());
        reference
    }
    fn property(
        &mut self,
        key: &str,
        kind: ValueType,
        category: Category,
        unit: &str,
        overridable: bool,
    ) -> Reference {
        self.add(
            key,
            Body::Property {
                specification: Property {
                    value_type: kind,
                    category,
                    constraint_scope: None,
                    unit: n(unit),
                    minimum: matches!(kind, ValueType::Number).then_some(r(0.0)),
                    maximum: matches!(kind, ValueType::Number).then_some(r(1000.0)),
                    choices: vec![],
                    vector_length: None,
                    overridable,
                    parameter_mapping: BTreeMap::new(),
                },
            },
        )
    }
    fn report(&self) -> Report {
        resolve(&self.spec, self.request.clone(), &self.all).unwrap()
    }
    fn override_value(&mut self, key: &str, value: Quantity) {
        self.request
            .overrides
            .entry(n("first"))
            .or_default()
            .insert(n(key), value);
    }
}
fn fixture() -> Fixture {
    let placeholder = Reference {
        catalog: id(1),
        id: id(90),
        revision: Counter(1),
        digest: Digest::from_bytes([0; 32]),
    };
    let mut f = Fixture {
        spec: Spec {
            schema: n(SCHEMA),
            contexts: BTreeMap::new(),
            defaults: BTreeMap::new(),
            property_sets: vec![],
            tasks: BTreeMap::new(),
            rules: BTreeMap::new(),
            constraints: BTreeMap::new(),
            steps: vec![],
        },
        request: Request {
            workflow: placeholder,
            contexts: BTreeMap::new(),
            property_sets: vec![],
            overrides: BTreeMap::new(),
            inputs: BTreeMap::new(),
            slot_index: Counter(0),
        },
        all: BTreeMap::new(),
        refs: BTreeMap::new(),
        counter: 100,
    };
    let width = f.property("width", ValueType::Number, Category::Object, "mm", false);
    let limit = f.property("limit", ValueType::Number, Category::Resource, "mm", false);
    let can = f.property(
        "can",
        ValueType::Boolean,
        Category::Resource,
        "unitless",
        false,
    );
    let implementation = f.property(
        "implementation",
        ValueType::Text,
        Category::Resource,
        "unitless",
        false,
    );
    let version = f.property(
        "version",
        ValueType::Text,
        Category::Resource,
        "unitless",
        false,
    );
    let target = f.property("target", ValueType::Number, Category::Execution, "mm", true);
    let clearance = f.property(
        "clearance",
        ValueType::Number,
        Category::Execution,
        "mm",
        true,
    );
    let timeout = f.property("timeout", ValueType::Number, Category::Execution, "s", true);
    let flag = f.property(
        "flag",
        ValueType::Boolean,
        Category::Execution,
        "unitless",
        false,
    );
    let done = f.property(
        "done",
        ValueType::Boolean,
        Category::Resource,
        "unitless",
        false,
    );
    let object = f.add(
        "object-type",
        Body::ObjectType {
            parent: None,
            fields: [(
                n("width"),
                Field {
                    property: width.clone(),
                    required: true,
                },
            )]
            .into(),
        },
    );
    let type_a = f.add(
        "type-a",
        Body::ObjectType {
            parent: Some(object.clone()),
            fields: BTreeMap::new(),
        },
    );
    let type_b = f.add(
        "type-b",
        Body::ObjectType {
            parent: Some(object.clone()),
            fields: BTreeMap::new(),
        },
    );
    let a = f.add(
        "model-a",
        Body::ObjectModel {
            object_type: type_a.clone(),
            values: [(n("width"), Value::Number(r(10.0)))].into(),
        },
    );
    let _b = f.add(
        "model-b",
        Body::ObjectModel {
            object_type: type_b.clone(),
            values: [(n("width"), Value::Number(r(20.0)))].into(),
        },
    );
    let actor_type = f.add(
        "actor-type",
        Body::ResourceType {
            parent: None,
            fields: [
                ("limit", limit),
                ("can", can),
                ("implementation", implementation),
                ("version", version),
            ]
            .into_iter()
            .map(|(k, p)| {
                (
                    n(k),
                    Field {
                        property: p,
                        required: true,
                    },
                )
            })
            .collect(),
        },
    );
    let actor = f.add(
        "actor",
        Body::ResourceModel {
            resource_type: actor_type.clone(),
            values: [
                (n("limit"), Value::Number(r(50.0))),
                (n("can"), Value::Boolean(true)),
                (n("implementation"), Value::Text("simulator".into())),
                (n("version"), Value::Text("1.0.0".into())),
            ]
            .into(),
        },
    );
    let set_a = f.add(
        "set-a",
        Body::PropertySet {
            values: [(
                n("clearance"),
                Assignment {
                    property: clearance.clone(),
                    value: Value::Number(r(2.0)),
                },
            )]
            .into(),
        },
    );
    let set_b = f.add(
        "set-b",
        Body::PropertySet {
            values: [(
                n("clearance"),
                Assignment {
                    property: clearance.clone(),
                    value: Value::Number(r(5.0)),
                },
            )]
            .into(),
        },
    );
    f.spec.contexts = [
        (
            n("object"),
            Slot {
                label: "Object".into(),
                kind: SlotKind::Object,
                accepted_types: vec![object],
                required: true,
                multiple: false,
            },
        ),
        (
            n("actor"),
            Slot {
                label: "Actor".into(),
                kind: SlotKind::Resource,
                accepted_types: vec![actor_type],
                required: true,
                multiple: false,
            },
        ),
    ]
    .into();
    f.spec.defaults = [(n("object"), vec![a]), (n("actor"), vec![actor])].into();
    f.spec.property_sets = vec![
        SetSelector {
            slot: n("object"),
            accepted_type: type_a,
            property_set: set_a,
        },
        SetSelector {
            slot: n("object"),
            accepted_type: type_b,
            property_set: set_b,
        },
    ];
    let properties = [
        (
            n("width"),
            Usage {
                property: width,
                sources: vec![ctx("object", "width")],
                default: None,
            },
        ),
        (
            n("clearance"),
            Usage {
                property: clearance,
                sources: vec![Source::Override, Source::PropertySet, Source::Default],
                default: Some(q(1.0, 1.0, "mm")),
            },
        ),
        (
            n("target"),
            Usage {
                property: target.clone(),
                sources: vec![Source::Override, Source::Rule { rule: n("sum") }],
                default: None,
            },
        ),
        (
            n("timeout"),
            Usage {
                property: timeout,
                sources: vec![
                    Source::Override,
                    Source::Input { key: n("deadline") },
                    Source::Default,
                ],
                default: Some(q(10.0, 10.0, "s")),
            },
        ),
        (
            n("flag"),
            Usage {
                property: flag,
                sources: vec![Source::Default],
                default: Some(Quantity {
                    unit: n("unitless"),
                    data: Data::Boolean { value: false },
                }),
            },
        ),
    ]
    .into();
    f.spec.rules.insert(
        n("sum"),
        Rule {
            output: target,
            operation: Operation::Add {
                left: op("width"),
                right: op("clearance"),
            },
        },
    );
    f.spec.constraints = [
        (
            n("limit"),
            Constraint {
                left: op("target"),
                right: Operand::Context {
                    slot: n("actor"),
                    field: n("limit"),
                    index: 0,
                    frame_field: None,
                },
                relation: Relation::Le,
                property: n("target"),
                message: "target exceeds actor limit".into(),
            },
        ),
        (
            n("floor"),
            Constraint {
                left: op("target"),
                right: op("width"),
                relation: Relation::Ge,
                property: n("target"),
                message: "target is below object width".into(),
            },
        ),
    ]
    .into();
    f.spec.tasks.insert(
        n("act"),
        Task {
            label: "Act".into(),
            contexts: vec![n("object"), n("actor")],
            properties,
            constraints: vec![n("limit"), n("floor")],
            capabilities: vec![Capability {
                name: n("act"),
                slot: n("actor"),
                field: n("can"),
            }],
            skills: vec![Skill {
                capability: n("act"),
                slot: n("actor"),
                implementation_field: n("implementation"),
                version_field: n("version"),
                primitive: n("set"),
                parameters: [(n("value"), n("target")), (n("flag"), n("flag"))].into(),
            }],
            timeout_property: n("timeout"),
            done: Done {
                observation: n("done"),
                property: done,
                equals: Quantity {
                    unit: n("unitless"),
                    data: Data::Boolean { value: true },
                },
            },
            on_failure: KnownFailure::Stop,
            on_unknown: UnknownOutcome::HoldAndReconcile,
        },
    );
    f.spec.steps = vec![Step {
        id: n("first"),
        task: n("act"),
    }];
    f.request.workflow.digest = canonical::digest("test/workflow", &f.spec).unwrap();
    f
}
fn value<'a>(report: &'a Report, key: &str) -> &'a Resolved {
    &report.steps[0].properties[&n(key)]
}
#[test]
fn changed_model_selects_new_values_and_sources_without_changing_the_rules() {
    let mut f = fixture();
    let a = f.report();
    assert!(a.valid && a.concrete);
    assert_eq!(value(&a, "target").value, q(12.0, 12.0, "mm"));
    assert!(
        value(&a, "target")
            .origins
            .iter()
            .any(|o| o.kind == "RULE" && o.path == "rules/sum")
    );
    assert!(matches!(
        value(&a, "flag").value.data,
        Data::Boolean { value: false }
    ));
    f.request
        .contexts
        .insert(n("object"), vec![f.refs["model-b"].clone()]);
    let b = f.report();
    assert!(b.valid);
    assert_eq!(value(&b, "target").value, q(25.0, 25.0, "mm"));
    assert!(
        value(&b, "target")
            .origins
            .iter()
            .any(|o| o.reference.as_ref() == Some(&f.refs["set-b"]))
    );
    assert_eq!(a.request.workflow, b.request.workflow);
}
#[test]
fn overrides_are_checked_after_selection_and_zero_is_not_missing() {
    let mut f = fixture();
    f.override_value("target", q(60.0, 60.0, "mm"));
    let result = f.report();
    assert!(!result.valid);
    assert_eq!(value(&result, "target").selected_source, "OVERRIDE");
    assert_eq!(value(&result, "target").value, q(60.0, 60.0, "mm"));
    assert!(
        result
            .violations
            .iter()
            .any(|v| v.location == "nodes/first/properties/target"
                && v.code == "CONSTRAINT_VIOLATION")
    );
    f.override_value("target", q(0.0, 0.0, "mm"));
    let result = f.report();
    assert_eq!(value(&result, "target").value, q(0.0, 0.0, "mm"));
    assert!(!result.valid);
    assert!(!result.violations.iter().any(|v| v.code == "VALUE_MISSING"));
}
#[test]
fn ranges_use_the_worst_combination_and_never_become_concrete_commands() {
    let mut f = fixture();
    f.override_value("target", q(20.0, 60.0, "mm"));
    assert!(!f.report().valid);
    f.override_value("target", q(20.0, 40.0, "mm"));
    let report = f.report();
    assert!(report.valid);
    assert!(!report.concrete);
    assert_eq!(report.status, "BOUNDED_INPUT_NOT_EXECUTABLE");
    f.request.overrides.clear();
    f.override_value("clearance", q(5.0, 10.0, "mm"));
    let report = f.report();
    assert!(report.valid);
    let Data::Number { range } = &value(&report, "target").value.data else {
        panic!("number")
    };
    assert!(range.min.get() <= 15.0 && range.max.get() >= 20.0);
}
#[test]
fn unit_errors_and_conflicting_sets_cannot_hide_behind_fallback_or_override() {
    let mut f = fixture();
    f.override_value("target", q(40.0, 40.0, "N"));
    let report = f.report();
    assert!(!report.valid);
    assert!(!report.steps[0].properties.contains_key(&n("target")));
    f.request.overrides.clear();
    f.request.property_sets.push(f.refs["set-b"].clone());
    f.override_value("clearance", q(3.0, 3.0, "mm"));
    let report = f.report();
    assert!(!report.valid);
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.code == "PROPERTY_SET_CONFLICT")
    );
}
#[test]
fn cycles_missing_context_and_wrong_type_references_are_explicit() {
    let mut f = fixture();
    f.spec.rules.get_mut(&n("sum")).unwrap().operation = Operation::Add {
        left: Operand::Rule { name: n("sum") },
        right: op("clearance"),
    };
    let report = f.report();
    assert!(!report.valid);
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.code == "RESOLUTION_CYCLE")
    );
    let mut f = fixture();
    f.request.contexts.insert(n("object"), vec![]);
    let report = f.report();
    assert!(!report.valid);
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.code == "CONTEXT_CARDINALITY")
    );
    let mut f = fixture();
    f.spec
        .contexts
        .get_mut(&n("object"))
        .unwrap()
        .accepted_types = vec![f.refs["model-a"].clone()];
    assert!(
        f.report()
            .violations
            .iter()
            .any(|v| v.code == "CONTEXT_TYPE_REFERENCE")
    );
}
#[test]
fn invalid_literal_and_input_are_rejected_even_if_an_override_would_mask_them() {
    let mut f = fixture();
    f.spec.rules.get_mut(&n("sum")).unwrap().operation = Operation::Add {
        left: Operand::Literal {
            value: q(5.0, 1.0, "mm"),
        },
        right: op("clearance"),
    };
    assert!(
        f.report()
            .violations
            .iter()
            .any(|v| v.code == "RULE_LITERAL_INVALID")
    );
    let mut f = fixture();
    f.override_value("timeout", q(20.0, 20.0, "s"));
    f.request.inputs.insert(n("deadline"), q(10.0, 10.0, "mm"));
    let report = f.report();
    assert!(!report.valid);
    assert!(report.violations.iter().any(|v| v.code == "INPUT_INVALID"));
}

#[test]
fn object_instances_inherit_models_keep_value_sources_and_obey_type_checks() {
    let mut f = fixture();
    let model = f.refs["model-a"].clone();
    let instance = f.add(
        "actual-a-1",
        Body::ObjectInstance {
            base: model.clone(),
            values: BTreeMap::new(),
        },
    );
    f.request
        .contexts
        .insert(n("object"), vec![instance.clone()]);
    let inherited = f.report();
    assert!(inherited.valid && inherited.concrete);
    assert_eq!(value(&inherited, "target").value, q(12.0, 12.0, "mm"));
    let effective = rx_domain::definition::resolve(&f.all[&instance], &f.all).unwrap();
    assert_eq!(effective.values[&n("width")].declared_by, model);
    let changed = f.add(
        "actual-a-2",
        Body::ObjectInstance {
            base: model,
            values: [(n("width"), Value::Number(r(15.0)))].into(),
        },
    );
    f.request
        .contexts
        .insert(n("object"), vec![changed.clone()]);
    let report = f.report();
    assert!(report.valid);
    assert_eq!(value(&report, "target").value, q(17.0, 17.0, "mm"));
    let effective = rx_domain::definition::resolve(&f.all[&changed], &f.all).unwrap();
    assert_eq!(effective.values[&n("width")].declared_by, changed);
    assert!(!effective.shadowed[&n("width")].is_empty());
    for base in [
        f.refs["object-type"].clone(),
        f.refs["actor-type"].clone(),
        instance,
    ] {
        let invalid = f.add(
            "wrong-base",
            Body::ObjectInstance {
                base,
                values: BTreeMap::new(),
            },
        );
        assert!(rx_domain::definition::resolve(&f.all[&invalid], &f.all).is_err());
    }
}
