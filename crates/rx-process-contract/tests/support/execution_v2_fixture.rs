use rx_domain::{
    canonical,
    definition::Reference,
    intent::{Body, Intent, Kind, ProgramGoal},
    types::*,
};
use rx_process_contract::{ActionBinding, execution_v2::*};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

pub(super) fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
pub(super) fn id(s: u8) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{s:012}")).unwrap()
}
pub(super) fn digest(s: u8) -> Digest {
    Digest::from_bytes([s; 32])
}
pub(super) fn artifact(schema: &str, bytes: &[u8]) -> ArtifactRef {
    ArtifactRef {
        schema_id: n(schema),
        sha256: Digest::from_bytes(Sha256::digest(bytes).into()),
        size_bytes: Counter(bytes.len() as u64),
    }
}
pub(super) fn reference() -> Reference {
    Reference {
        catalog: id(1),
        id: id(2),
        revision: Counter(1),
        digest: digest(3),
    }
}
pub(super) fn index(models: usize, slots: usize) -> ReportIndex {
    ReportIndex {
        schema: n(INDEX_SCHEMA),
        entries: (0..models)
            .flat_map(|m| (0..slots).map(move |s| (m as u8, s as u16, digest(5))))
            .collect(),
    }
}
pub(super) fn policy(models: usize, slots: usize) -> Policy {
    let template = Intent {
        kind: Kind::FiniteAction,
        target: n("device"),
        profile_digest: digest(1),
        site_config_digest: digest(2),
        calibration_digests: vec![],
        resource_set: vec![n("resource")],
        execution_timeout_ms: Counter(1000),
        prepare_validity_ms: Counter(100),
        completion_rule: n("done"),
        cancel_rule: n("stop"),
        body: Body::Program(ProgramGoal {
            program: artifact("test.program.v1", b"program"),
            parameter_set: artifact(PARAMETER_SCHEMA, b"template"),
        }),
    };
    Policy {
        schema: n(POLICY_SCHEMA),
        workflow: reference(),
        definition_closure: artifact("rx.execution-input-closure.v2", b"closure"),
        resolver_digest: digest(2),
        compiler_digest: digest(3),
        candidates: (0..models)
            .map(|m| Candidate {
                key: n(&format!("variant/{m}")),
                object_model: reference(),
                context_digest: digest(m as u8 + 1),
            })
            .collect(),
        slot_order: (0..slots).map(|s| s as u16).collect(),
        templates: BTreeMap::from([(
            n("node"),
            ActionBinding {
                host: n("host"),
                intent: template,
            },
        )]),
        node_contracts: BTreeMap::from([(
            n("node"),
            NodeContract {
                implementation: "fixture".into(),
                version: "1".into(),
                primitive: n("act"),
                parameters: BTreeMap::new(),
            },
        )]),
        report_index: artifact(
            INDEX_SCHEMA,
            &canonical::bytes(&index(models, slots)).unwrap(),
        ),
    }
}
pub(super) fn selected(p: &Policy) -> (Selection, Intent, Vec<u8>) {
    let bytes = br#"{"force":25,"schema":"rx.workflow-parameters.v2"}"#.to_vec();
    let parameter = artifact(PARAMETER_SCHEMA, &bytes);
    let mut intent = p.templates[&n("node")].intent.clone();
    let Body::Program(goal) = &mut intent.body else {
        unreachable!()
    };
    goal.parameter_set = parameter.clone();
    (
        Selection {
            schema: n("rx.execution-selection.v2"),
            publication: id(3),
            policy_digest: p.digest().unwrap(),
            configuration_digest: digest(9),
            run: id(4),
            part: id(5),
            ordinal: Counter(1),
            object: reference(),
            object_values_digest: digest(8),
            candidate: 0,
            slot: 0,
            report_digest: digest(5),
            node: n("node"),
            parameter,
            intent_digest: intent.digest().unwrap(),
            authority_generation: Counter(1),
        },
        intent,
        bytes,
    )
}

pub(super) fn host_v2_observation() -> host_configuration::Observation {
    use host_configuration as h;
    use rx_domain::host_configuration as old;
    let policy = policy(1, 2);
    let policy_ref = artifact(POLICY_SCHEMA, &canonical::bytes(&policy).unwrap());
    let context = old::Request {
        schema: n("rx.host-process-configuration-request.v1"),
        id: id(6),
        change: id(3),
        preparation: Counter(1),
        plan_digest: digest(5),
        host: n("host"),
        expected_host_boot: id(4),
        expected_delivery_journal: id(5),
        binding_digest: digest(6),
        cells: vec![old::CellTarget {
            cell: n("cell"),
            expected_context: None,
            before_configuration: digest(7),
            after_configuration: digest(8),
            recipe: artifact(PLAN_SCHEMA, b"plan"),
            definition: digest(9),
            envelope: digest(10),
            environment: n("SIMULATION"),
            required_intents: vec![policy.templates[&n("node")].intent.digest().unwrap()],
            required_conditions: vec![n("ready")],
            epoch: Counter(1),
            scopes: [(n("scope"), Counter(1))].into(),
            fence_request: id(7),
        }],
    };
    let request = h::Request {
        schema: n(h::REQUEST_SCHEMA),
        context: context.clone(),
        policies: [(
            n("cell"),
            h::CellPolicy {
                publication: reference(),
                reference: policy_ref.clone(),
                policy,
                packages: [(
                    n("node"),
                    h::Package {
                        manifest: digest(11),
                        signature: digest(12),
                        catalog: artifact(TEMPLATE_CATALOG_SCHEMA, b"catalog"),
                        template: n("template"),
                    },
                )]
                .into(),
            },
        )]
        .into(),
    };
    let policies: BTreeMap<_, _> = [(
        n("cell"),
        h::AppliedPolicy {
            publication: reference(),
            policy: policy_ref,
            request: context.id.clone(),
            receipt_sequence: Counter(1),
            configuration: digest(8),
        },
    )]
    .into();
    let at = TimePoint {
        clock_id: "test".into(),
        ticks_ns: Counter(1000),
    };
    let facts = old::Receipt {
        schema: n("rx.host-process-configuration-receipt.v1"),
        request_digest: context.digest().unwrap(),
        request: context,
        host_boot: id(4),
        journal: id(5),
        sequence: Counter(1),
        status: old::Status::AppliedUnqualified,
        effect: old::Effect::Installed,
        reason: None,
        quiescence: Some(old::Quiescence {
            device_session: id(8),
            observed_at: at.clone(),
            uncertainty_ns: Counter(0),
            resources: vec![n("resource")],
        }),
        recorded_at: at,
    };
    h::Observation {
        schema: n(h::OBSERVATION_SCHEMA),
        snapshot: old::Snapshot {
            schema: n("rx.host-process-configuration-snapshot.v1"),
            host: n("host"),
            host_boot: id(4),
            delivery_journal: id(5),
            binding_digest: digest(6),
            evidence_journal: None,
            installation_identity: None,
            binding_commit: None,
            cells: vec![old::CellObservation {
                cell: n("cell"),
                definition: digest(9),
                envelope: digest(10),
                environment: n("SIMULATION"),
                epoch: Counter(1),
                scopes: [(n("scope"), Counter(1))].into(),
                blocked: vec![],
                applied: Some(old::AppliedContext {
                    cell: n("cell"),
                    configuration: digest(8),
                    change: id(3),
                    request: id(6),
                    receipt_sequence: Counter(1),
                    binding_digest: digest(6),
                }),
            }],
        },
        receipt: Some(h::Receipt {
            schema: n(h::RECEIPT_SCHEMA),
            request_digest: request.digest().unwrap(),
            request,
            context: facts,
            policies: policies.clone(),
        }),
        policies,
        context_matches_current_host: true,
        activation_authorized: false,
    }
}
