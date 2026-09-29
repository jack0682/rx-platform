use super::*;
use rx_application::{
    observation::Disposition,
    observation_link::{Read, Register},
};
use rx_domain::host_snapshot::{HostSnapshot, SourceObservation};
fn setup() -> (Fixture, Register) {
    let mut f = fixture_configured((2, false, false, false, None, false, false), |mut c| {
        c.steps.retain(|s| s.host == name("host/1"));
        c
    });
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
        resource_fences: BTreeMap::new(),
        captured_at: f.clock.now(),
        sources_available: true,
        observations: vec![SourceObservation {
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
        }],
        block_ids: vec![],
        pending_operations: vec![],
        pending_permits: vec![],
    };
    let template = host_link::Prepare {
        host: name("host/0"),
        platform_session: id(),
        snapshot,
        read_started: f.clock.now(),
        ttl_ms: Counter(1000),
        provenance: None,
    };
    let register = Register {
        host: template.host.clone(),
        platform_session: template.platform_session.clone(),
        read_started: template.read_started.clone(),
        provenance: host_binding_baseline_tests::provenance(&template),
    };
    (f, register)
}
fn read(link: &observation_link::Link, input: &Register) -> Read {
    Read {
        link: link.id.clone(),
        snapshot: input.provenance.host_read.clone(),
        read_started: input.read_started.clone(),
    }
}
#[test]
fn observer_registers_and_ingests_without_operating_registration_or_qualification() {
    let (mut f, input) = setup();
    let link = f.app.register_observation_link(input.clone()).unwrap();
    let original = input.provenance.host_read.observations[0].clone();
    let batch = f.app.ingest_observation_read(read(&link, &input)).unwrap();
    assert_eq!(batch.entries[0].disposition, Disposition::Current);
    assert_eq!(
        f.app
            .ingest_observation_read(read(&link, &input))
            .unwrap()
            .entries[0]
            .disposition,
        Disposition::Duplicate
    );
    let fact = f
        .app
        .inspect_fact(&f.operator, &f.configuration.id, &name("ready"))
        .unwrap();
    assert_eq!(fact.evidence_id, original.evidence_id);
    assert_eq!(fact.acquired_at, original.acquired_at);
    assert_eq!(fact.source_generation, original.generation);
    assert!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .qualification
            .is_none()
    );
    assert!(f.app.pending_deliveries(128).unwrap().is_empty());
    let d = f.app.overview(&f.operator).unwrap().cells[0]
        .diagnostics
        .clone();
    assert!(d.sources[0].usable);
    assert_eq!(
        d.hosts[0].context,
        rx_application::diagnostics::HostContext::ObservationOnly
    );
    assert_eq!(d.conditions[0].verdict, rx_domain::condition::Verdict::Pass);

    let mut repo = f.app.into_repository();
    repo.transact(|tx| {
        assert!(tx.scan("host/")?.is_empty());
        assert!(tx.scan("host-link-plan/")?.is_empty());
        assert!(tx.scan("permit/")?.is_empty());
        assert_eq!(tx.scan("observation-link/")?.len(), 1);
        Ok(())
    })
    .unwrap();
}
#[test]
fn observer_registration_recovers_transaction_loss_and_read_only_reconnect_without_rewriting_provenance()
 {
    for fault in [1, 2] {
        let (mut f, input) = setup();
        f.failure.store(fault, Ordering::SeqCst);
        assert!(f.app.register_observation_link(input.clone()).is_err());
        let link = f.app.register_observation_link(input.clone()).unwrap();
        let mut reconnect = input.clone();
        reconnect.platform_session = id();
        reconnect.provenance.host_read.observations[0].evidence_id = id();
        let reused = f.app.register_observation_link(reconnect).unwrap();
        assert_eq!(reused.id, link.id);
        assert_eq!(reused.platform_session, link.platform_session);
        assert_eq!(
            reused.provenance.host_read.observations[0].evidence_id,
            input.provenance.host_read.observations[0].evidence_id
        );
        let mut repo = f.app.into_repository();
        repo.transact(|tx| {
            assert_eq!(tx.scan("observation-link/")?.len(), 1);
            Ok(())
        })
        .unwrap();
    }
}
#[test]
fn observer_rejects_wrong_identity_scope_pin_context_or_stale_registration_before_writes() {
    for mutation in [
        "host",
        "boot",
        "delivery",
        "definition",
        "schema",
        "unit",
        "missing",
        "resource",
        "pending",
        "stale",
        "unbounded",
        "bad_pin",
        "wrong_proof",
    ] {
        let (mut f, mut input) = setup();
        match mutation {
            "host" => input.host = name("host/1"),
            "boot" => input.provenance.host_read.host_boot = id(),
            "delivery" => input.provenance.configuration.snapshot.delivery_journal = id(),
            "definition" => input.provenance.host_read.definition = Digest::from_bytes([99; 32]),
            "schema" => input.provenance.host_read.observations[0].schema = name("other/v1"),
            "unit" => input.provenance.host_read.observations[0].unit = name("mm"),
            "missing" => input.provenance.host_read.observations.clear(),
            "resource" => {
                input
                    .provenance
                    .host_read
                    .resource_fences
                    .insert(name("controller/1"), Counter(1));
            }
            "pending" => input.provenance.host_read.pending_operations.push(id()),
            "stale" => {
                f.clock.0.store(30_000, Ordering::SeqCst);
                input.provenance.host_read.captured_at = f.clock.now();
            }
            "unbounded" => input.provenance.host_read.observations[0].origin_age_bounded = false,
            "bad_pin" => {
                input.provenance.transport.host_read_binding = Digest::from_bytes([99; 32])
            }
            "wrong_proof" => input.provenance.configuration.snapshot.host = name("host/1"),
            _ => unreachable!(),
        }
        assert!(
            f.app.register_observation_link(input).is_err(),
            "{mutation}"
        );
        let mut repo = f.app.into_repository();
        repo.transact(|tx| {
            assert!(tx.scan("observation-link/")?.is_empty());
            assert!(tx.scan("fact/")?.is_empty());
            Ok(())
        })
        .unwrap();
    }
    let (mut f, input) = link_fixture();
    let proof = host_binding_baseline_tests::provenance(&input);
    assert!(
        f.app
            .register_observation_link(Register {
                host: input.host,
                platform_session: input.platform_session,
                read_started: input.read_started,
                provenance: proof
            })
            .is_err()
    );
}
#[test]
fn observer_generation_loss_invalidates_and_does_not_silently_adopt_or_reissue_authority() {
    let (mut f, input) = setup();
    let link = f.app.register_observation_link(input.clone()).unwrap();
    f.app.ingest_observation_read(read(&link, &input)).unwrap();
    let before = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1
        .epoch;
    let mut changed = read(&link, &input);
    changed.snapshot.observations[0].generation = id();
    changed.snapshot.observations[0].evidence_id = id();
    assert_eq!(
        f.app
            .ingest_observation_read(changed.clone())
            .unwrap()
            .entries[0]
            .disposition,
        Disposition::GenerationChanged
    );
    let after = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1
        .epoch;
    assert!(after > before);
    f.app.ingest_observation_read(changed.clone()).unwrap();
    assert_eq!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch,
        after
    );
    let mut next = input.clone();
    next.provenance.host_read = changed.snapshot;
    assert!(f.app.register_observation_link(next).is_err());
    let fact = f
        .app
        .inspect_fact(&f.operator, &f.configuration.id, &name("ready"))
        .unwrap();
    assert!(!fact.quality_good);
}
#[test]
fn observer_producer_restart_invalidates_immediately_and_old_link_cannot_ingest() {
    let (mut f, input) = setup();
    let link = f.app.register_observation_link(input.clone()).unwrap();
    f.app.ingest_observation_read(read(&link, &input)).unwrap();
    let before = f
        .app
        .inspect_cell(&f.operator, &f.configuration.id)
        .unwrap()
        .1
        .epoch;
    f.app
        .open_evidence_producer(&input.host, id(), id(), Digest::from_bytes([81; 32]))
        .unwrap();
    assert!(
        f.app
            .inspect_cell(&f.operator, &f.configuration.id)
            .unwrap()
            .1
            .epoch
            > before
    );
    assert!(f.app.ingest_observation_read(read(&link, &input)).is_err());
    assert!(f.app.register_observation_link(input).is_err());
    assert!(
        !f.app.overview(&f.operator).unwrap().cells[0]
            .diagnostics
            .sources[0]
            .usable
    );
}
#[test]
fn observer_stale_value_remains_old_and_wrong_reads_do_not_overwrite_it() {
    let (mut f, input) = setup();
    let link = f.app.register_observation_link(input.clone()).unwrap();
    f.app.ingest_observation_read(read(&link, &input)).unwrap();
    f.clock.0.store(30_000, Ordering::SeqCst);
    let mut stale = read(&link, &input);
    stale.read_started = f.clock.now();
    stale.snapshot.captured_at = f.clock.now();
    f.app.ingest_observation_read(stale.clone()).unwrap();
    assert_eq!(
        f.app
            .inspect_fact(&f.operator, &f.configuration.id, &name("ready"))
            .unwrap()
            .acquired_at
            .ticks_ns,
        Counter(1000)
    );
    assert!(
        !f.app.overview(&f.operator).unwrap().cells[0]
            .diagnostics
            .sources[0]
            .usable
    );
    for mutation in ["cell", "boot", "delivery", "schema", "future", "missing"] {
        let mut bad = stale.clone();
        match mutation {
            "cell" => bad.snapshot.cell = name("cell/b"),
            "boot" => bad.snapshot.host_boot = id(),
            "delivery" => bad.snapshot.delivery_journal = id(),
            "schema" => bad.snapshot.observations[0].schema = name("other/v1"),
            "future" => bad.snapshot.observations[0].acquired_at.ticks_ns = Counter(99_000),
            "missing" => bad.snapshot.observations.clear(),
            _ => unreachable!(),
        }
        assert!(f.app.ingest_observation_read(bad).is_err(), "{mutation}");
    }
}

#[test]
fn observer_and_operating_registration_paths_cannot_silently_promote_each_other() {
    let (mut f, input) = setup();
    let control = host_link::Prepare {
        host: input.host.clone(),
        platform_session: input.platform_session.clone(),
        snapshot: input.provenance.host_read.clone(),
        read_started: input.read_started.clone(),
        ttl_ms: Counter(1000),
        provenance: Some(input.provenance.clone()),
    };
    f.app.register_observation_link(input.clone()).unwrap();
    assert!(f.app.prepare_host_link(control.clone()).is_err());
    let snapshot = &input.provenance.host_read;
    let registration = HostRegistration {
        id: input.host.clone(),
        cell: snapshot.cell.clone(),
        boot_id: snapshot.host_boot.clone(),
        session: f.hosts[0].session.clone(),
        delivery_journal: snapshot.delivery_journal.clone(),
        epoch: snapshot.epoch,
        scopes: snapshot.scopes.clone(),
        source_sessions: snapshot
            .observations
            .iter()
            .map(|s| (s.source.clone(), s.generation.clone()))
            .collect(),
        grant: Grant {
            id: id(),
            owner: Name::new(f.app.installation.id.as_str()).unwrap(),
            resources: vec![],
            fence: Counter(1),
            valid_until: expiry(90_000),
            ttl_ms: Counter(1000),
        },
    };
    assert!(f.app.register_host(&f.hosts[0], registration).is_err());
    let (mut other, input) = setup();
    let control = host_link::Prepare {
        host: input.host.clone(),
        platform_session: input.platform_session.clone(),
        snapshot: input.provenance.host_read.clone(),
        read_started: input.read_started.clone(),
        ttl_ms: Counter(1000),
        provenance: Some(input.provenance.clone()),
    };
    other.app.prepare_host_link(control).unwrap();
    assert!(other.app.register_observation_link(input).is_err());
}
#[test]
fn observer_revoked_identity_cannot_supply_usable_conditions_or_ingest() {
    let (mut f, input) = setup();
    let link = f.app.register_observation_link(input.clone()).unwrap();
    f.app.ingest_observation_read(read(&link, &input)).unwrap();
    assert!(
        f.app.overview(&f.operator).unwrap().cells[0]
            .diagnostics
            .sources[0]
            .usable
    );
    let mut host = principal("host/0", &[Role::Host]);
    host.active = false;
    f.app
        .put_principal(&f.admin, host, Some(Counter(1)))
        .unwrap();
    assert!(f.app.ingest_observation_read(read(&link, &input)).is_err());
    let d = f.app.overview(&f.operator).unwrap().cells[0]
        .diagnostics
        .clone();
    assert!(!d.sources[0].usable);
    assert_eq!(
        d.conditions[0].verdict,
        rx_domain::condition::Verdict::Unknown
    );
}
