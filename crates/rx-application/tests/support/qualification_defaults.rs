use super::*;
use rx_application::{closure, intervention::CaseType, procedure};

type PolicyApp<A> = Engine<FaultRepository, ManualClock, A>;

struct DefaultPolicyAuthority;
impl QualificationAuthority for DefaultPolicyAuthority {
    // Preserve ordinary qualification; inherit both policy-verification defaults.
    fn verify(&self, c: &CellConfiguration, e: &[ArtifactRef], d: &[Digest]) -> bool {
        SimulationAuthority.verify(c, e, d)
    }
}

fn procedure_policy(f: &Fixture) -> procedure::Policy {
    procedure::Policy {
        external_procedure: artifact(92, "rx.test.external-procedure.v1"),
        dependencies: vec![artifact(93, "rx.test.entry-function.v1")],
        schema: name("rx.procedure-policy.v1"),
        cell: f.configuration.id.clone(),
        definition: f.configuration.definition.sha256,
        envelope: f.configuration.envelope.sha256,
        case_types: vec![CaseType::PlannedAccess],
        entry_conditions: f.configuration.start_conditions.clone(),
        steps: [
            procedure::Action::EntryConditionsReported,
            procedure::Action::WorkStarted,
            procedure::Action::WorkFinished,
            procedure::Action::PersonnelAccounted,
            procedure::Action::HandoverAccepted,
        ]
        .into_iter()
        .map(|action| procedure::Step {
            id: name(&format!("step/{action:?}")),
            action,
            actors: vec![f.operator.principal.clone()],
            required_evidence: vec![],
            conditions: vec![],
        })
        .collect(),
        maximum_report_age_ns: Counter(20000),
    }
}

fn policy_engine<A: QualificationAuthority>(
    f: Fixture,
    authority: A,
) -> (tempfile::TempDir, PolicyApp<A>, Identity) {
    let installation = f.app.installation.id.clone();
    let mut app = Engine::open(
        f.app.into_repository(),
        f.clock,
        authority,
        installation,
        principal(
            "admin",
            &[
                Role::AccountAdmin,
                Role::Engineer,
                Role::Verifier,
                Role::Observer,
            ],
        ),
    )
    .unwrap();
    // Reopening requires a fresh admin session. Keep the recorded policies and
    // observations at the same manual-clock instant; do not restore motion rights.
    let login = app
        .authenticated_session(&f.admin.principal, id(), expiry(100000))
        .unwrap();
    let admin = Identity {
        session: login.id,
        ..f.admin
    };
    (f._directory, app, admin)
}

#[test]
fn procedure_admission_requires_an_explicit_qualification_capability() {
    let allowed = fixture(1, true);
    let policy = procedure_policy(&allowed);
    let reference = content_ref("rx.procedure-policy.v1", &policy);
    let (_allowed_dir, mut trusted, admin) = policy_engine(allowed, SimulationAuthority);
    trusted
        .admit_procedure(&admin, policy.clone(), reference.clone())
        .unwrap();

    let (_denied_dir, mut defaults, admin) =
        policy_engine(fixture(1, true), DefaultPolicyAuthority);
    let result = defaults.admit_procedure(&admin, policy, reference);
    assert!(
        matches!(
            result,
            Err(StoreError::Rejected(Rejection::QualificationRequired))
        ),
        "procedure admission with inherited defaults returned {result:?}"
    );
}

#[test]
fn close_policy_admission_requires_an_explicit_qualification_capability() {
    let mut allowed = fixture(1, true);
    let mut denied = fixture(1, true);
    let procedure = procedure_policy(&allowed);
    let procedure_reference = content_ref("rx.procedure-policy.v1", &procedure);
    // Close-policy admission requires an admitted procedure. Establish it with
    // the existing trusted authority before testing the independent close guard.
    for f in [&mut allowed, &mut denied] {
        f.app
            .admit_procedure(&f.admin, procedure.clone(), procedure_reference.clone())
            .unwrap();
    }
    let policy = closure::Policy {
        schema: name("rx.close-policy.v1"),
        cell: allowed.configuration.id.clone(),
        procedure: procedure_reference,
        external_procedure: artifact(94, "rx.test.out-of-service-procedure.v1"),
        dependencies: vec![artifact(95, "rx.test.external-containment.v1")],
        contexts: vec![closure::ScopePolicy {
            cell: allowed.configuration.id.clone(),
            definition: allowed.configuration.definition.sha256,
            envelope: allowed.configuration.envelope.sha256,
            conditions: allowed.configuration.start_conditions.clone(),
        }],
        maximum_validity_ns: Counter(5000),
    };
    let reference = content_ref("rx.close-policy.v1", &policy);
    let (_allowed_dir, mut trusted, admin) = policy_engine(allowed, SimulationAuthority);
    trusted
        .admit_close_policy(&admin, policy.clone(), reference.clone())
        .unwrap();

    let (_denied_dir, mut defaults, admin) = policy_engine(denied, DefaultPolicyAuthority);
    let result = defaults.admit_close_policy(&admin, policy, reference);
    assert!(
        matches!(
            result,
            Err(StoreError::Rejected(Rejection::QualificationRequired))
        ),
        "close-policy admission with inherited defaults returned {result:?}"
    );
}
