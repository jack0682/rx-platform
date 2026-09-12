use rx_application::*;
#[path = "support/native_outcomes.rs"]
mod native_outcomes_tests;
#[path = "support/runtime_invalidation_tests.rs"]
mod runtime_invalidation_tests;
use rx_domain::{condition::Condition, fault::Rejection, intent::*, types::*};
use rx_ports::{Record, Repository, StoreError, StoredEvent};
use rx_storage::SqliteRepository;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
};

fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}
fn id() -> Id {
    Id::new(uuid::Uuid::now_v7().to_string()).unwrap()
}
fn artifact(n: u8, schema: &str) -> ArtifactRef {
    ArtifactRef {
        sha256: Digest::from_bytes([n; 32]),
        schema_id: name(schema),
        size_bytes: Counter(1),
    }
}
#[derive(Clone)]
struct ManualClock(Arc<AtomicU64>);
impl Clock for ManualClock {
    fn now(&self) -> TimePoint {
        TimePoint {
            clock_id: "test/boottime".into(),
            ticks_ns: Counter(self.0.load(Ordering::SeqCst)),
        }
    }
}
struct SimulationAuthority;
impl QualificationAuthority for SimulationAuthority {
    fn verify_close_policy(
        &self,
        c: &CellConfiguration,
        _: &ArtifactRef,
        p: &rx_application::closure::Policy,
    ) -> bool {
        c.environment == Environment::Simulation && p.schema.as_str() == "rx.close-policy.v1"
    }
    fn verify_procedure(
        &self,
        c: &CellConfiguration,
        _: &ArtifactRef,
        p: &rx_application::procedure::Policy,
    ) -> bool {
        c.environment == Environment::Simulation && p.schema.as_str() == "rx.procedure-policy.v1"
    }
    fn verify(&self, c: &CellConfiguration, e: &[ArtifactRef], d: &[Digest]) -> bool {
        c.environment == Environment::Simulation
            && e == [artifact(9, "rx.validation.simulation.v1")]
            && d.contains(&c.definition.sha256)
            && d.contains(&c.envelope.sha256)
            && d.contains(&c.recipe.sha256)
            && d.contains(&c.site_config_digest)
    }
}
struct FaultRepository {
    inner: SqliteRepository,
    mode: Arc<AtomicU8>,
}
impl Repository for FaultRepository {
    fn pending_outbox_after(
        &mut self,
        after: Option<&Id>,
        limit: usize,
    ) -> rx_ports::Result<Vec<rx_ports::OutboxRecord>> {
        self.inner.pending_outbox_after(after, limit)
    }
    fn control_events_after(
        &mut self,
        after: Counter,
        limit: usize,
    ) -> rx_ports::Result<Vec<StoredEvent>> {
        self.inner.control_events_after(after, limit)
    }
    fn control_snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<Record>)> {
        self.inner.control_snapshot()
    }
    fn journal_head(&mut self) -> rx_ports::Result<Counter> {
        self.inner.journal_head()
    }
    fn pending_outbox(&mut self, limit: usize) -> rx_ports::Result<Vec<rx_ports::OutboxRecord>> {
        self.inner.pending_outbox(limit)
    }
    fn transact<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn rx_ports::Transaction) -> rx_ports::Result<T>,
    ) -> rx_ports::Result<T> {
        let mode = self.mode.swap(0, Ordering::SeqCst);
        let result = self.inner.transact(|tx| {
            let value = operation(tx)?;
            if mode == 1 {
                return Err(StoreError::Unavailable("injected before commit".into()));
            }
            Ok(value)
        })?;
        if mode == 2 {
            return Err(StoreError::Unavailable("committed, response lost".into()));
        }
        Ok(result)
    }
    fn snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<Record>)> {
        self.inner.snapshot()
    }
    fn events_after(&mut self, a: Counter, l: usize) -> rx_ports::Result<Vec<StoredEvent>> {
        self.inner.events_after(a, l)
    }
}
type App = Engine<FaultRepository, ManualClock, SimulationAuthority>;
struct Fixture {
    _directory: tempfile::TempDir,
    app: App,
    clock: ManualClock,
    failure: Arc<AtomicU8>,
    admin: Identity,
    operator: Identity,
    executor: Identity,
    hosts: Vec<Identity>,
    configuration: CellConfiguration,
    registrations: Vec<HostRegistration>,
}
fn principal(label: &str, roles: &[Role]) -> Principal {
    Principal {
        id: name(label),
        client_namespace: name(&format!("client/{label}")),
        roles: roles.iter().copied().collect(),
        cells: [name("cell/a"), name("cell/b")].into_iter().collect(),
        active: true,
    }
}
fn expiry(ticks: u64) -> TimePoint {
    TimePoint {
        clock_id: "test/boottime".into(),
        ticks_ns: Counter(ticks),
    }
}
fn fixture(host_count: usize, qualify: bool) -> Fixture {
    fixture_mode(host_count, qualify, false)
}
fn fixture_mode(host_count: usize, qualify: bool, maintained: bool) -> Fixture {
    fixture_complete(host_count, qualify, maintained, false)
}
fn fixture_complete(
    host_count: usize,
    qualify: bool,
    maintained: bool,
    native_result: bool,
) -> Fixture {
    fixture_with_process(host_count, qualify, maintained, native_result, None)
}
#[derive(Clone, Copy)]
enum TestProcess {
    Branch,
    Wait,
}
fn fixture_with_process(
    host_count: usize,
    qualify: bool,
    maintained: bool,
    native_result: bool,
    process: Option<TestProcess>,
) -> Fixture {
    fixture_with_peer(
        host_count,
        qualify,
        maintained,
        native_result,
        process,
        false,
    )
}
fn fixture_with_peer(
    host_count: usize,
    qualify: bool,
    maintained: bool,
    native_result: bool,
    process: Option<TestProcess>,
    peer_mode: bool,
) -> Fixture {
    fixture_registration(
        host_count,
        qualify,
        maintained,
        native_result,
        process,
        peer_mode,
        true,
    )
}
fn fixture_registration(
    host_count: usize,
    qualify: bool,
    maintained: bool,
    native_result: bool,
    process: Option<TestProcess>,
    peer_mode: bool,
    register: bool,
) -> Fixture {
    fixture_configured(
        (
            host_count,
            qualify,
            maintained,
            native_result,
            process,
            peer_mode,
            register,
        ),
        |configuration| configuration,
    )
}
fn fixture_configured(
    settings: (usize, bool, bool, bool, Option<TestProcess>, bool, bool),
    configure: impl FnOnce(CellConfiguration) -> CellConfiguration,
) -> Fixture {
    let (host_count, qualify, maintained, native_result, process, peer_mode, register) = settings;
    let directory = tempfile::tempdir().unwrap();
    let failure = Arc::new(AtomicU8::new(0));
    let repository = FaultRepository {
        inner: SqliteRepository::open(directory.path().join("platform.db")).unwrap(),
        mode: failure.clone(),
    };
    let clock = ManualClock(Arc::new(AtomicU64::new(1000)));
    let admin_p = principal(
        "admin",
        &[
            Role::AccountAdmin,
            Role::Engineer,
            Role::Verifier,
            Role::Observer,
        ],
    );
    let mut app = Engine::open(
        repository,
        clock.clone(),
        SimulationAuthority,
        id(),
        admin_p,
    )
    .unwrap();
    let login = app
        .authenticated_session(&name("admin"), id(), expiry(100000))
        .unwrap();
    let admin = Identity {
        principal: name("admin"),
        session: login.id,
        terminal: None,
    };
    let operator = add_identity(
        &mut app,
        &admin,
        "operator",
        &[Role::Operator, Role::Observer],
    );
    let executor = if peer_mode {
        app.put_principal(
            &admin,
            principal("executor", &[Role::Executor, Role::Observer]),
            None,
        )
        .unwrap();
        let session = app
            .open_executor_peer(&name("executor"), id(), Digest::from_bytes([71; 32]))
            .unwrap();
        Identity {
            principal: name("executor"),
            session: session.id,
            terminal: None,
        }
    } else {
        add_identity(
            &mut app,
            &admin,
            "executor",
            &[Role::Executor, Role::Observer],
        )
    };
    let hosts = (0..host_count)
        .map(|i| add_identity(&mut app, &admin, &format!("host/{i}"), &[Role::Host]))
        .collect::<Vec<_>>();
    let terminal = Terminal {
        id: name("panel/main"),
        certificate_digest: Digest::from_bytes([77; 32]),
        cells: [name("cell/a"), name("cell/b")].into_iter().collect(),
        active: true,
    };
    app.put_terminal(&admin, terminal, None).unwrap();
    let bound = app
        .authenticated_terminal_user_session(
            &operator.principal,
            id(),
            Counter(99_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap();
    let operator = Identity {
        session: bound.id,
        terminal: Some((name("panel/main"), Digest::from_bytes([77; 32]))),
        ..operator
    };
    let condition = Condition::Eq {
        fact: name("ready"),
        schema: name("boolean/v1"),
        unit: name("unitless"),
        expected: TypedValue::Boolean(true),
    };
    let steps: Vec<StepBinding> = (0..host_count)
        .map(|i| StepBinding {
            id: name(&format!("step/{i}")),
            host: name(&format!("host/{i}")),
            intent: Intent {
                kind: Kind::EnsureState,
                target: name(&format!("sim/device-{i}")),
                profile_digest: Digest::from_bytes([21; 32]),
                site_config_digest: Digest::from_bytes([4; 32]),
                calibration_digests: vec![],
                resource_set: vec![name(&format!("controller/{i}"))],
                execution_timeout_ms: Counter(5000),
                prepare_validity_ms: Counter(1000),
                completion_rule: name("sim/closed"),
                cancel_rule: name("sim/stop"),
                body: Body::Predicate(PredicateGoal {
                    predicate_id: name("fixture.closed"),
                    target: TypedValue::Boolean(true),
                    settle_ms: Counter(0),
                }),
            },
            predecessors: vec![],
            conditions: vec![condition.clone()],
            completion: CompletionRule::Unobservable,
            condition_ids: vec![name("sim/ready")],
            condition_revision: Counter(1),
            handover_max_age_ns: Counter(500),
        })
        .collect();
    let mut steps: Vec<StepBinding> = steps;
    if native_result {
        for step in &mut steps {
            step.intent.kind = Kind::FiniteAction;
            step.intent.body = Body::Program(ProgramGoal {
                program: artifact(10, "rx.sim.program.v1"),
                parameter_set: artifact(11, "rx.sim.parameters.v1"),
            });
            step.completion = CompletionRule::Native {
                schema: name("rx.sim.completed.v1"),
                success: vec![Integer(0)],
                failure: vec![Integer(1)],
                postconditions: vec![],
            };
        }
    }
    let configuration = CellConfiguration {
        process: None,
        id: name("cell/a"),
        environment: Environment::Simulation,
        definition: artifact(1, "rx.cell-definition.v1"),
        envelope: artifact(2, "rx.operating-envelope.v1"),
        recipe: artifact(3, "rx.resolved-recipe.v1"),
        site_config_digest: Digest::from_bytes([4; 32]),
        scopes: vec![name("zone/a")],
        hosts: hosts.iter().map(|h| h.principal.clone()).collect(),
        executor: name("executor"),
        maximum_budget: Counter(10),
        permit_ttl_ns: Counter(5000),
        start_timeout_ns: Counter(30000),
        start_conditions: vec![condition.clone()],
        steps,
        maintained_conditions: if maintained { vec![condition] } else { vec![] },
        fact_specs: vec![FactSpec {
            id: name("ready"),
            host: name("host/0"),
            schema: name("boolean/v1"),
            unit: name("unitless"),
            maximum_age_ns: Counter(20000),
            maximum_uncertainty_ns: Counter(0),
        }],
    };
    let configuration = if let Some(kind) = process {
        process_configuration(configuration, kind)
    } else {
        configuration
    };
    let configuration = configure(configuration);
    app.install_cell(&admin, configuration.clone()).unwrap();
    if peer_mode {
        app.negotiate_executor_cell(&executor, configuration.definition.sha256)
            .unwrap();
    }
    if qualify {
        qualify_cell(&mut app, &admin, &configuration);
    }
    let registrations = if register {
        register_hosts(&mut app, &hosts, &configuration)
    } else {
        vec![]
    };
    if register {
        report_ready(&mut app, &hosts[0], &configuration, &registrations[0]);
    }
    Fixture {
        _directory: directory,
        app,
        clock,
        failure,
        admin,
        operator,
        executor,
        hosts,
        configuration,
        registrations,
    }
}
fn add_identity(app: &mut App, admin: &Identity, label: &str, roles: &[Role]) -> Identity {
    app.put_principal(admin, principal(label, roles), None)
        .unwrap();
    let session = app
        .authenticated_session(&name(label), id(), expiry(100000))
        .unwrap();
    Identity {
        principal: name(label),
        session: session.id,
        terminal: None,
    }
}
fn qualify_cell(app: &mut App, admin: &Identity, c: &CellConfiguration) {
    let rev = app.inspect_cell(admin, &c.id).unwrap().0;
    app.qualify(
        admin,
        &c.id,
        rev,
        vec![artifact(9, "rx.validation.simulation.v1")],
        vec![
            c.definition.sha256,
            c.envelope.sha256,
            c.recipe.sha256,
            c.site_config_digest,
        ],
    )
    .unwrap();
}
fn register_hosts(
    app: &mut App,
    hosts: &[Identity],
    c: &CellConfiguration,
) -> Vec<HostRegistration> {
    let (_, cell) = app.inspect_cell(&hosts[0], &c.id).unwrap();
    hosts
        .iter()
        .map(|h| {
            let r = HostRegistration {
                id: h.principal.clone(),
                session: h.session.clone(),
                boot_id: id(),
                delivery_journal: id(),
                cell: c.id.clone(),
                epoch: cell.epoch,
                scopes: cell.scope_epochs.clone(),
                source_sessions: c
                    .fact_specs
                    .iter()
                    .filter(|spec| spec.host == h.principal)
                    .map(|spec| (spec.id.clone(), id()))
                    .collect(),
                grant: Grant {
                    id: id(),
                    fence: Counter(1),
                    resources: c
                        .steps
                        .iter()
                        .filter(|s| s.host == h.principal)
                        .flat_map(|s| s.intent.resource_set.clone())
                        .collect(),
                    owner: name(app.installation.id.as_str()),
                    valid_until: expiry(50000),
                    ttl_ms: Counter(1),
                },
            };
            app.register_host(h, r.clone()).unwrap();
            r
        })
        .collect()
}
fn report_ready(app: &mut App, host: &Identity, c: &CellConfiguration, r: &HostRegistration) {
    app.report_fact(
        host,
        FactRecord {
            cell: c.id.clone(),
            id: name("ready"),
            source_host: host.principal.clone(),
            source_generation: r.source_sessions[&name("ready")].clone(),
            schema: name("boolean/v1"),
            unit: name("unitless"),
            acquired_at: expiry(1000),
            maximum_age_ns: Counter(20000),
            acquisition_uncertainty_ns: Counter(0),
            quality_good: true,
            origin_age_bounded: true,
            disputed: false,
            value: TypedValue::Boolean(true),
            evidence_id: id(),
        },
    )
    .unwrap();
}
fn create_command(f: &Fixture, revision: Counter) -> CreateRun {
    CreateRun {
        cell: f.configuration.id.clone(),
        recipe_digest: f.configuration.recipe.sha256,
        site_config_digest: f.configuration.site_config_digest,
        expected_cell: revision,
    }
}
fn start_command(f: &Fixture, run: &Run, cell: Counter, limit: u64) -> StartRun {
    StartRun {
        run: run.id.clone(),
        envelope_digest: f.configuration.envelope.sha256,
        purpose: Purpose::Production,
        budget_unit: rx_domain::budget::BudgetUnit::PartAttempt,
        budget_limit: Counter(limit),
        expected_cell: cell,
        expected_run: Counter(1),
    }
}
fn work_command(f: &Fixture, a: &Activation, cr: Counter, rr: Counter) -> SubmitWork {
    SubmitWork {
        run: a.run.clone(),
        activation: a.id.clone(),
        part: a.part.clone(),
        slot: name("main"),
        intent: f.configuration.steps[0].intent.clone(),
        expected_cell: cr,
        expected_run: rr,
    }
}
fn start(f: &mut Fixture, limit: u64) -> Run {
    let cell_revision = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .0;
    let run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(f, cell_revision))
        .unwrap();
    let attempt = f
        .app
        .start_run(
            &f.operator,
            id().as_str(),
            start_command(f, &run, cell_revision, limit),
        )
        .unwrap();
    for (host, r) in f.hosts.iter().zip(&f.registrations) {
        f.app
            .acknowledge_arm(
                host,
                ArmAcknowledgment {
                    attempt: attempt.id.clone(),
                    host_boot: r.boot_id.clone(),
                    delivery_journal: r.delivery_journal.clone(),
                    sequence: Counter(1),
                    epoch: r.epoch,
                    scopes: r.scopes.clone(),
                },
            )
            .unwrap();
    }
    f.app.inspect_run(&f.operator, &run.id).unwrap().1
}
fn activation(f: &mut Fixture, run: &Run) -> Activation {
    let part = f
        .app
        .begin_part(
            &f.executor,
            id().as_str(),
            &run.id,
            run.budget.as_ref().unwrap().revision(),
        )
        .unwrap();
    let rev = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
    f.app
        .resolve_activation(&f.executor, &run.id, &name("step/0"), part.ordinal, rev)
        .unwrap()
}
fn submit(f: &mut Fixture, a: &Activation, key: &str) -> rx_ports::Result<Work> {
    let cr = f.app.inspect_cell(&f.executor, &f.configuration.id)?.0;
    let rr = f.app.inspect_run(&f.executor, &a.run)?.0;
    f.app.submit(&f.executor, key, work_command(f, a, cr, rr))
}

#[test]
fn uncommissioned_cell_cannot_start_from_package_presence() {
    let mut f = fixture(1, false);
    let rev = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .0;
    let run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, rev))
        .unwrap();
    assert!(matches!(
        f.app
            .start_run(&f.operator, id().as_str(), start_command(&f, &run, rev, 1)),
        Err(StoreError::Rejected(Rejection::NotCommissioned))
    ));
}

#[test]
fn start_requires_all_hosts_and_cannot_be_completed_by_partial_ack() {
    let mut f = fixture(2, true);
    let rev = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .0;
    let run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, rev))
        .unwrap();
    let attempt = f
        .app
        .start_run(&f.operator, id().as_str(), start_command(&f, &run, rev, 1))
        .unwrap();
    let r = &f.registrations[0];
    let partial = f
        .app
        .acknowledge_arm(
            &f.hosts[0],
            ArmAcknowledgment {
                attempt: attempt.id,
                host_boot: r.boot_id.clone(),
                delivery_journal: r.delivery_journal.clone(),
                sequence: Counter(1),
                epoch: r.epoch,
                scopes: r.scopes.clone(),
            },
        )
        .unwrap();
    assert_eq!(partial.status, StartStatus::Arming);
    let run = f.app.inspect_run(&f.operator, &run.id).unwrap().1;
    assert_eq!(run.state, RunState::Prepared);
    assert!(run.mandate.is_none());
}

#[test]
fn unregistered_terminal_cannot_start_even_with_operator_role() {
    let mut f = fixture(1, true);
    let rev = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .0;
    let run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, rev))
        .unwrap();
    let identity = Identity {
        terminal: None,
        ..f.operator.clone()
    };
    assert!(matches!(
        f.app
            .start_run(&identity, id().as_str(), start_command(&f, &run, rev, 1)),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
}

#[test]
fn same_slot_new_key_and_lost_reply_do_not_create_another_operation() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let a = activation(&mut f, &run);
    let key_ = id().to_string();
    let cr = f
        .app
        .inspect_cell(&f.executor, &f.configuration.id)
        .unwrap()
        .0;
    let rr = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .submit(&f.executor, &key_, work_command(&f, &a, cr, rr))
            .is_err()
    );
    let work = submit(&mut f, &a, &key_).unwrap();
    let another = submit(&mut f, &a, id().as_str()).unwrap();
    assert_eq!(work.operation.id(), another.operation.id());
    assert_eq!(
        f.app
            .inspect_run(&f.executor, &run.id)
            .unwrap()
            .1
            .budget
            .as_ref()
            .unwrap()
            .consumed(),
        Counter(1)
    );
}

#[test]
fn changed_intent_under_same_key_is_a_conflict() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let a = activation(&mut f, &run);
    let key_ = id().to_string();
    submit(&mut f, &a, &key_).unwrap();
    let mut intent = f.configuration.steps[0].intent.clone();
    intent.execution_timeout_ms = Counter(6000);
    let cr = f
        .app
        .inspect_cell(&f.executor, &f.configuration.id)
        .unwrap()
        .0;
    let rr = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
    assert!(matches!(
        f.app.submit(
            &f.executor,
            &key_,
            SubmitWork {
                intent,
                ..work_command(&f, &a, cr, rr)
            }
        ),
        Err(StoreError::KeyConflict)
    ));
}

#[test]
fn t1_failure_rolls_back_slot_resource_permit_and_outbox_together() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let a = activation(&mut f, &run);
    let key_ = id().to_string();
    let cr = f
        .app
        .inspect_cell(&f.executor, &f.configuration.id)
        .unwrap()
        .0;
    let rr = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
    f.failure.store(1, Ordering::SeqCst);
    assert!(
        f.app
            .submit(&f.executor, &key_, work_command(&f, &a, cr, rr))
            .is_err()
    );
    assert!(submit(&mut f, &a, &key_).is_ok());
    let mut repo = f.app.into_repository();
    let rows = repo.snapshot().unwrap().1;
    for schema in [
        "rx.internal.work.v1",
        "rx.internal.permit.v1",
        "rx.internal.resource.v1",
    ] {
        assert_eq!(
            rows.iter()
                .filter(|r| r.document.schema.as_str() == schema)
                .count(),
            1
        );
    }
}

#[test]
fn permit_deadline_cannot_outlive_the_observation_that_supported_it() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let a = activation(&mut f, &run);
    f.clock.0.store(20999, Ordering::SeqCst);
    let work = submit(&mut f, &a, id().as_str()).unwrap();
    let mut repo = f.app.into_repository();
    let row = repo
        .snapshot()
        .unwrap()
        .1
        .into_iter()
        .find(|r| r.document.schema.as_str() == "rx.internal.permit.v1")
        .unwrap();
    let permit: Permit = serde_json::from_value(row.document.value).unwrap();
    assert_eq!(permit.id, work.permit);
    assert_eq!(permit.expires_at.ticks_ns, Counter(21000));
}

#[test]
fn stale_observation_and_wrong_executor_do_not_reach_t1() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let a = activation(&mut f, &run);
    let other = add_identity(
        &mut f.app,
        &f.admin,
        "other-executor",
        &[Role::Executor, Role::Observer],
    );
    let cr = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .0;
    let rr = f.app.inspect_run(&f.operator, &run.id).unwrap().0;
    assert!(matches!(
        f.app
            .submit(&other, id().as_str(), work_command(&f, &a, cr, rr)),
        Err(StoreError::Rejected(Rejection::MandateRevoked))
    ));
    f.clock.0.store(21001, Ordering::SeqCst);
    assert!(matches!(
        submit(&mut f, &a, id().as_str()),
        Err(StoreError::Rejected(Rejection::ConditionUnknown))
    ));
    let mut repo = f.app.into_repository();
    assert!(
        !repo
            .snapshot()
            .unwrap()
            .1
            .iter()
            .any(|r| r.document.schema.as_str() == "rx.internal.work.v1")
    );
}

#[test]
fn role_revocation_is_checked_before_cached_result_lookup() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let a = activation(&mut f, &run);
    let key_ = id().to_string();
    submit(&mut f, &a, &key_).unwrap();
    let mut executor = principal("executor", &[Role::Observer]);
    executor.roles = BTreeSet::from([Role::Observer]);
    f.app
        .put_principal(&f.admin, executor, Some(Counter(1)))
        .unwrap();
    let cr = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .0;
    let rr = f.app.inspect_run(&f.operator, &run.id).unwrap().0;
    assert!(matches!(
        f.app
            .submit(&f.executor, &key_, work_command(&f, &a, cr, rr)),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
}

#[test]
fn restarting_runtime_preserves_consumption_and_revokes_mandate() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 2);
    let _a = activation(&mut f, &run);
    let installation = f.app.installation.id.clone();
    let store_generation = f.app.installation.store_generation.clone();
    let repository = f.app.into_repository();
    let mut app = Engine::open(
        repository,
        f.clock,
        SimulationAuthority,
        installation,
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    assert_eq!(app.installation.store_generation, store_generation);
    assert!(matches!(
        app.inspect_run(&f.operator, &run.id),
        Err(StoreError::Rejected(Rejection::Unauthenticated))
    ));
    let new = app
        .authenticated_terminal_user_session(
            &name("operator"),
            id(),
            Counter(99_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap();
    let operator = Identity {
        session: new.id,
        ..f.operator
    };
    let restored = app.inspect_run(&operator, &run.id).unwrap().1;
    assert_eq!(restored.budget.as_ref().unwrap().consumed(), Counter(1));
    assert_eq!(restored.state, RunState::RecoveryRequired);
    let (_, cell) = app.inspect_cell(&operator, &name("cell/a")).unwrap();
    assert_eq!(cell.epoch, Counter(2));
    assert!(!cell.blocks.is_empty());
}

fn add_cell_b(f: &mut Fixture, shared: bool) {
    let mut configuration = f.configuration.clone();
    configuration.id = name("cell/b");
    configuration.scopes = vec![name("zone/b")];
    if !shared {
        configuration.steps[0].intent.resource_set = vec![name("controller/b")];
    }
    f.app.install_cell(&f.admin, configuration.clone()).unwrap();
    qualify_cell(&mut f.app, &f.admin, &configuration);
    let (_, cell) = f.app.inspect_cell(&f.operator, &configuration.id).unwrap();
    let mut registrations = f.registrations.clone();
    for (host, r) in f.hosts.iter().zip(registrations.iter_mut()) {
        r.cell = configuration.id.clone();
        r.epoch = cell.epoch;
        r.scopes = cell.scope_epochs.clone();
        r.grant.resources = configuration
            .steps
            .iter()
            .filter(|s| s.host == host.principal)
            .flat_map(|s| s.intent.resource_set.clone())
            .collect();
        r.grant.id = id();
        f.app.register_host(host, r.clone()).unwrap();
    }
    report_ready(&mut f.app, &f.hosts[0], &configuration, &registrations[0]);
    f.configuration = configuration;
    f.registrations = registrations;
}

#[test]
fn one_physical_resource_is_reserved_across_cells_not_per_cell_alias() {
    let mut f = fixture(1, true);
    let run_a = start(&mut f, 1);
    let a = activation(&mut f, &run_a);
    submit(&mut f, &a, id().as_str()).unwrap();
    add_cell_b(&mut f, true);
    let run_b = start(&mut f, 1);
    let b = activation(&mut f, &run_b);
    assert!(matches!(
        submit(&mut f, &b, id().as_str()),
        Err(StoreError::Rejected(Rejection::Busy))
    ));
}

#[test]
fn hold_propagates_to_shared_resources_and_preserves_independent_cells() {
    for shared in [false, true] {
        let mut f = fixture(1, true);
        let run_a = start(&mut f, 1);
        add_cell_b(&mut f, shared);
        let run_b = start(&mut f, 1);
        f.app
            .hold(&f.operator, id().as_str(), &name("cell/a"))
            .unwrap();
        assert_eq!(
            f.app.inspect_run(&f.operator, &run_a.id).unwrap().1.state,
            RunState::RecoveryRequired
        );
        assert_eq!(
            f.app.inspect_run(&f.operator, &run_b.id).unwrap().1.state,
            if shared {
                RunState::RecoveryRequired
            } else {
                RunState::Executing
            }
        );
    }
}

#[test]
fn hold_proves_unsent_work_not_executed_without_releasing_resource() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let a = activation(&mut f, &run);
    let work = submit(&mut f, &a, id().as_str()).unwrap();
    let key = id().to_string();
    let cell = f.app.hold(&f.operator, &key, &name("cell/a")).unwrap();
    let again = f.app.hold(&f.operator, &key, &name("cell/a")).unwrap();
    assert_eq!(cell.epoch, again.epoch);
    let work = f
        .app
        .inspect_work(&f.operator, work.operation.id())
        .unwrap();
    assert_eq!(
        work.operation.outcome(),
        rx_domain::operation::Outcome::NotExecuted
    );
    assert_eq!(
        work.operation.disposition(),
        rx_domain::operation::Disposition::Quarantined
    );
}

#[test]
fn pending_start_has_a_deadline_and_key_covers_budget_intent() {
    let mut f = fixture(1, true);
    let cr = f.app.inspect_cell(&f.operator, &name("cell/a")).unwrap().0;
    let run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, cr))
        .unwrap();
    assert!(run.budget.is_none());
    let key = id().to_string();
    let request = start_command(&f, &run, cr, 1);
    let attempt = f.app.start_run(&f.operator, &key, request.clone()).unwrap();
    let changed = StartRun {
        budget_limit: Counter(2),
        ..request.clone()
    };
    assert!(matches!(
        f.app.start_run(&f.operator, &key, changed),
        Err(StoreError::KeyConflict)
    ));
    f.clock.0.store(31001, Ordering::SeqCst);
    let r = &f.registrations[0];
    let result = f
        .app
        .acknowledge_arm(
            &f.hosts[0],
            ArmAcknowledgment {
                attempt: attempt.id,
                host_boot: r.boot_id.clone(),
                delivery_journal: r.delivery_journal.clone(),
                sequence: Counter(1),
                epoch: r.epoch,
                scopes: r.scopes.clone(),
            },
        )
        .unwrap();
    assert_eq!(result.status, StartStatus::Rejected);
    assert_eq!(
        f.app.start_run(&f.operator, &key, request).unwrap().status,
        StartStatus::Rejected
    );
    assert!(
        f.app
            .inspect_run(&f.operator, &run.id)
            .unwrap()
            .1
            .mandate
            .is_none()
    );
}

#[test]
fn activation_visits_are_unique_for_the_whole_run_across_parts() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 2);
    let first = activation(&mut f, &run);
    let current = f.app.inspect_run(&f.executor, &run.id).unwrap().1;
    let second = activation(&mut f, &current);
    assert_eq!(first.visit, Counter(1));
    assert_eq!(second.visit, Counter(2));
    assert_ne!(first.id, second.id);
    let rev = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
    let recovered = f
        .app
        .resolve_activation(&f.executor, &run.id, &name("step/0"), Counter(1), rev)
        .unwrap();
    assert_eq!(recovered.id, first.id);
}

#[test]
fn conflicting_observation_is_committed_as_invalidation_even_though_api_rejects_it() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let mut fact = f
        .app
        .inspect_fact(&f.operator, &name("cell/a"), &name("ready"))
        .unwrap();
    fact.value = TypedValue::Boolean(false);
    assert!(matches!(
        f.app.report_fact(&f.hosts[0], fact),
        Err(StoreError::KeyConflict)
    ));
    let (_, cell) = f.app.inspect_cell(&f.operator, &name("cell/a")).unwrap();
    assert!(
        cell.blocks
            .iter()
            .any(|b| b.reason == BlockReason::IntegrityConflict)
    );
    assert_eq!(
        f.app.inspect_run(&f.operator, &run.id).unwrap().1.state,
        RunState::RecoveryRequired
    );
}

#[test]
fn device_generation_change_is_preserved_and_does_not_disappear_on_rejection() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let mut fact = f
        .app
        .inspect_fact(&f.operator, &name("cell/a"), &name("ready"))
        .unwrap();
    fact.evidence_id = id();
    fact.source_generation = id();
    assert!(matches!(
        f.app.report_fact(&f.hosts[0], fact),
        Err(StoreError::Rejected(Rejection::ContinuityUnproven))
    ));
    let (_, cell) = f.app.inspect_cell(&f.operator, &name("cell/a")).unwrap();
    assert!(
        cell.blocks
            .iter()
            .any(|b| b.reason == BlockReason::DeviceRestart)
    );
    assert_eq!(
        f.app.inspect_run(&f.operator, &run.id).unwrap().1.state,
        RunState::RecoveryRequired
    );
}

#[test]
fn maintained_condition_recovery_does_not_revive_an_old_mandate() {
    let mut f = fixture_mode(1, true, true);
    let run = start(&mut f, 1);
    let mut fact = f
        .app
        .inspect_fact(&f.operator, &name("cell/a"), &name("ready"))
        .unwrap();
    fact.evidence_id = id();
    fact.value = TypedValue::Boolean(false);
    f.app.report_fact(&f.hosts[0], fact.clone()).unwrap();
    fact.evidence_id = id();
    fact.value = TypedValue::Boolean(true);
    f.app.report_fact(&f.hosts[0], fact).unwrap();
    assert_eq!(
        f.app.inspect_run(&f.operator, &run.id).unwrap().1.state,
        RunState::RecoveryRequired
    );
    let (_, cell) = f.app.inspect_cell(&f.operator, &name("cell/a")).unwrap();
    assert!(
        cell.blocks
            .iter()
            .any(|b| b.reason == BlockReason::ConditionLost)
    );
}

fn native_started(f: &mut Fixture) -> (Work, NativeEvidence) {
    let run = start(f, 1);
    let a = activation(f, &run);
    let work = submit(f, &a, id().as_str()).unwrap();
    let op = work.operation.id().clone();
    assert!(f.app.begin_delivery(&op).unwrap());
    let invocation = id();
    f.app
        .record_host_receipt(
            &f.hosts[0],
            &op,
            HostReceipt {
                operation: op.clone(),
                digest: work.intent.digest().unwrap(),
                invocation: Some(invocation.clone()),
                journal: f.registrations[0].delivery_journal.clone(),
                sequence: Counter(10),
                state: ReceiptState::Prepared,
            },
        )
        .unwrap();
    let next = f
        .app
        .pending_deliveries(128)
        .unwrap()
        .into_iter()
        .find(|d| matches!(d.payload, Delivery::Authorize { .. }))
        .unwrap();
    assert!(f.app.begin_delivery(&next.id).unwrap());
    f.app
        .record_host_receipt(
            &f.hosts[0],
            &next.id,
            HostReceipt {
                operation: op.clone(),
                digest: work.intent.digest().unwrap(),
                invocation: Some(invocation.clone()),
                journal: f.registrations[0].delivery_journal.clone(),
                sequence: Counter(11),
                state: ReceiptState::ResultCaptured,
            },
        )
        .unwrap();
    let evidence = NativeEvidence {
        native_details: None,
        id: id(),
        operation: op,
        invocation,
        profile_digest: work.intent.profile_digest,
        device_session: id(),
        status_schema: name("rx.sim.completed.v1"),
        status: Integer(0),
        captured_at: expiry(1000),
    };
    (work, evidence)
}

#[test]
fn t2_commits_native_evidence_outcome_and_contiguous_cursor_atomically() {
    let mut f = fixture_complete(1, true, false, true);
    let (work, evidence) = native_started(&mut f);
    assert_eq!(
        f.app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap()
            .operation
            .outcome(),
        rx_domain::operation::Outcome::None
    );
    let batch = EvidenceBatch {
        journal: id(),
        first: Counter(1),
        records: vec![evidence],
    };
    let ack = f.app.ingest_evidence(&f.hosts[0], batch.clone()).unwrap();
    assert_eq!(ack.through, Counter(1));
    let done = f
        .app
        .inspect_work(&f.operator, work.operation.id())
        .unwrap();
    assert_eq!(
        done.operation.outcome(),
        rx_domain::operation::Outcome::Succeeded
    );
    assert_eq!(
        done.operation.disposition(),
        rx_domain::operation::Disposition::Held
    );
    let revision = done.operation.revision();
    f.app.ingest_evidence(&f.hosts[0], batch).unwrap();
    assert_eq!(
        f.app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap()
            .operation
            .revision(),
        revision
    );
}

#[test]
fn t2_rollback_and_lost_ack_preserve_exactly_one_recorded_result() {
    for failure in [1, 2] {
        let mut f = fixture_complete(1, true, false, true);
        let (work, evidence) = native_started(&mut f);
        let batch = EvidenceBatch {
            journal: id(),
            first: Counter(1),
            records: vec![evidence],
        };
        f.failure.store(failure, Ordering::SeqCst);
        assert!(f.app.ingest_evidence(&f.hosts[0], batch.clone()).is_err());
        assert_eq!(
            f.app.ingest_evidence(&f.hosts[0], batch).unwrap().through,
            Counter(1)
        );
        assert_eq!(
            f.app
                .inspect_work(&f.operator, work.operation.id())
                .unwrap()
                .operation
                .outcome(),
            rx_domain::operation::Outcome::Succeeded
        );
        let mut repo = f.app.into_repository();
        assert_eq!(
            repo.snapshot()
                .unwrap()
                .1
                .iter()
                .filter(|r| r.key.as_str().starts_with("evidence/"))
                .count(),
            1
        );
    }
}

#[test]
fn evidence_gap_is_not_skipped_and_conflicting_sequence_is_quarantined() {
    let mut f = fixture_complete(1, true, false, true);
    let (work, evidence) = native_started(&mut f);
    let journal = id();
    assert!(
        f.app
            .ingest_evidence(
                &f.hosts[0],
                EvidenceBatch {
                    journal: journal.clone(),
                    first: Counter(2),
                    records: vec![evidence.clone()]
                }
            )
            .is_err()
    );
    let batch = EvidenceBatch {
        journal,
        first: Counter(1),
        records: vec![evidence],
    };
    f.app.ingest_evidence(&f.hosts[0], batch.clone()).unwrap();
    let mut changed = batch;
    changed.records[0].status = Integer(1);
    assert!(f.app.ingest_evidence(&f.hosts[0], changed).is_err());
    let result = f
        .app
        .inspect_work(&f.operator, work.operation.id())
        .unwrap();
    assert_eq!(
        result.operation.outcome(),
        rx_domain::operation::Outcome::Succeeded
    );
    assert_eq!(
        result.operation.integrity(),
        rx_domain::operation::Integrity::Disputed
    );
    assert_eq!(
        result.operation.disposition(),
        rx_domain::operation::Disposition::Quarantined
    );
}

#[test]
fn a_new_contradictory_native_result_preserves_historical_outcome() {
    let mut f = fixture_complete(1, true, false, true);
    let (work, evidence) = native_started(&mut f);
    let journal = id();
    f.app
        .ingest_evidence(
            &f.hosts[0],
            EvidenceBatch {
                journal: journal.clone(),
                first: Counter(1),
                records: vec![evidence.clone()],
            },
        )
        .unwrap();
    let mut late = evidence;
    late.id = id();
    late.status = Integer(1);
    f.app
        .ingest_evidence(
            &f.hosts[0],
            EvidenceBatch {
                journal,
                first: Counter(2),
                records: vec![late],
            },
        )
        .unwrap();
    let result = f
        .app
        .inspect_work(&f.operator, work.operation.id())
        .unwrap();
    assert_eq!(
        result.operation.outcome(),
        rx_domain::operation::Outcome::Succeeded
    );
    assert_eq!(
        result.operation.integrity(),
        rx_domain::operation::Integrity::Disputed
    );
}

#[test]
fn native_receipt_capture_does_not_stand_in_for_completion_policy() {
    let mut f = fixture(1, true);
    let (work, evidence) = native_started(&mut f);
    f.app
        .ingest_evidence(
            &f.hosts[0],
            EvidenceBatch {
                journal: id(),
                first: Counter(1),
                records: vec![evidence],
            },
        )
        .unwrap();
    let result = f
        .app
        .inspect_work(&f.operator, work.operation.id())
        .unwrap();
    assert_eq!(
        result.operation.outcome(),
        rx_domain::operation::Outcome::None
    );
    assert_eq!(
        result.operation.phase(),
        rx_domain::operation::Phase::Reconciling
    );
}

#[test]
fn changing_evidence_journal_does_not_silently_reset_sequence_history() {
    let mut f = fixture_complete(1, true, false, true);
    let (work, evidence) = native_started(&mut f);
    f.app
        .ingest_evidence(
            &f.hosts[0],
            EvidenceBatch {
                journal: id(),
                first: Counter(1),
                records: vec![evidence.clone()],
            },
        )
        .unwrap();
    let replacement = EvidenceBatch {
        journal: id(),
        first: Counter(1),
        records: vec![evidence],
    };
    assert!(
        f.app
            .ingest_evidence(&f.hosts[0], replacement.clone())
            .is_err()
    );
    let (_, cell) = f.app.inspect_cell(&f.operator, &name("cell/a")).unwrap();
    assert!(!cell.blocks.is_empty());
    let epoch = cell.epoch;
    assert!(f.app.ingest_evidence(&f.hosts[0], replacement).is_err());
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &name("cell/a"))
            .unwrap()
            .1
            .epoch,
        epoch
    );
    assert_eq!(
        f.app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap()
            .operation
            .outcome(),
        rx_domain::operation::Outcome::Succeeded
    );
}

fn completed_work(f: &mut Fixture) -> (Work, NativeEvidence) {
    let (work, evidence) = native_started(f);
    f.app
        .ingest_evidence(
            &f.hosts[0],
            EvidenceBatch {
                journal: id(),
                first: Counter(1),
                records: vec![evidence.clone()],
            },
        )
        .unwrap();
    (
        f.app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap(),
        evidence,
    )
}
fn handover_proof(f: &mut Fixture, work: &Work, evidence: &NativeEvidence) -> ReleaseResources {
    ReleaseResources {
        operation: work.operation.id().clone(),
        expected_operation: work.operation.revision(),
        expected_cell: f.app.inspect_cell(&f.operator, &work.cell).unwrap().0,
        observations: ["no-pending", "control", "support"]
            .into_iter()
            .map(|predicate| HandoverObservation {
                id: id(),
                operation: work.operation.id().clone(),
                invocation: evidence.invocation.clone(),
                profile_digest: work.intent.profile_digest,
                device_session: evidence.device_session.clone(),
                host_boot: f.registrations[0].boot_id.clone(),
                source: name(&format!("handover/{}/{predicate}", work.operation.id())),
                schema: name("rx.handover.v1"),
                value: true,
                observed_at: expiry(1000),
                uncertainty_ns: Counter(0),
                quality_good: true,
                origin_age_bounded: true,
            })
            .collect(),
    }
}
#[test]
fn resource_release_requires_all_current_handover_facts() {
    for kind in 0..5 {
        let mut f = fixture_complete(1, true, false, true);
        let (work, evidence) = completed_work(&mut f);
        let mut proof = handover_proof(&mut f, &work, &evidence);
        match kind {
            0 => proof.observations[2].value = false,
            1 => proof.observations[0].host_boot = id(),
            2 => proof.observations[1].device_session = id(),
            3 => {
                proof.observations.pop();
            }
            _ => {
                f.clock.0.store(2000, Ordering::SeqCst);
            }
        }
        assert!(
            f.app
                .release_resources(&f.hosts[0], id().as_str(), proof)
                .is_err()
        );
        assert_eq!(
            f.app
                .inspect_work(&f.operator, work.operation.id())
                .unwrap()
                .operation
                .disposition(),
            rx_domain::operation::Disposition::Held
        );
    }
}
#[test]
fn handover_release_is_atomic_and_duplicate_proof_does_not_release_another_owner() {
    let mut f = fixture_complete(1, true, false, true);
    let (work, evidence) = completed_work(&mut f);
    let proof = handover_proof(&mut f, &work, &evidence);
    let key = id().to_string();
    f.failure.store(1, Ordering::SeqCst);
    assert!(
        f.app
            .release_resources(&f.hosts[0], &key, proof.clone())
            .is_err()
    );
    assert_eq!(
        f.app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap()
            .operation
            .disposition(),
        rx_domain::operation::Disposition::Held
    );
    let released = f
        .app
        .release_resources(&f.hosts[0], &key, proof.clone())
        .unwrap();
    assert_eq!(
        released.operation.disposition(),
        rx_domain::operation::Disposition::Released
    );
    let again = f.app.release_resources(&f.hosts[0], &key, proof).unwrap();
    assert_eq!(released.operation.revision(), again.operation.revision());
}
#[test]
fn part_completion_waits_for_release_and_closes_exhausted_run() {
    let mut f = fixture_complete(1, true, false, true);
    let (work, evidence) = completed_work(&mut f);
    let revision = f.app.inspect_run(&f.executor, &work.run).unwrap().0;
    assert!(
        f.app
            .complete_part(
                &f.executor,
                id().as_str(),
                work.part.as_ref().unwrap(),
                revision
            )
            .is_err()
    );
    let proof = handover_proof(&mut f, &work, &evidence);
    f.app
        .release_resources(&f.hosts[0], id().as_str(), proof)
        .unwrap();
    let part = f
        .app
        .complete_part(
            &f.executor,
            id().as_str(),
            work.part.as_ref().unwrap(),
            revision,
        )
        .unwrap();
    assert_eq!(part.disposition, PartDisposition::ConfirmedCompleted);
    assert_eq!(
        f.app.inspect_run(&f.operator, &work.run).unwrap().1.state,
        RunState::Completed
    );
}

#[test]
fn native_metadata_changes_conflict_and_ack_cursor_is_part_of_t2_commit() {
    let mut f = fixture_complete(1, true, false, true);
    let (work, mut evidence) = native_started(&mut f);
    evidence.native_details = Some(NativeEvidenceDetails {
        native_id: Some("vendor-result-1".into()),
        native_data: Some(artifact(66, "vendor/report.v1")),
    });
    let mut batch = EvidenceBatch {
        journal: id(),
        first: Counter(1),
        records: vec![evidence.clone()],
    };
    let commit = f
        .app
        .ingest_evidence_commit(&f.hosts[0], batch.clone())
        .unwrap();
    assert_eq!(commit.installation, f.app.installation.id);
    assert_eq!(commit.store_generation, f.app.installation.store_generation);
    assert!(commit.platform_sequence.0 > 0);
    let recorded = f
        .app
        .inspect_native_evidence(&f.hosts[0], &evidence.id)
        .unwrap();
    assert_eq!(
        recorded
            .native_details
            .as_ref()
            .unwrap()
            .native_id
            .as_deref(),
        Some("vendor-result-1")
    );
    batch.records[0]
        .native_details
        .as_mut()
        .unwrap()
        .native_data = Some(artifact(67, "vendor/report.v1"));
    assert!(f.app.ingest_evidence(&f.hosts[0], batch).is_err());
    let work = f
        .app
        .inspect_work(&f.operator, work.operation.id())
        .unwrap();
    assert_eq!(
        work.operation.integrity(),
        rx_domain::operation::Integrity::Disputed
    );
    assert_eq!(
        work.operation.outcome(),
        rx_domain::operation::Outcome::Succeeded
    );
}

#[test]
fn producer_reconnect_requires_current_identity_and_cell_negotiation() {
    let mut f = fixture(1, true);
    let peer = name("host/0");
    let boot = id();
    let journal = id();
    let binding = Digest::from_bytes([31; 32]);
    let first = f
        .app
        .open_evidence_producer(&peer, boot.clone(), journal.clone(), binding)
        .unwrap();
    let identity = Identity {
        principal: peer.clone(),
        session: first.id.clone(),
        terminal: None,
    };
    let epoch = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1
        .epoch;
    let repeated = f
        .app
        .open_evidence_producer(&peer, boot.clone(), journal.clone(), binding)
        .unwrap();
    assert_eq!(first.id, repeated.id);
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch,
        epoch
    );
    let probe = EvidenceBatch {
        journal: journal.clone(),
        first: Counter(1),
        records: vec![],
    };
    assert!(matches!(
        f.app.publish_evidence(&identity, probe.clone()),
        Err(StoreError::Rejected(Rejection::CapabilityMissing))
    ));
    f.app
        .negotiate_evidence_cell(&identity, f.configuration.definition.sha256)
        .unwrap();
    let committed = f.app.publish_evidence(&identity, probe.clone()).unwrap();
    assert_eq!(committed.producer.through, Counter(0));
    assert!(committed.platform_sequence.0 > 0);
    let renewed = f
        .app
        .open_evidence_producer(&peer, id(), journal, binding)
        .unwrap();
    assert_ne!(renewed.id, first.id);
    assert!(matches!(
        f.app.publish_evidence(&identity, probe),
        Err(StoreError::Rejected(Rejection::Unauthenticated))
    ));
    assert!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch
            > epoch
    );
}

#[test]
fn control_journal_is_atomic_and_audit_sessions_do_not_advance_its_cursor() {
    use rx_application::control_journal::{CHANGE_SCHEMA, ControlChange, EntityKind};
    let mut f = fixture_complete(1, true, false, true);
    let (work, evidence) = native_started(&mut f);
    let batch = EvidenceBatch {
        journal: id(),
        first: Counter(1),
        records: vec![evidence.clone()],
    };
    f.failure.store(1, Ordering::SeqCst);
    assert!(
        f.app
            .ingest_evidence_commit(&f.hosts[0], batch.clone())
            .is_err()
    );
    let commit = f
        .app
        .ingest_evidence_commit(&f.hosts[0], batch.clone())
        .unwrap();
    let same = f.app.ingest_evidence_commit(&f.hosts[0], batch).unwrap();
    assert_eq!(
        same.platform_sequence, commit.platform_sequence,
        "a duplicate inbox batch cannot invent another control change"
    );
    f.app
        .authenticated_user_session(&name("operator"), id(), Counter(1000))
        .unwrap();
    let mut repository = f.app.into_repository();
    let (through, entities) = repository.control_snapshot().unwrap();
    assert_eq!(through, commit.platform_sequence);
    assert!(repository.journal_head().unwrap() > through);
    let events = repository.control_events_after(Counter(0), 128).unwrap();
    assert_eq!(events.len() as u64, through.0);
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event.seq, Counter(index as u64 + 1));
    }
    let mut observed = 0;
    for event in events {
        let change: ControlChange = rx_application::persistence::decode(
            &Record {
                key: name("event"),
                revision: Counter(1),
                document: event.document,
            },
            CHANGE_SCHEMA,
        )
        .unwrap();
        if change.entity_kind == EntityKind::Work && change.evidence_ids.contains(&evidence.id) {
            observed += 1;
            let result: Work =
                rx_application::persistence::decode(&change.entity, "rx.internal.work.v1").unwrap();
            assert_eq!(result.operation.id(), work.operation.id());
            assert_eq!(
                result.operation.outcome(),
                rx_domain::operation::Outcome::Succeeded
            );
        }
        assert!(!change.entity.document.schema.as_str().contains("session"));
    }
    assert_eq!(observed, 1);
    assert!(
        entities
            .iter()
            .any(|entity| entity.document.schema.as_str() == "rx.internal.work.v1")
    );
}

#[test]
fn direct_state_puts_during_hold_are_captured_at_one_final_transaction_cut() {
    use rx_application::control_journal::{CHANGE_SCHEMA, ControlChange, EntityKind};
    let mut f = fixture_complete(1, true, false, true);
    let (work, _) = native_started(&mut f);
    f.app
        .hold(&f.operator, &id().to_string(), &f.configuration.id)
        .unwrap();
    let mut repository = f.app.into_repository();
    let (head, entities) = repository.control_snapshot().unwrap();
    let events = repository.control_events_after(Counter(0), 128).unwrap();
    let cell = entities
        .iter()
        .find(|row| row.document.schema.as_str() == "rx.internal.cell.v1")
        .unwrap();
    let state: Cell = rx_application::persistence::decode(cell, "rx.internal.cell.v1").unwrap();
    assert!(!state.blocks.is_empty());
    let latest = events
        .iter()
        .rev()
        .find_map(|event| {
            let change: ControlChange = rx_application::persistence::decode(
                &Record {
                    key: name("event"),
                    revision: Counter(1),
                    document: event.document.clone(),
                },
                CHANGE_SCHEMA,
            )
            .unwrap();
            if change.entity_kind == EntityKind::Work {
                Some(change)
            } else {
                None
            }
        })
        .unwrap();
    let stored: Work =
        rx_application::persistence::decode(&latest.entity, "rx.internal.work.v1").unwrap();
    assert_eq!(stored.operation.id(), work.operation.id());
    assert_eq!(
        stored.operation.disposition(),
        rx_domain::operation::Disposition::Quarantined
    );
    assert_eq!(events.last().unwrap().seq, head);
}

#[test]
fn prepared_is_not_an_authorization_ack_and_duplicate_receipts_complete_the_right_message() {
    let mut f = fixture_complete(1, true, false, true);
    let run = start(&mut f, 1);
    let activation = activation(&mut f, &run);
    let work = submit(&mut f, &activation, id().as_str()).unwrap();
    let op = work.operation.id().clone();
    let invocation = id();
    let receipt = |seq, state| HostReceipt {
        operation: op.clone(),
        digest: work.intent.digest().unwrap(),
        invocation: Some(invocation.clone()),
        journal: f.registrations[0].delivery_journal.clone(),
        sequence: Counter(seq),
        state,
    };
    f.app.begin_delivery(&op).unwrap();
    f.app
        .record_host_receipt(&f.hosts[0], &op, receipt(10, ReceiptState::Prepared))
        .unwrap();
    let authorization = rx_application::engine::authorization_delivery_id(&op);
    f.app.begin_delivery(&authorization).unwrap();
    f.app
        .record_host_receipt(
            &f.hosts[0],
            &authorization,
            receipt(11, ReceiptState::Prepared),
        )
        .unwrap();
    assert!(
        f.app
            .pending_deliveries(128)
            .unwrap()
            .iter()
            .any(|d| d.id == authorization)
    );
    let sent = receipt(12, ReceiptState::SendEntered);
    f.app
        .record_host_receipt(&f.hosts[0], &op, sent.clone())
        .unwrap();
    f.app
        .record_host_receipt(&f.hosts[0], &authorization, sent.clone())
        .unwrap();
    assert!(
        !f.app
            .pending_deliveries(128)
            .unwrap()
            .iter()
            .any(|d| d.id == authorization)
    );
    assert!(f.app.record_host_receipt(&f.hosts[0], &id(), sent).is_err());
    assert_eq!(
        f.app
            .inspect_work(&f.operator, &op)
            .unwrap()
            .operation
            .outcome(),
        rx_domain::operation::Outcome::None
    );
}

#[test]
fn an_entered_send_with_no_remote_receipt_remains_unknown_and_is_not_reissued() {
    let mut f = fixture_complete(1, true, false, true);
    let run = start(&mut f, 1);
    let activation = activation(&mut f, &run);
    let work = submit(&mut f, &activation, id().as_str()).unwrap();
    let first = f
        .app
        .plan_delivery(&f.hosts[0], work.operation.id())
        .unwrap();
    assert!(first.first_emission);
    f.app
        .note_delivery_attention(work.operation.id(), DeliveryIssue::RemoteNotFound)
        .unwrap();
    let repeated = f
        .app
        .plan_delivery(&f.hosts[0], work.operation.id())
        .unwrap();
    assert!(!repeated.first_emission);
    let current = f
        .app
        .inspect_work(&f.operator, work.operation.id())
        .unwrap();
    assert_eq!(
        current.operation.knowledge(),
        rx_domain::operation::Knowledge::Unknown
    );
    assert_eq!(
        current.operation.outcome(),
        rx_domain::operation::Outcome::None
    );
    assert_eq!(
        current.operation.disposition(),
        rx_domain::operation::Disposition::Quarantined
    );
}

#[test]
fn early_native_evidence_is_reapplied_when_the_receipt_establishes_correlation() {
    let mut f = fixture_complete(1, true, false, true);
    let run = start(&mut f, 1);
    let activation = activation(&mut f, &run);
    let work = submit(&mut f, &activation, id().as_str()).unwrap();
    f.app.begin_delivery(work.operation.id()).unwrap();
    let invocation = id();
    let evidence = NativeEvidence {
        native_details: None,
        id: id(),
        operation: work.operation.id().clone(),
        invocation: invocation.clone(),
        profile_digest: work.intent.profile_digest,
        device_session: id(),
        status_schema: name("rx.sim.completed.v1"),
        status: Integer(0),
        captured_at: expiry(1000),
    };
    f.app
        .ingest_evidence(
            &f.hosts[0],
            EvidenceBatch {
                journal: id(),
                first: Counter(1),
                records: vec![evidence.clone()],
            },
        )
        .unwrap();
    assert_eq!(
        f.app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap()
            .operation
            .outcome(),
        rx_domain::operation::Outcome::None
    );
    let result = f
        .app
        .record_host_receipt(
            &f.hosts[0],
            work.operation.id(),
            HostReceipt {
                operation: work.operation.id().clone(),
                digest: work.intent.digest().unwrap(),
                invocation: Some(invocation),
                journal: f.registrations[0].delivery_journal.clone(),
                sequence: Counter(10),
                state: ReceiptState::Prepared,
            },
        )
        .unwrap();
    assert_eq!(
        result.operation.outcome(),
        rx_domain::operation::Outcome::Succeeded
    );
    assert!(result.operation.evidence_ids().contains(&evidence.id));
    assert_eq!(
        f.app
            .inspect_permit(&f.operator, &work.permit)
            .unwrap()
            .state,
        PermitState::Consumed
    );
    assert!(
        !f.app
            .pending_deliveries(128)
            .unwrap()
            .iter()
            .any(|row| matches!(row.payload, Delivery::Authorize { .. }))
    );
}

fn process_configuration(
    mut configuration: CellConfiguration,
    kind: TestProcess,
) -> CellConfiguration {
    use rx_process_contract::*;
    let process = name("test/process");
    let make = |label: &str, body: CompiledBody| {
        let source = SourceLocation {
            flow: name("main"),
            node: name(label),
            instantiation: vec![],
        };
        let digest =
            rx_domain::canonical::digest("RX-PROCESS-NODE-v1", &(&process, &source)).unwrap();
        CompiledNode {
            id: name(&format!("node/{digest}")),
            source,
            body,
        }
    };
    let left = make(
        "left",
        CompiledBody::Operation {
            binding: name("left"),
        },
    );
    let right = make(
        "right",
        CompiledBody::Operation {
            binding: name("right"),
        },
    );
    let original = configuration.steps[0].clone();
    let mut alternative = original.clone();
    alternative.intent.target = name("sim/device-alternative");
    alternative.intent.resource_set = vec![name("controller/alternative")];
    let mut bindings = BTreeMap::from([(
        name("left"),
        ActionBinding {
            host: original.host.clone(),
            intent: original.intent.clone(),
        },
    )]);
    let root = match kind {
        TestProcess::Branch => {
            bindings.insert(
                name("right"),
                ActionBinding {
                    host: alternative.host.clone(),
                    intent: alternative.intent.clone(),
                },
            );
            configuration.steps = vec![
                StepBinding {
                    id: left.id.clone(),
                    ..original
                },
                StepBinding {
                    id: right.id.clone(),
                    ..alternative
                },
            ];
            make(
                "branch",
                CompiledBody::Branch {
                    condition: name("select"),
                    when_true: Box::new(left),
                    when_false: Box::new(right),
                },
            )
        }
        TestProcess::Wait => {
            configuration.steps = vec![StepBinding {
                id: left.id.clone(),
                ..original
            }];
            let wait = make(
                "wait",
                CompiledBody::Wait {
                    condition: name("select"),
                    timeout_ns: Counter(500),
                },
            );
            make(
                "sequence",
                CompiledBody::Sequence {
                    children: vec![wait, left],
                },
            )
        }
    };
    configuration.fact_specs.push(FactSpec {
        id: name("select"),
        host: configuration.hosts[0].clone(),
        schema: name("boolean/v1"),
        unit: name("unitless"),
        maximum_age_ns: Counter(20000),
        maximum_uncertainty_ns: Counter(0),
    });
    let resolved = ResolvedProcess {
        schema: name("rx.resolved-process.v1"),
        package_digest: None,
        source_digest: Digest::from_bytes([81; 32]),
        process,
        root,
        bindings,
        conditions: BTreeMap::from([(
            name("select"),
            Condition::Eq {
                fact: name("select"),
                schema: name("boolean/v1"),
                unit: name("unitless"),
                expected: TypedValue::Boolean(true),
            },
        )]),
    };
    configuration.recipe = ArtifactRef {
        sha256: rx_process_contract::frontier::resolved_digest(&resolved).unwrap(),
        schema_id: resolved.schema.clone(),
        size_bytes: Counter(rx_domain::canonical::bytes(&resolved).unwrap().len() as u64),
    };
    configuration.process = Some(Box::new(resolved));
    configuration
}
fn report_select(f: &mut Fixture, value: bool) {
    let mut fact = f
        .app
        .inspect_fact(&f.operator, &f.configuration.id, &name("ready"))
        .unwrap();
    fact.id = name("select");
    fact.source_generation = f.registrations[0].source_sessions[&name("select")].clone();
    fact.evidence_id = id();
    fact.value = TypedValue::Boolean(value);
    f.app.report_fact(&f.hosts[0], fact).unwrap();
}

#[test]
fn p_commits_branch_choice_and_rejects_the_untaken_activation() {
    let mut f = fixture_with_process(1, true, false, true, Some(TestProcess::Branch));
    let run = start(&mut f, 1);
    let part = f
        .app
        .begin_part(
            &f.executor,
            id().as_str(),
            &run.id,
            run.budget.as_ref().unwrap().revision(),
        )
        .unwrap();
    let root = f.configuration.process.as_ref().unwrap().root.id.clone();
    let left = f.configuration.steps[0].id.clone();
    let right = f.configuration.steps[1].id.clone();
    let rev = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
    assert!(
        f.app
            .resolve_activation(&f.executor, &run.id, &left, part.ordinal, rev)
            .is_err()
    );
    assert!(
        f.app
            .choose_process_branch(
                &f.executor,
                id().as_str(),
                &run.id,
                &root,
                part.ordinal,
                rev
            )
            .is_err()
    );
    report_select(&mut f, true);
    let key_ = id();
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .choose_process_branch(
                &f.executor,
                key_.as_str(),
                &run.id,
                &root,
                part.ordinal,
                rev
            )
            .is_err()
    );
    let choice = f
        .app
        .choose_process_branch(
            &f.executor,
            key_.as_str(),
            &run.id,
            &root,
            part.ordinal,
            rev,
        )
        .unwrap();
    assert!(choice.chosen);
    report_select(&mut f, false);
    let repeated = f
        .app
        .choose_process_branch(
            &f.executor,
            id().as_str(),
            &run.id,
            &root,
            part.ordinal,
            rev,
        )
        .unwrap();
    assert_eq!(choice.decision, repeated.decision);
    let progress = f
        .app
        .process_progress(&f.executor, &run.id, part.ordinal)
        .unwrap();
    assert_eq!(progress.frontier.operations, vec![left.clone()]);
    assert!(
        f.app
            .resolve_activation(
                &f.executor,
                &run.id,
                &right,
                part.ordinal,
                progress.run_revision
            )
            .is_err()
    );
    let activation = f
        .app
        .resolve_activation(
            &f.executor,
            &run.id,
            &left,
            part.ordinal,
            progress.run_revision,
        )
        .unwrap();
    let work = submit(&mut f, &activation, id().as_str()).unwrap();
    assert_eq!(
        work.intent.digest().unwrap(),
        f.configuration.steps[0].intent.digest().unwrap()
    );
    let current = f
        .app
        .process_progress(&f.executor, &run.id, part.ordinal)
        .unwrap();
    assert!(current.frontier.operations.is_empty());
}

#[test]
fn wait_deadline_and_success_are_decided_by_p_and_cannot_be_reset_by_retries() {
    for satisfied in [true, false] {
        let mut f = fixture_with_process(1, true, false, true, Some(TestProcess::Wait));
        let run = start(&mut f, 1);
        let part = f
            .app
            .begin_part(
                &f.executor,
                id().as_str(),
                &run.id,
                run.budget.as_ref().unwrap().revision(),
            )
            .unwrap();
        let wait =
            rx_process_contract::validation::nodes(f.configuration.process.as_ref().unwrap())
                .into_iter()
                .find(|node| matches!(node.body, rx_process_contract::CompiledBody::Wait { .. }))
                .unwrap()
                .id
                .clone();
        let rev = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
        let window = f
            .app
            .start_process_wait(
                &f.executor,
                id().as_str(),
                &run.id,
                &wait,
                part.ordinal,
                rev,
            )
            .unwrap();
        f.clock.0.store(1200, Ordering::SeqCst);
        let same = f
            .app
            .start_process_wait(
                &f.executor,
                id().as_str(),
                &run.id,
                &wait,
                part.ordinal,
                rev,
            )
            .unwrap();
        assert_eq!(window.id, same.id);
        assert_eq!(window.expires_at, same.expires_at);
        let progress = f
            .app
            .process_progress(&f.executor, &run.id, part.ordinal)
            .unwrap();
        assert!(
            f.app
                .check_process_wait(
                    &f.executor,
                    &run.id,
                    &wait,
                    part.ordinal,
                    progress.run_revision
                )
                .unwrap()
                .is_none()
        );
        if satisfied {
            report_select(&mut f, true);
        } else {
            f.clock.0.store(1500, Ordering::SeqCst);
        }
        let result = f
            .app
            .check_process_wait(
                &f.executor,
                &run.id,
                &wait,
                part.ordinal,
                progress.run_revision,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            matches!(
                result,
                rx_process_contract::frontier::WaitProgress::Satisfied { .. }
            ),
            satisfied
        );
        let progress = f
            .app
            .process_progress(&f.executor, &run.id, part.ordinal)
            .unwrap();
        if satisfied {
            assert_eq!(
                progress.frontier.operations,
                vec![f.configuration.steps[0].id.clone()]
            );
        } else {
            assert_eq!(
                progress.frontier.state,
                rx_process_contract::frontier::State::Failed
            );
            assert!(progress.frontier.operations.is_empty());
        }
    }
}

#[test]
fn resolved_plan_binding_and_digest_cannot_be_substituted_in_cell_configuration() {
    let mut f = fixture(1, false);
    let mut configuration = process_configuration(f.configuration.clone(), TestProcess::Branch);
    configuration.id = name("cell/b");
    configuration.steps[0].intent.target = name("unapproved/target");
    assert!(f.app.install_cell(&f.admin, configuration).is_err());
}

#[test]
fn process_checkpoint_survives_runtime_restart_without_restoring_execution_authority() {
    let mut f = fixture_with_process(1, true, false, true, Some(TestProcess::Branch));
    let run = start(&mut f, 1);
    let part = f
        .app
        .begin_part(
            &f.executor,
            id().as_str(),
            &run.id,
            run.budget.as_ref().unwrap().revision(),
        )
        .unwrap();
    report_select(&mut f, true);
    let branch = f.configuration.process.as_ref().unwrap().root.id.clone();
    let revision = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
    let choice = f
        .app
        .choose_process_branch(
            &f.executor,
            id().as_str(),
            &run.id,
            &branch,
            part.ordinal,
            revision,
        )
        .unwrap();
    let saved = f.app.run_checkpoint(&f.executor, &run.id).unwrap();
    let saved_bytes = f
        .app
        .checkpoint_artifact(&f.executor, &run.id, &saved.checkpoint.payload)
        .unwrap();
    let state: checkpoint_artifact::ExecutorState =
        rx_domain::canonical::decode_json(&saved_bytes).unwrap();
    assert_eq!(state.process_checkpoints.len(), 1);
    assert_eq!(
        state.process_checkpoints[0].branches[&branch].decision,
        choice.decision
    );
    assert_eq!(state.revision, saved.revision);
    let installation = f.app.installation.id.clone();
    let repository = f.app.into_repository();
    let mut reopened = Engine::open(
        repository,
        f.clock.clone(),
        SimulationAuthority,
        installation,
        principal("unused", &[Role::AccountAdmin]),
    )
    .unwrap();
    let login = reopened
        .authenticated_session(&name("executor"), id(), expiry(100000))
        .unwrap();
    let executor = Identity {
        principal: name("executor"),
        session: login.id,
        terminal: None,
    };
    let progress = reopened
        .process_progress(&executor, &run.id, part.ordinal)
        .unwrap();
    assert_eq!(
        progress.checkpoint.branches[&branch].decision,
        choice.decision
    );
    assert!(
        progress
            .checkpoint
            .decision_times
            .contains_key(&choice.decision)
    );
    assert!(!progress.admission_allowed);
    assert_eq!(
        reopened
            .checkpoint_artifact(&executor, &run.id, &saved.checkpoint.payload)
            .unwrap(),
        saved_bytes
    );
    let current = reopened.run_checkpoint(&executor, &run.id).unwrap();
    assert_ne!(
        current.checkpoint.payload.sha256,
        saved.checkpoint.payload.sha256
    );
    assert_eq!(current.run.state, RunState::RecoveryRequired);
    let current_bytes = reopened
        .checkpoint_artifact(&executor, &run.id, &current.checkpoint.payload)
        .unwrap();
    let current_state: checkpoint_artifact::ExecutorState =
        rx_domain::canonical::decode_json(&current_bytes).unwrap();
    assert_eq!(
        current_state.process_checkpoints[0].branches[&branch].decision,
        choice.decision
    );
    assert_eq!(
        reopened.inspect_run(&executor, &run.id).unwrap().1.state,
        RunState::RecoveryRequired
    );
    assert!(
        reopened
            .resolve_activation(
                &executor,
                &run.id,
                &f.configuration.steps[0].id,
                part.ordinal,
                progress.run_revision
            )
            .is_err()
    );
}

#[test]
fn part_completion_requires_only_the_committed_branch_and_its_real_release_proof() {
    let _ = completed_branch_fixture();
}
fn completed_branch_fixture() -> (Fixture, Run) {
    let mut f = fixture_with_peer(1, true, false, true, Some(TestProcess::Branch), true);
    let run = start(&mut f, 1);
    let part = f
        .app
        .begin_part(
            &f.executor,
            id().as_str(),
            &run.id,
            run.budget.as_ref().unwrap().revision(),
        )
        .unwrap();
    report_select(&mut f, true);
    let branch = f.configuration.process.as_ref().unwrap().root.id.clone();
    let rev = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
    f.app
        .choose_process_branch(
            &f.executor,
            id().as_str(),
            &run.id,
            &branch,
            part.ordinal,
            rev,
        )
        .unwrap();
    let progress = f
        .app
        .process_progress(&f.executor, &run.id, part.ordinal)
        .unwrap();
    let selected = progress.frontier.operations[0].clone();
    let activation = f
        .app
        .resolve_activation(
            &f.executor,
            &run.id,
            &selected,
            part.ordinal,
            progress.run_revision,
        )
        .unwrap();
    let work = submit(&mut f, &activation, id().as_str()).unwrap();
    let op = work.operation.id().clone();
    let invocation = id();
    f.app.begin_delivery(&op).unwrap();
    f.app
        .record_host_receipt(
            &f.hosts[0],
            &op,
            HostReceipt {
                operation: op.clone(),
                digest: work.intent.digest().unwrap(),
                invocation: Some(invocation.clone()),
                journal: f.registrations[0].delivery_journal.clone(),
                sequence: Counter(10),
                state: ReceiptState::Prepared,
            },
        )
        .unwrap();
    let authorization = rx_application::engine::authorization_delivery_id(&op);
    f.app.begin_delivery(&authorization).unwrap();
    f.app
        .record_host_receipt(
            &f.hosts[0],
            &authorization,
            HostReceipt {
                operation: op.clone(),
                digest: work.intent.digest().unwrap(),
                invocation: Some(invocation.clone()),
                journal: f.registrations[0].delivery_journal.clone(),
                sequence: Counter(11),
                state: ReceiptState::ResultCaptured,
            },
        )
        .unwrap();
    let evidence = NativeEvidence {
        native_details: None,
        id: id(),
        operation: op.clone(),
        invocation,
        profile_digest: work.intent.profile_digest,
        device_session: id(),
        status_schema: name("rx.sim.completed.v1"),
        status: Integer(0),
        captured_at: expiry(1000),
    };
    f.app
        .ingest_evidence(
            &f.hosts[0],
            EvidenceBatch {
                journal: id(),
                first: Counter(1),
                records: vec![evidence.clone()],
            },
        )
        .unwrap();
    let revision = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
    assert!(
        f.app
            .complete_part(&f.executor, id().as_str(), &part.id, revision)
            .is_err()
    );
    let completed = f.app.inspect_work(&f.operator, &op).unwrap();
    let proof = handover_proof(&mut f, &completed, &evidence);
    f.app
        .release_resources(&f.hosts[0], id().as_str(), proof)
        .unwrap();
    let done = f
        .app
        .complete_part(&f.executor, id().as_str(), &part.id, revision)
        .unwrap();
    assert_eq!(done.disposition, PartDisposition::ConfirmedCompleted);
    assert_eq!(
        f.app.inspect_run(&f.operator, &run.id).unwrap().1.state,
        RunState::Completed
    );
    (f, run)
}

#[test]
fn checkpoint_artifact_and_run_projection_commit_with_slot_or_roll_back_together() {
    use rx_application::checkpoint_artifact::{ExecutorState, RUN_SNAPSHOT, RunSnapshot, SCHEMA};
    use rx_domain::canonical;
    use sha2::{Digest as _, Sha256};
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let activation = activation(&mut f, &run);
    let before = f.app.run_checkpoint(&f.executor, &run.id).unwrap();
    let original = f
        .app
        .checkpoint_artifact(&f.executor, &run.id, &before.checkpoint.payload)
        .unwrap();
    assert_eq!(before.checkpoint.activations.len(), 1);
    assert!(before.checkpoint.activations[0].slots.is_empty());
    assert_eq!(before.revision, before.checkpoint.revision);
    assert_eq!(before.checkpoint.payload.schema_id.as_str(), SCHEMA);
    assert_eq!(
        before.checkpoint.payload.sha256,
        Digest::from_bytes(Sha256::digest(&original).into())
    );
    assert_eq!(
        before.checkpoint.payload.size_bytes.0,
        original.len() as u64
    );
    let cr = f
        .app
        .inspect_cell(&f.executor, &f.configuration.id)
        .unwrap()
        .0;
    let command = work_command(&f, &activation, cr, before.revision);
    let request_key = id();
    f.failure.store(1, Ordering::SeqCst);
    assert!(
        f.app
            .submit(&f.executor, request_key.as_str(), command.clone())
            .is_err()
    );
    let rolled_back = f.app.run_checkpoint(&f.executor, &run.id).unwrap();
    assert_eq!(
        canonical::bytes(&rolled_back).unwrap(),
        canonical::bytes(&before).unwrap()
    );
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .submit(&f.executor, request_key.as_str(), command.clone())
            .is_err()
    );
    let work = f
        .app
        .submit(&f.executor, request_key.as_str(), command)
        .unwrap();
    let after = f.app.run_checkpoint(&f.executor, &run.id).unwrap();
    let bytes = f
        .app
        .checkpoint_artifact(&f.executor, &run.id, &after.checkpoint.payload)
        .unwrap();
    let state: ExecutorState = canonical::decode_json(&bytes).unwrap();
    assert_eq!(after.revision, before.revision.increment().unwrap());
    assert_eq!(state.revision, after.revision);
    assert_eq!(state.run.id, run.id);
    assert_eq!(state.activations[0].id, activation.id);
    let slot = &state.activations[0].slots[0];
    assert_eq!(&slot.operation, work.operation.id());
    assert_eq!(slot.intent_digest, work.intent.digest().unwrap());
    assert_eq!(slot.slot, name("main"));
    assert_eq!(
        canonical::bytes(&state.activations).unwrap(),
        canonical::bytes(&after.checkpoint.activations).unwrap()
    );
    assert_ne!(
        after.checkpoint.payload.sha256,
        before.checkpoint.payload.sha256
    );
    assert_eq!(
        f.app
            .checkpoint_artifact(&f.executor, &run.id, &before.checkpoint.payload)
            .unwrap(),
        original
    );
    let mut repo = f.app.into_repository();
    let (_, projections) = repo.control_snapshot().unwrap();
    let stored: RunSnapshot = projections
        .into_iter()
        .filter(|r| r.document.schema.as_str() == RUN_SNAPSHOT)
        .map(|r| serde_json::from_value::<RunSnapshot>(r.document.value).unwrap())
        .find(|r| r.run.id == run.id)
        .unwrap();
    assert_eq!(
        canonical::bytes(&stored).unwrap(),
        canonical::bytes(&after).unwrap()
    );
    let (_, rows) = repo.snapshot().unwrap();
    let artifacts: Vec<ExecutorState> = rows
        .into_iter()
        .filter(|r| r.document.schema.as_str() == SCHEMA)
        .map(|r| serde_json::from_value(r.document.value).unwrap())
        .collect();
    assert_eq!(
        artifacts
            .iter()
            .filter(|r| r.run.id == run.id && r.revision == after.revision)
            .count(),
        1
    );
}

#[test]
fn checkpoint_artifact_requires_current_run_scope_and_exact_owned_reference() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let snapshot = f.app.run_checkpoint(&f.operator, &run.id).unwrap();
    let mut wrong = snapshot.checkpoint.payload.clone();
    wrong.size_bytes = wrong.size_bytes.increment().unwrap();
    assert!(matches!(
        f.app.checkpoint_artifact(&f.operator, &run.id, &wrong),
        Err(StoreError::Invalid(_))
    ));
    let revision = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .0;
    let another = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, revision))
        .unwrap();
    assert!(matches!(
        f.app
            .checkpoint_artifact(&f.operator, &another.id, &snapshot.checkpoint.payload),
        Err(StoreError::Rejected(Rejection::NotFound))
    ));
    let reader = add_identity(&mut f.app, &f.admin, "reader", &[Role::Observer]);
    let mut restricted = principal("reader", &[Role::Observer]);
    restricted.cells = [name("cell/b")].into_iter().collect();
    f.app
        .put_principal(&f.admin, restricted, Some(Counter(1)))
        .unwrap();
    assert!(matches!(
        f.app.run_checkpoint(&reader, &run.id),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    assert!(matches!(
        f.app
            .checkpoint_artifact(&reader, &run.id, &snapshot.checkpoint.payload),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
}

#[test]
fn executor_reconnect_is_idempotent_but_changed_incarnation_revokes_authority() {
    for failure_mode in [1, 2] {
        let mut f = fixture(1, true);
        let run = start(&mut f, 1);
        let legacy = f.executor.clone();
        let peer_boot = id();
        let binding = Digest::from_bytes([41; 32]);
        let before_epoch = f
            .app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch;
        f.failure.store(failure_mode, Ordering::SeqCst);
        assert!(
            f.app
                .open_executor_peer(&name("executor"), peer_boot.clone(), binding)
                .is_err()
        );
        if failure_mode == 1 {
            assert_eq!(
                f.app.inspect_run(&legacy, &run.id).unwrap().1.state,
                RunState::Executing
            );
        } else {
            assert!(f.app.inspect_run(&legacy, &run.id).is_err());
        }
        let session = f
            .app
            .open_executor_peer(&name("executor"), peer_boot.clone(), binding)
            .unwrap();
        let epoch = f
            .app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch;
        assert_eq!(epoch, before_epoch.increment().unwrap());
        let repeated = f
            .app
            .open_executor_peer(&name("executor"), peer_boot.clone(), binding)
            .unwrap();
        assert_eq!(session.id, repeated.id);
        assert_eq!(
            f.app
                .inspect_cell(&f.operator, &f.configuration.id)
                .unwrap()
                .1
                .epoch,
            epoch
        );
        let current = Identity {
            principal: name("executor"),
            session: session.id.clone(),
            terminal: None,
        };
        assert!(matches!(
            f.app.executor_run_checkpoint(&current, &run.id),
            Err(StoreError::Rejected(Rejection::CapabilityMissing))
        ));
        f.app
            .negotiate_executor_cell(&current, f.configuration.definition.sha256)
            .unwrap();
        assert_eq!(
            f.app
                .executor_run_checkpoint(&current, &run.id)
                .unwrap()
                .run
                .state,
            RunState::RecoveryRequired
        );
        assert!(f.app.inspect_service_peer(&legacy).is_err());
        let second_boot = id();
        let second = f
            .app
            .open_executor_peer(&name("executor"), second_boot.clone(), binding)
            .unwrap();
        assert_ne!(session.id, second.id);
        assert!(f.app.inspect_service_peer(&current).is_err());
        let second_epoch = f
            .app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch;
        assert!(matches!(
            f.app
                .open_executor_peer(&name("executor"), peer_boot.clone(), binding),
            Err(StoreError::Rejected(Rejection::Unauthenticated))
        ));
        assert_eq!(
            f.app
                .open_executor_peer(&name("executor"), second_boot.clone(), binding)
                .unwrap()
                .id,
            second.id
        );
        assert_eq!(
            f.app
                .inspect_cell(&f.operator, &f.configuration.id)
                .unwrap()
                .1
                .epoch,
            second_epoch
        );
        let newer = Identity {
            principal: name("executor"),
            session: second.id,
            terminal: None,
        };
        assert!(f.app.executor_run_checkpoint(&newer, &run.id).is_err());
        let installation = f.app.installation.id.clone();
        let repository = f.app.into_repository();
        let mut reopened = Engine::open(
            repository,
            f.clock,
            SimulationAuthority,
            installation,
            principal("unused", &[Role::AccountAdmin]),
        )
        .unwrap();
        assert!(reopened.inspect_service_peer(&newer).is_err());
        assert!(matches!(
            reopened.open_executor_peer(&name("executor"), peer_boot, binding),
            Err(StoreError::Rejected(Rejection::Unauthenticated))
        ));
        let recovered = reopened
            .open_executor_peer(&name("executor"), second_boot, binding)
            .unwrap();
        assert_ne!(recovered.id, newer.session);
        let after_restart = Identity {
            principal: name("executor"),
            session: recovered.id,
            terminal: None,
        };
        assert!(
            reopened
                .executor_run_checkpoint(&after_restart, &run.id)
                .is_err()
        );
        reopened
            .negotiate_executor_cell(&after_restart, f.configuration.definition.sha256)
            .unwrap();
        assert_eq!(
            reopened
                .executor_run_checkpoint(&after_restart, &run.id)
                .unwrap()
                .run
                .state,
            RunState::RecoveryRequired
        );
    }
}

#[test]
fn executor_part_and_activation_mutations_preserve_full_request_identity_and_atomic_results() {
    for failure_mode in [1, 2] {
        let mut f = fixture_with_peer(1, true, false, false, None, true);
        let run = start(&mut f, 1);
        let cell_revision = f
            .app
            .inspect_cell(&f.executor, &f.configuration.id)
            .unwrap()
            .0;
        let command = BeginPartRequest {
            cell: f.configuration.id.clone(),
            run: run.id.clone(),
            mandate: run.mandate.clone().unwrap(),
            expected_budget: run.budget.as_ref().unwrap().revision(),
            expected_cell: Some(cell_revision),
        };
        let key = id();
        f.failure.store(failure_mode, Ordering::SeqCst);
        assert!(
            f.app
                .executor_begin_part(&f.executor, key.as_str(), command.clone())
                .is_err()
        );
        let observed = f.app.inspect_run(&f.executor, &run.id).unwrap().1;
        assert_eq!(
            observed.part_ids.len(),
            if failure_mode == 1 { 0 } else { 1 }
        );
        let part = f
            .app
            .executor_begin_part(&f.executor, key.as_str(), command.clone())
            .unwrap();
        assert_eq!(part.revision, Counter(1));
        assert_eq!(part.part.ordinal, Counter(1));
        assert_eq!(
            f.app
                .executor_begin_part(&f.executor, key.as_str(), command.clone())
                .unwrap()
                .part
                .id,
            part.part.id
        );
        let mut changed = command.clone();
        changed.expected_budget = Counter(2);
        assert!(matches!(
            f.app
                .executor_begin_part(&f.executor, key.as_str(), changed),
            Err(StoreError::KeyConflict)
        ));
        let mut wrong_mandate = command.clone();
        wrong_mandate.mandate = id();
        assert!(matches!(
            f.app
                .executor_begin_part(&f.executor, id().as_str(), wrong_mandate),
            Err(StoreError::Rejected(Rejection::MandateRevoked))
        ));
        let revision = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
        let resolve = ResolveActivationRequest {
            run: run.id.clone(),
            node: name("step/0"),
            visit: part.part.ordinal,
            expected_run: revision,
        };
        let activation_key = id();
        f.failure.store(failure_mode, Ordering::SeqCst);
        assert!(
            f.app
                .executor_resolve_activation(&f.executor, activation_key.as_str(), resolve.clone())
                .is_err()
        );
        let activation = f
            .app
            .executor_resolve_activation(&f.executor, activation_key.as_str(), resolve.clone())
            .unwrap();
        let checkpoint = f.app.run_checkpoint(&f.executor, &run.id).unwrap();
        assert_eq!(checkpoint.revision, revision.increment().unwrap());
        assert_eq!(checkpoint.checkpoint.activations.len(), 1);
        assert_eq!(checkpoint.checkpoint.activations[0].id, activation.id);
        assert_eq!(
            f.app
                .executor_resolve_activation(&f.executor, id().as_str(), resolve.clone())
                .unwrap()
                .id,
            activation.id
        );
        assert_eq!(
            f.app.inspect_run(&f.executor, &run.id).unwrap().0,
            checkpoint.revision
        );
        let mut changed = resolve.clone();
        changed.visit = Counter(2);
        assert!(matches!(
            f.app.executor_resolve_activation(
                &f.executor,
                activation_key.as_str(),
                changed.clone()
            ),
            Err(StoreError::KeyConflict)
        ));
        assert!(
            f.app
                .executor_resolve_activation(&f.executor, id().as_str(), changed)
                .is_err()
        );
        let mut revoked = principal("executor", &[Role::Executor, Role::Observer]);
        revoked.active = false;
        f.app
            .put_principal(&f.admin, revoked, Some(Counter(1)))
            .unwrap();
        assert!(matches!(
            f.app
                .executor_begin_part(&f.executor, key.as_str(), command),
            Err(StoreError::Rejected(Rejection::Forbidden))
        ));
        assert!(matches!(
            f.app
                .executor_resolve_activation(&f.executor, activation_key.as_str(), resolve),
            Err(StoreError::Rejected(Rejection::Forbidden))
        ));
    }
}

#[test]
fn admission_receipt_is_anchored_to_the_original_control_record_and_survives_later_changes() {
    use rx_application::control_journal::{
        ADMISSION_SCHEMA, CHANGE_SCHEMA, ControlChange, EntityKind,
    };
    use rx_domain::canonical;
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let activation = activation(&mut f, &run);
    let cell_revision = f
        .app
        .inspect_cell(&f.executor, &f.configuration.id)
        .unwrap()
        .0;
    let run_revision = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
    let command = work_command(&f, &activation, cell_revision, run_revision);
    let key = id();
    f.failure.store(1, Ordering::SeqCst);
    assert!(
        f.app
            .submit(&f.executor, key.as_str(), command.clone())
            .is_err()
    );
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .submit(&f.executor, key.as_str(), command.clone())
            .is_err()
    );
    let work = f.app.submit(&f.executor, key.as_str(), command).unwrap();
    let receipt = f
        .app
        .admission_receipt(&f.executor, work.operation.id())
        .unwrap();
    assert_eq!(receipt.operation, work.operation.id().clone());
    assert_eq!(receipt.operation_revision, Counter(1));
    assert_eq!(receipt.intent_digest, work.intent.digest().unwrap());
    assert_eq!(
        receipt.journal,
        control_journal::journal_id(&f.app.installation).unwrap()
    );
    let mut restarted = f.app.installation.clone();
    restarted.runtime_boot = id();
    assert_eq!(
        receipt.journal,
        control_journal::journal_id(&restarted).unwrap()
    );
    restarted.store_generation = id();
    assert_ne!(
        receipt.journal,
        control_journal::journal_id(&restarted).unwrap()
    );
    f.app
        .hold(&f.operator, id().as_str(), &f.configuration.id)
        .unwrap();
    let repeated = f
        .app
        .admission_receipt(&f.operator, work.operation.id())
        .unwrap();
    assert_eq!(
        canonical::bytes(&receipt).unwrap(),
        canonical::bytes(&repeated).unwrap()
    );
    let mut repository = f.app.into_repository();
    let (_, rows) = repository.snapshot().unwrap();
    assert_eq!(
        rows.iter()
            .filter(|r| r.document.schema.as_str() == ADMISSION_SCHEMA)
            .count(),
        1
    );
    let records = repository
        .control_events_after(Counter(receipt.sequence.0 - 1), 1)
        .unwrap();
    let event = records
        .into_iter()
        .find(|e| e.seq == receipt.sequence)
        .unwrap();
    assert_eq!(event.document.schema.as_str(), CHANGE_SCHEMA);
    let change: ControlChange = serde_json::from_value(event.document.value).unwrap();
    assert_eq!(change.entity_kind, EntityKind::Work);
    let original: Work = serde_json::from_value(change.entity.document.value).unwrap();
    assert_eq!(original.operation.id(), &receipt.operation);
    assert_eq!(original.operation.revision(), receipt.operation_revision);
    assert_eq!(
        original.operation.phase(),
        rx_domain::operation::Phase::Admitted
    );
    assert!(original.invocation.is_none());
}

#[test]
fn executor_submit_checks_the_whole_envelope_and_recovers_an_immutable_receipt_after_t1_loss() {
    use rx_domain::canonical;
    for failure_mode in [1, 2] {
        let mut f = fixture_with_peer(1, true, false, false, None, true);
        let run = start(&mut f, 1);
        let part = f
            .app
            .begin_part(
                &f.executor,
                id().as_str(),
                &run.id,
                run.budget.as_ref().unwrap().revision(),
            )
            .unwrap();
        let revision = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
        let activation = f
            .app
            .resolve_activation(
                &f.executor,
                &run.id,
                &name("step/0"),
                part.ordinal,
                revision,
            )
            .unwrap();
        let revision = f.app.inspect_run(&f.executor, &run.id).unwrap().0;
        let cell_revision = f
            .app
            .inspect_cell(&f.executor, &f.configuration.id)
            .unwrap()
            .0;
        let command = ExecutorSubmitRequest {
            cell: f.configuration.id.clone(),
            mandate: run.mandate.unwrap(),
            work: SubmitWork {
                run: run.id.clone(),
                activation: activation.id,
                part: Some(part.id),
                slot: name("main"),
                intent: f.configuration.steps[0].intent.clone(),
                expected_cell: cell_revision,
                expected_run: revision,
            },
        };
        let mut invalid = command.clone();
        invalid.mandate = id();
        assert!(matches!(
            f.app.executor_submit(&f.executor, id().as_str(), invalid),
            Err(StoreError::Rejected(Rejection::MandateRevoked))
        ));
        let mut invalid = command.clone();
        invalid.work.part = Some(id());
        assert!(matches!(
            f.app.executor_submit(&f.executor, id().as_str(), invalid),
            Err(StoreError::Rejected(Rejection::InvalidInput))
        ));
        let mut stale = command.clone();
        stale.work.expected_cell = Counter(1);
        assert!(
            f.app
                .executor_submit(&f.executor, id().as_str(), stale)
                .is_err()
        );
        let key = id();
        f.failure.store(failure_mode, Ordering::SeqCst);
        assert!(
            f.app
                .executor_submit(&f.executor, key.as_str(), command.clone())
                .is_err()
        );
        assert_eq!(
            f.app.overview(&f.operator).unwrap().cells[0].work.len(),
            if failure_mode == 1 { 0 } else { 1 }
        );
        let receipt = f
            .app
            .executor_submit(&f.executor, key.as_str(), command.clone())
            .unwrap();
        assert_eq!(receipt.operation_revision, Counter(1));
        assert_eq!(receipt.intent_digest, command.work.intent.digest().unwrap());
        assert!(receipt.sequence.0 > 0);
        let mut changed = command.clone();
        changed.work.expected_run = revision.increment().unwrap();
        assert!(matches!(
            f.app.executor_submit(&f.executor, key.as_str(), changed),
            Err(StoreError::KeyConflict)
        ));
        let mut changed = command.clone();
        changed.mandate = id();
        assert!(matches!(
            f.app
                .executor_submit(&f.executor, key.as_str(), changed.clone()),
            Err(StoreError::KeyConflict)
        ));
        assert!(matches!(
            f.app.executor_submit(&f.executor, id().as_str(), changed),
            Err(StoreError::Rejected(Rejection::MandateRevoked))
        ));
        let second = f
            .app
            .executor_submit(&f.executor, id().as_str(), command.clone())
            .unwrap();
        assert_eq!(
            canonical::bytes(&second).unwrap(),
            canonical::bytes(&receipt).unwrap()
        );
        f.app
            .hold(&f.operator, id().as_str(), &f.configuration.id)
            .unwrap();
        let after = f
            .app
            .executor_submit(&f.executor, key.as_str(), command.clone())
            .unwrap();
        assert_eq!(
            canonical::bytes(&after).unwrap(),
            canonical::bytes(&receipt).unwrap()
        );
        assert_eq!(f.app.overview(&f.operator).unwrap().cells[0].work.len(), 1);
        let mut revoked = principal("executor", &[Role::Executor, Role::Observer]);
        revoked.active = false;
        f.app
            .put_principal(&f.admin, revoked, Some(Counter(1)))
            .unwrap();
        assert!(matches!(
            f.app.executor_submit(&f.executor, key.as_str(), command),
            Err(StoreError::Rejected(Rejection::Forbidden))
        ));
        assert!(
            f.app
                .executor_work(&f.executor, &receipt.operation)
                .is_err()
        );
    }
}

#[test]
fn live_execution_snapshot_preserves_the_cut_and_does_not_reuse_aged_maintained_conditions() {
    use rx_process_contract::execution_validation;
    let mut f = fixture_with_peer(1, true, true, true, Some(TestProcess::Branch), true);
    let run = start(&mut f, 1);
    let part = f
        .app
        .begin_part(
            &f.executor,
            id().as_str(),
            &run.id,
            run.budget.as_ref().unwrap().revision(),
        )
        .unwrap();
    let initial = f
        .app
        .execution_snapshot(&f.executor, &run.id, part.ordinal)
        .unwrap();
    let plan = f.configuration.process.as_ref().unwrap();
    let frontier = execution_validation::validate(&initial, plan).unwrap();
    assert!(initial.request_admission_allowed && initial.admission_reason.is_none());
    assert_eq!(frontier.decisions, vec![plan.root.id.clone()]);
    let restored = f
        .app
        .executor_artifact(&f.executor, &run.id, &initial.run.checkpoint.payload)
        .unwrap();
    let restored: rx_process_contract::execution::ExecutorState =
        rx_domain::canonical::decode_json(&restored).unwrap();
    assert_eq!(restored.run.id, run.id);
    assert_eq!(restored.revision, initial.run.revision);
    let process = f
        .app
        .executor_artifact(&f.executor, &run.id, &initial.resolved)
        .unwrap();
    assert_eq!(process, rx_domain::canonical::bytes(plan).unwrap());
    report_select(&mut f, true);
    let node = f.configuration.process.as_ref().unwrap().root.id.clone();
    let choice = f
        .app
        .choose_process_branch(
            &f.executor,
            id().as_str(),
            &run.id,
            &node,
            part.ordinal,
            initial.run.revision,
        )
        .unwrap();
    let decided = f
        .app
        .execution_snapshot(&f.executor, &run.id, part.ordinal)
        .unwrap();
    assert!(decided.sequence > initial.sequence);
    assert_eq!(
        decided.process_checkpoint.branches[&node].decision,
        choice.decision
    );
    execution_validation::validate(&decided, f.configuration.process.as_ref().unwrap()).unwrap();
    let mut malformed = decided.clone();
    malformed.progress.branches.clear();
    assert!(
        execution_validation::validate(&malformed, f.configuration.process.as_ref().unwrap())
            .is_err()
    );
    f.clock.0.store(21001, Ordering::SeqCst);
    let aged = f
        .app
        .execution_snapshot(&f.executor, &run.id, part.ordinal)
        .unwrap();
    assert_eq!(aged.run.run.state, RunState::Executing);
    assert!(!aged.request_admission_allowed);
    assert_eq!(aged.admission_reason, Some(Rejection::ConditionUnknown));
    let step = f.configuration.steps[0].id.clone();
    assert!(matches!(
        f.app
            .resolve_activation(&f.executor, &run.id, &step, part.ordinal, aged.run.revision),
        Err(StoreError::Rejected(Rejection::ConditionUnknown))
    ));
}

#[test]
fn pause_commits_reply_fence_and_unsent_sealing_atomically_without_refunding_budget() {
    use rx_domain::{
        canonical,
        operation::{Disposition, Outcome},
    };
    for failure_mode in [1, 2] {
        let mut f = fixture_with_peer(1, true, false, false, None, true);
        let mut shared = f.configuration.clone();
        shared.id = name("cell/b");
        f.app.install_cell(&f.admin, shared).unwrap();
        let run = start(&mut f, 1);
        let activation = activation(&mut f, &run);
        let work = submit(&mut f, &activation, id().as_str()).unwrap();
        let before = f.app.run_checkpoint(&f.executor, &run.id).unwrap();
        let command = PauseRunRequest {
            run: run.id.clone(),
            expected_run: before.revision,
        };
        let key = id();
        f.failure.store(failure_mode, Ordering::SeqCst);
        assert!(
            f.app
                .pause_executor_run(&f.executor, key.as_str(), command.clone())
                .is_err()
        );
        let state = f.app.inspect_run(&f.operator, &run.id).unwrap().1.state;
        assert_eq!(
            state,
            if failure_mode == 1 {
                RunState::Executing
            } else {
                RunState::Paused
            }
        );
        let paused = f
            .app
            .pause_executor_run(&f.executor, key.as_str(), command.clone())
            .unwrap();
        assert_eq!(paused.run.state, RunState::Paused);
        assert_eq!(paused.revision, before.revision.increment().unwrap());
        assert_eq!(paused.run.part_ids, before.run.part_ids);
        assert_eq!(paused.run.budget, before.run.budget);
        assert!(paused.run.executor_session.is_none());
        assert_eq!(
            f.app
                .inspect_cell(&f.operator, &f.configuration.id)
                .unwrap()
                .1
                .epoch,
            Counter(2)
        );
        assert_eq!(
            f.app
                .inspect_cell(&f.operator, &name("cell/b"))
                .unwrap()
                .1
                .epoch,
            Counter(2)
        );
        let repeated = f
            .app
            .pause_executor_run(&f.executor, key.as_str(), command)
            .unwrap();
        assert_eq!(
            canonical::bytes(&repeated).unwrap(),
            canonical::bytes(&paused).unwrap()
        );
        assert_eq!(
            f.app
                .inspect_cell(&f.operator, &f.configuration.id)
                .unwrap()
                .1
                .epoch,
            Counter(2)
        );
        let current = f
            .app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap();
        assert_eq!(current.operation.outcome(), Outcome::NotExecuted);
        assert_eq!(current.operation.disposition(), Disposition::Quarantined);
        assert!(f.app.begin_delivery(work.operation.id()).is_err());
        assert!(
            f.app
                .begin_part(
                    &f.executor,
                    id().as_str(),
                    &run.id,
                    paused.run.budget.as_ref().unwrap().revision()
                )
                .is_err()
        );
        let mut changed = PauseRunRequest {
            run: run.id.clone(),
            expected_run: paused.revision,
        };
        assert!(matches!(
            f.app
                .pause_executor_run(&f.executor, key.as_str(), changed.clone()),
            Err(StoreError::KeyConflict)
        ));
        f.app
            .hold(&f.operator, id().as_str(), &f.configuration.id)
            .unwrap();
        let (revision, _) = f.app.inspect_run(&f.operator, &run.id).unwrap();
        changed.expected_run = revision;
        assert_eq!(
            f.app
                .pause_executor_run(&f.executor, id().as_str(), changed)
                .unwrap()
                .run
                .state,
            RunState::RecoveryRequired
        );
    }
}

#[test]
fn late_prepared_receipt_after_pause_does_not_create_an_authorization() {
    let mut f = fixture_with_peer(1, true, false, false, None, true);
    let run = start(&mut f, 1);
    let activation = activation(&mut f, &run);
    let work = submit(&mut f, &activation, id().as_str()).unwrap();
    let operation = work.operation.id().clone();
    f.app.begin_delivery(&operation).unwrap();
    let (revision, _) = f.app.inspect_run(&f.executor, &run.id).unwrap();
    f.app
        .pause_executor_run(
            &f.executor,
            id().as_str(),
            PauseRunRequest {
                run: run.id,
                expected_run: revision,
            },
        )
        .unwrap();
    f.app
        .record_host_receipt(
            &f.hosts[0],
            &operation,
            HostReceipt {
                operation: operation.clone(),
                digest: work.intent.digest().unwrap(),
                invocation: Some(id()),
                journal: f.registrations[0].delivery_journal.clone(),
                sequence: Counter(10),
                state: ReceiptState::Prepared,
            },
        )
        .unwrap();
    assert_eq!(
        f.app
            .inspect_permit(&f.operator, &work.permit)
            .unwrap()
            .state,
        PermitState::Voided
    );
    let mut repository = f.app.into_repository();
    assert!(
        repository
            .transact(
                |tx| tx.outbox(&rx_application::engine::authorization_delivery_id(
                    &operation
                ))
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn pause_after_emit_is_not_cancellation_and_late_correlated_result_is_still_recorded() {
    use rx_domain::operation::{Knowledge, Outcome};
    let mut f = fixture_with_peer(1, true, false, true, None, true);
    let run = start(&mut f, 1);
    let activation = activation(&mut f, &run);
    let work = submit(&mut f, &activation, id().as_str()).unwrap();
    let operation = work.operation.id().clone();
    let invocation = id();
    f.app.begin_delivery(&operation).unwrap();
    f.app
        .record_host_receipt(
            &f.hosts[0],
            &operation,
            HostReceipt {
                operation: operation.clone(),
                digest: work.intent.digest().unwrap(),
                invocation: Some(invocation.clone()),
                journal: f.registrations[0].delivery_journal.clone(),
                sequence: Counter(10),
                state: ReceiptState::Prepared,
            },
        )
        .unwrap();
    let authorize = rx_application::engine::authorization_delivery_id(&operation);
    f.app.begin_delivery(&authorize).unwrap();
    let (revision, _) = f.app.inspect_run(&f.executor, &run.id).unwrap();
    f.app
        .pause_executor_run(
            &f.executor,
            id().as_str(),
            PauseRunRequest {
                run: run.id,
                expected_run: revision,
            },
        )
        .unwrap();
    let unknown = f.app.inspect_work(&f.operator, &operation).unwrap();
    assert_eq!(unknown.operation.outcome(), Outcome::None);
    assert_eq!(unknown.operation.knowledge(), Knowledge::Unknown);
    let evidence = NativeEvidence {
        id: id(),
        operation: operation.clone(),
        invocation,
        profile_digest: work.intent.profile_digest,
        device_session: id(),
        status_schema: name("rx.sim.completed.v1"),
        status: Integer(0),
        captured_at: expiry(1000),
        native_details: None,
    };
    f.app
        .ingest_evidence(
            &f.hosts[0],
            EvidenceBatch {
                journal: id(),
                first: Counter(1),
                records: vec![evidence],
            },
        )
        .unwrap();
    assert_eq!(
        f.app
            .inspect_work(&f.operator, &operation)
            .unwrap()
            .operation
            .outcome(),
        Outcome::Succeeded
    );
}

#[test]
fn pausing_a_prepared_run_without_arm_does_not_fence_other_cell_work() {
    let mut f = fixture_with_peer(1, true, false, false, None, true);
    let (revision, _) = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    let run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, revision))
        .unwrap();
    let paused = f
        .app
        .pause_executor_run(
            &f.executor,
            id().as_str(),
            PauseRunRequest {
                run: run.id,
                expected_run: Counter(1),
            },
        )
        .unwrap();
    assert_eq!(paused.run.state, RunState::Paused);
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch,
        Counter(1)
    );
}

#[test]
fn pause_rejects_a_late_arm_acknowledgment_from_start_preparation() {
    let mut f = fixture_with_peer(1, true, false, false, None, true);
    let (revision, _) = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    let run = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, revision))
        .unwrap();
    let attempt = f
        .app
        .start_run(
            &f.operator,
            id().as_str(),
            start_command(&f, &run, revision, 1),
        )
        .unwrap();
    let (revision, _) = f.app.inspect_run(&f.executor, &run.id).unwrap();
    let paused = f
        .app
        .pause_executor_run(
            &f.executor,
            id().as_str(),
            PauseRunRequest {
                run: run.id.clone(),
                expected_run: revision,
            },
        )
        .unwrap();
    assert_eq!(paused.run.state, RunState::Paused);
    assert!(paused.run.pending_attempt.is_none());
    let h = &f.registrations[0];
    assert!(
        f.app
            .acknowledge_arm(
                &f.hosts[0],
                ArmAcknowledgment {
                    attempt: attempt.id,
                    host_boot: h.boot_id.clone(),
                    delivery_journal: h.delivery_journal.clone(),
                    sequence: Counter(1),
                    epoch: h.epoch,
                    scopes: h.scopes.clone()
                }
            )
            .is_err()
    );
    assert!(
        f.app
            .inspect_run(&f.operator, &run.id)
            .unwrap()
            .1
            .mandate
            .is_none()
    );
}

#[test]
fn reconciliation_requests_are_durable_coalesced_and_cannot_claim_release() {
    let mut f = fixture_with_peer(1, true, false, true, None, true);
    let (work, evidence) = native_started(&mut f);
    f.app
        .request_reconciliation(&f.executor, work.operation.id())
        .unwrap();
    let first = f.app.pending_reconciliations(&f.hosts[0], None, 8).unwrap();
    assert_eq!(first.len(), 1);
    f.app
        .request_reconciliation(&f.executor, work.operation.id())
        .unwrap();
    let repeated = f.app.pending_reconciliations(&f.hosts[0], None, 8).unwrap();
    assert_eq!(repeated[0].id, first[0].id);
    assert_eq!(repeated[0].generation, Counter(1));
    assert!(
        f.app
            .update_reconciliation(
                &f.hosts[0],
                work.operation.id(),
                &first[0].id,
                ReconciliationState::Complete,
                None
            )
            .is_err()
    );
    f.app
        .ingest_evidence(
            &f.hosts[0],
            EvidenceBatch {
                journal: id(),
                first: Counter(1),
                records: vec![evidence],
            },
        )
        .unwrap();
    let plan = f
        .app
        .plan_reconciliation(&f.hosts[0], work.operation.id(), &first[0].id)
        .unwrap();
    assert_eq!(
        plan.work.operation.outcome(),
        rx_domain::operation::Outcome::Succeeded
    );
    assert_ne!(
        plan.work.operation.disposition(),
        rx_domain::operation::Disposition::Released
    );
    f.app
        .update_reconciliation(
            &f.hosts[0],
            work.operation.id(),
            &first[0].id,
            ReconciliationState::Attention,
            Some(ReconciliationIssue::ContinuityUnproven),
        )
        .unwrap();
    f.app
        .request_reconciliation(&f.executor, work.operation.id())
        .unwrap();
    let next = f.app.pending_reconciliations(&f.hosts[0], None, 8).unwrap();
    assert_ne!(next[0].id, first[0].id);
    assert_eq!(next[0].generation, Counter(2));
    assert!(
        f.app
            .update_reconciliation(
                &f.hosts[0],
                work.operation.id(),
                &first[0].id,
                ReconciliationState::Attention,
                None
            )
            .is_err()
    );
    let mut repo = f.app.into_repository();
    let rows = repo.snapshot().unwrap().1;
    let saved = rows
        .iter()
        .filter(|r| r.document.schema.as_str() == "rx.internal.reconciliation-request.v1")
        .collect::<Vec<_>>();
    assert_eq!(saved.len(), 1);
    let plan: ReconciliationRequest =
        serde_json::from_value(saved[0].document.value.clone()).unwrap();
    assert_eq!(plan.id, next[0].id);
    assert_eq!(plan.state, ReconciliationState::Pending);
}

#[test]
fn false_handover_facts_are_retained_and_identity_conflicts_block_release() {
    use rx_domain::operation::Disposition;
    let mut f = fixture_with_peer(1, true, false, true, None, true);
    let (work, evidence) = completed_work(&mut f);
    f.app
        .request_reconciliation(&f.executor, work.operation.id())
        .unwrap();
    let plan = f
        .app
        .pending_reconciliations(&f.hosts[0], None, 8)
        .unwrap()
        .remove(0);
    let mut proof = handover_proof(&mut f, &work, &evidence);
    proof
        .observations
        .iter_mut()
        .find(|o| o.source.as_str().ends_with("/support"))
        .unwrap()
        .value = false;
    f.app
        .record_reconciliation_observations(
            &f.hosts[0],
            work.operation.id(),
            &plan.id,
            &proof.observations,
        )
        .unwrap();
    assert!(
        f.app
            .release_resources(&f.hosts[0], id().as_str(), proof.clone())
            .is_err()
    );
    assert_ne!(
        f.app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap()
            .operation
            .disposition(),
        Disposition::Released
    );
    proof
        .observations
        .iter_mut()
        .find(|o| o.source.as_str().ends_with("/support"))
        .unwrap()
        .value = true;
    assert!(matches!(
        f.app.record_reconciliation_observations(
            &f.hosts[0],
            work.operation.id(),
            &plan.id,
            &proof.observations
        ),
        Err(StoreError::Integrity(_))
    ));
    let cell = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1;
    assert!(
        cell.blocks
            .iter()
            .any(|b| b.reason == BlockReason::IntegrityConflict)
    );
    let mut repo = f.app.into_repository();
    let rows = repo.snapshot().unwrap().1;
    let observations = rows
        .iter()
        .filter(|r| r.document.schema.as_str() == "rx.internal.handover-observation.v1")
        .map(|r| serde_json::from_value::<HandoverObservation>(r.document.value.clone()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(observations.len(), 3);
    assert!(
        !observations
            .iter()
            .find(|o| o.source.as_str().ends_with("/support"))
            .unwrap()
            .value
    );
}

fn checkpoint_fixture(kind: TestProcess) -> (Fixture, CheckpointTarget) {
    let mut f = fixture_with_peer(1, true, false, true, Some(kind), true);
    let run = start(&mut f, 1);
    let part = f
        .app
        .begin_part(
            &f.executor,
            id().as_str(),
            &run.id,
            run.budget.as_ref().unwrap().revision(),
        )
        .unwrap();
    let node = rx_process_contract::validation::nodes(f.configuration.process.as_ref().unwrap())
        .into_iter()
        .find(|n| {
            matches!(
                n.body,
                rx_process_contract::CompiledBody::Branch { .. }
                    | rx_process_contract::CompiledBody::Wait { .. }
            )
        })
        .unwrap()
        .id
        .clone();
    let action = if matches!(kind, TestProcess::Branch) {
        CheckpointAction::ChooseBranch
    } else {
        CheckpointAction::StartWait
    };
    (
        f,
        CheckpointTarget {
            run: run.id,
            node,
            visit: part.ordinal,
            action,
        },
    )
}
fn prepared(f: &mut Fixture, target: &CheckpointTarget) -> PreparedCheckpoint {
    let CheckpointPreparation::Ready { proposal } = f
        .app
        .prepare_process_checkpoint(&f.executor, target.clone())
        .unwrap()
    else {
        panic!("ready proposal expected")
    };
    *proposal
}
fn checkpoint_command(
    target: &CheckpointTarget,
    prepared: &PreparedCheckpoint,
) -> CommitCheckpoint {
    CommitCheckpoint {
        run: target.run.clone(),
        expected_revision: prepared.expected_revision,
        new_checkpoint: prepared.checkpoint.clone(),
    }
}
#[test]
fn checkpoint_proposal_is_not_an_applied_decision_and_commit_response_survives_loss() {
    use rx_domain::canonical::bytes;
    let (mut f, target) = checkpoint_fixture(TestProcess::Branch);
    let revision = f.app.inspect_run(&f.executor, &target.run).unwrap().0;
    assert!(matches!(
        f.app
            .prepare_process_checkpoint(&f.executor, target.clone())
            .unwrap(),
        CheckpointPreparation::Waiting { .. }
    ));
    report_select(&mut f, true);
    let proposal = prepared(&mut f, &target);
    assert_eq!(
        bytes(&proposal).unwrap(),
        bytes(&prepared(&mut f, &target)).unwrap()
    );
    assert_eq!(
        f.app.inspect_run(&f.executor, &target.run).unwrap().0,
        revision
    );
    assert!(
        f.app
            .process_progress(&f.executor, &target.run, target.visit)
            .unwrap()
            .checkpoint
            .branches
            .is_empty()
    );
    let command = checkpoint_command(&target, &proposal);
    let key = id();
    f.failure.store(1, Ordering::SeqCst);
    assert!(
        f.app
            .commit_process_checkpoint(&f.executor, key.as_str(), command.clone())
            .is_err()
    );
    assert!(
        f.app
            .process_progress(&f.executor, &target.run, target.visit)
            .unwrap()
            .checkpoint
            .branches
            .is_empty()
    );
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .commit_process_checkpoint(&f.executor, key.as_str(), command.clone())
            .is_err()
    );
    let applied = f
        .app
        .commit_process_checkpoint(&f.executor, key.as_str(), command.clone())
        .unwrap();
    assert_eq!(applied.revision, revision.increment().unwrap());
    assert_eq!(applied.checkpoint.payload, proposal.checkpoint.payload);
    assert!(
        f.app
            .process_progress(&f.executor, &target.run, target.visit)
            .unwrap()
            .checkpoint
            .branches[&target.node]
            .chosen
    );
    let mut changed = command.clone();
    changed.expected_revision = changed.expected_revision.increment().unwrap();
    assert!(matches!(
        f.app
            .commit_process_checkpoint(&f.executor, key.as_str(), changed),
        Err(StoreError::KeyConflict)
    ));
    f.app
        .hold(&f.operator, id().as_str(), &f.configuration.id)
        .unwrap();
    assert_eq!(
        bytes(&applied).unwrap(),
        bytes(
            &f.app
                .commit_process_checkpoint(&f.executor, key.as_str(), command.clone())
                .unwrap()
        )
        .unwrap()
    );
    let mut revoked = principal("executor", &[Role::Executor, Role::Observer]);
    revoked.active = false;
    f.app
        .put_principal(&f.admin, revoked, Some(Counter(1)))
        .unwrap();
    assert!(matches!(
        f.app
            .commit_process_checkpoint(&f.executor, key.as_str(), command),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
}
#[test]
fn checkpoint_commit_rejects_changed_mapping_artifact_and_stale_condition() {
    let (mut f, target) = checkpoint_fixture(TestProcess::Branch);
    report_select(&mut f, true);
    let proposal = prepared(&mut f, &target);
    let original = checkpoint_command(&target, &proposal);
    let mut wrong = vec![];
    let mut value = original.clone();
    value.new_checkpoint.run = id();
    wrong.push(value);
    let mut value = original.clone();
    value.new_checkpoint.revision = value.new_checkpoint.revision.increment().unwrap();
    wrong.push(value);
    let mut value = original.clone();
    value.new_checkpoint.payload.size_bytes = Counter(1);
    wrong.push(value);
    let mut value = original.clone();
    value.new_checkpoint.executor_schema = name("rx.unknown.v1");
    wrong.push(value);
    let mut value = original.clone();
    value.new_checkpoint.activations.push(
        rx_application::checkpoint_artifact::ActivationSnapshot {
            id: id(),
            node: target.node.clone(),
            visit: target.visit,
            slots: vec![],
        },
    );
    wrong.push(value);
    for command in wrong {
        assert!(
            f.app
                .commit_process_checkpoint(&f.executor, id().as_str(), command)
                .is_err()
        );
    }
    report_select(&mut f, false);
    assert!(matches!(
        f.app
            .commit_process_checkpoint(&f.executor, id().as_str(), original),
        Err(StoreError::Rejected(Rejection::StaleRevision))
    ));
    assert_eq!(
        f.app.inspect_run(&f.executor, &target.run).unwrap().0,
        proposal.expected_revision
    );
    let new = prepared(&mut f, &target);
    assert_ne!(new.checkpoint.payload, proposal.checkpoint.payload);
    f.app
        .commit_process_checkpoint(
            &f.executor,
            id().as_str(),
            checkpoint_command(&target, &new),
        )
        .unwrap();
    assert!(
        !f.app
            .process_progress(&f.executor, &target.run, target.visit)
            .unwrap()
            .checkpoint
            .branches[&target.node]
            .chosen
    );
}
#[test]
fn checkpoint_wait_uses_p_time_and_success_cannot_be_committed_after_deadline() {
    let (mut f, mut target) = checkpoint_fixture(TestProcess::Wait);
    let proposal = prepared(&mut f, &target);
    let command = checkpoint_command(&target, &proposal);
    let key = id();
    f.clock.0.store(1100, Ordering::SeqCst);
    f.app
        .commit_process_checkpoint(&f.executor, key.as_str(), command.clone())
        .unwrap();
    let original_window = f
        .app
        .process_progress(&f.executor, &target.run, target.visit)
        .unwrap()
        .checkpoint
        .wait_windows[&target.node]
        .clone();
    assert_eq!(original_window.started_at.ticks_ns, Counter(1000));
    assert_eq!(original_window.expires_at.ticks_ns, Counter(1500));
    assert!(matches!(
        f.app
            .prepare_process_checkpoint(&f.executor, target.clone())
            .unwrap(),
        CheckpointPreparation::AlreadyApplied { .. }
    ));
    target.action = CheckpointAction::CheckWait;
    assert!(matches!(
        f.app
            .prepare_process_checkpoint(&f.executor, target.clone())
            .unwrap(),
        CheckpointPreparation::Waiting { .. }
    ));
    report_select(&mut f, true);
    f.clock.0.store(1400, Ordering::SeqCst);
    let success = prepared(&mut f, &target);
    f.clock.0.store(1500, Ordering::SeqCst);
    assert!(matches!(
        f.app.commit_process_checkpoint(
            &f.executor,
            id().as_str(),
            checkpoint_command(&target, &success)
        ),
        Err(StoreError::Rejected(Rejection::StaleRevision))
    ));
    let timeout = prepared(&mut f, &target);
    f.app
        .commit_process_checkpoint(
            &f.executor,
            id().as_str(),
            checkpoint_command(&target, &timeout),
        )
        .unwrap();
    let progress = f
        .app
        .process_progress(&f.executor, &target.run, target.visit)
        .unwrap();
    assert!(matches!(
        progress.checkpoint.waits[&target.node],
        rx_process_contract::frontier::WaitProgress::TimedOut { .. }
    ));
    assert_eq!(
        progress.checkpoint.wait_windows[&target.node].id,
        original_window.id
    );
    assert!(progress.frontier.operations.is_empty());
    assert_eq!(
        f.app
            .commit_process_checkpoint(&f.executor, key.as_str(), command)
            .unwrap()
            .checkpoint
            .payload,
        proposal.checkpoint.payload
    );
}

#[test]
fn checkpoint_preparation_loss_and_expiry_never_advance_the_run() {
    let (mut f, target) = checkpoint_fixture(TestProcess::Branch);
    report_select(&mut f, true);
    let before = f.app.inspect_run(&f.executor, &target.run).unwrap().0;
    f.failure.store(1, Ordering::SeqCst);
    assert!(
        f.app
            .prepare_process_checkpoint(&f.executor, target.clone())
            .is_err()
    );
    assert_eq!(
        f.app.inspect_run(&f.executor, &target.run).unwrap().0,
        before
    );
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .prepare_process_checkpoint(&f.executor, target.clone())
            .is_err()
    );
    let proposal = prepared(&mut f, &target);
    assert_eq!(
        f.app.inspect_run(&f.executor, &target.run).unwrap().0,
        before
    );
    f.clock
        .0
        .store(proposal.valid_until.ticks_ns.0, Ordering::SeqCst);
    assert!(matches!(
        f.app.commit_process_checkpoint(
            &f.executor,
            id().as_str(),
            checkpoint_command(&target, &proposal)
        ),
        Err(StoreError::Rejected(Rejection::Expired))
    ));
    assert_eq!(
        f.app.inspect_run(&f.executor, &target.run).unwrap().0,
        before
    );
}

#[test]
fn checkpoint_successor_preserves_an_existing_child_slot_from_another_visit() {
    use rx_domain::canonical::bytes;
    let mut f = fixture_with_peer(1, true, false, true, Some(TestProcess::Branch), true);
    let run = start(&mut f, 2);
    let part = f
        .app
        .begin_part(
            &f.executor,
            id().as_str(),
            &run.id,
            run.budget.as_ref().unwrap().revision(),
        )
        .unwrap();
    report_select(&mut f, true);
    let root = f.configuration.process.as_ref().unwrap().root.id.clone();
    let mut target = CheckpointTarget {
        run: run.id.clone(),
        node: root,
        visit: part.ordinal,
        action: CheckpointAction::ChooseBranch,
    };
    let proposal = prepared(&mut f, &target);
    let selected = f
        .app
        .commit_process_checkpoint(
            &f.executor,
            id().as_str(),
            checkpoint_command(&target, &proposal),
        )
        .unwrap();
    let leaf = f.configuration.steps[0].id.clone();
    let activation = f
        .app
        .resolve_activation(&f.executor, &run.id, &leaf, part.ordinal, selected.revision)
        .unwrap();
    let work = submit(&mut f, &activation, id().as_str()).unwrap();
    let current = f.app.inspect_run(&f.executor, &run.id).unwrap().1;
    let next = f
        .app
        .begin_part(
            &f.executor,
            id().as_str(),
            &run.id,
            current.budget.as_ref().unwrap().revision(),
        )
        .unwrap();
    target.visit = next.ordinal;
    let proposal = prepared(&mut f, &target);
    assert_eq!(proposal.checkpoint.activations.len(), 1);
    assert_eq!(
        proposal.checkpoint.activations[0].slots[0].operation,
        *work.operation.id()
    );
    let command = checkpoint_command(&target, &proposal);
    let mut dropped = command.clone();
    dropped.new_checkpoint.activations.clear();
    assert!(
        f.app
            .commit_process_checkpoint(&f.executor, id().as_str(), dropped)
            .is_err()
    );
    let mut replaced = command.clone();
    replaced.new_checkpoint.activations[0].slots[0].operation = id();
    assert!(
        f.app
            .commit_process_checkpoint(&f.executor, id().as_str(), replaced)
            .is_err()
    );
    let committed = f
        .app
        .commit_process_checkpoint(&f.executor, id().as_str(), command)
        .unwrap();
    assert_eq!(
        bytes(&committed.checkpoint.activations).unwrap(),
        bytes(&proposal.checkpoint.activations).unwrap()
    );
    let unchanged = f
        .app
        .inspect_work(&f.executor, work.operation.id())
        .unwrap();
    assert_eq!(bytes(&unchanged).unwrap(), bytes(&work).unwrap());
}

#[test]
fn production_completion_requires_release_and_preserves_original_reply_and_budget() {
    use rx_process_contract::production::CompletePart;
    let mut f = fixture_with_peer(1, true, false, true, None, true);
    let (work, evidence) = completed_work(&mut f);
    let part = work.part.clone().unwrap();
    let view = f.app.production_view(&f.executor, &work.run).unwrap();
    assert_eq!(view.parts.len(), 1);
    assert_eq!(view.parts[0].disposition, PartDisposition::InProgress);
    let mut command = CompletePart {
        cell: f.configuration.id.clone(),
        run: work.run.clone(),
        part: part.clone(),
        expected_run: view.run.revision,
        expected_part: view.parts[0].revision,
    };
    assert!(
        f.app
            .executor_complete_part(&f.executor, id().as_str(), command.clone())
            .is_err()
    );
    let proof = handover_proof(&mut f, &work, &evidence);
    f.app
        .release_resources(&f.hosts[0], id().as_str(), proof)
        .unwrap();
    let view = f.app.production_view(&f.executor, &work.run).unwrap();
    command.expected_run = view.run.revision;
    let key = id();
    f.failure.store(1, Ordering::SeqCst);
    assert!(
        f.app
            .executor_complete_part(&f.executor, key.as_str(), command.clone())
            .is_err()
    );
    assert_eq!(
        f.app.production_view(&f.executor, &work.run).unwrap().parts[0].disposition,
        PartDisposition::InProgress
    );
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .executor_complete_part(&f.executor, key.as_str(), command.clone())
            .is_err()
    );
    let reply = f
        .app
        .executor_complete_part(&f.executor, key.as_str(), command.clone())
        .unwrap();
    let after = f.app.production_view(&f.executor, &work.run).unwrap();
    assert_eq!(after.run.run.state, RunState::Completed);
    assert_eq!(
        after.parts[0].disposition,
        PartDisposition::ConfirmedCompleted
    );
    assert_eq!(
        after.run.run.budget.as_ref().unwrap().consumed(),
        Counter(1)
    );
    let duplicate = f
        .app
        .executor_complete_part(&f.executor, id().as_str(), command.clone())
        .unwrap();
    assert_eq!(reply.revision, duplicate.revision);
    assert_eq!(
        f.app
            .production_view(&f.executor, &work.run)
            .unwrap()
            .run
            .revision,
        after.run.revision
    );
    let mut changed = command.clone();
    changed.expected_part = changed.expected_part.increment().unwrap();
    assert!(matches!(
        f.app
            .executor_complete_part(&f.executor, key.as_str(), changed),
        Err(StoreError::KeyConflict)
    ));
    let mut revoked = principal("executor", &[Role::Executor, Role::Observer]);
    revoked.active = false;
    f.app
        .put_principal(&f.admin, revoked, Some(Counter(1)))
        .unwrap();
    assert!(matches!(
        f.app
            .executor_complete_part(&f.executor, key.as_str(), command),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
}
#[test]
fn production_read_before_first_part_has_complete_budget_and_detects_missing_parts() {
    let mut f = fixture_with_peer(1, true, false, true, None, true);
    let run = start(&mut f, 2);
    let view = f.app.production_view(&f.executor, &run.id).unwrap();
    assert!(view.parts.is_empty());
    assert!(view.admission_allowed);
    assert_eq!(
        view.run.run.budget.as_ref().unwrap().remaining(),
        Counter(2)
    );
    let part = f
        .app
        .executor_begin_part(
            &f.executor,
            id().as_str(),
            BeginPartRequest {
                cell: f.configuration.id.clone(),
                run: run.id.clone(),
                mandate: run.mandate.clone().unwrap(),
                expected_budget: view.run.run.budget.as_ref().unwrap().revision(),
                expected_cell: Some(view.cell_revision),
            },
        )
        .unwrap();
    let mut view = f.app.production_view(&f.executor, &run.id).unwrap();
    assert_eq!(view.parts[0].id, part.part.id);
    assert_eq!(view.parts[0].revision, part.revision);
    view.parts.clear();
    assert!(rx_process_contract::production::validate(&view).is_err());
}

fn case_request(
    f: &Fixture,
    kind: rx_application::intervention::CaseType,
) -> rx_application::intervention::OpenCase {
    rx_application::intervention::OpenCase {
        cell: f.configuration.id.clone(),
        expected_cell: None,
        kind,
        scopes: vec![],
        procedure: artifact(95, "rx.test.procedure.v1"),
        lead: name("lead"),
        operation_ids: vec![],
        material_ids: vec![],
    }
}
fn add_case_lead(f: &mut Fixture) {
    f.app
        .put_principal(&f.admin, principal("lead", &[Role::RecoveryLead]), None)
        .unwrap();
}
#[test]
fn physical_case_open_is_atomic_and_acknowledgment_cannot_restore_authority() {
    use rx_application::intervention::*;
    use rx_domain::canonical;
    let mut f = fixture_with_peer(1, true, false, true, None, true);
    add_case_lead(&mut f);
    let (work, _) = native_started(&mut f);
    let mut command = case_request(&f, CaseType::FaultRecovery);
    command.operation_ids.push(work.operation.id().clone());
    let before = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1;
    let key = id();
    f.failure.store(1, Ordering::SeqCst);
    assert!(
        f.app
            .open_case(&f.operator, key.as_str(), command.clone())
            .is_err()
    );
    assert!(
        f.app
            .cases(&f.operator, &f.configuration.id)
            .unwrap()
            .cases
            .is_empty()
    );
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch,
        before.epoch
    );
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .open_case(&f.operator, key.as_str(), command.clone())
            .is_err()
    );
    let opened = f
        .app
        .open_case(&f.operator, key.as_str(), command.clone())
        .unwrap();
    assert_eq!(opened.case.state, CaseState::ContainmentPending);
    let after = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1;
    assert!(after.epoch > before.epoch);
    assert!(
        after
            .blocks
            .iter()
            .any(|b| b.reason == BlockReason::CaseIntervention && b.latched)
    );
    assert_eq!(
        f.app.inspect_run(&f.executor, &work.run).unwrap().1.state,
        RunState::RecoveryRequired
    );
    assert_eq!(
        f.app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap()
            .operation
            .outcome(),
        rx_domain::operation::Outcome::None
    );
    let ack = AcknowledgeCase {
        cell: f.configuration.id.clone(),
        case: opened.case.id.clone(),
        expected_case: opened.revision,
        occurred_at: "2026-09-11T01:02:03Z".into(),
    };
    let ack_key = id();
    let result = f
        .app
        .acknowledge_case(&f.operator, ack_key.as_str(), ack.clone())
        .unwrap();
    assert_eq!(result.snapshot.case.state, CaseState::ContainmentPending);
    assert!(result.snapshot.case.participants.is_empty());
    assert_eq!(result.acknowledgments.len(), 1);
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch,
        after.epoch
    );
    assert_eq!(
        canonical::bytes(&result).unwrap(),
        canonical::bytes(
            &f.app
                .acknowledge_case(&f.operator, ack_key.as_str(), ack.clone())
                .unwrap()
        )
        .unwrap()
    );
    let mut changed = ack;
    changed.occurred_at = "2026-09-11T01:02:04Z".into();
    assert!(matches!(
        f.app
            .acknowledge_case(&f.operator, ack_key.as_str(), changed),
        Err(StoreError::KeyConflict)
    ));
    assert_eq!(
        f.app
            .cases(&f.operator, &f.configuration.id)
            .unwrap()
            .cases
            .len(),
        1
    );
}
#[test]
fn diagnostic_case_keeps_epoch_and_mandate_but_blocks_new_admission() {
    use rx_application::intervention::*;
    let mut f = fixture_with_peer(1, true, false, true, None, true);
    add_case_lead(&mut f);
    let run = start(&mut f, 2);
    let before = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1;
    let diagnostic = f
        .app
        .open_case(
            &f.operator,
            id().as_str(),
            case_request(&f, CaseType::DiagnosticOnly),
        )
        .unwrap();
    let after = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1;
    assert_eq!(after.epoch, before.epoch);
    assert!(
        after
            .blocks
            .iter()
            .any(|b| b.reason == BlockReason::CaseDiagnostic && !b.latched)
    );
    assert_eq!(
        f.app.inspect_run(&f.executor, &run.id).unwrap().1.state,
        RunState::Executing
    );
    assert!(
        f.app
            .begin_part(
                &f.executor,
                id().as_str(),
                &run.id,
                run.budget.as_ref().unwrap().revision()
            )
            .is_err()
    );
    let other = f
        .app
        .open_case(
            &f.operator,
            id().as_str(),
            case_request(&f, CaseType::PlannedAccess),
        )
        .unwrap();
    let epoch = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1
        .epoch;
    f.app
        .acknowledge_case(
            &f.operator,
            id().as_str(),
            AcknowledgeCase {
                cell: f.configuration.id.clone(),
                case: diagnostic.case.id,
                expected_case: diagnostic.revision,
                occurred_at: "2026-09-11T01:02:03+00:00".into(),
            },
        )
        .unwrap();
    let cell = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1;
    assert_eq!(cell.epoch, epoch);
    assert!(cell.open_cases.contains(&other.case.id));
    assert!(
        cell.blocks
            .iter()
            .any(|b| other.case.block_ids.contains(&b.id))
    );
}

fn content_ref<T: serde::Serialize>(schema: &str, value: &T) -> ArtifactRef {
    use sha2::Digest as _;
    let bytes = rx_domain::canonical::bytes(value).unwrap();
    ArtifactRef {
        sha256: Digest::from_bytes(sha2::Sha256::digest(&bytes).into()),
        schema_id: name(schema),
        size_bytes: Counter(bytes.len() as u64),
    }
}
fn procedure_report(
    f: &Fixture,
    case: &rx_application::intervention::CaseSnapshot,
    actor: &Identity,
    action: rx_application::procedure::Action,
    people: Vec<Name>,
) -> rx_application::procedure::Submission {
    use rx_application::procedure::*;
    let scopes = f.configuration.scopes.clone();
    let assertions = Assertions {
        schema: name("rx.procedure-assertions.v1"),
        case_id: case.case.id.clone(),
        case_revision: case.revision,
        procedure_digest: case.case.procedure.sha256,
        actor: actor.principal.clone(),
        action,
        occurred_at: "2026-09-11T02:03:04Z".into(),
        scope_ids: scopes.clone(),
        source: actor.principal.clone(),
        physical_claims: vec![Claim {
            subject: name("fixture/workpiece"),
            predicate: name("reported"),
            value: TypedValue::Boolean(true),
        }],
        source_event: Some(id()),
        step: Some(name(&format!("step/{action:?}"))),
        people,
        evidence_ids: vec![],
        observed_at: Some(expiry(1000)),
        valid_until: Some(expiry(50000)),
        certainty: Some(Certainty::Reported),
    };
    let record = rx_application::procedure::Record {
        id: id(),
        case: case.case.id.clone(),
        case_revision: case.revision,
        action,
        actor: actor.principal.clone(),
        scope_ids: scopes,
        occurred_at: assertions.occurred_at.clone(),
        evidence_ids: vec![],
        assertions: content_ref("rx.procedure-assertions.v1", &assertions),
    };
    Submission {
        cell: f.configuration.id.clone(),
        expected_cell: None,
        expected_case: case.revision,
        record,
        assertions: Some(assertions),
    }
}
fn set_report_evidence(report: &mut rx_application::procedure::Submission, evidence: Vec<Id>) {
    report.record.evidence_ids = evidence.clone();
    report.assertions.as_mut().unwrap().evidence_ids = evidence;
    report.record.assertions = content_ref(
        "rx.procedure-assertions.v1",
        report.assertions.as_ref().unwrap(),
    );
}
fn confirm_case_fences(f: &mut Fixture) {
    confirm_case_fences_from(f, 10);
}
fn confirm_case_fences_from(f: &mut Fixture, mut sequence: u64) {
    for pending in f.app.pending_deliveries(128).unwrap() {
        sequence += 1;
        if let Delivery::Fence { host, .. } = &pending.payload {
            let identity = f.hosts.iter().find(|h| &h.principal == host).unwrap();
            let plan = f.app.plan_delivery(identity, &pending.id).unwrap();
            let Delivery::Fence {
                cell,
                epoch,
                scopes,
                ..
            } = plan.payload
            else {
                panic!("fence")
            };
            f.app
                .finish_fence_delivery(
                    identity,
                    &pending.id,
                    FenceAcknowledgment {
                        cell,
                        invalidation: pending.id.clone(),
                        epoch,
                        scopes,
                        host_boot: plan.registration.boot_id,
                        journal: plan.registration.delivery_journal,
                        sequence: Counter(sequence),
                    },
                )
                .unwrap();
        }
    }
}
#[test]
fn stale_procedure_report_keeps_external_fact_and_latch_but_never_grants_entry() {
    use rx_application::{intervention::*, procedure::*};
    let mut f = fixture(1, true);
    add_case_lead(&mut f);
    let opened = f
        .app
        .open_case(
            &f.operator,
            id().as_str(),
            case_request(&f, CaseType::DiagnosticOnly),
        )
        .unwrap();
    f.app
        .acknowledge_case(
            &f.operator,
            id().as_str(),
            AcknowledgeCase {
                cell: f.configuration.id.clone(),
                case: opened.case.id.clone(),
                expected_case: opened.revision,
                occurred_at: "2026-09-11T01:02:03Z".into(),
            },
        )
        .unwrap();
    let before = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1
        .epoch;
    let report = procedure_report(
        &f,
        &opened,
        &f.operator,
        Action::WorkStarted,
        vec![f.operator.principal.clone()],
    );
    let key = id();
    f.failure.store(1, Ordering::SeqCst);
    assert!(
        f.app
            .record_procedure(&f.operator, key.as_str(), report.clone())
            .is_err()
    );
    assert!(
        f.app
            .inspect_case(&f.operator, &f.configuration.id, &opened.case.id)
            .unwrap()
            .procedure_records
            .is_empty()
    );
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .record_procedure(&f.operator, key.as_str(), report.clone())
            .is_err()
    );
    let receipt = f
        .app
        .record_procedure(&f.operator, key.as_str(), report.clone())
        .unwrap();
    assert!(receipt.facts_recorded);
    assert_eq!(receipt.transition_error, Some(Rejection::StaleRevision));
    assert_eq!(receipt.case.case.kind, CaseType::FaultRecovery);
    assert_eq!(receipt.case.case.state, CaseState::Escalated);
    assert!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch
            > before
    );
    assert_eq!(
        f.app
            .inspect_case(&f.operator, &f.configuration.id, &opened.case.id)
            .unwrap()
            .procedure_records
            .len(),
        1
    );
    let epoch = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1
        .epoch;
    f.app
        .record_procedure(&f.operator, key.as_str(), report)
        .unwrap();
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch,
        epoch
    );
}
#[test]
fn procedure_requires_real_fence_and_condition_evidence_then_accounts_for_every_worker() {
    for invalidate_entry in [false, true] {
        use rx_application::{intervention::*, procedure::*};
        let mut f = fixture(1, true);
        let lead = add_identity(
            &mut f.app,
            &f.admin,
            "lead",
            &[Role::RecoveryLead, Role::Observer],
        );
        let other = add_identity(
            &mut f.app,
            &f.admin,
            "operator2",
            &[Role::Operator, Role::Observer],
        );
        let policy = Policy {
            external_procedure: artifact(92, "rx.test.external-procedure.v1"),
            dependencies: vec![artifact(93, "rx.test.entry-function.v1")],
            schema: name("rx.procedure-policy.v1"),
            cell: f.configuration.id.clone(),
            definition: f.configuration.definition.sha256,
            envelope: f.configuration.envelope.sha256,
            case_types: vec![CaseType::PlannedAccess],
            entry_conditions: f.configuration.start_conditions.clone(),
            steps: [
                Action::EntryConditionsReported,
                Action::WorkStarted,
                Action::WorkFinished,
                Action::PersonnelAccounted,
                Action::HandoverAccepted,
            ]
            .into_iter()
            .map(|action| Step {
                id: name(&format!("step/{action:?}")),
                action,
                actors: vec![
                    lead.principal.clone(),
                    f.operator.principal.clone(),
                    other.principal.clone(),
                ],
                required_evidence: vec![],
                conditions: vec![],
            })
            .collect(),
            maximum_report_age_ns: Counter(20000),
        };
        let reference = content_ref("rx.procedure-policy.v1", &policy);
        f.app
            .admit_procedure(&f.admin, policy, reference.clone())
            .unwrap();
        let mut command = case_request(&f, CaseType::PlannedAccess);
        command.procedure = reference;
        let mut case = f
            .app
            .open_case(&f.operator, id().as_str(), command)
            .unwrap();
        let evidence = f
            .app
            .inspect_fact(&f.operator, &f.configuration.id, &name("ready"))
            .unwrap()
            .evidence_id;
        let mut report =
            procedure_report(&f, &case, &lead, Action::EntryConditionsReported, vec![]);
        set_report_evidence(&mut report, vec![evidence.clone()]);
        let denied = f
            .app
            .record_procedure(&lead, id().as_str(), report)
            .unwrap();
        assert_eq!(denied.transition_error, Some(Rejection::ConditionUnknown));
        assert_eq!(denied.case.case.state, CaseState::ContainmentPending);
        case = denied.case;
        confirm_case_fences(&mut f);
        let mut report =
            procedure_report(&f, &case, &lead, Action::EntryConditionsReported, vec![]);
        set_report_evidence(&mut report, vec![evidence]);
        let entry = f
            .app
            .record_procedure(&lead, id().as_str(), report)
            .unwrap();
        assert!(entry.transition_error.is_none());
        assert_eq!(entry.case.case.state, CaseState::ProcedureActive);
        case = entry.case;
        if invalidate_entry {
            f.app
                .hold(&f.operator, id().as_str(), &f.configuration.id)
                .unwrap();
            let report = procedure_report(
                &f,
                &case,
                &f.operator,
                Action::WorkStarted,
                vec![f.operator.principal.clone()],
            );
            let denied = f
                .app
                .record_procedure(&f.operator, id().as_str(), report)
                .unwrap();
            assert_eq!(denied.transition_error, Some(Rejection::ConditionUnknown));
            assert_eq!(denied.case.case.state, CaseState::Escalated);
            continue;
        }
        for actor in [f.operator.clone(), other.clone()] {
            let report = procedure_report(
                &f,
                &case,
                &actor,
                Action::WorkStarted,
                vec![actor.principal.clone()],
            );
            let receipt = f
                .app
                .record_procedure(&actor, id().as_str(), report)
                .unwrap();
            assert!(receipt.transition_error.is_none());
            case = receipt.case;
        }
        let report = procedure_report(
            &f,
            &case,
            &f.operator,
            Action::WorkFinished,
            vec![f.operator.principal.clone()],
        );
        case = f
            .app
            .record_procedure(&f.operator, id().as_str(), report)
            .unwrap()
            .case;
        let report = procedure_report(
            &f,
            &case,
            &lead,
            Action::PersonnelAccounted,
            vec![f.operator.principal.clone()],
        );
        let denied = f
            .app
            .record_procedure(&lead, id().as_str(), report)
            .unwrap();
        assert_eq!(denied.transition_error, Some(Rejection::ConditionUnknown));
        case = denied.case;
        let report = procedure_report(
            &f,
            &case,
            &other,
            Action::WorkFinished,
            vec![other.principal.clone()],
        );
        case = f
            .app
            .record_procedure(&other, id().as_str(), report)
            .unwrap()
            .case;
        for action in [Action::PersonnelAccounted, Action::HandoverAccepted] {
            let report = procedure_report(
                &f,
                &case,
                &lead,
                action,
                vec![f.operator.principal.clone(), other.principal.clone()],
            );
            let result = f
                .app
                .record_procedure(&lead, id().as_str(), report)
                .unwrap();
            assert!(result.transition_error.is_none());
            case = result.case;
        }
        assert_eq!(case.case.state, CaseState::Revalidating);
        assert_ne!(case.case.state, CaseState::ReadyForRestart);
        assert!(
            !f.app
                .inspect_cell(&f.operator, &f.configuration.id)
                .unwrap()
                .1
                .blocks
                .is_empty()
        );
    }
}

fn closure_fixture() -> (
    Fixture,
    Identity,
    rx_application::intervention::CaseSnapshot,
) {
    closure_fixture_mode(false)
}
fn closure_fixture_mode(
    entered: bool,
) -> (
    Fixture,
    Identity,
    rx_application::intervention::CaseSnapshot,
) {
    use rx_application::{intervention::*, procedure::*};
    let mut f = fixture(1, true);
    let mut operation = None;
    if entered {
        let run = start(&mut f, 1);
        let activation = activation(&mut f, &run);
        let work = submit(&mut f, &activation, id().as_str()).unwrap();
        let id = work.operation.id().clone();
        f.app.begin_delivery(&id).unwrap();
        operation = Some(id);
    }
    let lead = add_identity(
        &mut f.app,
        &f.admin,
        "lead",
        &[Role::RecoveryLead, Role::Observer],
    );
    let policy = Policy {
        external_procedure: artifact(92, "rx.test.external-procedure.v1"),
        dependencies: vec![artifact(93, "rx.test.entry-function.v1")],
        schema: name("rx.procedure-policy.v1"),
        cell: f.configuration.id.clone(),
        definition: f.configuration.definition.sha256,
        envelope: f.configuration.envelope.sha256,
        case_types: vec![CaseType::PlannedAccess],
        entry_conditions: f.configuration.start_conditions.clone(),
        steps: [
            Action::EntryConditionsReported,
            Action::WorkStarted,
            Action::WorkFinished,
            Action::PersonnelAccounted,
            Action::HandoverAccepted,
        ]
        .into_iter()
        .map(|action| Step {
            id: name(&format!("step/{action:?}")),
            action,
            actors: vec![lead.principal.clone(), f.operator.principal.clone()],
            required_evidence: vec![],
            conditions: vec![],
        })
        .collect(),
        maximum_report_age_ns: Counter(20000),
    };
    let reference = content_ref("rx.procedure-policy.v1", &policy);
    f.app
        .admit_procedure(&f.admin, policy, reference.clone())
        .unwrap();
    let mut command = case_request(&f, CaseType::PlannedAccess);
    command.procedure = reference.clone();
    command.operation_ids = operation.into_iter().collect();
    let mut case = f
        .app
        .open_case(&f.operator, id().as_str(), command)
        .unwrap();
    confirm_case_fences(&mut f);
    for action in [
        Action::EntryConditionsReported,
        Action::WorkStarted,
        Action::WorkFinished,
        Action::PersonnelAccounted,
        Action::HandoverAccepted,
    ] {
        let actor = if matches!(action, Action::WorkStarted | Action::WorkFinished) {
            f.operator.clone()
        } else {
            lead.clone()
        };
        let mut report = procedure_report(
            &f,
            &case,
            &actor,
            action,
            if action == Action::EntryConditionsReported {
                vec![]
            } else {
                vec![f.operator.principal.clone()]
            },
        );
        if action == Action::EntryConditionsReported {
            let fact = f
                .app
                .inspect_fact(&lead, &f.configuration.id, &name("ready"))
                .unwrap();
            set_report_evidence(&mut report, vec![fact.evidence_id]);
        }
        let receipt = f
            .app
            .record_procedure(&actor, id().as_str(), report)
            .unwrap();
        assert!(
            receipt.transition_error.is_none(),
            "{:?}",
            receipt.transition_error
        );
        case = receipt.case;
    }
    confirm_case_fences_from(&mut f, 20);
    let policy = rx_application::closure::Policy {
        schema: name("rx.close-policy.v1"),
        cell: f.configuration.id.clone(),
        procedure: reference,
        external_procedure: artifact(94, "rx.test.out-of-service-procedure.v1"),
        dependencies: vec![artifact(95, "rx.test.external-containment.v1")],
        contexts: vec![rx_application::closure::ScopePolicy {
            cell: f.configuration.id.clone(),
            definition: f.configuration.definition.sha256,
            envelope: f.configuration.envelope.sha256,
            conditions: f.configuration.start_conditions.clone(),
        }],
        maximum_validity_ns: Counter(5000),
    };
    let reference = content_ref("rx.close-policy.v1", &policy);
    f.app
        .admit_close_policy(&f.admin, policy, reference)
        .unwrap();
    (f, lead, case)
}
fn close_preparation(
    f: &mut Fixture,
    lead: &Identity,
    case: &Id,
) -> rx_application::closure::PrepareClose {
    let detail = f.app.inspect_case(lead, &f.configuration.id, case).unwrap();
    let progress = detail.procedure_progress;
    rx_application::closure::PrepareClose {
        cell: f.configuration.id.clone(),
        expected_cell: f.app.inspect_cell(lead, &f.configuration.id).unwrap().0,
        case_revisions: vec![rx_application::closure::CaseRevision {
            case: case.clone(),
            revision: detail.snapshot.revision,
        }],
        evidence_ids: vec![
            progress.personnel_record.unwrap(),
            progress.handover_record.unwrap(),
            f.app
                .inspect_fact(lead, &f.configuration.id, &name("ready"))
                .unwrap()
                .evidence_id,
        ],
    }
}
fn close_command(
    p: &rx_application::closure::PrepareClose,
    clearance: &Id,
) -> rx_application::closure::CloseWithoutRestart {
    rx_application::closure::CloseWithoutRestart {
        cell: p.cell.clone(),
        expected_cell: p.expected_cell,
        case_revisions: p.case_revisions.clone(),
        clearance: clearance.clone(),
    }
}
#[test]
fn non_operating_close_is_atomic_idempotent_and_retains_other_case_restrictions() {
    use rx_application::{closure::Disposition, intervention::*};
    for fault in [0, 1, 2] {
        let (mut f, lead, case) = closure_fixture();
        let other = f
            .app
            .open_case(
                &f.operator,
                id().as_str(),
                case_request(&f, CaseType::DiagnosticOnly),
            )
            .unwrap();
        let prep = close_preparation(&mut f, &lead, &case.case.id);
        let prep_key = id();
        let clearance = f
            .app
            .prepare_close(&lead, prep_key.as_str(), prep.clone())
            .unwrap();
        assert_eq!(clearance.disposition, Disposition::RemainOutOfService);
        assert_eq!(clearance.valid_until, expiry(6000));
        assert_eq!(clearance.consumed_by, None);
        assert_eq!(
            f.app
                .inspect_case(&lead, &prep.cell, &case.case.id)
                .unwrap()
                .snapshot
                .case
                .state,
            CaseState::Revalidating
        );
        let command = close_command(&prep, &clearance.id);
        let close_key = id();
        f.failure.store(fault, Ordering::SeqCst);
        let result = f
            .app
            .close_without_restart(&lead, close_key.as_str(), command.clone());
        if fault != 0 {
            assert!(matches!(result, Err(StoreError::Unavailable(_))));
        }
        if fault == 1 {
            assert_eq!(
                f.app
                    .inspect_case(&lead, &prep.cell, &case.case.id)
                    .unwrap()
                    .snapshot
                    .case
                    .state,
                CaseState::Revalidating
            );
            assert!(
                !f.app
                    .inspect_cell(&lead, &prep.cell)
                    .unwrap()
                    .1
                    .blocks
                    .iter()
                    .any(|b| b.reason == BlockReason::OutOfService)
            );
        }
        let receipt = f
            .app
            .close_without_restart(&lead, close_key.as_str(), command.clone())
            .unwrap();
        assert_eq!(
            f.app
                .close_without_restart(&lead, close_key.as_str(), command.clone())
                .unwrap()
                .id,
            receipt.id
        );
        let cell = f.app.inspect_cell(&lead, &prep.cell).unwrap().1;
        assert_eq!(
            cell.epoch.0,
            clearance.contexts[0].reference.cell_epoch.0 + 1
        );
        assert_eq!(
            cell.blocks
                .iter()
                .filter(|b| b.reason == BlockReason::OutOfService && b.latched)
                .count(),
            1
        );
        assert!(
            other
                .case
                .block_ids
                .iter()
                .all(|id| cell.blocks.iter().any(|b| &b.id == id))
        );
        assert!(!cell.open_cases.contains(&case.case.id));
        assert!(cell.open_cases.contains(&other.case.id));
        assert_eq!(
            f.app
                .inspect_case(&lead, &prep.cell, &case.case.id)
                .unwrap()
                .snapshot
                .case
                .state,
            CaseState::Closed
        );
        assert_eq!(
            f.app
                .inspect_case(&lead, &prep.cell, &other.case.id)
                .unwrap()
                .snapshot
                .case
                .state,
            CaseState::Open
        );
        // Original prepare receipt remains historical and cannot become a fresh clearance.
        assert_eq!(
            f.app
                .prepare_close(&lead, prep_key.as_str(), prep.clone())
                .unwrap()
                .id,
            clearance.id
        );
        let mut second = command.clone();
        second.expected_cell = f.app.inspect_cell(&lead, &prep.cell).unwrap().0;
        assert!(
            f.app
                .close_without_restart(&lead, id().as_str(), second)
                .is_err()
        );
        let mut different = command.clone();
        different.clearance = id();
        assert!(matches!(
            f.app
                .close_without_restart(&lead, close_key.as_str(), different),
            Err(StoreError::KeyConflict)
        ));
        assert!(
            f.app
                .pending_deliveries(128)
                .unwrap()
                .iter()
                .all(|p| !matches!(p.payload, Delivery::Arm { .. }))
        );
        let records = f.app.into_repository().control_snapshot().unwrap().1;
        let clearances: Vec<_> = records
            .iter()
            .filter(|r| r.document.schema.as_str() == "rx.internal.clearance.v1")
            .collect();
        assert_eq!(clearances.len(), 1);
        let stored: rx_application::closure::Clearance =
            rx_application::persistence::decode(clearances[0], "rx.internal.clearance.v1").unwrap();
        assert_eq!(stored.consumed_by, Some(receipt.id));
    }
}
#[test]
fn non_operating_close_rechecks_time_case_epoch_evidence_and_current_actor() {
    use rx_application::intervention::*;
    for change in [
        "expiry", "case", "epoch", "evidence", "role", "cohort", "false", "source", "scope",
    ] {
        let (mut f, lead, case) = closure_fixture();
        let prep = close_preparation(&mut f, &lead, &case.case.id);
        let key = id();
        let clearance = f
            .app
            .prepare_close(&lead, key.as_str(), prep.clone())
            .unwrap();
        let mut command = close_command(&prep, &clearance.id);
        match change {
            "expiry" => {
                f.clock.0.store(6000, Ordering::SeqCst);
            }
            "case" => {
                f.app
                    .acknowledge_case(
                        &lead,
                        id().as_str(),
                        AcknowledgeCase {
                            cell: prep.cell.clone(),
                            case: case.case.id.clone(),
                            expected_case: case.revision,
                            occurred_at: "2026-09-11T02:03:05Z".into(),
                        },
                    )
                    .unwrap();
            }
            "epoch" => {
                f.app.hold(&f.operator, id().as_str(), &prep.cell).unwrap();
                command.expected_cell = f.app.inspect_cell(&lead, &prep.cell).unwrap().0;
            }
            "evidence" => {
                report_ready(
                    &mut f.app,
                    &f.hosts[0],
                    &f.configuration,
                    &f.registrations[0],
                );
            }
            "role" => {
                f.app
                    .put_principal(
                        &f.admin,
                        principal("lead", &[Role::Observer]),
                        Some(Counter(1)),
                    )
                    .unwrap();
                assert!(
                    f.app
                        .prepare_close(&lead, key.as_str(), prep.clone())
                        .is_err()
                );
            }
            "cohort" => {
                command.case_revisions[0].revision = Counter(1);
            }
            "false" | "source" => {
                let mut fact = f
                    .app
                    .inspect_fact(&lead, &prep.cell, &name("ready"))
                    .unwrap();
                fact.evidence_id = id();
                if change == "false" {
                    fact.value = TypedValue::Boolean(false);
                } else {
                    fact.source_generation = id();
                }
                let result = f.app.report_fact(&f.hosts[0], fact);
                if change == "false" {
                    result.unwrap();
                } else {
                    assert!(result.is_err());
                }
                command.expected_cell = f.app.inspect_cell(&lead, &prep.cell).unwrap().0;
            }
            "scope" => {
                let config = f.configuration.clone();
                let hosts = f.registrations.clone();
                add_cell_b(&mut f, true);
                f.configuration = config;
                f.registrations = hosts;
            }
            _ => unreachable!(),
        }
        assert!(
            f.app
                .close_without_restart(&lead, id().as_str(), command)
                .is_err(),
            "{change}"
        );
        assert_ne!(
            f.app
                .inspect_case(&f.operator, &prep.cell, &case.case.id)
                .unwrap()
                .snapshot
                .case
                .state,
            CaseState::Closed
        );
        assert!(
            !f.app
                .inspect_cell(&f.operator, &prep.cell)
                .unwrap()
                .1
                .blocks
                .iter()
                .any(|b| b.reason == BlockReason::OutOfService)
        );
    }
}
#[test]
fn non_operating_close_does_not_accept_missing_people_proof_or_invented_evidence() {
    for change in ["missing", "invented", "duplicate", "new_work"] {
        let (mut f, lead, case) = closure_fixture();
        let mut prep = close_preparation(&mut f, &lead, &case.case.id);
        match change {
            "missing" => {
                prep.evidence_ids.pop();
            }
            "invented" => {
                prep.evidence_ids.push(id());
            }
            "duplicate" => {
                prep.evidence_ids.push(prep.evidence_ids[0].clone());
            }
            "new_work" => {
                let report = procedure_report(
                    &f,
                    &case,
                    &f.operator,
                    rx_application::procedure::Action::WorkStarted,
                    vec![f.operator.principal.clone()],
                );
                let receipt = f
                    .app
                    .record_procedure(&f.operator, id().as_str(), report)
                    .unwrap();
                let detail = f
                    .app
                    .inspect_case(&lead, &prep.cell, &case.case.id)
                    .unwrap();
                assert!(detail.procedure_progress.personnel_record.is_none());
                assert!(detail.procedure_progress.handover_record.is_none());
                assert!(!detail.procedure_progress.people[&f.operator.principal].accounted);
                prep.expected_cell = f.app.inspect_cell(&lead, &prep.cell).unwrap().0;
                prep.case_revisions[0].revision = receipt.case.revision;
            }
            _ => unreachable!(),
        }
        assert!(
            f.app.prepare_close(&lead, id().as_str(), prep).is_err(),
            "{change}"
        );
    }
}

#[test]
fn non_operating_close_preserves_unknown_effect_and_does_not_release_resources() {
    let (mut f, lead, case) = closure_fixture_mode(true);
    let operation = &case.case.operation_ids[0];
    let before = f.app.inspect_work(&f.operator, operation).unwrap();
    assert_eq!(
        before.operation.knowledge(),
        rx_domain::operation::Knowledge::Unknown
    );
    let prep = close_preparation(&mut f, &lead, &case.case.id);
    let clearance = f
        .app
        .prepare_close(&lead, id().as_str(), prep.clone())
        .unwrap();
    f.app
        .close_without_restart(&lead, id().as_str(), close_command(&prep, &clearance.id))
        .unwrap();
    let after = f.app.inspect_work(&f.operator, operation).unwrap();
    assert_eq!(before.operation.knowledge(), after.operation.knowledge());
    assert_eq!(before.operation.outcome(), after.operation.outcome());
    assert_eq!(
        before.operation.disposition(),
        after.operation.disposition()
    );
    assert_eq!(
        f.app.inspect_run(&f.operator, &after.run).unwrap().1.state,
        RunState::RecoveryRequired
    );
    let records = f.app.into_repository().snapshot().unwrap().1;
    let reservations: Vec<_> = records
        .iter()
        .filter(|r| r.document.schema.as_str() == "rx.internal.resource.v1")
        .collect();
    assert!(!reservations.is_empty());
    assert!(reservations.iter().any(|r| {
        let resource: Resource =
            rx_application::persistence::decode(r, "rx.internal.resource.v1").unwrap();
        resource.holder.as_ref() == Some(operation) && resource.quarantined
    }));
}
#[test]
fn non_operating_close_preparation_lost_reply_and_rollback_do_not_duplicate_clearances() {
    for failure in [1, 2] {
        let (mut f, lead, case) = closure_fixture();
        let prep = close_preparation(&mut f, &lead, &case.case.id);
        let key = id();
        f.failure.store(failure, Ordering::SeqCst);
        assert!(matches!(
            f.app.prepare_close(&lead, key.as_str(), prep.clone()),
            Err(StoreError::Unavailable(_))
        ));
        let first = f
            .app
            .prepare_close(&lead, key.as_str(), prep.clone())
            .unwrap();
        assert_eq!(
            first.id,
            f.app.prepare_close(&lead, key.as_str(), prep).unwrap().id
        );
        let rows = f.app.into_repository().snapshot().unwrap().1;
        assert_eq!(
            rows.iter()
                .filter(|r| r.document.schema.as_str() == "rx.internal.clearance.v1")
                .count(),
            1
        );
    }
}

#[test]
fn closed_case_duplicate_reports_never_replay_but_new_physical_reports_reopen_containment() {
    use rx_application::{intervention::CaseState, procedure::*};
    let (mut f, lead, case) = closure_fixture();
    let detail = f
        .app
        .inspect_case(&lead, &f.configuration.id, &case.case.id)
        .unwrap();
    let original = detail
        .procedure_records
        .iter()
        .find(|r| r.record.action == Action::WorkStarted)
        .unwrap()
        .clone();
    let prep = close_preparation(&mut f, &lead, &case.case.id);
    let clearance = f
        .app
        .prepare_close(&lead, id().as_str(), prep.clone())
        .unwrap();
    f.app
        .close_without_restart(&lead, id().as_str(), close_command(&prep, &clearance.id))
        .unwrap();
    let closed = f
        .app
        .inspect_case(&lead, &prep.cell, &case.case.id)
        .unwrap()
        .snapshot;
    let before = f.app.inspect_cell(&lead, &prep.cell).unwrap();
    let duplicate = Submission {
        cell: prep.cell.clone(),
        expected_cell: None,
        expected_case: original.record.case_revision,
        record: original.record,
        assertions: Some(original.assertions),
    };
    let result = f
        .app
        .record_procedure(&f.operator, id().as_str(), duplicate)
        .unwrap();
    assert!(result.facts_recorded);
    assert_eq!(result.transition_error, Some(Rejection::StaleRevision));
    assert_eq!(result.case.revision, closed.revision);
    assert_eq!(result.case.case.state, CaseState::Closed);
    let after = f.app.inspect_cell(&lead, &prep.cell).unwrap();
    assert_eq!(before.0, after.0);
    assert_eq!(before.1.epoch, after.1.epoch);
    // A new nonphysical attestation is historical reporting; it cannot reopen workflow.
    let report = procedure_report(
        &f,
        &closed,
        &lead,
        Action::PersonnelAccounted,
        vec![f.operator.principal.clone()],
    );
    let receipt = f
        .app
        .record_procedure(&lead, id().as_str(), report)
        .unwrap();
    assert_eq!(receipt.case.case.state, CaseState::Closed);
    assert_eq!(receipt.transition_error, Some(Rejection::BlockedByCase));
    // New physical change must be preserved and restore case membership and restrictions.
    let report = procedure_report(
        &f,
        &receipt.case,
        &f.operator,
        Action::WorkStarted,
        vec![f.operator.principal.clone()],
    );
    let receipt = f
        .app
        .record_procedure(&f.operator, id().as_str(), report)
        .unwrap();
    assert!(receipt.facts_recorded);
    assert_eq!(receipt.transition_error, Some(Rejection::BlockedByCase));
    assert_eq!(receipt.case.case.state, CaseState::Escalated);
    let cell = f.app.inspect_cell(&lead, &prep.cell).unwrap().1;
    assert!(cell.open_cases.contains(&case.case.id));
    assert!(
        cell.blocks
            .iter()
            .any(|b| b.reason == BlockReason::OutOfService && b.latched)
    );
    assert!(cell.epoch > before.1.epoch);
    let progress = f
        .app
        .inspect_case(&lead, &prep.cell, &case.case.id)
        .unwrap()
        .procedure_progress;
    assert!(progress.people[&f.operator.principal].active);
    assert!(progress.personnel_record.is_none());
}

#[test]
fn recorded_cell_context_tracks_start_intervention_and_real_block_creation_revisions() {
    let mut f = fixture(1, false);
    let (_, cell) = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    assert_eq!(cell.mode, Some(OperatingMode::Setup));
    assert_eq!(cell.commissioning, Some(Commissioning::NotCommissioned));
    qualify_cell(&mut f.app, &f.admin, &f.configuration);
    let _run = start(&mut f, 1);
    let (before, cell) = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    assert_eq!(cell.mode, Some(OperatingMode::Automatic));
    assert_eq!(cell.commissioning, Some(Commissioning::Commissioned));
    add_case_lead(&mut f);
    let case = f
        .app
        .open_case(
            &f.operator,
            id().as_str(),
            case_request(&f, rx_application::intervention::CaseType::Maintenance),
        )
        .unwrap();
    let (revision, cell) = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    assert_eq!(cell.mode, Some(OperatingMode::Maintenance));
    assert_eq!(
        cell.commissioning,
        Some(Commissioning::RevalidationRequired)
    );
    for block in &cell.blocks {
        assert_eq!(block.created_revision, Some(Counter(before.0 + 1)));
        assert_eq!(block.case_id, Some(case.case.id.clone()));
        assert!(block.created_revision.unwrap() < revision);
    }
    let recorded = cell.blocks[0].created_revision;
    f.app
        .acknowledge_case(
            &f.operator,
            id().as_str(),
            rx_application::intervention::AcknowledgeCase {
                cell: f.configuration.id.clone(),
                case: case.case.id,
                expected_case: case.revision,
                occurred_at: "2026-09-11T02:03:04Z".into(),
            },
        )
        .unwrap();
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .blocks[0]
            .created_revision,
        recorded
    );
}
#[test]
fn operator_service_reconnect_cannot_start_work_or_replace_executor_authority() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 1);
    let before = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    let mut p = principal("operator-api", &[Role::OperatorApi, Role::Observer]);
    p.cells = [f.configuration.id.clone()].into_iter().collect();
    f.app.put_principal(&f.admin, p.clone(), None).unwrap();
    assert!(
        f.app
            .authenticated_user_session(&p.id, id(), Counter(1000))
            .is_err()
    );
    let boot = id();
    let binding = Digest::from_bytes([113; 32]);
    let session = f
        .app
        .open_operator_peer(&p.id, boot.clone(), binding)
        .unwrap();
    let peer = Identity {
        principal: p.id.clone(),
        session: session.id.clone(),
        terminal: None,
    };
    assert!(f.app.inspect_peer_cell(&peer, &f.configuration.id).is_err());
    f.app
        .negotiate_operator_cell(&peer, f.configuration.definition.sha256)
        .unwrap();
    assert!(f.app.inspect_peer_cell(&peer, &f.configuration.id).is_ok());
    assert!(f.app.inspect_peer_cell(&peer, &name("cell/b")).is_err());
    assert!(
        f.app
            .hold(&peer, id().as_str(), &f.configuration.id)
            .is_err()
    );
    assert!(f.app.open_executor_peer(&p.id, id(), binding).is_err());
    assert_eq!(
        session.id,
        f.app
            .open_operator_peer(&p.id, boot.clone(), binding)
            .unwrap()
            .id
    );
    let next = f.app.open_operator_peer(&p.id, id(), binding).unwrap();
    assert_ne!(next.id, session.id);
    assert!(f.app.inspect_peer_cell(&peer, &f.configuration.id).is_err());
    assert!(f.app.open_operator_peer(&p.id, boot, binding).is_err());
    let next = Identity {
        principal: p.id.clone(),
        session: next.id,
        terminal: None,
    };
    assert!(f.app.inspect_peer_cell(&next, &f.configuration.id).is_err());
    f.app
        .negotiate_operator_cell(&next, f.configuration.definition.sha256)
        .unwrap();
    p.roles.insert(Role::RecoveryLead);
    f.app
        .put_principal(&f.admin, p.clone(), Some(Counter(1)))
        .unwrap();
    assert!(f.app.inspect_peer_cell(&next, &f.configuration.id).is_err());
    assert!(f.app.open_operator_peer(&p.id, id(), binding).is_err());
    let after = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    assert_eq!(before.0, after.0);
    assert_eq!(before.1.epoch, after.1.epoch);
    assert_eq!(
        f.app.inspect_run(&f.operator, &run.id).unwrap().1.state,
        RunState::Executing
    );
}

#[test]
fn cell_operating_context_rejects_competing_active_purposes() {
    let mut f = fixture(1, true);
    let _active = start(&mut f, 1);
    let revision = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .0;
    let next = f
        .app
        .create_run(&f.operator, id().as_str(), create_command(&f, revision))
        .unwrap();
    let mut command = start_command(&f, &next, revision, 1);
    command.purpose = Purpose::Setup;
    command.budget_unit = rx_domain::budget::BudgetUnit::OperationCount;
    assert!(matches!(
        f.app.start_run(&f.operator, id().as_str(), command),
        Err(StoreError::Rejected(Rejection::Busy))
    ));
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .mode,
        Some(OperatingMode::Automatic)
    );
}

#[test]
fn user_session_requires_its_issued_terminal_binding_and_registry_revision() {
    let mut f = fixture(1, true);
    let unbound = f
        .app
        .authenticated_user_session(&f.operator.principal, id(), Counter(99000))
        .unwrap();
    let forged = Identity {
        principal: f.operator.principal.clone(),
        session: unbound.id,
        terminal: f.operator.terminal.clone(),
    };
    assert!(matches!(
        f.app.inspect_cell(&forged, &f.configuration.id),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    let original = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    let same = Terminal {
        id: name("panel/main"),
        certificate_digest: Digest::from_bytes([77; 32]),
        cells: [name("cell/a"), name("cell/b")].into_iter().collect(),
        active: true,
    };
    assert_eq!(
        f.app
            .put_terminal(&f.admin, same.clone(), Some(Counter(1)))
            .unwrap(),
        Counter(1)
    );
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .0,
        original.0
    );
    let mut restricted = same;
    restricted.cells = [name("cell/a")].into_iter().collect();
    f.app
        .put_terminal(&f.admin, restricted, Some(Counter(1)))
        .unwrap();
    assert!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .is_err()
    );
    let login = f
        .app
        .authenticated_terminal_user_session(
            &f.operator.principal,
            id(),
            Counter(99000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap();
    let current = Identity {
        principal: f.operator.principal.clone(),
        session: login.id,
        terminal: f.operator.terminal.clone(),
    };
    assert_eq!(
        f.app.session_profile(&current).unwrap().cells,
        [name("cell/a")].into_iter().collect()
    );
    let (_, cell) = f.app.inspect_cell(&current, &f.configuration.id).unwrap();
    assert!(cell.epoch > original.1.epoch);
    assert!(
        cell.blocks
            .iter()
            .any(|b| b.reason == BlockReason::AuthorityRevoked)
    );
}

#[test]
fn service_roles_cannot_impersonate_human_procedure_reporters_or_case_leads() {
    let mut f = fixture(1, true);
    add_case_lead(&mut f);
    let case = f
        .app
        .open_case(
            &f.operator,
            id().as_str(),
            case_request(&f, rx_application::intervention::CaseType::PlannedAccess),
        )
        .unwrap();
    let service = add_identity(
        &mut f.app,
        &f.admin,
        "mixed-service",
        &[Role::Executor, Role::Operator, Role::RecoveryLead],
    );
    let report = procedure_report(
        &f,
        &case,
        &service,
        rx_application::procedure::Action::WorkStarted,
        vec![service.principal.clone()],
    );
    assert!(matches!(
        f.app.record_procedure(&service, id().as_str(), report),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    let mut open = case_request(&f, rx_application::intervention::CaseType::PlannedAccess);
    open.lead = service.principal;
    assert!(f.app.open_case(&f.operator, id().as_str(), open).is_err());
    assert_eq!(
        f.app
            .inspect_case(&f.operator, &f.configuration.id, &case.case.id)
            .unwrap()
            .snapshot
            .revision,
        case.revision
    );
}

#[test]
fn runtime_stop_barrier_is_atomic_blocks_new_authority_and_keeps_unknown_work() {
    use rx_application::lifecycle::{Attention, Phase};
    for failure in [1, 2] {
        let mut f = fixture(1, true);
        let run = start(&mut f, 1);
        let activation = activation(&mut f, &run);
        let work = submit(&mut f, &activation, id().as_str()).unwrap();
        f.app.begin_delivery(work.operation.id()).unwrap();
        let before = f
            .app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap();
        f.failure.store(failure, Ordering::SeqCst);
        assert!(matches!(
            f.app.request_runtime_stop(),
            Err(StoreError::Unavailable(_))
        ));
        if failure == 1 {
            assert_eq!(f.app.runtime_lifecycle().unwrap().phase, Phase::Serving);
            assert_eq!(
                f.app
                    .inspect_cell(&f.operator, &f.configuration.id)
                    .unwrap()
                    .1
                    .epoch,
                before.1.epoch
            );
        }
        let first = f.app.request_runtime_stop().unwrap();
        let second = f.app.request_runtime_stop().unwrap();
        assert_eq!(first.lifecycle.stop_id, second.lifecycle.stop_id);
        assert_eq!(first.lifecycle.phase, Phase::StopRequested);
        assert!(first.attention.iter().any(
            |a| matches!(a,Attention::WorkRetained{operation} if operation==work.operation.id())
        ));
        let after = f
            .app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap();
        assert_eq!(after.1.epoch.0, before.1.epoch.0 + 1);
        let create = create_command(&f, after.0);
        assert!(matches!(
            f.app.create_run(&f.operator, id().as_str(), create),
            Err(StoreError::Rejected(Rejection::Busy))
        ));
        let unknown = f
            .app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap();
        assert_eq!(
            unknown.operation.knowledge(),
            rx_domain::operation::Knowledge::Unknown
        );
        let stopped = f.app.commit_runtime_process_stop().unwrap();
        assert_eq!(stopped.lifecycle.phase, Phase::StopCommitted);
        assert!(!stopped.physical_shutdown_assessed);
        assert!(stopped.attention_count.0 > 0);
        assert_eq!(
            f.app
                .inspect_work(&f.operator, work.operation.id())
                .unwrap()
                .operation
                .outcome(),
            unknown.operation.outcome()
        );
        assert_eq!(
            f.app.runtime_lifecycle().unwrap().phase,
            Phase::StopCommitted
        );
    }
}

fn link_fixture() -> (Fixture, rx_application::host_link::Prepare) {
    use rx_domain::host_snapshot::*;
    let mut f = fixture_registration(1, true, false, false, None, false, false);
    let boot = id();
    let journal = id();
    let session = f
        .app
        .open_evidence_producer(
            &name("host/0"),
            boot.clone(),
            journal.clone(),
            Digest::from_bytes([81; 32]),
        )
        .unwrap();
    f.hosts[0].session = session.id;
    f.app
        .negotiate_evidence_cell(&f.hosts[0], f.configuration.definition.sha256)
        .unwrap();
    let source = SourceObservation {
        source: name("ready"),
        generation: id(),
        schema: name("boolean/v1"),
        unit: name("unitless"),
        value: TypedValue::Boolean(true),
        acquired_at: f.clock.now(),
        uncertainty_ns: Counter(0),
        quality_good: true,
        origin_age_bounded: true,
        disputed: false,
        evidence_id: id(),
    };
    let snapshot = HostSnapshot {
        schema: name("rx.host-snapshot.v1"),
        host: name("host/0"),
        host_boot: boot,
        delivery_journal: id(),
        evidence_journal: journal,
        cell: f.configuration.id.clone(),
        definition: f.configuration.definition.sha256,
        envelope: f.configuration.envelope.sha256,
        environment: name("SIMULATION"),
        epoch: Counter(1),
        scopes: f
            .configuration
            .scopes
            .iter()
            .map(|s| (s.clone(), Counter(1)))
            .collect(),
        resource_fences: f.configuration.steps[0]
            .intent
            .resource_set
            .iter()
            .map(|r| (r.clone(), Counter(0)))
            .collect(),
        captured_at: f.clock.now(),
        sources_available: true,
        observations: vec![source],
        block_ids: vec![],
        pending_operations: vec![],
        pending_permits: vec![],
    };
    let prepare = rx_application::host_link::Prepare {
        host: name("host/0"),
        platform_session: id(),
        snapshot,
        read_started: f.clock.now(),
        ttl_ms: Counter(1000),
    };
    (f, prepare)
}
fn link_commit(
    f: &Fixture,
    p: &rx_application::host_link::Plan,
) -> rx_application::host_link::Commit {
    rx_application::host_link::Commit {
        plan: p.id.clone(),
        fence_receipt: FenceAcknowledgment {
            cell: p.cell.clone(),
            invalidation: p.fence_request.clone(),
            epoch: p.epoch,
            scopes: p.scopes.clone(),
            host_boot: p.host_boot.clone(),
            journal: p.delivery_journal.clone(),
            sequence: Counter(1),
        },
        grant: Grant {
            id: id(),
            fence: p.fence,
            resources: p.resources.clone(),
            owner: name(f.app.installation.id.as_str()),
            ttl_ms: p.ttl_ms,
            valid_until: expiry(1_000_001_000),
        },
        grant_sent_at: f.clock.now(),
    }
}
#[test]
fn host_bootstrap_plan_and_registration_are_durable_and_do_not_fabricate_readiness() {
    for fault in [1, 2] {
        let (mut f, input) = link_fixture();
        f.failure.store(fault, Ordering::SeqCst);
        assert!(matches!(
            f.app.prepare_host_link(input.clone()),
            Err(StoreError::Unavailable(_))
        ));
        let plan = f.app.prepare_host_link(input.clone()).unwrap();
        assert_eq!(f.app.prepare_host_link(input).unwrap().id, plan.id);
        let request = link_commit(&f, &plan);
        f.failure.store(fault, Ordering::SeqCst);
        assert!(matches!(
            f.app.commit_host_link(request.clone()),
            Err(StoreError::Unavailable(_))
        ));
        let registered = f.app.commit_host_link(request.clone()).unwrap();
        assert_eq!(registered.grant.id, request.grant.id);
        assert_eq!(
            f.app.commit_host_link(request.clone()).unwrap().grant.id,
            registered.grant.id
        );
        let mut changed = request;
        changed.grant.id = id();
        assert!(matches!(
            f.app.commit_host_link(changed),
            Err(StoreError::KeyConflict)
        ));
        assert!(
            f.app
                .inspect_fact(&f.operator, &f.configuration.id, &name("ready"))
                .is_err()
        );
        assert!(
            f.app
                .pending_deliveries(128)
                .unwrap()
                .iter()
                .all(|p| !matches!(p.payload, Delivery::Arm { .. } | Delivery::Prepare { .. }))
        );
    }
}
#[test]
fn host_bootstrap_rejects_wrong_incarnation_missing_sources_and_changed_context() {
    for kind in ["boot", "journal", "source", "ahead", "pending", "epoch"] {
        let (mut f, mut input) = link_fixture();
        if kind == "epoch" {
            let plan = f.app.prepare_host_link(input).unwrap();
            let request = link_commit(&f, &plan);
            f.app
                .hold(&f.operator, id().as_str(), &f.configuration.id)
                .unwrap();
            assert!(f.app.commit_host_link(request).is_err());
            continue;
        }
        match kind {
            "boot" => input.snapshot.host_boot = id(),
            "journal" => input.snapshot.evidence_journal = id(),
            "source" => input.snapshot.observations.clear(),
            "ahead" => input.snapshot.epoch = Counter(2),
            "pending" => input.snapshot.pending_operations.push(id()),
            _ => unreachable!(),
        }
        assert!(f.app.prepare_host_link(input).is_err(), "{kind}");
    }
}
#[test]
fn host_bootstrap_never_reuses_a_changed_delivery_journal_in_the_same_boot() {
    for bound in [false, true] {
        for same_session in [false, true] {
            let (mut f, input) = link_fixture();
            let plan = f.app.prepare_host_link(input.clone()).unwrap();
            if bound {
                let request = link_commit(&f, &plan);
                f.app.commit_host_link(request).unwrap();
            }
            let mut changed = input.clone();
            changed.snapshot.delivery_journal = id();
            if !same_session {
                changed.platform_session = id();
            }
            assert!(matches!(
                f.app.prepare_host_link(changed),
                Err(StoreError::Rejected(
                    rx_domain::fault::Rejection::ContinuityUnproven
                ))
            ));
            // Rejection does not replace the durable plan or its original requests.
            let preserved = f.app.prepare_host_link(input).unwrap();
            assert_eq!(preserved.id, plan.id);
            assert_eq!(preserved.grant_request, plan.grant_request);
            assert_eq!(preserved.delivery_journal, plan.delivery_journal);
        }
    }
}
#[test]
fn renewal_uses_durable_sequence_and_original_send_time_without_replaying_permission() {
    let (mut f, input) = link_fixture();
    let plan = f.app.prepare_host_link(input).unwrap();
    let command = link_commit(&f, &plan);
    f.app.commit_host_link(command).unwrap();
    f.clock.0.store(500_001_000, Ordering::SeqCst);
    let renewal = f.app.prepare_host_renewal(&plan.id).unwrap();
    assert_eq!(
        renewal.request,
        f.app.prepare_host_renewal(&plan.id).unwrap().request
    );
    let mut grant = renewal.grant.clone();
    grant.valid_until = expiry(1_500_001_000);
    f.failure.store(2, Ordering::SeqCst);
    assert!(matches!(
        f.app.commit_host_renewal(renewal.clone(), grant.clone()),
        Err(StoreError::Unavailable(_))
    ));
    assert_eq!(
        f.app
            .commit_host_renewal(renewal.clone(), grant.clone())
            .unwrap()
            .grant
            .valid_until,
        grant.valid_until
    );
    let second = f.app.prepare_host_renewal(&plan.id).unwrap();
    assert_eq!(second.sequence, Counter(2));
    f.operator.session = f
        .app
        .authenticated_terminal_user_session(
            &f.operator.principal,
            id(),
            Counter(2_000_000_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap()
        .id;
    f.app
        .hold(&f.operator, id().as_str(), &f.configuration.id)
        .unwrap();
    assert!(f.app.commit_host_renewal(second, grant).is_err());
}

fn two_source_fixture() -> (Fixture, Vec<FactRecord>) {
    let mut f = fixture_configured((1, true, true, false, None, false, true), |mut c| {
        let mut spec = c.fact_specs[0].clone();
        spec.id = name("ready/second");
        c.fact_specs.push(spec);
        c.maintained_conditions = vec![Condition::Any {
            children: vec![
                c.start_conditions[0].clone(),
                Condition::Eq {
                    fact: name("ready/second"),
                    schema: name("boolean/v1"),
                    unit: name("unitless"),
                    expected: TypedValue::Boolean(true),
                },
            ],
        }];
        c
    });
    let first = f
        .app
        .inspect_fact(&f.operator, &f.configuration.id, &name("ready"))
        .unwrap();
    let mut second = first.clone();
    second.id = name("ready/second");
    second.source_generation = f.registrations[0].source_sessions[&second.id].clone();
    second.value = TypedValue::Boolean(false);
    second.evidence_id = id();
    f.app.report_fact(&f.hosts[0], second.clone()).unwrap();
    (f, vec![first, second])
}
#[test]
fn observation_batch_evaluates_the_complete_cut_and_recovers_commit_reply_loss() {
    use rx_application::observation::Disposition;
    for fault in [1, 2] {
        let (mut f, mut facts) = two_source_fixture();
        let run = start(&mut f, 1);
        facts[0].value = TypedValue::Boolean(false);
        facts[1].value = TypedValue::Boolean(true);
        for fact in &mut facts {
            fact.evidence_id = id();
        }
        f.failure.store(fault, Ordering::SeqCst);
        assert!(matches!(
            f.app.report_facts(&f.hosts[0], facts.clone()),
            Err(StoreError::Unavailable(_))
        ));
        if fault == 1 {
            assert_eq!(
                f.app
                    .inspect_fact(&f.operator, &f.configuration.id, &facts[0].id)
                    .unwrap()
                    .value,
                TypedValue::Boolean(true)
            );
            assert_eq!(
                f.app
                    .inspect_fact(&f.operator, &f.configuration.id, &facts[1].id)
                    .unwrap()
                    .value,
                TypedValue::Boolean(false)
            );
        }
        let receipt = f.app.report_facts(&f.hosts[0], facts.clone()).unwrap();
        assert!(receipt.maintained_revoked.is_empty());
        assert_eq!(
            f.app.inspect_run(&f.operator, &run.id).unwrap().1.state,
            RunState::Executing
        );
        for fact in &facts {
            assert_eq!(
                f.app
                    .inspect_fact(&f.operator, &f.configuration.id, &fact.id)
                    .unwrap()
                    .evidence_id,
                fact.evidence_id
            );
        }
        let repeated = f.app.report_facts(&f.hosts[0], facts).unwrap();
        assert!(
            repeated
                .entries
                .iter()
                .all(|e| e.disposition == Disposition::Duplicate)
        );
    }
}
#[test]
fn observation_batch_rejects_bad_last_source_before_writing_any_fact() {
    let (mut f, mut facts) = two_source_fixture();
    let before = facts.clone();
    for fact in &mut facts {
        fact.evidence_id = id();
        fact.value = TypedValue::Boolean(false);
    }
    facts[1].unit = name("millimetres");
    assert!(f.app.report_facts(&f.hosts[0], facts).is_err());
    for fact in before {
        assert_eq!(
            f.app
                .inspect_fact(&f.operator, &f.configuration.id, &fact.id)
                .unwrap()
                .evidence_id,
            fact.evidence_id
        );
    }
}
#[test]
fn observation_batch_keeps_conflict_evidence_and_other_source_truth_without_repeated_revocation() {
    use rx_application::observation::Disposition;
    let (mut f, mut facts) = two_source_fixture();
    let run = start(&mut f, 1);
    let original = facts[0].clone();
    facts[0].value = TypedValue::Boolean(false); // same evidence ID, different body
    facts[1].evidence_id = id();
    facts[1].value = TypedValue::Boolean(false);
    let receipt = f.app.report_facts(&f.hosts[0], facts.clone()).unwrap();
    assert_eq!(
        receipt.entries[0].disposition,
        Disposition::IntegrityConflict
    );
    assert_eq!(receipt.entries[1].disposition, Disposition::Current);
    assert_eq!(
        f.app
            .inspect_fact(&f.operator, &f.configuration.id, &original.id)
            .unwrap()
            .value,
        original.value
    );
    assert_eq!(
        f.app
            .inspect_fact(&f.operator, &f.configuration.id, &facts[1].id)
            .unwrap()
            .evidence_id,
        facts[1].evidence_id
    );
    assert_eq!(
        f.app.inspect_run(&f.operator, &run.id).unwrap().1.state,
        RunState::RecoveryRequired
    );
    let epoch = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1
        .epoch;
    f.app.report_facts(&f.hosts[0], facts).unwrap();
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch,
        epoch
    );
}
#[test]
fn observation_generation_loss_is_latched_once_and_does_not_adopt_the_new_source() {
    use rx_application::observation::Disposition;
    let (mut f, mut facts) = two_source_fixture();
    let run = start(&mut f, 1);
    facts[0].source_generation = id();
    facts[0].evidence_id = id();
    let receipt = f.app.report_facts(&f.hosts[0], facts.clone()).unwrap();
    assert_eq!(
        receipt.entries[0].disposition,
        Disposition::GenerationChanged
    );
    assert!(
        !f.app
            .inspect_fact(&f.operator, &f.configuration.id, &facts[0].id)
            .unwrap()
            .quality_good
    );
    let epoch = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1
        .epoch;
    facts[0].evidence_id = id();
    f.app.report_facts(&f.hosts[0], facts).unwrap();
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch,
        epoch
    );
    assert_eq!(
        f.app.inspect_run(&f.operator, &run.id).unwrap().1.state,
        RunState::RecoveryRequired
    );
}
#[test]
fn maintained_watchdog_revokes_expired_facts_without_fabricating_a_negative_sample() {
    let mut f = fixture_mode(1, true, true);
    let run = start(&mut f, 1);
    let original = f
        .app
        .inspect_fact(&f.operator, &f.configuration.id, &name("ready"))
        .unwrap();
    f.clock.0.store(21000, Ordering::SeqCst);
    assert!(f.app.check_maintained_conditions().unwrap().is_empty());
    f.clock.0.store(21001, Ordering::SeqCst);
    f.failure.store(1, Ordering::SeqCst);
    assert!(f.app.check_maintained_conditions().is_err());
    assert_eq!(
        f.app.inspect_run(&f.operator, &run.id).unwrap().1.state,
        RunState::Executing
    );
    assert_eq!(
        f.app.check_maintained_conditions().unwrap(),
        vec![f.configuration.id.clone()]
    );
    let recorded = f
        .app
        .inspect_fact(&f.operator, &f.configuration.id, &name("ready"))
        .unwrap();
    assert_eq!(recorded.evidence_id, original.evidence_id);
    assert_eq!(recorded.value, TypedValue::Boolean(true));
    assert!(recorded.quality_good);
    let epoch = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1
        .epoch;
    assert!(f.app.check_maintained_conditions().unwrap().is_empty());
    let mut fresh = original;
    fresh.acquired_at = f.clock.now();
    fresh.evidence_id = id();
    f.app.report_fact(&f.hosts[0], fresh).unwrap();
    assert_eq!(
        f.app.inspect_run(&f.operator, &run.id).unwrap().1.state,
        RunState::RecoveryRequired
    );
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch,
        epoch
    );
}
#[test]
fn host_read_ingestion_checks_context_and_preserves_the_native_source_fields() {
    use rx_application::observation::{Disposition, HostRead};
    for mutation in [
        "none",
        "delayed",
        "boot",
        "journal",
        "definition",
        "missing",
        "future",
        "unavailable",
    ] {
        let (mut f, input) = link_fixture();
        let plan = f.app.prepare_host_link(input.clone()).unwrap();
        f.app.commit_host_link(link_commit(&f, &plan)).unwrap();
        let mut snapshot = input.snapshot;
        match mutation {
            "none" => {}
            "delayed" => {
                f.clock.0.store(200_001_000, Ordering::SeqCst);
            }
            "boot" => snapshot.host_boot = id(),
            "journal" => snapshot.delivery_journal = id(),
            "definition" => snapshot.definition = Digest::from_bytes([99; 32]),
            "missing" => snapshot.observations.clear(),
            "future" => snapshot.observations[0].acquired_at.ticks_ns = Counter(2000),
            "unavailable" => {
                snapshot.sources_available = false;
                snapshot.observations.clear();
            }
            _ => unreachable!(),
        }
        let raw = snapshot.observations.first().cloned();
        let result = f.app.ingest_host_read(HostRead {
            plan: plan.id,
            snapshot,
            read_started: input.read_started,
        });
        if matches!(mutation, "none" | "delayed") {
            assert_eq!(result.unwrap().entries[0].disposition, Disposition::Current);
            let fact = f
                .app
                .inspect_fact(&f.hosts[0], &f.configuration.id, &name("ready"))
                .unwrap();
            let raw = raw.unwrap();
            assert_eq!(fact.source_generation, raw.generation);
            assert_eq!(fact.acquired_at, raw.acquired_at);
            assert_eq!(fact.evidence_id, raw.evidence_id);
            assert!(
                f.app
                    .pending_deliveries(128)
                    .unwrap()
                    .iter()
                    .all(|d| !matches!(d.payload, Delivery::Arm { .. } | Delivery::Prepare { .. }))
            );
        } else {
            assert!(result.is_err(), "{mutation}");
            assert!(
                f.app
                    .inspect_fact(&f.hosts[0], &f.configuration.id, &name("ready"))
                    .is_err()
            );
        }
    }
}

#[test]
fn repeated_disputed_samples_do_not_create_unbounded_latches_but_a_new_episode_does() {
    let (mut f, mut facts) = two_source_fixture();
    let run = start(&mut f, 1);
    facts[0].disputed = true;
    facts[0].evidence_id = id();
    f.app.report_facts(&f.hosts[0], facts.clone()).unwrap();
    let epoch = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1
        .epoch;
    facts[0].evidence_id = id();
    f.app.report_facts(&f.hosts[0], facts.clone()).unwrap();
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch,
        epoch
    );
    facts[0].disputed = false;
    facts[0].evidence_id = id();
    f.app.report_facts(&f.hosts[0], facts.clone()).unwrap();
    assert_eq!(
        f.app.inspect_run(&f.operator, &run.id).unwrap().1.state,
        RunState::RecoveryRequired
    );
    facts[0].disputed = true;
    facts[0].evidence_id = id();
    f.app.report_facts(&f.hosts[0], facts).unwrap();
    assert!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch
            > epoch
    );
}

#[test]
fn operator_diagnostics_share_the_condition_evaluator_and_do_not_change_authority() {
    use rx_application::diagnostics::{HostContext, SourceIssue};
    let (mut f, mut facts) = two_source_fixture();
    let before = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    let view = f.app.overview(&f.operator).unwrap();
    let d = &view.cells[0].diagnostics;
    assert!(matches!(d.hosts[0].context, HostContext::Current));
    assert_eq!(d.conditions[0].verdict, rx_domain::condition::Verdict::Pass);
    assert_eq!(
        d.sources[0].observation.as_ref().unwrap().evidence_id,
        facts[0].evidence_id
    );
    assert!(d.sources.iter().all(|s| s.usable));
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .0,
        before.0
    );
    facts[0].value = TypedValue::Boolean(false);
    facts[0].evidence_id = id();
    f.app.report_facts(&f.hosts[0], facts.clone()).unwrap();
    let view = f.app.overview(&f.operator).unwrap();
    assert_eq!(
        view.cells[0].diagnostics.conditions[0].verdict,
        rx_domain::condition::Verdict::Fail
    );
    assert!(view.cells[0].diagnostics.sources[0].usable); // a usable false observation is distinct from unavailable data
    f.clock.0.store(21001, Ordering::SeqCst);
    let view = f.app.overview(&f.operator).unwrap();
    let d = &view.cells[0].diagnostics;
    assert_eq!(
        d.conditions[0].verdict,
        rx_domain::condition::Verdict::Unknown
    );
    assert!(d.sources[0].issues.contains(&SourceIssue::AgeExceeded));
    assert!(!d.sources[0].usable);
    assert_eq!(
        d.sources[0].observation.as_ref().unwrap().value,
        TypedValue::Boolean(false)
    );
}
#[test]
fn operator_diagnostics_distinguish_missing_generation_and_expression_mismatch() {
    use rx_application::diagnostics::{ConditionReason, SourceIssue};
    let (mut f, _) = link_fixture();
    let view = f.app.overview(&f.operator).unwrap();
    assert!(
        view.cells[0].diagnostics.sources[0]
            .issues
            .contains(&SourceIssue::MissingObservation)
    );
    let mut f = fixture_configured((1, false, false, false, None, false, true), |mut c| {
        c.start_conditions = vec![Condition::Range {
            fact: name("ready"),
            schema: name("boolean/v1"),
            unit: name("unitless"),
            min: Real::new(0.0).unwrap(),
            max: Real::new(1.0).unwrap(),
        }];
        c
    });
    let view = f.app.overview(&f.operator).unwrap();
    let d = &view.cells[0].diagnostics;
    assert!(d.sources[0].usable);
    assert!(matches!(
        d.conditions[0].reason,
        ConditionReason::ExpressionMismatch
    ));
    let mut fact = f
        .app
        .inspect_fact(&f.operator, &f.configuration.id, &name("ready"))
        .unwrap();
    fact.source_generation = id();
    fact.evidence_id = id();
    assert!(f.app.report_fact(&f.hosts[0], fact).is_err());
    let view = f.app.overview(&f.operator).unwrap();
    assert!(
        view.cells[0].diagnostics.sources[0]
            .issues
            .contains(&SourceIssue::GenerationMismatch)
    );
    assert!(!view.cells[0].diagnostics.sources[0].usable);
}
#[test]
fn operator_diagnostic_browser_fixtures_are_real_unqualified_read_models() {
    use rx_domain::condition::Verdict;
    let mut f = fixture_configured((1, false, true, false, None, false, false), |mut c| {
        c.fact_specs[0].maximum_age_ns = Counter(1_000_000_000);
        c
    });
    f.operator.session = f
        .app
        .authenticated_terminal_user_session(
            &f.operator.principal,
            id(),
            Counter(10_000_000_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap()
        .id;
    f.hosts[0].session = f
        .app
        .authenticated_session(&f.hosts[0].principal, id(), expiry(10_000_000_000))
        .unwrap()
        .id;
    let (_, cell) = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    let registration = HostRegistration {
        id: f.hosts[0].principal.clone(),
        session: f.hosts[0].session.clone(),
        boot_id: id(),
        delivery_journal: id(),
        cell: f.configuration.id.clone(),
        epoch: cell.epoch,
        scopes: cell.scope_epochs,
        source_sessions: [(name("ready"), id())].into_iter().collect(),
        grant: Grant {
            id: id(),
            fence: Counter(1),
            resources: vec![name("controller/0")],
            owner: name(f.app.installation.id.as_str()),
            valid_until: expiry(5_000_000_000),
            ttl_ms: Counter(5000),
        },
    };
    f.app
        .register_host(&f.hosts[0], registration.clone())
        .unwrap();
    let mut fact = FactRecord {
        cell: f.configuration.id.clone(),
        id: name("ready"),
        source_host: f.hosts[0].principal.clone(),
        source_generation: registration.source_sessions[&name("ready")].clone(),
        schema: name("boolean/v1"),
        unit: name("unitless"),
        acquired_at: f.clock.now(),
        maximum_age_ns: Counter(1_000_000_000),
        acquisition_uncertainty_ns: Counter(0),
        quality_good: true,
        origin_age_bounded: true,
        disputed: false,
        value: TypedValue::Boolean(true),
        evidence_id: id(),
    };
    f.app.report_fact(&f.hosts[0], fact.clone()).unwrap();
    let pass = f.app.overview(&f.operator).unwrap();
    assert_eq!(
        pass.cells[0].diagnostics.conditions[0].verdict,
        Verdict::Pass
    );
    assert!(pass.cells[0].cell.value.qualification.is_none());
    assert_eq!(
        pass.cells[0].diagnostics.display_valid_for_ns,
        Counter(1_000_000_000)
    );
    fact.value = TypedValue::Boolean(false);
    fact.evidence_id = id();
    f.app.report_fact(&f.hosts[0], fact).unwrap();
    let fail = f.app.overview(&f.operator).unwrap();
    assert_eq!(
        fail.cells[0].diagnostics.conditions[0].verdict,
        Verdict::Fail
    );
    f.clock.0.store(1_000_001_001, Ordering::SeqCst);
    let expired = f.app.overview(&f.operator).unwrap();
    assert_eq!(
        expired.cells[0].diagnostics.conditions[0].verdict,
        Verdict::Unknown
    );
    if let Ok(directory) = std::env::var("RX_DIAGNOSTIC_FIXTURES_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir(&directory).unwrap();
        for (label, view) in [("pass", pass), ("fail", fail), ("expired", expired)] {
            std::fs::write(
                directory.join(format!("{label}.json")),
                rx_domain::canonical::bytes(&view).unwrap(),
            )
            .unwrap();
        }
    }
}

fn draft_document() -> serde_json::Value {
    serde_json::json!({"schema":"rx.process-source.v1","process":"example/draft","entry":"main","conditions":{},"flows":[{"id":"main","root":"load","nodes":[{"id":"load","body":{"kind":"OPERATION","binding":"load"}}]}]})
}
#[test]
fn draft_saves_are_atomic_recoverable_and_never_change_the_installed_cell() {
    use rx_application::process_draft::{PreparedSave, Save};
    for failure in [1, 2] {
        let mut f = fixture(1, true);
        let before = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap();
        let key = id();
        let draft = id();
        let save = Save {
            id: draft.clone(),
            cell: f.configuration.id.clone(),
            expected: None,
            title: "Material supply draft".into(),
            document: draft_document(),
        };
        f.failure.store(failure, Ordering::SeqCst);
        assert!(
            f.app
                .save_process_draft(&f.admin, &key, PreparedSave::prepare(save.clone()).unwrap())
                .is_err()
        );
        if failure == 1 {
            assert!(
                f.app
                    .process_draft(&f.admin, &f.configuration.id, &draft, None)
                    .is_err()
            );
        }
        let stored = f
            .app
            .save_process_draft(&f.admin, &key, PreparedSave::prepare(save.clone()).unwrap())
            .unwrap();
        assert_eq!(stored.version.revision, Counter(1));
        assert!(stored.version.validation.structurally_valid);
        assert_eq!(
            stored.version.validation.required_bindings,
            vec![name("load")]
        );
        let recovered = f
            .app
            .save_process_draft(&f.admin, &key, PreparedSave::prepare(save.clone()).unwrap())
            .unwrap();
        assert_eq!(recovered.version.revision, Counter(1));
        assert_eq!(
            f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap().0,
            before.0
        );
        assert!(f.app.pending_deliveries(128).unwrap().is_empty());
        let mut changed = save;
        changed.title = "another title".into();
        assert!(matches!(
            f.app
                .save_process_draft(&f.admin, &key, PreparedSave::prepare(changed).unwrap()),
            Err(StoreError::KeyConflict)
        ));
    }
}
#[test]
fn draft_history_keeps_incomplete_sources_and_concurrent_updates_do_not_overwrite_them() {
    use rx_application::process_draft::{PreparedSave, Save};
    let mut f = fixture(1, false);
    let draft = id();
    let save = Save {
        id: draft.clone(),
        cell: f.configuration.id.clone(),
        expected: None,
        title: "draft".into(),
        document: draft_document(),
    };
    f.app
        .save_process_draft(
            &f.admin,
            &id(),
            PreparedSave::prepare(save.clone()).unwrap(),
        )
        .unwrap();
    let mut changed = save.clone();
    changed.expected = Some(Counter(1));
    changed.document["flows"][0]["nodes"][0]["body"] =
        serde_json::json!({"kind":"SEQUENCE","children":[]});
    let second = f
        .app
        .save_process_draft(
            &f.admin,
            &id(),
            PreparedSave::prepare(changed.clone()).unwrap(),
        )
        .unwrap();
    assert_eq!(second.version.revision, Counter(2));
    assert!(!second.version.validation.structurally_valid);
    assert!(matches!(
        f.app
            .save_process_draft(&f.admin, &id(), PreparedSave::prepare(changed).unwrap()),
        Err(StoreError::Rejected(
            rx_domain::fault::Rejection::StaleRevision
        ))
    ));
    let first = f
        .app
        .process_draft(&f.admin, &f.configuration.id, &draft, Some(Counter(1)))
        .unwrap();
    assert_eq!(first.document, save.document);
    assert_eq!(
        f.app
            .process_draft(&f.admin, &f.configuration.id, &draft, None)
            .unwrap()
            .version
            .revision,
        Counter(2)
    );
    let page = f
        .app
        .process_drafts(&f.admin, &f.configuration.id, None)
        .unwrap();
    assert_eq!(page.drafts.len(), 1);
    assert!(!page.drafts[0].structurally_valid);
    assert!(
        f.app
            .process_draft(&f.operator, &f.configuration.id, &draft, None)
            .is_err()
    );
}
#[test]
fn current_engineer_role_is_required_even_when_recovering_an_existing_draft_receipt() {
    use rx_application::process_draft::{PreparedSave, Save};
    let mut f = fixture(1, false);
    let key = id();
    let save = Save {
        id: id(),
        cell: f.configuration.id.clone(),
        expected: None,
        title: "draft".into(),
        document: draft_document(),
    };
    f.app
        .save_process_draft(&f.admin, &key, PreparedSave::prepare(save.clone()).unwrap())
        .unwrap();
    f.app
        .put_principal(
            &f.admin,
            principal("admin", &[Role::AccountAdmin, Role::Observer]),
            Some(Counter(1)),
        )
        .unwrap();
    assert!(matches!(
        f.app
            .save_process_draft(&f.admin, &key, PreparedSave::prepare(save).unwrap()),
        Err(StoreError::Rejected(rx_domain::fault::Rejection::Forbidden))
    ));
}

fn binding_draft(f: &mut Fixture) -> rx_application::process_draft::Detail {
    f.app
        .save_process_draft(
            &f.admin,
            &id(),
            rx_application::process_draft::PreparedSave::prepare(
                rx_application::process_draft::Save {
                    id: id(),
                    cell: f.configuration.id.clone(),
                    expected: None,
                    title: "binding draft".into(),
                    document: draft_document(),
                },
            )
            .unwrap(),
        )
        .unwrap()
}
fn binding_command(
    f: &mut Fixture,
    d: &rx_application::process_draft::Detail,
) -> rx_application::draft_bindings::Save {
    let catalog = f
        .app
        .draft_binding_catalog(&f.admin, &f.configuration.id)
        .unwrap();
    rx_application::draft_bindings::Save {
        device_plans: vec![],
        draft: d.version.id.clone(),
        cell: f.configuration.id.clone(),
        source_revision: d.version.revision,
        expected: None,
        catalog_digest: catalog.catalog_digest,
        selections: [(name("load"), f.configuration.steps[0].id.clone())]
            .into_iter()
            .collect(),
    }
}
#[test]
fn draft_binding_commit_loss_and_input_conflict_preserve_one_resolved_snapshot() {
    for fault in [1, 2] {
        let mut f = fixture(1, false);
        let d = binding_draft(&mut f);
        let command = binding_command(&mut f, &d);
        let key = id();
        let cell = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap().0;
        f.failure.store(fault, Ordering::SeqCst);
        assert!(
            f.app
                .save_draft_bindings(&f.admin, &key, command.clone())
                .is_err()
        );
        let saved = f
            .app
            .save_draft_bindings(&f.admin, &key, command.clone())
            .unwrap();
        assert_eq!(saved.revision, Counter(1));
        assert!(saved.complete);
        assert_eq!(
            saved.resolved[&name("load")].intent.digest().unwrap(),
            f.configuration.steps[0].intent.digest().unwrap()
        );
        assert_eq!(
            f.app
                .save_draft_bindings(&f.admin, &key, command.clone())
                .unwrap()
                .revision,
            Counter(1)
        );
        let mut changed = command;
        changed.selections.clear();
        assert!(matches!(
            f.app.save_draft_bindings(&f.admin, &key, changed),
            Err(StoreError::KeyConflict)
        ));
        assert_eq!(
            f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap().0,
            cell
        );
        assert!(f.app.pending_deliveries(128).unwrap().is_empty());
    }
}
#[test]
fn draft_binding_selection_requires_current_source_catalog_and_real_registered_step() {
    let mut f = fixture(1, false);
    let d = binding_draft(&mut f);
    let input = binding_command(&mut f, &d);
    for mode in ["catalog", "source", "binding", "step"] {
        let mut bad = input.clone();
        match mode {
            "catalog" => bad.catalog_digest = Digest::from_bytes([99; 32]),
            "source" => bad.source_revision = Counter(2),
            "binding" => {
                bad.selections
                    .insert(name("not-in-source"), f.configuration.steps[0].id.clone());
            }
            "step" => {
                bad.selections
                    .insert(name("load"), name("unregistered-step"));
            }
            _ => unreachable!(),
        }
        assert!(
            f.app.save_draft_bindings(&f.admin, &id(), bad).is_err(),
            "{mode}"
        );
    }
    let mut incomplete = input.clone();
    incomplete.selections.clear();
    let saved = f
        .app
        .save_draft_bindings(&f.admin, &id(), incomplete)
        .unwrap();
    assert!(!saved.complete);
    assert_eq!(saved.missing, vec![name("load")]);
    assert!(
        f.app
            .draft_compile_input(
                &f.admin,
                &f.configuration.id,
                &d.version.id,
                Counter(1),
                Counter(1)
            )
            .is_err()
    );
    let mut full = input;
    full.expected = Some(Counter(1));
    let saved = f.app.save_draft_bindings(&f.admin, &id(), full).unwrap();
    assert_eq!(saved.revision, Counter(2));
    let bundle = f
        .app
        .draft_compile_input(
            &f.admin,
            &f.configuration.id,
            &d.version.id,
            Counter(1),
            Counter(2),
        )
        .unwrap();
    assert_eq!(bundle.validate().unwrap().process, name("example/draft"));
}
#[test]
fn binding_history_is_not_rebound_by_source_changes_and_title_only_changes_keep_the_same_content() {
    use rx_application::{
        draft_bindings::StaleReason,
        process_draft::{PreparedSave, Save},
    };
    let mut f = fixture(1, false);
    let d = binding_draft(&mut f);
    let input = binding_command(&mut f, &d);
    f.app.save_draft_bindings(&f.admin, &id(), input).unwrap();
    let mut save = Save {
        id: d.version.id.clone(),
        cell: f.configuration.id.clone(),
        expected: Some(Counter(1)),
        title: "renamed".into(),
        document: d.document.clone(),
    };
    f.app
        .save_process_draft(
            &f.admin,
            &id(),
            PreparedSave::prepare(save.clone()).unwrap(),
        )
        .unwrap();
    assert!(
        f.app
            .draft_bindings(&f.admin, &f.configuration.id, &d.version.id, None)
            .unwrap()
            .stale
            .is_empty()
    );
    save.expected = Some(Counter(2));
    save.document["process"] = serde_json::json!("changed-process");
    f.app
        .save_process_draft(&f.admin, &id(), PreparedSave::prepare(save).unwrap())
        .unwrap();
    let view = f
        .app
        .draft_bindings(&f.admin, &f.configuration.id, &d.version.id, None)
        .unwrap();
    assert_eq!(view.stale, vec![StaleReason::SourceChanged]);
    assert_eq!(view.binding.unwrap().source_revision, Counter(1));
    assert!(
        f.app
            .draft_compile_input(
                &f.admin,
                &f.configuration.id,
                &d.version.id,
                Counter(3),
                Counter(1)
            )
            .is_err()
    );
}
#[test]
fn binding_catalog_change_survives_runtime_restart_and_cannot_be_exported_as_current() {
    use rx_application::{draft_bindings::StaleReason, persistence};
    let mut f = fixture(1, false);
    let d = binding_draft(&mut f);
    let input = binding_command(&mut f, &d);
    f.app.save_draft_bindings(&f.admin, &id(), input).unwrap();
    let installation = f.app.installation.id.clone();
    let cell = f.configuration.id.clone();
    let mut repo = f.app.into_repository();
    // Explicit fixture of a changed registered configuration; not an implementation of release activation.
    repo.transact(|tx| {
        let key = persistence::key("cell", &cell);
        let row = tx.get(&key)?.unwrap();
        let mut c: Cell = persistence::decode(&row, "rx.internal.cell.v1")?;
        c.configuration.steps[0].intent.execution_timeout_ms = Counter(9000);
        tx.put(
            &key,
            Some(row.revision),
            &persistence::doc("rx.internal.cell.v1", &c)?,
        )?;
        Ok(())
    })
    .unwrap();
    let mut app = Engine::open(
        repo,
        f.clock,
        SimulationAuthority,
        installation,
        principal(
            "admin",
            &[Role::AccountAdmin, Role::Engineer, Role::Observer],
        ),
    )
    .unwrap();
    let session = app
        .authenticated_session(&name("admin"), id(), expiry(100000))
        .unwrap();
    let actor = Identity {
        principal: name("admin"),
        session: session.id,
        terminal: None,
    };
    let view = app
        .draft_bindings(&actor, &cell, &d.version.id, None)
        .unwrap();
    assert!(view.stale.contains(&StaleReason::CatalogChanged));
    assert!(
        app.draft_compile_input(&actor, &cell, &d.version.id, Counter(1), Counter(1))
            .is_err()
    );
}
#[test]
fn binding_receipt_recovery_still_requires_current_engineer_role() {
    let mut f = fixture(1, false);
    let d = binding_draft(&mut f);
    let command = binding_command(&mut f, &d);
    let key = id();
    f.app
        .save_draft_bindings(&f.admin, &key, command.clone())
        .unwrap();
    f.app
        .put_principal(
            &f.admin,
            principal("admin", &[Role::AccountAdmin, Role::Observer]),
            Some(Counter(1)),
        )
        .unwrap();
    assert!(matches!(
        f.app.save_draft_bindings(&f.admin, &key, command),
        Err(StoreError::Rejected(rx_domain::fault::Rejection::Forbidden))
    ));
}

#[path = "support/device_binding.rs"]
mod device_binding_tests;
#[path = "support/device_catalog.rs"]
mod device_catalog_tests;
#[path = "support/device_review.rs"]
mod device_review_tests;
#[path = "support/package_intake.rs"]
mod intake_support;
fn intake_input(f: &mut Fixture, p: &intake_support::Fixture) -> package_intake::Submit {
    assert!(p.import_root.is_dir());
    f.app
        .configure_package_intake(Some((
            p.store.owner().clone(),
            p.policy.fingerprint().unwrap(),
            rx_package::content_digest(&p.policy_bytes),
        )))
        .unwrap();
    let context = f
        .app
        .package_intake_context(&f.admin, &f.configuration.id)
        .unwrap();
    package_intake::Submit {
        id: id(),
        cell: f.configuration.id.clone(),
        title: "Signed material-supply package".into(),
        relative_path: rx_package::PackagePath::new("test-package").unwrap(),
        object: p.object.clone(),
        configuration_digest: context.configuration_digest,
        policy_generation: context.registration.unwrap().generation,
    }
}
fn intake_prepared(
    f: &mut Fixture,
    p: &intake_support::Fixture,
    key: &Id,
    input: package_intake::Submit,
) -> package_intake::Prepared {
    let package_intake::Preflight::Verify(ticket) =
        f.app.prepare_package_intake(&f.admin, key, input).unwrap()
    else {
        panic!("new ticket required")
    };
    package_intake::Prepared::new(*ticket, p.store.verify_owned(&p.object, &p.policy).unwrap())
        .unwrap()
}
#[test]
fn package_intake_commits_receipt_event_and_request_atomically_without_cell_change() {
    for fault in [1, 2] {
        let mut f = fixture(1, true);
        let p = intake_support::fixture();
        let input = intake_input(&mut f, &p);
        let key = id();
        let before = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap();
        let prepared = intake_prepared(&mut f, &p, &key, input.clone());
        f.failure.store(fault, Ordering::SeqCst);
        assert!(f.app.commit_package_intake(prepared).is_err());
        let list = f.app.package_intakes(&f.admin, &input.cell, None).unwrap();
        assert_eq!(list.packages.len(), if fault == 1 { 0 } else { 1 });
        let receipt = match f
            .app
            .prepare_package_intake(&f.admin, &key, input.clone())
            .unwrap()
        {
            package_intake::Preflight::Recorded(v) => *v,
            package_intake::Preflight::Verify(t) => f
                .app
                .commit_package_intake(
                    package_intake::Prepared::new(
                        *t,
                        p.store.verify_owned(&p.object, &p.policy).unwrap(),
                    )
                    .unwrap(),
                )
                .unwrap(),
        };
        assert_eq!(receipt.state, package_intake::State::AwaitingReview);
        let package_intake::Preflight::Recorded(recovered) = f
            .app
            .prepare_package_intake(&f.admin, &key, input.clone())
            .unwrap()
        else {
            panic!("original result")
        };
        assert_eq!(recovered.id, receipt.id);
        assert_eq!(
            f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap().0,
            before.0
        );
        assert!(f.app.pending_deliveries(128).unwrap().is_empty());
        let view = f
            .app
            .package_intake(&f.admin, &input.cell, &input.id)
            .unwrap();
        assert!(
            view.review_context_current
                && view.content_reverification_required
                && !view.activation_authorized
        );
        let mut changed = input;
        changed.title = "Different body".into();
        assert!(matches!(
            f.app.prepare_package_intake(&f.admin, &key, changed),
            Err(StoreError::KeyConflict)
        ));
        assert_eq!(
            f.app
                .into_repository()
                .events_after(Counter(0), 128)
                .unwrap()
                .iter()
                .filter(|e| e.document.schema.as_str() == "rx.event.package-intake-submitted.v1")
                .count(),
            1
        );
    }
}
#[test]
fn package_intake_rechecks_authority_policy_and_time_after_file_work() {
    for mode in 0..4 {
        let mut f = fixture(1, true);
        f.admin.session = f
            .app
            .authenticated_session(&f.admin.principal, id(), expiry(u64::MAX))
            .unwrap()
            .id;
        let p = intake_support::fixture();
        let input = intake_input(&mut f, &p);
        let prepared = intake_prepared(&mut f, &p, &id(), input.clone());
        match mode {
            0 => {
                f.app.configure_package_intake(None).unwrap();
            }
            1 => {
                f.app
                    .configure_package_intake(Some((
                        p.store.owner().clone(),
                        p.policy.fingerprint().unwrap(),
                        rx_package::content_digest(&p.policy_bytes),
                    )))
                    .unwrap();
            }
            2 => {
                f.clock.0.store(31_000_000_000, Ordering::SeqCst);
            }
            _ => {
                f.app.request_runtime_stop().unwrap();
            }
        }
        assert!(f.app.commit_package_intake(prepared).is_err());
        assert!(
            f.app
                .package_intakes(&f.admin, &input.cell, None)
                .unwrap()
                .packages
                .is_empty()
        );
        p.store.verify(&p.object, &p.policy).unwrap();
    }
}
#[test]
fn package_intake_rejects_unregistered_store_or_different_verification_policy() {
    let mut f = fixture(1, true);
    let p = intake_support::fixture();
    let other = intake_support::fixture();
    let input = intake_input(&mut f, &p);
    let package_intake::Preflight::Verify(ticket) = f
        .app
        .prepare_package_intake(&f.admin, &id(), input.clone())
        .unwrap()
    else {
        panic!("ticket")
    };
    assert!(
        package_intake::Prepared::new(
            *ticket,
            other
                .store
                .verify_owned(&other.object, &other.policy)
                .unwrap()
        )
        .is_err()
    );
    let package_intake::Preflight::Verify(ticket) = f
        .app
        .prepare_package_intake(&f.admin, &id(), input)
        .unwrap()
    else {
        panic!("ticket")
    };
    let mut weaker = p.policy.clone();
    weaker.max_content_bytes += 1;
    assert!(
        package_intake::Prepared::new(*ticket, p.store.verify_owned(&p.object, &weaker).unwrap())
            .is_err()
    );
}
#[test]
fn package_intake_receipt_is_historical_after_policy_disable_and_current_role_revocation_applies_first()
 {
    let mut f = fixture(1, true);
    let p = intake_support::fixture();
    let input = intake_input(&mut f, &p);
    let key = id();
    let prepared = intake_prepared(&mut f, &p, &key, input.clone());
    f.app.commit_package_intake(prepared).unwrap();
    f.app.configure_package_intake(None).unwrap();
    let view = f
        .app
        .package_intake(&f.admin, &input.cell, &input.id)
        .unwrap();
    assert!(
        !view.review_context_current
            && view.content_reverification_required
            && !view.activation_authorized
    );
    assert!(matches!(
        f.app
            .prepare_package_intake(&f.admin, &key, input.clone())
            .unwrap(),
        package_intake::Preflight::Recorded(_)
    ));
    f.app
        .put_principal(
            &f.admin,
            principal("admin", &[Role::AccountAdmin, Role::Observer]),
            Some(Counter(1)),
        )
        .unwrap();
    assert!(matches!(
        f.app.prepare_package_intake(&f.admin, &key, input.clone()),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    assert!(
        f.app
            .package_intake(&f.admin, &input.cell, &input.id)
            .is_err()
    );
}

#[test]
fn package_intake_commit_rejects_role_or_terminal_revoked_while_worker_runs() {
    for terminal_revoked in [false, true] {
        let mut f = fixture(1, true);
        let p = intake_support::fixture();
        let input = intake_input(&mut f, &p);
        let identity = if terminal_revoked {
            let s = f
                .app
                .authenticated_terminal_user_session(
                    &f.admin.principal,
                    id(),
                    Counter(99_000),
                    Digest::from_bytes([77; 32]),
                )
                .unwrap();
            Identity {
                principal: f.admin.principal.clone(),
                session: s.id,
                terminal: Some((name("panel/main"), Digest::from_bytes([77; 32]))),
            }
        } else {
            f.admin.clone()
        };
        let package_intake::Preflight::Verify(ticket) = f
            .app
            .prepare_package_intake(&identity, &id(), input.clone())
            .unwrap()
        else {
            panic!("ticket")
        };
        let prepared = package_intake::Prepared::new(
            *ticket,
            p.store.verify_owned(&p.object, &p.policy).unwrap(),
        )
        .unwrap();
        if terminal_revoked {
            f.app
                .put_terminal(
                    &f.admin,
                    Terminal {
                        id: name("panel/main"),
                        certificate_digest: Digest::from_bytes([77; 32]),
                        cells: [name("cell/a"), name("cell/b")].into_iter().collect(),
                        active: false,
                    },
                    Some(Counter(1)),
                )
                .unwrap();
        } else {
            f.app
                .put_principal(
                    &f.admin,
                    principal(
                        "admin",
                        &[Role::AccountAdmin, Role::Verifier, Role::Observer],
                    ),
                    Some(Counter(1)),
                )
                .unwrap();
        }
        assert!(matches!(
            f.app.commit_package_intake(prepared),
            Err(StoreError::Rejected(Rejection::Forbidden))
        ));
        assert!(
            f.app
                .package_intakes(&f.admin, &input.cell, None)
                .unwrap()
                .packages
                .is_empty()
        );
    }
}
#[test]
fn package_intake_conflicting_ids_and_stale_configuration_never_create_two_receipts() {
    let mut f = fixture(1, true);
    let p = intake_support::fixture();
    let input = intake_input(&mut f, &p);
    let mut stale = input.clone();
    stale.configuration_digest = Digest::from_bytes([99; 32]);
    assert!(matches!(
        f.app.prepare_package_intake(&f.admin, &id(), stale),
        Err(StoreError::Rejected(Rejection::StaleRevision))
    ));
    let first = intake_prepared(&mut f, &p, &id(), input.clone());
    let second = intake_prepared(&mut f, &p, &id(), input.clone());
    f.app.commit_package_intake(first).unwrap();
    assert!(matches!(
        f.app.commit_package_intake(second),
        Err(StoreError::Rejected(Rejection::StaleRevision))
    ));
    assert_eq!(
        f.app
            .package_intakes(&f.admin, &input.cell, None)
            .unwrap()
            .packages
            .len(),
        1
    );
}
#[test]
fn package_intake_receipts_survive_restart_but_live_policy_and_preflight_do_not() {
    let mut f = fixture(1, true);
    let p = intake_support::fixture();
    let input = intake_input(&mut f, &p);
    let key = id();
    let prepared = intake_prepared(&mut f, &p, &key, input.clone());
    f.app.commit_package_intake(prepared).unwrap();
    let mut pending = input.clone();
    pending.id = id();
    let pending = intake_prepared(&mut f, &p, &id(), pending);
    let installation = f.app.installation.id.clone();
    let repository = f.app.into_repository();
    let mut app = Engine::open(
        repository,
        f.clock,
        SimulationAuthority,
        installation,
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    assert!(app.commit_package_intake(pending).is_err());
    let session = app
        .authenticated_session(&f.admin.principal, id(), expiry(100000))
        .unwrap();
    let identity = Identity {
        session: session.id,
        ..f.admin
    };
    let view = app
        .package_intake(&identity, &input.cell, &input.id)
        .unwrap();
    assert!(!view.review_context_current && !view.activation_authorized);
    assert!(
        app.package_intake_context(&identity, &input.cell)
            .unwrap()
            .registration
            .is_none()
    );
    assert!(matches!(
        app.prepare_package_intake(&identity, &key, input).unwrap(),
        package_intake::Preflight::Recorded(_)
    ));
}

#[path = "support/process_review.rs"]
mod review_support;
fn review_job(f: &mut Fixture, p: &review_support::Fixture, step: &str) -> process_review::Job {
    assert!(p.package.is_dir() && p.policy_path.is_file());
    f.app
        .configure_package_intake(Some((
            p.store.owner().clone(),
            p.policy.fingerprint().unwrap(),
            rx_package::content_digest(&p.policy_bytes),
        )))
        .unwrap();
    f.app
        .configure_process_review(Some(p.authority.digest().unwrap()))
        .unwrap();
    let context = f
        .app
        .package_intake_context(&f.admin, &f.configuration.id)
        .unwrap();
    let intake_id = id();
    let input = package_intake::Submit {
        id: intake_id.clone(),
        cell: f.configuration.id.clone(),
        title: "Review package".into(),
        relative_path: rx_package::PackagePath::new("package").unwrap(),
        object: p.object.clone(),
        configuration_digest: context.configuration_digest,
        policy_generation: context.registration.as_ref().unwrap().generation.clone(),
    };
    let package_intake::Preflight::Verify(t) = f
        .app
        .prepare_package_intake(&f.admin, &id(), input)
        .unwrap()
    else {
        panic!("intake ticket")
    };
    f.app
        .commit_package_intake(
            package_intake::Prepared::new(*t, p.store.verify_owned(&p.object, &p.policy).unwrap())
                .unwrap(),
        )
        .unwrap();
    f.app
        .create_process_review(
            &f.admin,
            &id(),
            process_review::Create {
                device_plans: vec![],
                id: id(),
                intake: intake_id,
                cell: f.configuration.id.clone(),
                configuration_digest: context.configuration_digest,
                policy_generation: context.registration.unwrap().generation,
                binding_selections: BTreeMap::from([(name("load"), name(step))]),
            },
        )
        .unwrap()
}
fn review_report_prepared(
    f: &mut Fixture,
    p: &review_support::Fixture,
    job: &process_review::Job,
    key: &Id,
    expected: Option<Counter>,
) -> process_review::Prepared {
    let (report, signature, data) = p.report(job);
    let input = process_review::Submit {
        review: job.request.id.clone(),
        cell: job.request.cell.clone(),
        expected,
        directory: rx_package::PackagePath::new("report").unwrap(),
        report_digest: report.digest().unwrap(),
    };
    let process_review::Preflight::Verify(ticket) =
        f.app.prepare_review_report(&f.admin, key, input).unwrap()
    else {
        panic!("report ticket")
    };
    let checked = process_review::Validated::check(
        job,
        p.store.verify_owned(&p.object, &p.policy).unwrap(),
        &p.authority,
        report,
        signature,
        Some(&data),
    )
    .unwrap();
    process_review::Prepared::new(*ticket, checked).unwrap()
}
fn review_decide(
    f: &mut Fixture,
    p: &review_support::Fixture,
    job: &process_review::Job,
    reviewer: &Identity,
    key: &Id,
    input: process_review::Decide,
) -> process_review::PreparedDecision {
    let process_review::DecisionPreflight::Verify(t) =
        f.app.prepare_review_decision(reviewer, key, input).unwrap()
    else {
        panic!("decision ticket")
    };
    if t.requires_verification() {
        let (report, signature, data) = p.report(job);
        let v = process_review::Validated::check(
            job,
            p.store.verify_owned(&p.object, &p.policy).unwrap(),
            &p.authority,
            report,
            signature,
            Some(&data),
        )
        .unwrap();
        process_review::PreparedDecision::approve(*t, v).unwrap()
    } else {
        process_review::PreparedDecision::reject(*t).unwrap()
    }
}
fn decision_input(v: &process_review::Version) -> process_review::Decide {
    process_review::Decide {
        review: v.review.clone(),
        cell: v.cell.clone(),
        report_revision: v.revision,
        review_digest: v.review_digest,
        expected: None,
        choice: process_review::Choice::Approve,
        note: "Reviewed signed software material and selected cell steps".into(),
    }
}
#[test]
fn process_review_signed_report_and_separate_reviewer_approval_are_atomic_and_non_actuating() {
    for fault in [1, 2] {
        let mut f = fixture(1, false);
        let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
        let job = review_job(&mut f, &p, "step/0");
        let before = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap().0;
        let key = id();
        let prepared = review_report_prepared(&mut f, &p, &job, &key, None);
        f.failure.store(fault, Ordering::SeqCst);
        assert!(f.app.commit_review_report(prepared).is_err());
        let detail = f
            .app
            .process_review(&f.admin, &job.request.cell, &job.request.id)
            .unwrap();
        assert_eq!(detail.verification.is_some(), fault == 2);
        let v = if let Some(v) = detail.verification {
            v
        } else {
            let prepared = review_report_prepared(&mut f, &p, &job, &key, None);
            f.app.commit_review_report(prepared).unwrap()
        };
        assert!(v.ready_for_software_approval);
        let input = decision_input(&v);
        assert!(matches!(
            f.app
                .prepare_review_decision(&f.admin, &id(), input.clone()),
            Err(StoreError::Rejected(Rejection::Forbidden))
        ));
        let reviewer = add_identity(
            &mut f.app,
            &f.admin,
            "independent-reviewer",
            &[Role::Verifier],
        );
        let key = id();
        let prepared = review_decide(&mut f, &p, &job, &reviewer, &key, input.clone());
        f.failure.store(fault, Ordering::SeqCst);
        assert!(f.app.commit_review_decision(prepared).is_err());
        let d = match f
            .app
            .prepare_review_decision(&reviewer, &key, input)
            .unwrap()
        {
            process_review::DecisionPreflight::Recorded(v) => *v,
            process_review::DecisionPreflight::Verify(t) => {
                let (r, s, b) = p.report(&job);
                let checked = process_review::Validated::check(
                    &job,
                    p.store.verify_owned(&p.object, &p.policy).unwrap(),
                    &p.authority,
                    r,
                    s,
                    Some(&b),
                )
                .unwrap();
                f.app
                    .commit_review_decision(
                        process_review::PreparedDecision::approve(*t, checked).unwrap(),
                    )
                    .unwrap()
            }
        };
        assert_eq!(d.scope, name("PROCESS_PACKAGE_SOFTWARE"));
        let view = f
            .app
            .process_review(&reviewer, &job.request.cell, &job.request.id)
            .unwrap();
        assert!(view.approval_matches_current_review && !view.activation_authorized);
        assert_eq!(
            f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap().0,
            before
        );
        assert!(f.app.pending_deliveries(128).unwrap().is_empty());
    }
}
#[test]
fn process_review_signature_and_compiled_bytes_cannot_be_replaced_or_rebound() {
    let mut f = fixture(1, false);
    let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let job = review_job(&mut f, &p, "step/0");
    let (report, mut signature, data) = p.report(&job);
    let replacement = if signature.signature.starts_with("00") {
        "01"
    } else {
        "00"
    };
    signature.signature.replace_range(0..2, replacement);
    assert!(
        process_review::Validated::check(
            &job,
            p.store.verify_owned(&p.object, &p.policy).unwrap(),
            &p.authority,
            report.clone(),
            signature,
            Some(&data)
        )
        .is_err()
    );
    let mut changed = report.clone();
    changed.request.id = id();
    assert!(
        process_review::Validated::check(
            &job,
            p.store.verify_owned(&p.object, &p.policy).unwrap(),
            &p.authority,
            changed.clone(),
            review_support::sign(&changed),
            Some(&data)
        )
        .is_err()
    );
    let signature = review_support::sign(&report);
    let mut changed = data.clone();
    changed.push(b' ');
    assert!(
        process_review::Validated::check(
            &job,
            p.store.verify_owned(&p.object, &p.policy).unwrap(),
            &p.authority,
            report,
            signature,
            Some(&changed)
        )
        .is_err()
    );
}
#[test]
fn process_review_signed_compiler_success_does_not_override_cell_binding_mismatch() {
    let mut f = fixture(2, false);
    let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let job = review_job(&mut f, &p, "step/1");
    let prepared = review_report_prepared(&mut f, &p, &job, &id(), None);
    let v = f.app.commit_review_report(prepared).unwrap();
    assert!(!v.ready_for_software_approval && !v.platform_issues.is_empty());
    let reviewer = add_identity(
        &mut f.app,
        &f.admin,
        "independent-reviewer",
        &[Role::Verifier],
    );
    assert!(
        f.app
            .prepare_review_decision(&reviewer, &id(), decision_input(&v))
            .is_err()
    );
    let mut reject = decision_input(&v);
    reject.choice = process_review::Choice::Reject;
    let prepared = review_decide(&mut f, &p, &job, &reviewer, &id(), reject);
    f.app.commit_review_decision(prepared).unwrap();
}
#[test]
fn process_review_new_report_invalidates_old_decision_and_cas_prevents_overwriting_reviews() {
    let mut f = fixture(1, false);
    let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let job = review_job(&mut f, &p, "step/0");
    let prepared = review_report_prepared(&mut f, &p, &job, &id(), None);
    let v = f.app.commit_review_report(prepared).unwrap();
    let reviewer = add_identity(
        &mut f.app,
        &f.admin,
        "independent-reviewer",
        &[Role::Verifier],
    );
    let input = decision_input(&v);
    let pending = review_decide(&mut f, &p, &job, &reviewer, &id(), input.clone());
    let prepared = review_decide(&mut f, &p, &job, &reviewer, &id(), input);
    f.app.commit_review_decision(prepared).unwrap();
    assert!(f.app.commit_review_decision(pending).is_err());
    let prepared = review_report_prepared(&mut f, &p, &job, &id(), Some(Counter(1)));
    let next = f.app.commit_review_report(prepared).unwrap();
    assert_ne!(next.review_digest, v.review_digest);
    assert!(
        !f.app
            .process_review(&reviewer, &job.request.cell, &job.request.id)
            .unwrap()
            .approval_matches_current_review
    );
}
#[test]
fn process_review_authority_and_role_changes_reject_pending_approvals() {
    for revoked_role in [false, true] {
        let mut f = fixture(1, false);
        let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
        let job = review_job(&mut f, &p, "step/0");
        let prepared = review_report_prepared(&mut f, &p, &job, &id(), None);
        let v = f.app.commit_review_report(prepared).unwrap();
        let reviewer = add_identity(
            &mut f.app,
            &f.admin,
            "independent-reviewer",
            &[Role::Verifier],
        );
        let prepared = review_decide(&mut f, &p, &job, &reviewer, &id(), decision_input(&v));
        if revoked_role {
            f.app
                .put_principal(
                    &f.admin,
                    principal("independent-reviewer", &[Role::Observer]),
                    Some(Counter(1)),
                )
                .unwrap();
        } else {
            f.app.configure_process_review(None).unwrap();
        }
        assert!(f.app.commit_review_decision(prepared).is_err());
        assert!(
            f.app
                .process_review(&f.admin, &job.request.cell, &job.request.id)
                .unwrap()
                .decision
                .is_none()
        );
    }
}

#[test]
fn process_review_restart_preserves_software_decision_but_rejects_old_pending_ticket() {
    let mut f = fixture(1, false);
    let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let job = review_job(&mut f, &p, "step/0");
    let prepared = review_report_prepared(&mut f, &p, &job, &id(), None);
    let version = f.app.commit_review_report(prepared).unwrap();
    let reviewer = add_identity(
        &mut f.app,
        &f.admin,
        "independent-reviewer",
        &[Role::Verifier],
    );
    let input = decision_input(&version);
    let prepared = review_decide(&mut f, &p, &job, &reviewer, &id(), input.clone());
    f.app.commit_review_decision(prepared).unwrap();
    let mut next = input;
    next.expected = Some(Counter(1));
    next.choice = process_review::Choice::Reject;
    let pending = review_decide(&mut f, &p, &job, &reviewer, &id(), next);
    let installation = f.app.installation.id.clone();
    let repository = f.app.into_repository();
    let mut app = Engine::open(
        repository,
        f.clock,
        SimulationAuthority,
        installation,
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    assert!(app.commit_review_decision(pending).is_err());
    let session = app
        .authenticated_session(&reviewer.principal, id(), expiry(100000))
        .unwrap();
    let identity = Identity {
        session: session.id,
        ..reviewer
    };
    let before = app
        .process_review(&identity, &job.request.cell, &job.request.id)
        .unwrap();
    assert!(!before.context_current && !before.approval_matches_current_review);
    assert_eq!(
        before.decision.unwrap().choice,
        process_review::Choice::Approve
    );
    drop(p.store);
    let store = rx_package::store::Store::open_existing(&p._dir.path().join("store")).unwrap();
    store.verify(&p.object, &p.policy).unwrap();
    app.configure_package_intake(Some((
        store.owner().clone(),
        p.policy.fingerprint().unwrap(),
        rx_package::content_digest(&p.policy_bytes),
    )))
    .unwrap();
    app.configure_process_review(Some(p.authority.digest().unwrap()))
        .unwrap();
    let after = app
        .process_review(&identity, &job.request.cell, &job.request.id)
        .unwrap();
    assert!(
        after.context_current
            && after.approval_matches_current_review
            && !after.activation_authorized
    );
    assert_eq!(after.decision.unwrap().revision, Counter(1));
}

#[test]
fn process_review_lists_are_scoped_and_history_is_read_only_with_latest_revision_visible() {
    let mut f = fixture(1, false);
    let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let job = review_job(&mut f, &p, "step/0");
    let prepared = review_report_prepared(&mut f, &p, &job, &id(), None);
    f.app.commit_review_report(prepared).unwrap();
    let prepared = review_report_prepared(&mut f, &p, &job, &id(), Some(Counter(1)));
    f.app.commit_review_report(prepared).unwrap();
    let page = f
        .app
        .process_reviews(&f.admin, &job.request.cell, &job.request.intake, None)
        .unwrap();
    assert_eq!(page.reviews.len(), 1);
    assert_eq!(page.reviews[0].report_revision, Some(Counter(2)));
    assert!(
        f.app
            .process_reviews(&f.operator, &job.request.cell, &job.request.intake, None)
            .is_err()
    );
    assert!(
        f.app
            .process_reviews(&f.admin, &name("cell/b"), &job.request.intake, None)
            .is_err()
    );
    assert!(
        f.app
            .process_reviews(
                &f.admin,
                &job.request.cell,
                &job.request.intake,
                Some(&job.request.id)
            )
            .unwrap()
            .reviews
            .is_empty()
    );
    let old = f
        .app
        .process_review_revision(
            &f.admin,
            &job.request.cell,
            &job.request.id,
            Some(Counter(1)),
        )
        .unwrap();
    assert_eq!(old.verification.unwrap().revision, Counter(1));
    assert_eq!(old.latest_report_revision, Some(Counter(2)));
    assert!(!old.is_latest && !old.approval_matches_current_review && !old.activation_authorized);
    assert!(
        f.app
            .process_review_revision(
                &f.admin,
                &job.request.cell,
                &job.request.id,
                Some(Counter(99))
            )
            .is_err()
    );
}

fn approved_change_review(
    f: &mut Fixture,
    p: &review_support::Fixture,
) -> (
    process_review::Job,
    process_review::Version,
    process_review::Decision,
    Identity,
) {
    let step = f.configuration.steps[0].id.to_string();
    let job = review_job(f, p, &step);
    let prepared = review_report_prepared(f, p, &job, &id(), None);
    let v = f.app.commit_review_report(prepared).unwrap();
    let reviewer = add_identity(&mut f.app, &f.admin, "change-reviewer", &[Role::Verifier]);
    let prepared = review_decide(f, p, &job, &reviewer, &id(), decision_input(&v));
    let d = f.app.commit_review_decision(prepared).unwrap();
    (job, v, d, reviewer)
}
fn change_proposal(
    f: &mut Fixture,
    p: &review_support::Fixture,
    job: &process_review::Job,
    v: &process_review::Version,
    d: &process_review::Decision,
    key: &Id,
    change_id: Id,
) -> process_change::Prepared {
    let input = process_change::Create {
        mode: process_change::Mode::Replace,
        id: change_id,
        cell: job.request.cell.clone(),
        review: process_change::ReviewRef {
            id: job.request.id.clone(),
            revision: v.revision,
            review_digest: v.review_digest,
            decision_revision: d.revision,
        },
        reason: "Prepare the approved process as a reviewed configuration change".into(),
    };
    let process_change::Preflight::Verify(ticket) =
        f.app.prepare_process_change(&f.admin, key, input).unwrap()
    else {
        panic!("new change ticket")
    };
    let (report, signature, bytes) = p.report(job);
    let checked = process_review::Validated::check(
        job,
        p.store.verify_owned(&p.object, &p.policy).unwrap(),
        &p.authority,
        report,
        signature,
        Some(&bytes),
    )
    .unwrap();
    process_change::Prepared::new(*ticket, checked).unwrap()
}
fn change_target(c: &process_change::Change) -> process_change::Transition {
    process_change::Transition {
        change: c.id.clone(),
        cell: c.cell.clone(),
        expected: c.revision,
        plan_digest: c.plan_digest,
    }
}
fn release_identity(f: &mut Fixture) -> Identity {
    let who = add_identity(
        &mut f.app,
        &f.admin,
        "release-manager",
        &[Role::ReleaseManager],
    );
    let s = f
        .app
        .authenticated_terminal_user_session(
            &who.principal,
            id(),
            Counter(99_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap();
    Identity {
        session: s.id,
        terminal: Some((name("panel/main"), Digest::from_bytes([77; 32]))),
        ..who
    }
}
fn stage_change(
    f: &mut Fixture,
    p: &review_support::Fixture,
    job: &process_review::Job,
    c: &process_change::Change,
    reviewer: &Identity,
    release: &Identity,
) -> process_change::Change {
    let reviewed=f.app.review_process_change_impact(reviewer,&id(),process_change::ReviewImpact {target:change_target(c),note:"Whole cell and shared-controller effects require requalification and recovery review".into()}).unwrap();
    let process_change::Preflight::Verify(t) = f
        .app
        .prepare_process_change_stage(release, &id(), change_target(&reviewed))
        .unwrap()
    else {
        panic!("stage ticket")
    };
    let (r, s, b) = p.report(job);
    let v = process_review::Validated::check(
        job,
        p.store.verify_owned(&p.object, &p.policy).unwrap(),
        &p.authority,
        r,
        s,
        Some(&b),
    )
    .unwrap();
    f.app
        .commit_process_change_stage(process_change::Prepared::new(*t, v).unwrap())
        .unwrap()
}
#[test]
fn process_change_proposal_and_stage_preserve_configuration_and_all_step_guards() {
    let mut f = fixture(1, false);
    let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let (job, v, d, reviewer) = approved_change_review(&mut f, &p);
    let release = release_identity(&mut f);
    let before = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap();
    let prepared = change_proposal(&mut f, &p, &job, &v, &d, &id(), id());
    let c = f.app.commit_process_change(prepared).unwrap();
    assert_eq!(c.state, process_change::State::Proposed);
    let staged = stage_change(&mut f, &p, &job, &c, &reviewer, &release);
    assert_eq!(staged.state, process_change::State::Staged);
    let detail = f.app.process_change(&f.admin, &c.cell, &c.id).unwrap();
    assert!(!detail.applied && !detail.activation_authorized);
    assert_ne!(detail.before.recipe, detail.after.recipe);
    assert_eq!(detail.after.steps[0].id, p.resolved.root.id);
    let mut expanded = detail.after.steps[0].clone();
    expanded.id = f.configuration.steps[0].id.clone();
    assert_eq!(
        rx_domain::canonical::bytes(&expanded).unwrap(),
        rx_domain::canonical::bytes(&f.configuration.steps[0]).unwrap()
    );
    assert_eq!(
        f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap().0,
        before.0
    );
    assert!(f.app.pending_deliveries(128).unwrap().is_empty());
}
#[test]
fn process_change_proposal_failure_is_atomic_and_lost_reply_recovers_original_plan() {
    for failure in [1, 2] {
        let mut f = fixture(1, false);
        let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
        let (job, v, d, _) = approved_change_review(&mut f, &p);
        let key = id();
        let cid = id();
        let prepared = change_proposal(&mut f, &p, &job, &v, &d, &key, cid.clone());
        f.failure.store(failure, Ordering::SeqCst);
        assert!(f.app.commit_process_change(prepared).is_err());
        let input = process_change::Create {
            mode: process_change::Mode::Replace,
            id: cid.clone(),
            cell: job.request.cell.clone(),
            review: process_change::ReviewRef {
                id: job.request.id.clone(),
                revision: v.revision,
                review_digest: v.review_digest,
                decision_revision: d.revision,
            },
            reason: "Prepare the approved process as a reviewed configuration change".into(),
        };
        let c = match f
            .app
            .prepare_process_change(&f.admin, &key, input.clone())
            .unwrap()
        {
            process_change::Preflight::Recorded(v) => *v,
            process_change::Preflight::Verify(t) => {
                let (r, s, b) = p.report(&job);
                let v = process_review::Validated::check(
                    &job,
                    p.store.verify_owned(&p.object, &p.policy).unwrap(),
                    &p.authority,
                    r,
                    s,
                    Some(&b),
                )
                .unwrap();
                f.app
                    .commit_process_change(process_change::Prepared::new(*t, v).unwrap())
                    .unwrap()
            }
        };
        assert_eq!(c.revision, Counter(1));
        let mut changed = input;
        changed.reason = "Different intent".into();
        assert!(matches!(
            f.app.prepare_process_change(&f.admin, &key, changed),
            Err(StoreError::KeyConflict)
        ));
    }
}
#[test]
fn process_change_preparation_is_atomic_preserves_unknown_work_and_requires_config_ack() {
    for failure in [1, 2] {
        let mut f = fixture(1, true);
        let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
        let (job, v, d, reviewer) = approved_change_review(&mut f, &p);
        let release = release_identity(&mut f);
        let prepared = change_proposal(&mut f, &p, &job, &v, &d, &id(), id());
        let c = f.app.commit_process_change(prepared).unwrap();
        let staged = stage_change(&mut f, &p, &job, &c, &reviewer, &release);
        let run = start(&mut f, 1);
        let a = activation(&mut f, &run);
        let work = submit(&mut f, &a, id().as_str()).unwrap();
        let delivery=f.app.pending_deliveries(128).unwrap().into_iter().find(|d|matches!(&d.payload,Delivery::Prepare {operation,..} if operation==work.operation.id())).unwrap();
        f.app.plan_delivery(&f.hosts[0], &delivery.id).unwrap();
        let before = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap();
        let input = process_change::BeginPreparation {
            target: change_target(&staged),
            refresh: false,
        };
        let key = id();
        f.failure.store(failure, Ordering::SeqCst);
        assert!(
            f.app
                .begin_process_change_preparation(&release, &key, input.clone())
                .is_err()
        );
        if failure == 1 {
            assert_eq!(
                f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap().0,
                before.0
            );
        }
        let started = f
            .app
            .begin_process_change_preparation(&release, &key, input.clone())
            .unwrap();
        let again = f
            .app
            .begin_process_change_preparation(&release, &key, input)
            .unwrap();
        assert_eq!(started.revision, again.revision);
        assert_eq!(
            started.preparation.as_ref().unwrap().fences[0].message,
            again.preparation.as_ref().unwrap().fences[0].message
        );
        let (_, after) = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap();
        assert_eq!(after.epoch.0, before.1.epoch.0 + 1);
        assert_eq!(
            rx_domain::canonical::bytes(&after.configuration).unwrap(),
            rx_domain::canonical::bytes(&before.1.configuration).unwrap()
        );
        assert!(
            after
                .blocks
                .iter()
                .any(|b| b.reason == BlockReason::ConfigurationChange)
        );
        let kept = f
            .app
            .inspect_work(&f.operator, work.operation.id())
            .unwrap();
        assert_eq!(
            kept.operation.outcome(),
            rx_domain::operation::Outcome::None
        );
        assert_eq!(
            kept.operation.knowledge(),
            rx_domain::operation::Knowledge::Unknown
        );
        assert_ne!(
            kept.operation.disposition(),
            rx_domain::operation::Disposition::Released
        );
        let view = f.app.process_change(&f.admin, &c.cell, &c.id).unwrap();
        assert!(
            view.blockers
                .iter()
                .any(|b| matches!(b, process_change::Blocker::OperationUnresolved { .. }))
        );
        assert!(view.blockers.iter().any(|b| matches!(
            b,
            process_change::Blocker::HostConfigurationAcknowledgementRequired { .. }
        )));
        assert!(!view.applied);
    }
}

#[test]
fn process_change_shared_host_closure_requires_rights_for_every_affected_cell() {
    let mut f = fixture(1, false);
    let mut other = f.configuration.clone();
    other.id = name("cell/b");
    other.scopes = vec![name("zone/b")];
    other.steps[0].intent.resource_set = vec![name("controller/other")];
    f.app.install_cell(&f.admin, other).unwrap();
    let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let (job, v, d, _) = approved_change_review(&mut f, &p);
    let prepared = change_proposal(&mut f, &p, &job, &v, &d, &id(), id());
    let c = f.app.commit_process_change(prepared).unwrap();
    assert_eq!(c.impact.cells.len(), 2);
    assert!(c.impact.cells.iter().any(|c| c.id == name("cell/b")));
    let limited = add_identity(&mut f.app, &f.admin, "limited-engineer", &[Role::Engineer]);
    let mut principal = principal("limited-engineer", &[Role::Engineer]);
    principal.cells = [name("cell/a")].into_iter().collect();
    f.app
        .put_principal(&f.admin, principal, Some(Counter(1)))
        .unwrap();
    assert!(matches!(
        f.app.process_change(&limited, &c.cell, &c.id),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    let create = process_change::Create {
        mode: process_change::Mode::Replace,
        id: id(),
        cell: c.cell,
        review: c.review,
        reason: "same Host also affects another cell".into(),
    };
    assert!(matches!(
        f.app.prepare_process_change(&limited, &id(), create),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
}
#[test]
fn process_change_fence_confirmation_is_not_configuration_application() {
    let mut f = fixture(1, false);
    let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let (job, v, d, reviewer) = approved_change_review(&mut f, &p);
    let release = release_identity(&mut f);
    let prepared = change_proposal(&mut f, &p, &job, &v, &d, &id(), id());
    let c = f.app.commit_process_change(prepared).unwrap();
    let staged = stage_change(&mut f, &p, &job, &c, &reviewer, &release);
    let started = f
        .app
        .begin_process_change_preparation(
            &release,
            &id(),
            process_change::BeginPreparation {
                target: change_target(&staged),
                refresh: false,
            },
        )
        .unwrap();
    let fence = started.preparation.as_ref().unwrap().fences[0].clone();
    f.app.plan_delivery(&f.hosts[0], &fence.message).unwrap();
    f.app
        .finish_fence_delivery(
            &f.hosts[0],
            &fence.message,
            FenceAcknowledgment {
                cell: fence.cell.clone(),
                invalidation: fence.message.clone(),
                epoch: fence.epoch,
                scopes: fence.scopes,
                host_boot: f.registrations[0].boot_id.clone(),
                journal: f.registrations[0].delivery_journal.clone(),
                sequence: Counter(17),
            },
        )
        .unwrap();
    let view = f.app.process_change(&f.admin, &c.cell, &c.id).unwrap();
    assert!(
        !view
            .blockers
            .iter()
            .any(|b| matches!(b, process_change::Blocker::HostFenceUnconfirmed { .. }))
    );
    assert!(view.blockers.iter().any(|b| matches!(
        b,
        process_change::Blocker::HostConfigurationAcknowledgementRequired { .. }
    )));
    assert!(!view.applied && !view.activation_authorized);
}
#[test]
fn process_change_revoked_software_review_rejects_pending_stage_without_installing_target() {
    let mut f = fixture(1, false);
    let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let (job, v, d, reviewer) = approved_change_review(&mut f, &p);
    let release = release_identity(&mut f);
    let prepared = change_proposal(&mut f, &p, &job, &v, &d, &id(), id());
    let c = f.app.commit_process_change(prepared).unwrap();
    assert!(
        f.app
            .review_process_change_impact(
                &f.admin,
                &id(),
                process_change::ReviewImpact {
                    target: change_target(&c),
                    note: "self impact review".into()
                }
            )
            .is_err()
    );
    let reviewed = f
        .app
        .review_process_change_impact(
            &reviewer,
            &id(),
            process_change::ReviewImpact {
                target: change_target(&c),
                note: "reviewed impact".into(),
            },
        )
        .unwrap();
    let process_change::Preflight::Verify(t) = f
        .app
        .prepare_process_change_stage(&release, &id(), change_target(&reviewed))
        .unwrap()
    else {
        panic!("ticket")
    };
    let (r, s, b) = p.report(&job);
    let validated = process_review::Validated::check(
        &job,
        p.store.verify_owned(&p.object, &p.policy).unwrap(),
        &p.authority,
        r,
        s,
        Some(&b),
    )
    .unwrap();
    let prepared = process_change::Prepared::new(*t, validated).unwrap();
    let mut reject = decision_input(&v);
    reject.choice = process_review::Choice::Reject;
    reject.expected = Some(Counter(1));
    let rejection = review_decide(&mut f, &p, &job, &reviewer, &id(), reject);
    f.app.commit_review_decision(rejection).unwrap();
    assert!(f.app.commit_process_change_stage(prepared).is_err());
    assert_eq!(
        f.app
            .process_change(&f.admin, &c.cell, &c.id)
            .unwrap()
            .change
            .state,
        process_change::State::ImpactReviewed
    );
}
#[test]
fn process_change_preparation_requires_current_terminal_and_serializes_overlapping_changes() {
    let mut f = fixture(1, false);
    let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let (job, v, d, reviewer) = approved_change_review(&mut f, &p);
    let release = release_identity(&mut f);
    let prepared = change_proposal(&mut f, &p, &job, &v, &d, &id(), id());
    let first = f.app.commit_process_change(prepared).unwrap();
    let first = stage_change(&mut f, &p, &job, &first, &reviewer, &release);
    let prepared = change_proposal(&mut f, &p, &job, &v, &d, &id(), id());
    let second = f.app.commit_process_change(prepared).unwrap();
    let second = stage_change(&mut f, &p, &job, &second, &reviewer, &release);
    let session = f
        .app
        .authenticated_session(&release.principal, id(), expiry(100000))
        .unwrap();
    let unbound = Identity {
        principal: release.principal.clone(),
        session: session.id,
        terminal: None,
    };
    assert!(matches!(
        f.app.begin_process_change_preparation(
            &unbound,
            &id(),
            process_change::BeginPreparation {
                target: change_target(&first),
                refresh: false
            }
        ),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    f.app
        .begin_process_change_preparation(
            &release,
            &id(),
            process_change::BeginPreparation {
                target: change_target(&first),
                refresh: false,
            },
        )
        .unwrap();
    assert!(matches!(
        f.app.begin_process_change_preparation(
            &release,
            &id(),
            process_change::BeginPreparation {
                target: change_target(&second),
                refresh: false
            }
        ),
        Err(StoreError::Rejected(Rejection::Busy))
    ));
}

#[test]
fn process_change_refresh_preserves_existing_blocks_and_replaces_only_preparation_context() {
    let mut f = fixture(1, false);
    let p = review_support::fixture(&f.configuration, Digest::from_bytes([71; 32]));
    let (job, v, d, reviewer) = approved_change_review(&mut f, &p);
    let release = release_identity(&mut f);
    let proposed = change_proposal(&mut f, &p, &job, &v, &d, &id(), id());
    let c = f.app.commit_process_change(proposed).unwrap();
    let staged = stage_change(&mut f, &p, &job, &c, &reviewer, &release);
    let first = f
        .app
        .begin_process_change_preparation(
            &release,
            &id(),
            process_change::BeginPreparation {
                target: change_target(&staged),
                refresh: false,
            },
        )
        .unwrap();
    f.app
        .hold(&f.operator, id().as_str(), &f.configuration.id)
        .unwrap();
    let (_, held) = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    assert!(
        f.app
            .process_change(&f.admin, &c.cell, &c.id)
            .unwrap()
            .blockers
            .iter()
            .any(|b| matches!(b, process_change::Blocker::PreparationStale))
    );
    assert!(
        f.app
            .begin_process_change_preparation(
                &release,
                &id(),
                process_change::BeginPreparation {
                    target: change_target(&first),
                    refresh: false
                }
            )
            .is_err()
    );
    let next = f
        .app
        .begin_process_change_preparation(
            &release,
            &id(),
            process_change::BeginPreparation {
                target: change_target(&first),
                refresh: true,
            },
        )
        .unwrap();
    assert_eq!(next.preparation.as_ref().unwrap().attempt, Counter(2));
    assert_ne!(
        next.preparation.as_ref().unwrap().fences[0].message,
        first.preparation.as_ref().unwrap().fences[0].message
    );
    let (_, after) = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap();
    assert!(
        held.blocks
            .iter()
            .all(|old| after.blocks.iter().any(|new| new.id == old.id))
    );
    assert!(
        !f.app
            .process_change(&f.admin, &c.cell, &c.id)
            .unwrap()
            .blockers
            .iter()
            .any(|b| matches!(b, process_change::Blocker::PreparationStale))
    );
}

#[path = "support/configuration_dispatch_tests.rs"]
mod configuration_dispatch_tests;

#[test]
fn confirmed_new_cell_fence_advances_registration_and_allows_only_original_grant_renewal() {
    let (mut f, input) = link_fixture();
    let plan = f.app.prepare_host_link(input).unwrap();
    let commit = link_commit(&f, &plan);
    let bound = f.app.commit_host_link(commit).unwrap();
    f.app
        .hold(&f.operator, id().as_str(), &f.configuration.id)
        .unwrap();
    let cell = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1;
    assert!(f.app.prepare_host_renewal(&plan.id).is_err());
    let delivery = f
        .app
        .pending_deliveries(128)
        .unwrap()
        .into_iter()
        .find(|d| matches!(&d.payload,Delivery::Fence{epoch,..} if *epoch==cell.epoch))
        .unwrap();
    f.app.plan_delivery(&f.hosts[0], &delivery.id).unwrap();
    f.app
        .finish_fence_delivery(
            &f.hosts[0],
            &delivery.id,
            FenceAcknowledgment {
                cell: f.configuration.id.clone(),
                invalidation: delivery.id.clone(),
                epoch: cell.epoch,
                scopes: cell.scope_epochs.clone(),
                host_boot: bound.boot_id.clone(),
                journal: bound.delivery_journal.clone(),
                sequence: Counter(700),
            },
        )
        .unwrap();
    let renewal = f.app.prepare_host_renewal(&plan.id).unwrap();
    assert_eq!(renewal.grant.id, bound.grant.id);
    assert_eq!(renewal.grant.fence, bound.grant.fence);
    assert_eq!(renewal.grant.resources, bound.grant.resources);
    let mut grant = renewal.grant.clone();
    grant.valid_until = expiry(renewal.sent_at.ticks_ns.0 + grant.ttl_ms.0 * 1_000_000);
    let registered = f.app.commit_host_renewal(renewal, grant).unwrap();
    assert_eq!(registered.epoch, cell.epoch);
    assert!(
        !f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .blocks
            .is_empty()
    );
}

#[test]
fn device_provenance_cannot_pass_the_legacy_process_review_even_when_actions_match() {
    let mut f = fixture(1, false);
    let p = review_support::fixture_mode(&f.configuration, Digest::from_bytes([71; 32]), true);
    let job = review_job(&mut f, &p, "step/0");
    let prepared = review_report_prepared(&mut f, &p, &job, &id(), None);
    let v = f.app.commit_review_report(prepared).unwrap();
    assert!(!v.ready_for_software_approval);
    assert!(
        v.platform_issues
            .iter()
            .any(|i| i.detail.contains("device-aware process review"))
    );
}
