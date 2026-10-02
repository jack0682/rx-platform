use rx_domain::{canonical, intent::Intent, types::*};
use rx_process_contract::execution_v2::*;
use std::collections::BTreeMap;

#[path = "support/execution_v2_fixture.rs"]
mod fixture;
use fixture::*;
#[test]
fn exact_index_wire_vector_and_dense_bound() {
    let mut i = index(1, 2);
    i.entries[0].2 = digest(0x11);
    i.entries[1].2 = digest(0x22);
    let expected = format!(
        r#"{{"entries":[[0,0,"{}"],[0,1,"{}"]],"schema":"rx.execution-report-index.v2"}}"#,
        "11".repeat(32),
        "22".repeat(32)
    );
    assert_eq!(canonical::bytes(&i).unwrap(), expected.as_bytes());
    for (models, expected_bytes) in [(2, 362633), (8, 1450373)] {
        let p = policy(models, 2400);
        let bytes = canonical::bytes(&index(models, 2400)).unwrap();
        assert_eq!(bytes.len(), expected_bytes);
        assert!(ReportIndex::decode(&bytes, &p).is_ok());
        assert!(Policy::decode(&canonical::bytes(&p).unwrap()).is_ok());
        if models == 8 {
            assert!(
                canonical::decode_json::<ReportIndex>(&bytes).is_err(),
                "legacy wire byte limit must remain unchanged"
            );
        }
    }
}

#[test]
fn index_cannot_omit_duplicate_reorder_or_exceed_published_domain() {
    let p = policy(2, 2);
    let base = index(2, 2);
    let mut changes = vec![];
    let mut i = base.clone();
    i.entries.pop();
    changes.push(i);
    let mut i = base.clone();
    i.entries[1] = i.entries[0];
    changes.push(i);
    let mut i = base.clone();
    i.entries.swap(0, 1);
    changes.push(i);
    let mut i = base.clone();
    i.entries[0].0 = 2;
    changes.push(i);
    let mut i = base.clone();
    i.entries[0].1 = 2;
    changes.push(i);
    let mut i = base.clone();
    i.entries[0].2 = digest(0);
    changes.push(i);
    let mut i = base;
    i.schema = n("rx.execution-report-index.v1");
    changes.push(i);
    for i in changes {
        let bytes = canonical::bytes(&i).unwrap();
        let mut updated = p.clone();
        updated.report_index = artifact(INDEX_SCHEMA, &bytes);
        assert!(ReportIndex::decode(&bytes, &updated).is_err());
    }
    assert!(index(9, 1).validate(9, 1).is_err());
    assert!(index(1, 2401).validate(1, 2401).is_err());
    assert!(index(0, 1).validate(0, 1).is_err());
}

#[test]
fn strict_artifact_decoding_refuses_forgery_and_ambiguous_json() {
    let p = policy(1, 1);
    let bytes = canonical::bytes(&index(1, 1)).unwrap();
    let mut altered = p.clone();
    altered.report_index.size_bytes.0 += 1;
    assert!(ReportIndex::decode(&bytes, &altered).is_err());
    let mut altered = p.clone();
    altered.report_index.sha256 = digest(9);
    assert!(ReportIndex::decode(&bytes, &altered).is_err());
    let mut altered = p.clone();
    altered.report_index.schema_id = n("unknown");
    assert!(ReportIndex::decode(&bytes, &altered).is_err());
    for malformed in [
        format!(" {}", String::from_utf8(bytes.clone()).unwrap()),
        String::from_utf8(bytes.clone())
            .unwrap()
            .replace("\"entries\":", "\"extra\":true,\"entries\":"),
        String::from_utf8(bytes.clone()).unwrap().replace(
            "\"entries\":",
            "\"schema\":\"rx.execution-report-index.v2\",\"entries\":",
        ),
    ] {
        let mut altered = p.clone();
        altered.report_index = artifact(INDEX_SCHEMA, malformed.as_bytes());
        assert!(ReportIndex::decode(malformed.as_bytes(), &altered).is_err());
    }
    let oversized = vec![b' '; MAX_INDEX_BYTES + 1];
    let mut altered = p.clone();
    altered.report_index = artifact(INDEX_SCHEMA, &oversized);
    assert!(ReportIndex::decode(&oversized, &altered).is_err());
}

#[test]
fn policy_bounds_and_published_order_cannot_be_loosened() {
    let base = policy(1, 2);
    let mut changes = vec![];
    let mut p = base.clone();
    p.slot_order.reverse();
    changes.push(p);
    let mut p = base.clone();
    p.slot_order[1] = 0;
    changes.push(p);
    let mut p = base.clone();
    p.candidates.push(p.candidates[0].clone());
    changes.push(p);
    let mut p = base.clone();
    p.candidates[0].object_model.catalog = id(9);
    changes.push(p);
    let mut p = base.clone();
    p.candidates[0].object_model.revision = Counter(0);
    changes.push(p);
    let mut p = base.clone();
    p.definition_closure.size_bytes = Counter(MAX_DEFINITION_BYTES + 1);
    changes.push(p);
    let mut p = base.clone();
    p.compiler_digest = digest(0);
    changes.push(p);
    let mut p = base;
    p.schema = n("rx.execution-policy.v1");
    changes.push(p);
    changes.extend([policy(9, 1), policy(1, 2401)]);
    for p in changes {
        assert!(p.validate().is_err());
    }
}

#[test]
fn cross_run_and_other_selection_replay_is_rejected_even_with_same_parameter() {
    let p = policy(2, 2);
    let idx = ReportIndex::decode(&canonical::bytes(&index(2, 2)).unwrap(), &p).unwrap();
    let (authorized, intent, bytes) = selected(&p);
    authorized
        .verify_request(&authorized, &p, &idx, &n("host"), &intent, &bytes)
        .unwrap();
    let raw = serde_json::to_value(&authorized).unwrap();
    for (field, value) in [
        ("run", serde_json::json!(id(9))),
        ("part", serde_json::json!(id(9))),
        ("publication", serde_json::json!(id(9))),
        ("ordinal", serde_json::json!("2")),
        ("slot", serde_json::json!(1)),
        ("candidate", serde_json::json!(1)),
        ("authority_generation", serde_json::json!("2")),
        ("object_values_digest", serde_json::json!(digest(9))),
        ("configuration_digest", serde_json::json!(digest(8))),
        ("report_digest", serde_json::json!(digest(9))),
    ] {
        let mut changed = raw.clone();
        changed[field] = value;
        let submitted: Selection = serde_json::from_value(changed).unwrap();
        assert!(
            authorized
                .verify_request(&submitted, &p, &idx, &n("host"), &intent, &bytes)
                .is_err(),
            "{field}"
        );
    }
    let mut new_run = authorized.clone();
    new_run.run = id(9);
    new_run.part = id(10);
    new_run
        .verify_request(&new_run, &p, &idx, &n("host"), &intent, &bytes)
        .unwrap();
}

#[test]
fn parameter_tampering_and_non_parameter_intent_changes_are_rejected() {
    let p = policy(1, 1);
    let idx = ReportIndex::decode(&canonical::bytes(&index(1, 1)).unwrap(), &p).unwrap();
    let (authorized, intent, bytes) = selected(&p);
    assert!(
        authorized
            .verify_request(&authorized, &p, &idx, &n("foreign"), &intent, &bytes)
            .is_err()
    );
    assert!(
        authorized
            .verify_request(&authorized, &p, &idx, &n("host"), &intent, b"changed")
            .is_err()
    );
    let raw = serde_json::to_value(&intent).unwrap();
    for (field, value) in [
        ("target", serde_json::json!("foreign")),
        ("profile_digest", serde_json::json!(digest(9))),
        ("site_config_digest", serde_json::json!(digest(9))),
        ("calibration_digests", serde_json::json!([digest(9)])),
        ("resource_set", serde_json::json!(["foreign"])),
        ("execution_timeout_ms", serde_json::json!("999")),
        ("prepare_validity_ms", serde_json::json!("99")),
        ("completion_rule", serde_json::json!("weaker")),
        ("cancel_rule", serde_json::json!("different")),
    ] {
        let mut changed = raw.clone();
        changed[field] = value;
        let submitted: Intent = serde_json::from_value(changed).unwrap();
        assert!(
            authorized
                .verify_request(&authorized, &p, &idx, &n("host"), &submitted, &bytes)
                .is_err(),
            "{field}"
        );
    }
}

#[test]
fn v2_plan_pins_graph_order_and_refuses_legacy_or_altered_templates() {
    use rx_process_contract::{CompiledBody, CompiledNode, ResolvedProcess, SourceLocation};
    let mut p = policy(1, 1);
    p.templates
        .insert(n("second"), p.templates[&n("node")].clone());
    p.node_contracts
        .insert(n("second"), p.node_contracts[&n("node")].clone());
    let process = n("plan/test");
    let make = |label: &str, body| {
        let source = SourceLocation {
            flow: n("main"),
            node: n(label),
            instantiation: vec![],
        };
        let hash = canonical::digest("RX-PROCESS-NODE-v1", &(&process, &source)).unwrap();
        CompiledNode {
            id: n(&format!("node/{hash}")),
            source,
            body,
        }
    };
    let a = make("a", CompiledBody::Operation { binding: n("node") });
    let b = make(
        "b",
        CompiledBody::Operation {
            binding: n("second"),
        },
    );
    let binding = Binding {
        schema: n(BINDING_SCHEMA),
        publication: reference(),
        policy: artifact(POLICY_SCHEMA, &canonical::bytes(&p).unwrap()),
        nodes: [(a.id.clone(), n("node")), (b.id.clone(), n("second"))].into(),
    };
    let plan = Plan {
        schema: n(PLAN_SCHEMA),
        binding,
        process: ResolvedProcess {
            schema: n("rx.resolved-process.v1"),
            package_digest: None,
            source_digest: digest(9),
            process: process.clone(),
            root: make(
                "root",
                CompiledBody::Sequence {
                    children: vec![a, b],
                },
            ),
            bindings: p.templates.clone(),
            conditions: BTreeMap::new(),
        },
    };
    plan.verify_policy(&p, &[n("node"), n("second")]).unwrap();
    assert_eq!(plan.reference().unwrap().schema_id, n(PLAN_SCHEMA));
    assert!(canonical::decode_json::<ResolvedProcess>(&canonical::bytes(&plan).unwrap()).is_err());
    let mut reordered = plan.clone();
    let CompiledBody::Sequence { children } = &mut reordered.process.root.body else {
        unreachable!()
    };
    children.reverse();
    assert!(
        reordered
            .verify_policy(&p, &[n("node"), n("second")])
            .is_err()
    );
    let mut mutated = plan.clone();
    mutated
        .process
        .bindings
        .get_mut(&n("node"))
        .unwrap()
        .intent
        .execution_timeout_ms = Counter(1);
    assert!(
        mutated
            .verify_policy(&p, &[n("node"), n("second")])
            .is_err()
    );
    let mut omitted = plan.clone();
    omitted.binding.nodes.pop_first();
    assert!(omitted.validate().is_err());
    let mut legacy = plan;
    legacy.schema = n("rx.resolved-process.v1");
    assert!(legacy.validate().is_err());
}

#[test]
fn host_v2_receipt_binds_policy_and_never_accepts_bare_v1_confirmation() {
    let value = host_v2_observation();
    value.validate().unwrap();
    let old = rx_domain::host_configuration::Observation {
        schema: n("rx.host-process-configuration-observation.v1"),
        snapshot: value.snapshot.clone(),
        receipt: Some(value.receipt.as_ref().unwrap().context.clone()),
        context_matches_current_host: true,
        activation_authorized: false,
    };
    old.validate().unwrap();
    assert!(
        canonical::decode_json::<host_configuration::Observation>(&canonical::bytes(&old).unwrap())
            .is_err()
    );
    let mut missing = value.clone();
    missing.policies.clear();
    assert!(missing.validate().is_err());
    let mut wrong = value.clone();
    wrong
        .receipt
        .as_mut()
        .unwrap()
        .policies
        .get_mut(&n("cell"))
        .unwrap()
        .policy
        .sha256 = digest(99);
    assert!(wrong.validate().is_err());
    let mut old_boot = value.clone();
    old_boot.snapshot.host_boot = id(99);
    assert!(old_boot.validate().is_err());
    old_boot.context_matches_current_host = false;
    old_boot.validate().unwrap();
    let mut absent = value;
    absent.receipt = None;
    absent.context_matches_current_host = false;
    absent.validate().unwrap();
}

#[test]
fn host_v2_request_rejects_missing_policy_foreign_host_and_missing_package() {
    let request = host_v2_observation().receipt.unwrap().request;
    request.validate().unwrap();
    let mut wrong = request.clone();
    wrong.policies.clear();
    assert!(wrong.validate().is_err());
    let mut wrong = request.clone();
    wrong.context.host = n("other");
    assert!(wrong.validate().is_err());
    let mut wrong = request.clone();
    wrong.policies.get_mut(&n("cell")).unwrap().packages.clear();
    assert!(wrong.validate().is_err());
    let mut observer = request.clone();
    observer.context.host = n("observer");
    observer.context.cells[0].required_intents.clear();
    observer
        .policies
        .get_mut(&n("cell"))
        .unwrap()
        .packages
        .clear();
    observer.validate().unwrap();
    let mut wrong = request;
    wrong.context.cells[0].required_intents.clear();
    assert!(wrong.validate().is_err());
}
