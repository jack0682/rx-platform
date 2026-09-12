use super::*;
use rx_application::{persistence as p, runtime_invalidation as origin};
use std::sync::Mutex;

// Test-only shared access to the same repository lets assertions inspect or inject
// missing/corrupt provenance without reopening Engine and creating another P boot.
#[derive(Clone)]
struct SharedRepository(Arc<Mutex<FaultRepository>>);
impl Repository for SharedRepository {
    fn transact<T>(
        &mut self,
        f: impl FnOnce(&mut dyn rx_ports::Transaction) -> rx_ports::Result<T>,
    ) -> rx_ports::Result<T> {
        self.0.lock().unwrap().transact(f)
    }
    fn pending_outbox_after(
        &mut self,
        after: Option<&Id>,
        limit: usize,
    ) -> rx_ports::Result<Vec<rx_ports::OutboxRecord>> {
        self.0.lock().unwrap().pending_outbox_after(after, limit)
    }
    fn control_events_after(
        &mut self,
        after: Counter,
        limit: usize,
    ) -> rx_ports::Result<Vec<StoredEvent>> {
        self.0.lock().unwrap().control_events_after(after, limit)
    }
    fn control_snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<Record>)> {
        self.0.lock().unwrap().control_snapshot()
    }
    fn journal_head(&mut self) -> rx_ports::Result<Counter> {
        self.0.lock().unwrap().journal_head()
    }
    fn pending_outbox(&mut self, limit: usize) -> rx_ports::Result<Vec<rx_ports::OutboxRecord>> {
        self.0.lock().unwrap().pending_outbox(limit)
    }
    fn snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<Record>)> {
        self.0.lock().unwrap().snapshot()
    }
    fn events_after(&mut self, after: Counter, limit: usize) -> rx_ports::Result<Vec<StoredEvent>> {
        self.0.lock().unwrap().events_after(after, limit)
    }
}
#[derive(Clone)]
struct Pin {
    boot: Id,
    evidence_journal: Id,
    authentication: Digest,
}
struct Restarted {
    _directory: tempfile::TempDir,
    app: Engine<SharedRepository, ManualClock, SimulationAuthority>,
    repository: SharedRepository,
    clock: ManualClock,
    failure: Arc<AtomicU8>,
    principal: Name,
    pin: Pin,
    old_session: Session,
}
impl Restarted {
    fn reopen(&mut self, pin: &Pin) -> rx_ports::Result<Session> {
        self.app.open_evidence_producer(
            &self.principal,
            pin.boot.clone(),
            pin.evidence_journal.clone(),
            pin.authentication,
        )
    }
    fn authorities(&mut self) -> Vec<Record> {
        self.repository
            .transact(|tx| {
                let mut records = Vec::new();
                for prefix in [
                    "cell/",
                    "host/",
                    "run/",
                    "attempt/",
                    "mandate/",
                    "permit/",
                    "work/",
                    "part/",
                    "resource/",
                    "fact/",
                    "factevidence/",
                    "source-generation-loss/",
                    "observationincident/",
                    "runtimeinvalidationorigin/",
                    "qualificationhistory/",
                    "host-link-plan/",
                    "host-link-renewal/",
                ] {
                    records.extend(tx.scan(prefix)?);
                }
                records.sort_by(|a, b| a.key.cmp(&b.key));
                Ok(records)
            })
            .unwrap()
    }
    fn device_restarts(&mut self) -> usize {
        self.repository
            .transact(|tx| {
                let mut count = 0;
                for row in tx.scan("cell/")? {
                    let cell: Cell = p::decode(&row, "rx.internal.cell.v1")?;
                    count += cell
                        .blocks
                        .iter()
                        .filter(|b| b.reason == BlockReason::DeviceRestart)
                        .count();
                }
                Ok(count)
            })
            .unwrap()
    }
    fn change_session(&mut self, change: impl FnOnce(&mut Session)) {
        self.repository
            .transact(|tx| {
                let (revision, mut session): (_, Session) = p::load(
                    tx,
                    "session",
                    &self.old_session.id,
                    "rx.internal.session.v1",
                )?;
                change(&mut session);
                p::save(
                    tx,
                    "session",
                    &session.id,
                    Some(revision),
                    "rx.internal.session.v1",
                    &session,
                )?;
                Ok(())
            })
            .unwrap();
    }
}

fn restarted(cells: usize, graceful: bool, with_work: bool) -> Restarted {
    restarted_after(cells, graceful, with_work, |_| {})
}

fn restarted_after(
    cells: usize,
    graceful: bool,
    with_work: bool,
    before_restart: impl FnOnce(&mut Fixture),
) -> Restarted {
    let mut f = fixture_registration(1, with_work, false, false, None, false, false);
    let peer = f.hosts[0].principal.clone();
    let pin = Pin {
        boot: id(),
        evidence_journal: id(),
        authentication: Digest::from_bytes([31; 32]),
    };
    let producer = f
        .app
        .open_evidence_producer(
            &peer,
            pin.boot.clone(),
            pin.evidence_journal.clone(),
            pin.authentication,
        )
        .unwrap();
    f.hosts[0].session = producer.id.clone();
    let mut configurations = vec![f.configuration.clone()];
    if cells == 2 {
        let mut second = f.configuration.clone();
        second.id = name("cell/b");
        second.definition = artifact(12, "rx.cell-definition.v1");
        f.app.install_cell(&f.admin, second.clone()).unwrap();
        configurations.push(second);
    }
    let delivery = id();
    let source = id();
    for configuration in &configurations {
        f.app
            .negotiate_evidence_cell(&f.hosts[0], configuration.definition.sha256)
            .unwrap();
        let (_, cell) = f.app.inspect_cell(&f.admin, &configuration.id).unwrap();
        let registration = HostRegistration {
            id: peer.clone(),
            session: producer.id.clone(),
            boot_id: pin.boot.clone(),
            delivery_journal: delivery.clone(),
            cell: configuration.id.clone(),
            epoch: cell.epoch,
            scopes: cell.scope_epochs,
            source_sessions: configuration
                .fact_specs
                .iter()
                .map(|spec| (spec.id.clone(), source.clone()))
                .collect(),
            grant: Grant {
                id: id(),
                fence: Counter(1),
                resources: configuration
                    .steps
                    .iter()
                    .flat_map(|step| step.intent.resource_set.clone())
                    .collect(),
                owner: name(f.app.installation.id.as_str()),
                valid_until: expiry(50_000),
                ttl_ms: Counter(1),
            },
        };
        f.app
            .register_host(&f.hosts[0], registration.clone())
            .unwrap();
        f.registrations.push(registration);
    }
    report_ready(
        &mut f.app,
        &f.hosts[0],
        &f.configuration,
        &f.registrations[0],
    );
    if with_work {
        let run = start(&mut f, 1);
        let activation = activation(&mut f, &run);
        submit(&mut f, &activation, id().as_str()).unwrap();
    }
    before_restart(&mut f);
    if graceful {
        f.app.request_runtime_stop().unwrap();
        f.app.commit_runtime_process_stop().unwrap();
    }
    let installation = f.app.installation.id.clone();
    let repository = SharedRepository(Arc::new(Mutex::new(f.app.into_repository())));
    let app = Engine::open(
        repository.clone(),
        f.clock.clone(),
        SimulationAuthority,
        installation,
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    Restarted {
        _directory: f._directory,
        app,
        repository,
        clock: f.clock,
        failure: f.failure,
        principal: peer,
        pin,
        old_session: producer,
    }
}

#[test]
fn p_only_restart_replaces_only_evidence_session_and_preserves_all_authority_records() {
    for graceful in [false, true] {
        for with_work in [false, true] {
            let mut f = restarted(1, graceful, with_work);
            let before = f.authorities();
            let outbox = f.repository.pending_outbox(128).unwrap();
            let next = f.reopen(&f.pin.clone()).unwrap();
            assert_ne!(next.id, f.old_session.id);
            assert_eq!(next.runtime_boot, f.app.installation.runtime_boot);
            assert!(next.active);
            assert_eq!(f.authorities(), before);
            assert_eq!(f.repository.pending_outbox(128).unwrap(), outbox);
            assert_eq!(f.device_restarts(), 0);
            let producer = f.app.current_evidence_producer(&f.principal).unwrap();
            assert_eq!(producer.session, next.id);
            assert_eq!(producer.peer_boot, f.pin.boot);
            assert_eq!(producer.journal, f.pin.evidence_journal);
            assert!(
                producer.cells.is_empty(),
                "new session still requires cell negotiation"
            );
            let fresh = Identity {
                principal: f.principal.clone(),
                session: next.id.clone(),
                terminal: None,
            };
            assert!(matches!(
                f.app.publish_evidence(
                    &fresh,
                    EvidenceBatch {
                        journal: f.pin.evidence_journal.clone(),
                        first: Counter(1),
                        records: vec![]
                    }
                ),
                Err(StoreError::Rejected(Rejection::CapabilityMissing))
            ));
            let old = Identity {
                session: f.old_session.id.clone(),
                ..fresh
            };
            assert!(matches!(
                f.app.inspect_evidence_producer(&old),
                Err(StoreError::Rejected(Rejection::Unauthenticated))
            ));
            for row in &before {
                if row.document.schema.as_str() == "rx.internal.cell.v1" {
                    let cell: Cell = p::decode(row, "rx.internal.cell.v1").unwrap();
                    assert!(
                        cell.blocks
                            .iter()
                            .any(|b| b.reason == BlockReason::RuntimeRestart && b.latched)
                    );
                    assert_eq!(
                        cell.blocks
                            .iter()
                            .any(|b| b.reason == BlockReason::AuthorityRevoked),
                        graceful
                    );
                }
                if row.document.schema.as_str() == "rx.internal.host-registration.v1" {
                    let host: HostRegistration =
                        p::decode(row, "rx.internal.host-registration.v1").unwrap();
                    assert_eq!(
                        host.session, f.old_session.id,
                        "operating registration was not rebound"
                    );
                }
            }
            if with_work {
                assert!(
                    before
                        .iter()
                        .any(|r| r.document.schema.as_str() == "rx.internal.permit.v1")
                );
                assert!(
                    before
                        .iter()
                        .any(|r| r.document.schema.as_str() == "rx.internal.mandate.v1")
                );
            }
        }
    }
}

#[test]
fn changed_host_journal_authentication_or_session_validity_still_adds_device_restart() {
    for change in [
        "boot",
        "journal",
        "authentication",
        "clock",
        "inactive",
        "expired",
    ] {
        let mut f = restarted(1, false, false);
        let mut pin = f.pin.clone();
        match change {
            "boot" => pin.boot = id(),
            "journal" => pin.evidence_journal = id(),
            "authentication" => pin.authentication = Digest::from_bytes([99; 32]),
            "clock" => f.change_session(|s| s.expires_at.clock_id = "test/other-clock".into()),
            "inactive" => f.change_session(|s| s.active = false),
            "expired" => f.change_session(|s| s.expires_at.ticks_ns = Counter(1000)),
            _ => unreachable!(),
        }
        let before = f.device_restarts();
        f.reopen(&pin).unwrap();
        assert_eq!(f.device_restarts(), before + 1, "{change}");
    }
}

fn change_coverage(f: &mut Restarted, cell_id: &Name, kind: &str) {
    let current_boot = f.app.installation.runtime_boot.clone();
    f.repository
        .transact(|tx| {
            let (revision, mut cell): (_, Cell) =
                p::load(tx, "cell", cell_id, "rx.internal.cell.v1")?;
            let position = cell
                .blocks
                .iter()
                .position(|b| b.reason == BlockReason::RuntimeRestart)
                .unwrap();
            if kind == "configuration" {
                cell.configuration.site_config_digest = Digest::from_bytes([99; 32]);
            } else if kind == "scopes" {
                cell.scope_epochs.insert(name("zone/uncovered"), Counter(1));
            } else if kind == "removed" {
                cell.blocks.remove(position);
            } else {
                let mut value = origin::load(tx, &cell.blocks[position].id)?.unwrap();
                value.block.id = id();
                cell.blocks[position] = value.block.clone();
                match kind {
                    "missing" => {}
                    "previous_boot" => value.previous_runtime_boot = id(),
                    "current_boot" => {
                        value.runtime_boot = id();
                        assert_ne!(value.runtime_boot, current_boot);
                    }
                    "foreign_cell" => value.cell = name("cell/foreign"),
                    _ => unreachable!(),
                }
                if kind != "missing" {
                    p::save(
                        tx,
                        "runtimeinvalidationorigin",
                        &value.block.id,
                        None,
                        origin::SCHEMA,
                        &value,
                    )?;
                }
            }
            p::save(
                tx,
                "cell",
                cell_id,
                Some(revision),
                "rx.internal.cell.v1",
                &cell,
            )?;
            Ok(())
        })
        .unwrap();
}

#[test]
fn absent_or_wrong_boot_configuration_and_scope_coverage_cannot_suppress_invalidation() {
    for kind in [
        "missing",
        "removed",
        "previous_boot",
        "current_boot",
        "configuration",
        "scopes",
    ] {
        let mut f = restarted(1, false, false);
        change_coverage(&mut f, &name("cell/a"), kind);
        f.reopen(&f.pin.clone()).unwrap();
        assert_eq!(f.device_restarts(), 1, "{kind}");
    }
}

#[test]
fn current_origin_must_cover_every_registered_cell_of_the_host() {
    for missing in [false, true] {
        let mut f = restarted(2, false, false);
        if missing {
            change_coverage(&mut f, &name("cell/b"), "missing");
        }
        let before = f.authorities();
        f.reopen(&f.pin.clone()).unwrap();
        if missing {
            assert_eq!(
                f.device_restarts(),
                2,
                "all affected cells remain on the revocation path"
            );
        } else {
            assert_eq!(f.authorities(), before);
            assert_eq!(f.device_restarts(), 0);
        }
    }
}

#[test]
fn malformed_provenance_rolls_back_session_replacement_instead_of_claiming_runtime_only() {
    let mut f = restarted(1, false, false);
    change_coverage(&mut f, &name("cell/a"), "foreign_cell");
    let before = f.repository.snapshot().unwrap().1;
    assert!(matches!(
        f.reopen(&f.pin.clone()),
        Err(StoreError::Integrity(_))
    ));
    assert_eq!(f.repository.snapshot().unwrap().1, before);
}

#[test]
fn runtime_only_session_replacement_recovers_commit_loss_without_repeated_invalidation() {
    for mode in [1, 2] {
        let mut f = restarted(1, true, false);
        let before = f.authorities();
        f.failure.store(mode, Ordering::SeqCst);
        assert!(f.reopen(&f.pin.clone()).is_err());
        let next = f.reopen(&f.pin.clone()).unwrap();
        let repeated = f.reopen(&f.pin.clone()).unwrap();
        assert_eq!(next.id, repeated.id);
        assert_ne!(next.id, f.old_session.id);
        assert_eq!(f.authorities(), before);
        assert_eq!(f.device_restarts(), 0);
        let mut after = Counter(0);
        let mut opened = 0;
        loop {
            let events = f.repository.events_after(after, 128).unwrap();
            if events.is_empty() {
                break;
            }
            for event in events {
                assert!(event.seq > after);
                after = event.seq;
                if event.document.schema.as_str() == "rx.event.evidence-producer-opened.v1" {
                    opened += 1;
                }
            }
        }
        assert_eq!(opened, 2);
    }
}

#[test]
fn prior_source_generation_loss_remains_restricted_after_p_runtime_evidence_reconnect() {
    let changed_source = id();
    let evidence = id();
    let mut f = restarted_after(1, false, true, |f| {
        assert_ne!(
            f.registrations[0].source_sessions[&name("ready")],
            changed_source
        );
        let receipt = f
            .app
            .report_facts(
                &f.hosts[0],
                vec![FactRecord {
                    cell: f.configuration.id.clone(),
                    id: name("ready"),
                    source_host: f.hosts[0].principal.clone(),
                    source_generation: changed_source.clone(),
                    schema: name("boolean/v1"),
                    unit: name("unitless"),
                    acquired_at: expiry(1000),
                    maximum_age_ns: Counter(20000),
                    acquisition_uncertainty_ns: Counter(0),
                    quality_good: true,
                    origin_age_bounded: true,
                    disputed: false,
                    value: TypedValue::Boolean(true),
                    evidence_id: evidence.clone(),
                }],
            )
            .unwrap();
        assert_eq!(
            receipt.entries[0].disposition,
            rx_application::observation::Disposition::GenerationChanged
        );
        assert!(
            f.app
                .inspect_cell(&f.admin, &f.configuration.id)
                .unwrap()
                .1
                .blocks
                .iter()
                .any(|block| block.latched && block.reason == BlockReason::DeviceRestart)
        );
    });
    let before = f.authorities();
    let outbox = f.repository.pending_outbox(128).unwrap();
    let prior_losses = f.device_restarts();
    assert_eq!(prior_losses, 1);
    let fact = before
        .iter()
        .find(|record| record.document.schema.as_str() == "rx.internal.fact.v1")
        .unwrap();
    let fact: FactRecord = p::decode(fact, "rx.internal.fact.v1").unwrap();
    assert_eq!(fact.source_generation, changed_source);
    assert_eq!(fact.evidence_id, evidence);
    assert!(!fact.quality_good);
    let registration = before
        .iter()
        .find(|record| record.document.schema.as_str() == "rx.internal.host-registration.v1")
        .unwrap();
    let registration: HostRegistration =
        p::decode(registration, "rx.internal.host-registration.v1").unwrap();
    assert_ne!(registration.source_sessions[&name("ready")], changed_source);
    let session = f.reopen(&f.pin.clone()).unwrap();
    assert_ne!(session.id, f.old_session.id);
    assert_eq!(f.authorities(), before);
    assert_eq!(f.repository.pending_outbox(128).unwrap(), outbox);
    assert_eq!(f.device_restarts(), prior_losses);
}

#[test]
fn consecutive_p_restarts_preserve_historical_operating_session_and_each_runtime_block() {
    let mut f = restarted(1, false, false);
    let original_registration = f
        .authorities()
        .into_iter()
        .find(|r| r.document.schema.as_str() == "rx.internal.host-registration.v1")
        .unwrap();
    for restart in 1..=2 {
        let before = f.authorities();
        let next = f.reopen(&f.pin.clone()).unwrap();
        assert_eq!(f.authorities(), before);
        assert_eq!(f.device_restarts(), 0);
        assert!(f.authorities().contains(&original_registration));
        if restart == 1 {
            let installation = f.app.installation.id.clone();
            f.old_session = next;
            f.app = Engine::open(
                f.app.into_repository(),
                f.clock.clone(),
                SimulationAuthority,
                installation,
                principal("admin", &[Role::AccountAdmin]),
            )
            .unwrap();
        }
    }
}
