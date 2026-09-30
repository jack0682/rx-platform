use rx_application::host_binding_transition::*;
use rx_domain::{host_configuration::*, types::*};
use std::collections::BTreeMap;
fn n(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id(v: u8) -> Id {
    Id::new(format!("00000000-0000-4000-8000-{v:012}")).unwrap()
}
fn d(v: u8) -> Digest {
    Digest::from_bytes([v; 32])
}
fn time(v: u64) -> TimePoint {
    TimePoint {
        clock_id: "clock/test".into(),
        ticks_ns: Counter(v),
    }
}
fn setup() -> (Intent, Baseline, Observation, ReadContext) {
    let cells = BTreeMap::from([(
        n("cell/a"),
        CellIdentity {
            definition: d(3),
            envelope: d(4),
            environment: n("SIMULATION"),
        },
    )]);
    let intent = Intent {
        request: id(5),
        change: id(6),
        host: n("host/a"),
        cell: n("cell/a"),
        host_plan_digest: d(9),
        before_configuration: d(10),
        after_configuration: d(11),
        before_cells: cells.clone(),
        after_cells: cells,
        runtime_boot: id(1),
        created_at: time(100),
    };
    let before = Observation {
        schema: n("rx.host-process-configuration-observation.v1"),
        snapshot: Snapshot {
            schema: n("rx.host-process-configuration-snapshot.v1"),
            host: n("host/a"),
            host_boot: id(2),
            delivery_journal: id(3),
            evidence_journal: Some(id(4)),
            binding_digest: d(1),
            installation_identity: Some(d(1)),
            binding_commit: None,
            cells: vec![CellObservation {
                cell: n("cell/a"),
                definition: d(3),
                envelope: d(4),
                environment: n("SIMULATION"),
                epoch: Counter(1),
                scopes: BTreeMap::from([(n("scope/a"), Counter(1))]),
                blocked: vec![],
                applied: None,
            }],
        },
        receipt: None,
        context_matches_current_host: false,
        activation_authorized: false,
    };
    let mut read = ReadContext {
        runtime_boot: id(1),
        producer_session: id(7),
        host_boot: id(2),
        delivery_journal: id(3),
        started_at: time(101),
        now: time(102),
    };
    let baseline = capture_baseline(&intent, &before, &read).unwrap();
    let mut after = before;
    after.snapshot.host_boot = id(8);
    after.snapshot.installation_identity = Some(d(2));
    after.snapshot.binding_digest = d(7);
    after.snapshot.binding_commit = Some(BindingCommit {
        schema: n("rx.host-binding-commit-observation.v1"),
        request: id(5),
        plan_digest: d(9),
        cell: n("cell/a"),
        before_configuration: d(10),
        after_configuration: d(11),
        before_installation_identity: d(1),
        after_installation_identity: d(2),
        delivery_journal: id(3),
        evidence_journal: id(4),
        binding_digest: d(7),
    });
    read.host_boot = id(8);
    read.producer_session = id(9);
    read.started_at = time(103);
    read.now = time(104);
    (intent, baseline, after, read)
}
#[test]
fn exact_original_intent_matches_only_fresh_current_metadata() {
    let (i, b, a, r) = setup();
    confirm(&i, &b, &a, &r).unwrap();
    assert!(!a.activation_authorized);
    let mut r = r;
    r.now = time(4_000_000_104);
    assert_eq!(confirm(&i, &b, &a, &r), Err(Rejection::StaleRead));
}
#[test]
fn request_plan_configuration_and_both_journals_are_not_interchangeable() {
    let (i, b, a, mut r) = setup();
    let mut wrong = a.clone();
    wrong.snapshot.binding_commit.as_mut().unwrap().request = id(30);
    assert_eq!(confirm(&i, &b, &wrong, &r), Err(Rejection::RequestChanged));
    wrong = a.clone();
    wrong.snapshot.binding_commit.as_mut().unwrap().plan_digest = d(30);
    assert_eq!(confirm(&i, &b, &wrong, &r), Err(Rejection::PlanChanged));
    wrong = a.clone();
    wrong
        .snapshot
        .binding_commit
        .as_mut()
        .unwrap()
        .after_configuration = d(30);
    assert_eq!(
        confirm(&i, &b, &wrong, &r),
        Err(Rejection::ConfigurationChanged)
    );
    wrong = a.clone();
    wrong
        .snapshot
        .binding_commit
        .as_mut()
        .unwrap()
        .before_installation_identity = d(30);
    assert_eq!(
        confirm(&i, &b, &wrong, &r),
        Err(Rejection::InstallationChanged)
    );
    wrong = a.clone();
    wrong.snapshot.evidence_journal = Some(id(30));
    wrong
        .snapshot
        .binding_commit
        .as_mut()
        .unwrap()
        .evidence_journal = id(30);
    assert_eq!(confirm(&i, &b, &wrong, &r), Err(Rejection::JournalChanged));
    wrong = a;
    wrong.snapshot.delivery_journal = id(30);
    wrong
        .snapshot
        .binding_commit
        .as_mut()
        .unwrap()
        .delivery_journal = id(30);
    r.delivery_journal = id(30);
    assert_eq!(confirm(&i, &b, &wrong, &r), Err(Rejection::JournalChanged));
}
#[test]
fn absent_baseline_identity_stale_boot_cohort_and_permission_claim_are_rejected() {
    let (i, b, a, mut r) = setup();
    let mut legacy = a.clone();
    legacy.snapshot.binding_commit = None;
    legacy.snapshot.evidence_journal = None;
    assert!(matches!(
        capture_baseline(&i, &legacy, &r),
        Err(Rejection::MissingIdentity)
    ));
    let mut wrong = a.clone();
    wrong.snapshot.binding_commit = None;
    assert_eq!(confirm(&i, &b, &wrong, &r), Err(Rejection::MissingCommit));
    wrong = a.clone();
    wrong.activation_authorized = true;
    assert_eq!(
        confirm(&i, &b, &wrong, &r),
        Err(Rejection::InvalidObservation)
    );
    wrong = a.clone();
    let mut other = wrong.snapshot.cells[0].clone();
    other.cell = n("cell/other");
    wrong.snapshot.cells.push(other);
    assert_eq!(confirm(&i, &b, &wrong, &r), Err(Rejection::CohortChanged));
    r.runtime_boot = id(30);
    assert_eq!(confirm(&i, &b, &a, &r), Err(Rejection::RuntimeChanged));
    r.runtime_boot = i.runtime_boot.clone();
    r.host_boot = b.snapshot.host_boot.clone();
    wrong = a;
    wrong.snapshot.host_boot = r.host_boot.clone();
    assert_eq!(
        confirm(&i, &b, &wrong, &r),
        Err(Rejection::HostNotRestarted)
    );
}
#[test]
fn an_already_committed_request_cannot_supply_its_own_before_baseline() {
    let (i, _b, a, r) = setup();
    assert!(matches!(
        capture_baseline(&i, &a, &r),
        Err(Rejection::AlreadyCommittedBeforeBaseline)
    ));
}
