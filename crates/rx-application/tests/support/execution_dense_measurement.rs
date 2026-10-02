//! Opt-in performance evidence using externally supplied scenario data; never qualification authority.
use super::*;
use rx_process_contract::execution_v2 as v2;
use serde_json::Value as Json;

fn references(value: &mut Json, refs: &BTreeMap<String, rx_domain::definition::Reference>) -> bool {
    match value {
        Json::Object(map) if map.len() == 1 && map.contains_key("$ref") => {
            let Some(reference) = refs.get(map["$ref"].as_str().unwrap()) else {
                return false;
            };
            *value = serde_json::to_value(reference).unwrap();
            true
        }
        Json::Object(map) => map.values_mut().all(|v| references(v, refs)),
        Json::Array(items) => items.iter_mut().all(|v| references(v, refs)),
        _ => true,
    }
}

#[test]
#[ignore = "manual 2400-slot measurement; RX_DENSE_FIXTURE must name the documented external data directory"]
fn external_dense_publication_and_qualification_timing() {
    let root =
        std::path::PathBuf::from(std::env::var("RX_DENSE_FIXTURE").expect("RX_DENSE_FIXTURE"));
    let read = |file: &str| -> Json {
        serde_json::from_slice(&std::fs::read(root.join(file)).unwrap()).unwrap()
    };
    // This is a new isolated test database. No installed user data is changed.
    let slots = match std::env::var("RX_DENSE_MEASUREMENT_SLOTS").as_deref() {
        Ok("2") => 2,
        Ok("2400") | Err(_) => 2400,
        _ => panic!("measurement slots must be 2 (fixture preflight) or 2400 (full measurement)"),
    };
    let mut f = execution_fixture();
    let mut refs = BTreeMap::new();
    let mut pending = Vec::new();
    for file in ["cell.json", "m2-definitions.json"] {
        pending.extend(read(file)["definitions"].as_array().unwrap().clone());
    }
    while !pending.is_empty() {
        let before = pending.len();
        pending.retain(|entry| {
            let mut body = entry["body"].clone();
            if !references(&mut body, &refs) {
                return true;
            }
            let mut input = save(&f.catalog.id, serde_json::from_value(body).unwrap());
            input.id = serde_json::from_value(entry["id"].clone()).unwrap();
            input.label = entry["label"].as_str().unwrap().to_owned();
            let reference = put(&mut f.app, &f.owner, input)
                .version
                .definition
                .reference;
            refs.insert(entry["key"].as_str().unwrap().to_owned(), reference);
            false
        });
        assert!(
            pending.len() < before,
            "unresolved external definition dependency"
        );
    }
    let mut spec = read("workflow.json")["spec"].clone();
    assert!(references(&mut spec, &refs));
    let version = f
        .app
        .save_workflow_model(
            &f.owner,
            &id(),
            wm::PreparedSave::prepare(wm::Save {
                catalog: f.catalog.id.clone(),
                id: id(),
                expected: None,
                label: "External dense timing SIMULATION".into(),
                spec: serde_json::from_value(spec).unwrap(),
            })
            .unwrap(),
        )
        .unwrap();
    let mut a = query(&version);
    a.contexts
        .insert(name("supply"), vec![refs["m2.tray.dense-site"].clone()]);
    a.contexts
        .insert(name("output"), vec![refs["m2.tray.dense-site"].clone()]);
    let mut b = a.clone();
    b.contexts
        .insert(name("part"), vec![refs["part.ECC_99-14"].clone()]);
    let blocked = f
        .app
        .prepare_execution_inputs(&f.owner, vec![a.clone(), b.clone()], 2400)
        .unwrap();
    for candidate in 0..2 {
        let report = blocked.resolve(candidate, 0).unwrap();
        assert!(!report.valid, "original dense geometry must remain blocked");
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.message.contains("pitch"))
        );
    }
    // Data-only positive control: preserve 40x60 count and all task/part rules;
    // widen pitches and footprint explicitly in this private fixture, not the source.
    let original = f
        .app
        .definition(
            &f.owner,
            &f.catalog.id,
            &refs["m2.tray.dense"].id,
            Some(refs["m2.tray.dense"].revision),
        )
        .unwrap();
    let mut body = original.version.definition.body;
    let Body::ResourceModel { values, .. } = &mut body else {
        panic!()
    };
    for (key, value) in [
        ("row_pitch", 80.0),
        ("column_pitch", 80.0),
        ("width", 4800.0),
        ("depth", 3200.0),
    ] {
        values.insert(name(key), Value::Number(Real::new(value).unwrap()));
    }
    // Keep original revisions immutable; create new model and instance identities.
    let model = put(&mut f.app, &f.owner, save(&f.catalog.id, body))
        .version
        .definition
        .reference;
    let original = f
        .app
        .definition(
            &f.owner,
            &f.catalog.id,
            &refs["m2.tray.dense-site"].id,
            Some(refs["m2.tray.dense-site"].revision),
        )
        .unwrap();
    let mut body = original.version.definition.body;
    let Body::ResourceInstance { base, .. } = &mut body else {
        panic!()
    };
    *base = model;
    let instance = put(&mut f.app, &f.owner, save(&f.catalog.id, body))
        .version
        .definition
        .reference;
    for request in [&mut a, &mut b] {
        request
            .contexts
            .insert(name("supply"), vec![instance.clone()]);
        request
            .contexts
            .insert(name("output"), vec![instance.clone()]);
    }
    f.requests = vec![a, b];
    f.snapshot = f
        .app
        .prepare_execution_inputs(&f.owner, f.requests.clone(), slots)
        .unwrap();
    let report = f.snapshot.resolve(0, 0).unwrap();
    assert!(report.valid && report.concrete, "{:?}", report.violations);
    let base_action = f.policy.templates[&name("node")].clone();
    f.policy.templates.clear();
    f.policy.node_contracts.clear();
    for step in &report.steps {
        let skill = &step.skills[0];
        let text = |value: &w::Resolved| match &value.value.data {
            w::Data::Text { value } => value.clone(),
            _ => panic!("skill text"),
        };
        f.policy
            .templates
            .insert(step.node.clone(), base_action.clone());
        f.policy.node_contracts.insert(
            step.node.clone(),
            v2::NodeContract {
                implementation: text(&skill.implementation),
                version: text(&skill.version),
                primitive: skill.primitive.clone(),
                parameters: skill
                    .parameters
                    .iter()
                    .map(|(parameter, property)| {
                        let resolved = &step.properties[property];
                        (
                            parameter.clone(),
                            v2::ParameterContract {
                                unit: resolved.value.unit.clone(),
                                frame: resolved.frame.clone(),
                                value_type: match resolved.value.data {
                                    w::Data::Number { .. } => ValueType::Number,
                                    w::Data::Vector { .. } => ValueType::Vector,
                                    w::Data::Boolean { .. } => ValueType::Boolean,
                                    w::Data::Text { .. } => ValueType::Text,
                                },
                            },
                        )
                    })
                    .collect(),
            },
        );
    }
    f.policy.slot_order = (0..slots).collect();
    f.policy.candidates[0].object_model = refs["part.ECC_51-14"].clone();
    f.policy.candidates[1].object_model = refs["part.ECC_99-14"].clone();
    assert_eq!(report.steps.len(), 8);
    qualification_case(f, false, true);
}
