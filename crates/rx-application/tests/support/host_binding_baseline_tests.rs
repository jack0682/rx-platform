use super::*;
use rx_application::host_binding_baseline::{self as baseline, BootstrapProvenance, TransportPin};
use rx_domain::{canonical, host_configuration as config};

fn manifest(bytes: &[u8]) -> Digest {
    let value: serde_json::Value = canonical::decode_json(bytes).unwrap();
    rx_package::content_digest(&canonical::bytes(&value).unwrap())
}
fn transport() -> TransportPin {
    let configuration: serde_json::Value = canonical::decode_json(include_bytes!(
        "../../../../spec/host-configuration/v1/binding.json"
    ))
    .unwrap();
    TransportPin {
        uri: "https://host.example:7443".into(),
        server_name: "host.example".into(),
        server_ca_digest: Digest::from_bytes([11; 32]),
        server_leaf_digest: Digest::from_bytes([12; 32]),
        platform_client_leaf_digest: Digest::from_bytes([13; 32]),
        platform_client_chain_digest: Digest::from_bytes([14; 32]),
        release: Digest::from_bytes([15; 32]),
        base_manifest: manifest(include_bytes!(
            "../../../../spec/contracts/v1.0/protocol_manifest.json"
        )),
        cell_manifest: manifest(include_bytes!(
            "../../../../spec/cell_operations/v1.0/protocol_manifest.json"
        )),
        host_read_binding: manifest(include_bytes!("../../../../spec/host-read/v1/binding.json")),
        host_configuration_binding: canonical::digest(
            "RX-HOST-CONFIGURATION-BINDING-v1",
            &configuration,
        )
        .unwrap(),
    }
}
fn provenance(input: &host_link::Prepare) -> BootstrapProvenance {
    let read = &input.snapshot;
    BootstrapProvenance {
        transport: transport(),
        configuration: config::Observation {
            schema: name("rx.host-process-configuration-observation.v1"),
            snapshot: config::Snapshot {
                schema: name("rx.host-process-configuration-snapshot.v1"),
                host: read.host.clone(),
                host_boot: read.host_boot.clone(),
                delivery_journal: read.delivery_journal.clone(),
                binding_digest: Digest::from_bytes([16; 32]),
                cells: vec![config::CellObservation {
                    cell: read.cell.clone(),
                    definition: read.definition,
                    envelope: read.envelope,
                    environment: read.environment.clone(),
                    epoch: read.epoch,
                    scopes: read.scopes.clone(),
                    blocked: read.block_ids.clone(),
                    applied: None,
                }],
            },
            receipt: None,
            context_matches_current_host: false,
            activation_authorized: false,
        },
        configuration_read_started: input.read_started.clone(),
        configuration_read_finished: input.read_started.clone(),
        host_read: read.clone(),
    }
}
fn fixture_with_provenance() -> (Fixture, host_link::Prepare) {
    let (f, mut input) = link_fixture();
    input.provenance = Some(provenance(&input));
    (f, input)
}

#[test]
fn only_new_normal_commit_creates_exact_immutable_baseline_and_initial_applied_none_is_valid() {
    let (mut f, input) = fixture_with_provenance();
    let plan = f.app.prepare_host_link(input.clone()).unwrap();
    assert!(f.app.host_binding_baseline(&plan.id).unwrap().is_none());
    let command = link_commit(&f, &plan);
    let registration = f.app.commit_host_link(command.clone()).unwrap();
    let saved = f.app.host_binding_baseline(&plan.id).unwrap().unwrap();
    let bound_plan = f.app.prepare_host_link(input.clone()).unwrap();
    assert_eq!(
        saved.original_plan_digest,
        canonical::digest("RX-HOST-LINK-BOUND-PLAN-v1", &bound_plan).unwrap()
    );
    assert_eq!(
        saved.original_registration_digest,
        canonical::digest("RX-HOST-LINK-REGISTRATION-v1", &registration).unwrap()
    );
    assert_eq!(
        saved.original_configuration_digest,
        runtime_invalidation::configuration_digest(&f.configuration).unwrap()
    );
    assert_eq!(saved.installation, f.app.installation.id);
    assert_eq!(saved.store_generation, f.app.installation.store_generation);
    assert_eq!(saved.runtime_boot, f.app.installation.runtime_boot);
    assert_eq!(
        saved.producer_authentication_binding,
        Digest::from_bytes([81; 32])
    );
    assert_eq!(saved.producer_session, plan.producer_session);
    assert_eq!(saved.platform_session, plan.platform_session);
    assert_eq!(
        saved.transport,
        input.provenance.as_ref().unwrap().transport
    );
    assert_eq!(saved.transport_digest, saved.transport.digest().unwrap());
    assert!(saved.applied_context.is_none());
    assert_eq!(saved.source_sessions, plan.source_sessions);
    let source = &input.snapshot.observations[0];
    assert_eq!(
        saved.source_references[&source.source].evidence,
        source.evidence_id
    );
    assert_eq!(
        saved.host_read.sha256,
        rx_package::content_digest(&canonical::bytes(&input.snapshot).unwrap())
    );
    let stable = saved.stable_identity();
    let before = saved.digest().unwrap();
    for _ in 0..2 {
        f.app.commit_host_link(command.clone()).unwrap();
        let read = f.app.host_binding_baseline(&plan.id).unwrap().unwrap();
        assert_eq!(read.digest().unwrap(), before);
        assert_eq!(read.stable_identity(), stable);
    }
    let mut repository = f.app.into_repository();
    repository
        .transact(|tx| {
            let rows = tx.scan(&format!("{}/", baseline::PREFIX))?;
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].revision, Counter(1));
            let sources = tx.scan(&format!("{}/", baseline::SOURCE_PREFIX))?;
            assert_eq!(sources.len(), 1);
            assert_eq!(sources[0].revision, Counter(1));
            Ok(())
        })
        .unwrap();
}

#[test]
fn baseline_bound_cut_uses_fence_while_pre_fence_config_and_read_keep_their_lower_epoch() {
    let (mut f, input) = fixture_with_provenance();
    f.app
        .hold(&f.operator, id().as_str(), &f.configuration.id)
        .unwrap();
    let plan = f.app.prepare_host_link(input.clone()).unwrap();
    assert!(plan.epoch > input.snapshot.epoch);
    let command = link_commit(&f, &plan);
    f.app.commit_host_link(command).unwrap();
    let saved = f.app.host_binding_baseline(&plan.id).unwrap().unwrap();
    assert_eq!(saved.epoch, plan.epoch);
    assert_eq!(saved.scopes, plan.scopes);
    assert_eq!(
        saved.host_read.sha256,
        rx_package::content_digest(&canonical::bytes(&input.snapshot).unwrap())
    );
}

#[test]
fn exact_initial_applied_context_is_preserved_without_claiming_activation() {
    let (mut f, mut input) = fixture_with_provenance();
    let proof = input.provenance.as_mut().unwrap();
    let applied = config::AppliedContext {
        cell: input.snapshot.cell.clone(),
        configuration: runtime_invalidation::configuration_digest(&f.configuration).unwrap(),
        change: id(),
        request: id(),
        receipt_sequence: Counter(7),
        binding_digest: proof.configuration.snapshot.binding_digest,
    };
    proof.configuration.snapshot.cells[0].applied = Some(applied.clone());
    let plan = f.app.prepare_host_link(input).unwrap();
    let command = link_commit(&f, &plan);
    f.app.commit_host_link(command).unwrap();
    let saved = f.app.host_binding_baseline(&plan.id).unwrap().unwrap();
    assert_eq!(
        canonical::bytes(saved.applied_context.as_ref().unwrap()).unwrap(),
        canonical::bytes(&applied).unwrap()
    );
}

#[test]
fn legacy_none_and_already_bound_missing_baseline_never_backfill_on_later_provenance() {
    let (mut f, mut input) = link_fixture();
    let plan = f.app.prepare_host_link(input.clone()).unwrap();
    assert!(plan.provenance.is_none());
    let command = link_commit(&f, &plan);
    f.app.commit_host_link(command.clone()).unwrap();
    assert!(f.app.host_binding_baseline(&plan.id).unwrap().is_none());
    input.provenance = Some(provenance(&input));
    let repeated = f.app.prepare_host_link(input).unwrap();
    assert_eq!(repeated.id, plan.id);
    assert!(repeated.provenance.is_none());
    f.app.commit_host_link(command).unwrap();
    assert!(f.app.host_binding_baseline(&plan.id).unwrap().is_none());
    let bytes = canonical::bytes(&plan).unwrap();
    assert!(
        !String::from_utf8(bytes.clone())
            .unwrap()
            .contains("provenance")
    );
    let restored: host_link::Plan = canonical::decode_json(&bytes).unwrap();
    assert!(restored.provenance.is_none());
}

#[test]
fn baseline_and_bound_transition_are_atomic_across_rollback_and_commit_response_loss() {
    for failure in [1, 2] {
        let (mut f, input) = fixture_with_provenance();
        let plan = f.app.prepare_host_link(input.clone()).unwrap();
        let command = link_commit(&f, &plan);
        let before_cell = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap();
        f.failure.store(failure, Ordering::SeqCst);
        assert!(f.app.commit_host_link(command.clone()).is_err());
        let saved = f.app.host_binding_baseline(&plan.id).unwrap();
        if failure == 1 {
            assert!(saved.is_none());
            assert!(f.app.bound_host_link(&plan.id).is_err());
            let pending = f.app.prepare_host_link(input).unwrap();
            assert_eq!(
                canonical::bytes(&pending).unwrap(),
                canonical::bytes(&plan).unwrap()
            );
        } else {
            assert!(saved.is_some());
            assert!(f.app.bound_host_link(&plan.id).is_ok());
        }
        assert_eq!(
            canonical::bytes(&f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap()).unwrap(),
            canonical::bytes(&before_cell).unwrap()
        );
        f.app.commit_host_link(command.clone()).unwrap();
        let original = f
            .app
            .host_binding_baseline(&plan.id)
            .unwrap()
            .unwrap()
            .digest()
            .unwrap();
        f.app.commit_host_link(command).unwrap();
        assert_eq!(
            f.app
                .host_binding_baseline(&plan.id)
                .unwrap()
                .unwrap()
                .digest()
                .unwrap(),
            original
        );
    }
}

#[test]
fn commit_body_and_reusable_transport_pin_changes_cannot_replace_the_original_baseline() {
    let (mut f, input) = fixture_with_provenance();
    let plan = f.app.prepare_host_link(input.clone()).unwrap();
    let command = link_commit(&f, &plan);
    f.app.commit_host_link(command.clone()).unwrap();
    let original = f
        .app
        .host_binding_baseline(&plan.id)
        .unwrap()
        .unwrap()
        .digest()
        .unwrap();
    let mut changed = command;
    changed.fence_receipt.sequence = Counter(99);
    assert!(matches!(
        f.app.commit_host_link(changed),
        Err(StoreError::KeyConflict)
    ));
    let mutations: &[fn(&mut TransportPin)] = &[
        |pin| pin.uri = "https://other.example:7443".into(),
        |pin| pin.server_name = "other.example".into(),
        |pin| pin.server_ca_digest = Digest::from_bytes([99; 32]),
        |pin| pin.server_leaf_digest = Digest::from_bytes([99; 32]),
        |pin| pin.platform_client_leaf_digest = Digest::from_bytes([99; 32]),
        |pin| pin.platform_client_chain_digest = Digest::from_bytes([99; 32]),
        |pin| pin.release = Digest::from_bytes([99; 32]),
    ];
    for mutate in mutations {
        let mut next = input.clone();
        mutate(&mut next.provenance.as_mut().unwrap().transport);
        assert!(matches!(
            f.app.prepare_host_link(next),
            Err(StoreError::KeyConflict)
        ));
        assert_eq!(
            f.app
                .host_binding_baseline(&plan.id)
                .unwrap()
                .unwrap()
                .digest()
                .unwrap(),
            original
        );
    }
}

#[test]
fn mismatched_actual_configuration_read_scope_and_time_never_prepare_a_baseline_plan() {
    let mutations: &[fn(&mut BootstrapProvenance)] = &[
        |p| p.configuration.snapshot.host = name("host/foreign"),
        |p| p.configuration.snapshot.host_boot = id(),
        |p| p.configuration.snapshot.delivery_journal = id(),
        |p| p.configuration.snapshot.cells[0].cell = name("cell/foreign"),
        |p| p.configuration.snapshot.cells[0].definition = Digest::from_bytes([99; 32]),
        |p| p.configuration.snapshot.cells[0].envelope = Digest::from_bytes([99; 32]),
        |p| p.configuration.snapshot.cells[0].epoch = Counter(99),
        |p| {
            *p.configuration.snapshot.cells[0]
                .scopes
                .values_mut()
                .next()
                .unwrap() = Counter(99)
        },
        |p| p.configuration_read_finished.ticks_ns = Counter(100_001_001),
        |p| p.configuration_read_started.clock_id = "test/other".into(),
        |p| p.host_read.observations[0].evidence_id = id(),
        |p| p.transport.host_read_binding = Digest::from_bytes([99; 32]),
        |p| p.transport.host_configuration_binding = Digest::from_bytes([99; 32]),
    ];
    for mutate in mutations {
        let (mut f, mut input) = fixture_with_provenance();
        mutate(input.provenance.as_mut().unwrap());
        assert!(f.app.prepare_host_link(input).is_err());
        let mut repository = f.app.into_repository();
        repository
            .transact(|tx| {
                assert!(tx.scan("host-link-plan/")?.is_empty());
                assert!(tx.scan(&format!("{}/", baseline::PREFIX))?.is_empty());
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn uri_pin_accepts_only_public_https_origin_without_credentials_or_request_data() {
    for uri in [
        "http://host.example",
        "https://",
        "https://user@host.example",
        "https://host.example?token=example",
        "https://host.example/#fragment",
        "https://host.example/private-path",
        "https://host.example/path/",
        "https://host.example\n",
        "https://host.example\\path",
    ] {
        let mut pin = transport();
        pin.uri = uri.into();
        assert!(pin.validate().is_err(), "{uri:?}");
    }
    for uri in ["https://host.example", "https://host.example/"] {
        let mut pin = transport();
        pin.uri = uri.into();
        assert!(pin.validate().is_ok());
    }
}

#[test]
fn historical_baseline_survives_renewal_and_later_registration_epoch_changes() {
    let (mut f, input) = fixture_with_provenance();
    let plan = f.app.prepare_host_link(input).unwrap();
    let command = link_commit(&f, &plan);
    f.app.commit_host_link(command).unwrap();
    let original = f.app.host_binding_baseline(&plan.id).unwrap().unwrap();
    f.clock.0.store(500_001_000, Ordering::SeqCst);
    let renewal = f.app.prepare_host_renewal(&plan.id).unwrap();
    let mut renewed = renewal.grant.clone();
    renewed.valid_until.ticks_ns =
        Counter(renewal.sent_at.ticks_ns.0 + renewal.grant.ttl_ms.0 * 1_000_000);
    let mut registration = f.app.commit_host_renewal(renewal, renewed).unwrap();
    assert_ne!(
        original.original_registration_digest,
        canonical::digest("RX-HOST-LINK-REGISTRATION-v1", &registration).unwrap()
    );
    assert_eq!(
        f.app
            .host_binding_baseline(&plan.id)
            .unwrap()
            .unwrap()
            .digest()
            .unwrap(),
        original.digest().unwrap()
    );
    // Use a fresh authenticated operator session because this fixture's terminal login
    // is intentionally short lived. The later registration projection is not history.
    f.operator.session = f
        .app
        .authenticated_terminal_user_session(
            &f.operator.principal,
            id(),
            Counter(99_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap()
        .id;
    f.app
        .hold(&f.operator, id().as_str(), &f.configuration.id)
        .unwrap();
    let (_, cell) = f
        .app
        .inspect_cell(&f.hosts[0], &f.configuration.id)
        .unwrap();
    registration.epoch = cell.epoch;
    registration.scopes = cell.scope_epochs;
    f.app.register_host(&f.hosts[0], registration).unwrap();
    assert_eq!(
        f.app
            .host_binding_baseline(&plan.id)
            .unwrap()
            .unwrap()
            .digest()
            .unwrap(),
        original.digest().unwrap()
    );
}

#[test]
fn known_pinned_plan_cannot_downgrade_to_none_even_after_its_expiry() {
    for bound in [false, true] {
        for expired in [false, true] {
            let (mut f, mut input) = fixture_with_provenance();
            let plan = f.app.prepare_host_link(input.clone()).unwrap();
            if bound {
                let command = link_commit(&f, &plan);
                f.app.commit_host_link(command).unwrap();
            }
            let original = f
                .app
                .host_binding_baseline(&plan.id)
                .unwrap()
                .map(|b| b.digest().unwrap());
            if expired {
                f.clock.0.store(2_000_000_000, Ordering::SeqCst);
            }
            input.read_started = f.clock.now();
            input.snapshot.captured_at = f.clock.now();
            input.provenance = None;
            assert!(matches!(
                f.app.prepare_host_link(input),
                Err(StoreError::KeyConflict)
            ));
            assert_eq!(
                f.app
                    .host_binding_baseline(&plan.id)
                    .unwrap()
                    .map(|b| b.digest().unwrap()),
                original
            );
        }
    }
}

// Read-only storage fault injection preserves row revision 1 so content/reference
// verification is exercised independently of the immutable-revision check.
struct ReadOverride<'a> {
    inner: &'a mut dyn rx_ports::Transaction,
    key: Name,
    replacement: Option<rx_ports::Document>,
}
impl rx_ports::Transaction for ReadOverride<'_> {
    fn get(&mut self, key: &Name) -> rx_ports::Result<Option<Record>> {
        if key != &self.key {
            return self.inner.get(key);
        }
        let Some(document) = &self.replacement else {
            return Ok(None);
        };
        Ok(self.inner.get(key)?.map(|mut record| {
            record.document = document.clone();
            record
        }))
    }
    fn append_control(
        &mut self,
        event: &Id,
        entity: &Record,
        document: &rx_ports::Document,
    ) -> rx_ports::Result<Counter> {
        self.inner.append_control(event, entity, document)
    }
    fn control_head(&mut self) -> rx_ports::Result<Counter> {
        self.inner.control_head()
    }
    fn outbox(&mut self, id: &Id) -> rx_ports::Result<Option<rx_ports::OutboxRecord>> {
        self.inner.outbox(id)
    }
    fn scan(&mut self, prefix: &str) -> rx_ports::Result<Vec<Record>> {
        self.inner.scan(prefix)
    }
    fn put(
        &mut self,
        key: &Name,
        expected: Option<Counter>,
        document: &rx_ports::Document,
    ) -> rx_ports::Result<Record> {
        self.inner.put(key, expected, document)
    }
    fn lookup(
        &mut self,
        scope: &rx_ports::RequestScope,
    ) -> rx_ports::Result<Option<rx_ports::SavedRequest>> {
        self.inner.lookup(scope)
    }
    fn remember(
        &mut self,
        scope: &rx_ports::RequestScope,
        request: &rx_ports::SavedRequest,
    ) -> rx_ports::Result<()> {
        self.inner.remember(scope, request)
    }
    fn append(&mut self, id: &Id, document: &rx_ports::Document) -> rx_ports::Result<Counter> {
        self.inner.append(id, document)
    }
    fn enqueue(&mut self, id: &Id, document: &rx_ports::Document) -> rx_ports::Result<()> {
        self.inner.enqueue(id, document)
    }
    fn transition_outbox(
        &mut self,
        id: &Id,
        expected: rx_ports::OutboxState,
        next: rx_ports::OutboxState,
    ) -> rx_ports::Result<()> {
        self.inner.transition_outbox(id, expected, next)
    }
}

#[test]
fn missing_or_tampered_original_objects_and_reference_metadata_fail_historical_read() {
    use rx_application::persistence as p;
    let (mut f, input) = fixture_with_provenance();
    let plan = f.app.prepare_host_link(input).unwrap();
    let command = link_commit(&f, &plan);
    f.app.commit_host_link(command).unwrap();
    let saved = f.app.host_binding_baseline(&plan.id).unwrap().unwrap();
    let mut repository = f.app.into_repository();
    repository
        .transact(|tx| {
            let source_key = p::key(baseline::SOURCE_PREFIX, &plan.id);
            let original_source: baseline::BaselineSource =
                p::decode(&tx.get(&source_key)?.unwrap(), baseline::SOURCE_SCHEMA)?;
            {
                let mut missing = ReadOverride {
                    inner: tx,
                    key: source_key.clone(),
                    replacement: None,
                };
                assert!(baseline::load(&mut missing, &plan.id).is_err());
            }
            let changes: &[fn(&mut baseline::BaselineSource)] = &[
                |s| s.producer.cells.clear(),
                |s| s.installation.clock_id = "test/changed-source-clock".into(),
                |s| {
                    s.plan.provenance.as_mut().unwrap().host_read.observations[0].value =
                        TypedValue::Boolean(false)
                },
                |s| {
                    s.plan
                        .provenance
                        .as_mut()
                        .unwrap()
                        .configuration
                        .snapshot
                        .binding_digest = Digest::from_bytes([99; 32])
                },
                |s| s.configuration.site_config_digest = Digest::from_bytes([99; 32]),
                |s| s.plan.valid_until.ticks_ns = Counter(999),
                |s| s.receipt.registration.grant.fence = Counter(999),
            ];
            for change in changes {
                let mut altered = original_source.clone();
                change(&mut altered);
                assert!(saved.verify_source(&altered).is_err());
                let mut corrupted = ReadOverride {
                    inner: tx,
                    key: source_key.clone(),
                    replacement: Some(p::doc(baseline::SOURCE_SCHEMA, &altered)?),
                };
                assert!(baseline::load(&mut corrupted, &plan.id).is_err());
            }
            for hash in [false, true] {
                let mut altered = saved.clone();
                if hash {
                    altered.host_read.sha256 = Digest::from_bytes([99; 32]);
                } else {
                    altered.configuration_read.size_bytes.0 += 1;
                }
                let mut corrupted = ReadOverride {
                    inner: tx,
                    key: p::key(baseline::PREFIX, &plan.id),
                    replacement: Some(p::doc(baseline::SCHEMA, &altered)?),
                };
                assert!(baseline::load(&mut corrupted, &plan.id).is_err());
            }
            for hash in [false, true] {
                let mut altered = saved.clone();
                if hash {
                    altered.source.sha256 = Digest::from_bytes([99; 32]);
                } else {
                    altered.source.size_bytes.0 += 1;
                }
                let mut corrupted = ReadOverride {
                    inner: tx,
                    key: p::key(baseline::PREFIX, &plan.id),
                    replacement: Some(p::doc(baseline::SCHEMA, &altered)?),
                };
                assert!(baseline::load(&mut corrupted, &plan.id).is_err());
            }
            let receipt_key = p::key("host-link-receipt", &plan.id);
            let mut receipt = original_source.receipt;
            receipt.request_digest = Digest::from_bytes([99; 32]);
            {
                let mut corrupted = ReadOverride {
                    inner: tx,
                    key: receipt_key,
                    replacement: Some(p::doc("rx.internal.host-link-receipt.v1", &receipt)?),
                };
                assert!(baseline::load(&mut corrupted, &plan.id).is_err());
            }
            assert_eq!(
                baseline::load(tx, &plan.id)?.unwrap().digest().unwrap(),
                saved.digest().unwrap()
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn missing_primary_baseline_is_integrity_when_pinned_bound_history_survives() {
    use rx_application::persistence as p;
    let (mut f, input) = fixture_with_provenance();
    assert!(f.app.host_binding_baseline(&id()).unwrap().is_none());
    let plan = f.app.prepare_host_link(input).unwrap();
    assert!(f.app.host_binding_baseline(&plan.id).unwrap().is_none());
    let command = link_commit(&f, &plan);
    f.app.commit_host_link(command).unwrap();
    let saved = f.app.host_binding_baseline(&plan.id).unwrap().unwrap();
    let mut repository = f.app.into_repository();
    repository
        .transact(|tx| {
            let mut missing = ReadOverride {
                inner: tx,
                key: p::key(baseline::PREFIX, &plan.id),
                replacement: None,
            };
            assert!(matches!(
                baseline::load(&mut missing, &plan.id),
                Err(StoreError::Integrity(_))
            ));
            {
                // Even if both immutable rows disappear, the known pinned bound Plan
                // must not be silently reclassified as a legacy None-provenance binding.
                let mut no_source = ReadOverride {
                    inner: &mut missing,
                    key: p::key(baseline::SOURCE_PREFIX, &plan.id),
                    replacement: None,
                };
                assert!(matches!(
                    baseline::load(&mut no_source, &plan.id),
                    Err(StoreError::Integrity(_))
                ));
                let mut no_plan = ReadOverride {
                    inner: &mut no_source,
                    key: p::key("host-link-plan", &plan.id),
                    replacement: None,
                };
                assert!(matches!(
                    baseline::load(&mut no_plan, &plan.id),
                    Err(StoreError::Integrity(_))
                ));
            }
            assert_eq!(
                baseline::load(missing.inner, &plan.id)?
                    .unwrap()
                    .digest()
                    .unwrap(),
                saved.digest().unwrap()
            );
            Ok(())
        })
        .unwrap();
}
