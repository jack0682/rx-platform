use rx_application::*;
use rx_domain::{condition::Condition, intent::*, types::*};
fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn artifact(n: u8) -> ArtifactRef {
    ArtifactRef {
        sha256: Digest::from_bytes([n; 32]),
        schema_id: name("rx.test.v1"),
        size_bytes: Counter(1),
    }
}
pub fn configuration(cell: &str) -> CellConfiguration {
    let condition = Condition::Eq {
        fact: name("ready"),
        schema: name("boolean/v1"),
        unit: name("unitless"),
        expected: TypedValue::Boolean(true),
    };
    CellConfiguration {
        process: None,
        id: name(cell),
        environment: Environment::Simulation,
        definition: artifact(1),
        envelope: artifact(2),
        recipe: artifact(3),
        site_config_digest: Digest::from_bytes([4; 32]),
        scopes: vec![name("zone/shared")],
        hosts: vec![name("host/sim")],
        executor: name("executor"),
        maximum_budget: Counter(10),
        permit_ttl_ns: Counter(100_000_000),
        start_timeout_ns: Counter(1_000_000_000),
        start_conditions: vec![condition.clone()],
        maintained_conditions: vec![],
        fact_specs: vec![FactSpec {
            id: name("ready"),
            host: name("host/sim"),
            schema: name("boolean/v1"),
            unit: name("unitless"),
            maximum_age_ns: Counter(1_000_000_000),
            maximum_uncertainty_ns: Counter(0),
        }],
        steps: vec![StepBinding {
            id: name("step/place"),
            host: name("host/sim"),
            predecessors: vec![],
            conditions: vec![condition],
            condition_ids: vec![name("ready")],
            condition_revision: Counter(1),
            handover_max_age_ns: Counter(1_000_000_000),
            completion: CompletionRule::Unobservable,
            intent: Intent {
                kind: Kind::EnsureState,
                target: name("device/sim"),
                profile_digest: Digest::from_bytes([8; 32]),
                site_config_digest: Digest::from_bytes([4; 32]),
                calibration_digests: vec![],
                resource_set: vec![name("controller/sim")],
                execution_timeout_ms: Counter(500),
                prepare_validity_ms: Counter(100),
                completion_rule: name("sim/closed"),
                cancel_rule: name("sim/stop"),
                body: rx_domain::intent::Body::Predicate(PredicateGoal {
                    predicate_id: name("closed"),
                    target: TypedValue::Boolean(true),
                    settle_ms: Counter(0),
                }),
            },
        }],
    }
}
