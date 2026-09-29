use rx_domain::{
    intent::{Body, Intent, Kind, ProgramGoal},
    types::*,
};
use rx_process_contract::program_inputs::{self, Policy};
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn reference(byte: u8) -> ArtifactRef {
    ArtifactRef {
        sha256: Digest::from_bytes([byte; 32]),
        schema_id: n("job-input.v1"),
        size_bytes: Counter(100),
    }
}
fn template() -> Intent {
    Intent {
        kind: Kind::FiniteAction,
        target: n("device"),
        profile_digest: Digest::from_bytes([3; 32]),
        site_config_digest: Digest::from_bytes([4; 32]),
        calibration_digests: vec![],
        resource_set: vec![n("resource")],
        execution_timeout_ms: Counter(1000),
        prepare_validity_ms: Counter(100),
        completion_rule: n("completion"),
        cancel_rule: n("cancel"),
        body: Body::Program(ProgramGoal {
            program: ArtifactRef {
                schema_id: n("program.v1"),
                ..reference(1)
            },
            parameter_set: reference(2),
        }),
    }
}
fn policy(t: &Intent) -> Policy {
    Policy {
        schema: n(program_inputs::SCHEMA),
        template_digest: t.digest().unwrap(),
        parameter_sets: vec![reference(2), reference(5)],
    }
}
fn selected(t: &Intent) -> Intent {
    let mut t = t.clone();
    let Body::Program(g) = &mut t.body else {
        unreachable!()
    };
    g.parameter_set = reference(5);
    t
}
#[test]
fn absent_policy_preserves_exact_admission() {
    let t = template();
    assert!(program_inputs::accepts(&t, None, &t).unwrap());
    assert!(!program_inputs::accepts(&t, None, &selected(&t)).unwrap());
}
#[test]
fn approved_choice_changes_only_parameter_reference() {
    let t = template();
    let p = policy(&t);
    let alternative = selected(&t);
    assert!(program_inputs::accepts(&t, Some(&p), &t).unwrap());
    assert!(program_inputs::accepts(&t, Some(&p), &alternative).unwrap());
    assert_eq!(p.variants(&t).unwrap().len(), 2);
}
#[test]
fn every_non_input_field_remains_bound() {
    let t = template();
    let p = policy(&t);
    let mut mutations = vec![];
    let mut v = selected(&t);
    v.target = n("foreign");
    mutations.push(v);
    let mut v = selected(&t);
    v.resource_set = vec![n("foreign")];
    mutations.push(v);
    let mut v = selected(&t);
    v.profile_digest = Digest::from_bytes([9; 32]);
    mutations.push(v);
    let mut v = selected(&t);
    v.site_config_digest = Digest::from_bytes([9; 32]);
    mutations.push(v);
    let mut v = selected(&t);
    v.calibration_digests = vec![Digest::from_bytes([9; 32])];
    mutations.push(v);
    let mut v = selected(&t);
    v.execution_timeout_ms = Counter(999);
    mutations.push(v);
    let mut v = selected(&t);
    v.prepare_validity_ms = Counter(99);
    mutations.push(v);
    let mut v = selected(&t);
    v.completion_rule = n("weaker");
    mutations.push(v);
    let mut v = selected(&t);
    v.cancel_rule = n("different");
    mutations.push(v);
    let mut v = selected(&t);
    let Body::Program(g) = &mut v.body else {
        unreachable!()
    };
    g.program = reference(9);
    mutations.push(v);
    for v in mutations {
        assert!(!program_inputs::accepts(&t, Some(&p), &v).unwrap());
    }
}
#[test]
fn unapproved_hash_or_forged_reference_metadata_is_refused() {
    let t = template();
    let p = policy(&t);
    for r in [
        reference(9),
        ArtifactRef {
            size_bytes: Counter(99),
            ..reference(5)
        },
        ArtifactRef {
            schema_id: n("foreign.v1"),
            ..reference(5)
        },
    ] {
        let mut v = selected(&t);
        let Body::Program(g) = &mut v.body else {
            unreachable!()
        };
        g.parameter_set = r;
        assert!(!program_inputs::accepts(&t, Some(&p), &v).unwrap());
    }
}
#[test]
fn malformed_policy_never_falls_back_to_default_allow() {
    let t = template();
    let base = policy(&t);
    let mut invalid = vec![];
    let mut p = base.clone();
    p.schema = n("unknown.v1");
    invalid.push(p);
    let mut p = base.clone();
    p.template_digest = Digest::from_bytes([9; 32]);
    invalid.push(p);
    let mut p = base.clone();
    p.parameter_sets.clear();
    invalid.push(p);
    let mut p = base.clone();
    p.parameter_sets = vec![reference(5)];
    invalid.push(p);
    let mut p = base.clone();
    p.parameter_sets.push(reference(5));
    invalid.push(p);
    let mut p = base.clone();
    p.parameter_sets[1].size_bytes = Counter(0);
    invalid.push(p);
    let mut p = base.clone();
    p.parameter_sets[1].size_bytes = Counter(program_inputs::MAX_PARAMETER_BYTES + 1);
    invalid.push(p);
    let mut p = base.clone();
    p.parameter_sets[1].schema_id = n("foreign.v1");
    invalid.push(p);
    let mut p = base;
    p.parameter_sets = (0..65).map(reference).collect();
    invalid.push(p);
    for p in invalid {
        assert!(program_inputs::accepts(&t, Some(&p), &t).is_err());
    }
}
#[test]
fn policy_digest_is_order_independent_but_content_bound() {
    let t = template();
    let mut p = policy(&t);
    let digest = p.digest(&t).unwrap();
    p.parameter_sets.reverse();
    assert_eq!(digest, p.digest(&t).unwrap());
    p.parameter_sets[0] = reference(6);
    assert_ne!(digest, p.digest(&t).unwrap());
}

#[test]
fn legacy_action_bytes_omit_absent_policy_and_policy_requires_versioned_input() {
    use rx_process_contract::{compile_input::CompileInput, model::ActionBinding};
    use std::collections::BTreeMap;
    let t = template();
    let action = ActionBinding {
        host: n("host"),
        intent: t.clone(),
        program_inputs: None,
    };
    let bytes = rx_domain::canonical::bytes(&action).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(!value.as_object().unwrap().contains_key("program_inputs"));
    let source = serde_json::json!({"schema":"rx.process-source.v1","process":"input-test","entry":"main","conditions":{},"flows":[{"id":"main","root":"op","nodes":[{"id":"op","body":{"kind":"OPERATION","binding":"action"}}]}]});
    let bindings = BTreeMap::from([(
        n("action"),
        ActionBinding {
            program_inputs: Some(policy(&t)),
            ..action
        },
    )]);
    let mut input = CompileInput {
        schema: n("rx.process-compile-input.v1"),
        device_sources: BTreeMap::new(),
        draft: Id::new("00000000-0000-4000-8000-000000000001").unwrap(),
        cell: n("cell"),
        source_revision: Counter(1),
        binding_revision: Counter(1),
        source_document_digest: rx_domain::canonical::digest(
            "RX-PROCESS-DRAFT-DOCUMENT-v1",
            &source,
        )
        .unwrap(),
        bindings_digest: rx_process_contract::compile_input::bindings_digest(
            &bindings,
            &BTreeMap::new(),
        )
        .unwrap(),
        catalog_digest: Digest::from_bytes([1; 32]),
        source,
        bindings,
    };
    assert!(input.validate().is_err());
    input.schema = n("rx.process-compile-input.v3");
    assert!(input.validate().is_ok());
}
