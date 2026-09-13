use super::*;
use rx_application::{
    host_binding_baseline as baseline, host_recovery as recovery, persistence as p,
};
use rx_domain::{canonical, host_configuration as hc};
use std::sync::Mutex;

#[derive(Clone)]
struct Shared(Arc<Mutex<FaultRepository>>, Arc<AtomicU8>, Arc<AtomicU8>);
impl Repository for Shared {
    fn transact<T>(
        &mut self,
        f: impl FnOnce(&mut dyn rx_ports::Transaction) -> rx_ports::Result<T>,
    ) -> rx_ports::Result<T> {
        let mut repository = self.0.lock().unwrap();
        if self.1.load(Ordering::SeqCst) > 0 && self.1.fetch_sub(1, Ordering::SeqCst) == 1 {
            repository
                .mode
                .store(self.2.load(Ordering::SeqCst), Ordering::SeqCst);
        }
        repository.transact(f)
    }
    fn pending_outbox_after(
        &mut self,
        a: Option<&Id>,
        l: usize,
    ) -> rx_ports::Result<Vec<rx_ports::OutboxRecord>> {
        self.0.lock().unwrap().pending_outbox_after(a, l)
    }
    fn control_events_after(&mut self, a: Counter, l: usize) -> rx_ports::Result<Vec<StoredEvent>> {
        self.0.lock().unwrap().control_events_after(a, l)
    }
    fn control_snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<Record>)> {
        self.0.lock().unwrap().control_snapshot()
    }
    fn journal_head(&mut self) -> rx_ports::Result<Counter> {
        self.0.lock().unwrap().journal_head()
    }
    fn pending_outbox(&mut self, l: usize) -> rx_ports::Result<Vec<rx_ports::OutboxRecord>> {
        self.0.lock().unwrap().pending_outbox(l)
    }
    fn snapshot(&mut self) -> rx_ports::Result<(Counter, Vec<Record>)> {
        self.0.lock().unwrap().snapshot()
    }
    fn events_after(&mut self, a: Counter, l: usize) -> rx_ports::Result<Vec<StoredEvent>> {
        self.0.lock().unwrap().events_after(a, l)
    }
}
fn manifest(bytes: &[u8]) -> Digest {
    let v: serde_json::Value = canonical::decode_json(bytes).unwrap();
    rx_package::content_digest(&canonical::bytes(&v).unwrap())
}
fn pin() -> baseline::TransportPin {
    let configuration: serde_json::Value = canonical::decode_json(include_bytes!(
        "../../../../spec/host-configuration/v1/binding.json"
    ))
    .unwrap();
    baseline::TransportPin {
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
fn configuration_read(snapshot: &rx_domain::host_snapshot::HostSnapshot) -> hc::Observation {
    hc::Observation {
        schema: name("rx.host-process-configuration-observation.v1"),
        snapshot: hc::Snapshot {
            schema: name("rx.host-process-configuration-snapshot.v1"),
            host: snapshot.host.clone(),
            host_boot: snapshot.host_boot.clone(),
            delivery_journal: snapshot.delivery_journal.clone(),
            binding_digest: Digest::from_bytes([85; 32]),
            cells: vec![hc::CellObservation {
                cell: snapshot.cell.clone(),
                definition: snapshot.definition,
                envelope: snapshot.envelope,
                environment: snapshot.environment.clone(),
                epoch: snapshot.epoch,
                scopes: snapshot.scopes.clone(),
                blocked: snapshot.block_ids.clone(),
                applied: None,
            }],
        },
        receipt: None,
        context_matches_current_host: false,
        activation_authorized: false,
    }
}
fn proof(input: &host_link::Prepare) -> baseline::BootstrapProvenance {
    baseline::BootstrapProvenance {
        transport: pin(),
        configuration: configuration_read(&input.snapshot),
        configuration_read_started: input.read_started.clone(),
        configuration_read_finished: input.read_started.clone(),
        host_read: input.snapshot.clone(),
    }
}
struct Test {
    _directory: tempfile::TempDir,
    app: Engine<Shared, ManualClock, SimulationAuthority>,
    repository: Shared,
    clock: ManualClock,
    failure: Arc<AtomicU8>,
    actor: Identity,
    input: host_link::Prepare,
    platform_session: Id,
    applied: Option<hc::AppliedContext>,
}
fn test(cells: usize, with_work: bool, renew: bool) -> Test {
    test_with_baseline(cells, with_work, renew, true)
}
fn test_with_baseline(cells: usize, with_work: bool, renew: bool, with_baseline: bool) -> Test {
    test_complete(cells, with_work, renew, with_baseline, false)
}
fn test_complete(
    cells: usize,
    with_work: bool,
    renew: bool,
    with_baseline: bool,
    with_apply: bool,
) -> Test {
    let (mut f, mut input) = link_fixture();
    if with_baseline {
        input.provenance = Some(proof(&input));
    }
    let plan = f.app.prepare_host_link(input.clone()).unwrap();
    let registration = f.app.commit_host_link(link_commit(&f, &plan)).unwrap();
    f.registrations.push(registration);
    if renew {
        f.clock.0.store(1001, Ordering::SeqCst);
        let renewal = f.app.prepare_host_renewal(&plan.id).unwrap();
        let mut grant = renewal.grant.clone();
        grant.valid_until.ticks_ns =
            Counter(renewal.sent_at.ticks_ns.0 + grant.ttl_ms.0 * 1_000_000);
        f.app.commit_host_renewal(renewal, grant).unwrap();
    }
    if cells == 2 {
        let mut second = f.configuration.clone();
        second.id = name("cell/b");
        second.definition = artifact(12, "rx.cell-definition.v1");
        second.steps[0].intent.resource_set = vec![name("robot/second")];
        f.app.install_cell(&f.admin, second.clone()).unwrap();
        f.app
            .negotiate_evidence_cell(&f.hosts[0], second.definition.sha256)
            .unwrap();
        let mut next = input.clone();
        next.snapshot.cell = second.id.clone();
        next.snapshot.definition = second.definition.sha256;
        next.snapshot.resource_fences = [(name("robot/second"), Counter(0))].into();
        next.snapshot.observations[0].evidence_id = id();
        next.provenance = Some(proof(&next));
        let plan = f.app.prepare_host_link(next).unwrap();
        let mut command = link_commit(&f, &plan);
        // Both cells share the Host delivery journal, so the second receipt has
        // a new journal sequence even though its cell/resource set is different.
        command.fence_receipt.sequence = Counter(2);
        f.app.commit_host_link(command).unwrap();
    }
    if with_work {
        report_ready(
            &mut f.app,
            &f.hosts[0],
            &f.configuration,
            &f.registrations[0],
        );
        let run = start(&mut f, 1);
        let activation = activation(&mut f, &run);
        let work = submit(&mut f, &activation, id().as_str()).unwrap();
        f.app
            .plan_delivery(&f.hosts[0], work.operation.id())
            .unwrap();
        f.app
            .record_host_receipt(
                &f.hosts[0],
                work.operation.id(),
                HostReceipt {
                    operation: work.operation.id().clone(),
                    digest: work.intent.digest().unwrap(),
                    invocation: Some(id()),
                    journal: work.host_journal,
                    sequence: Counter(10),
                    state: ReceiptState::Prepared,
                },
            )
            .unwrap();
        let message = f
            .app
            .pending_deliveries(128)
            .unwrap()
            .into_iter()
            .find(|d| matches!(d.payload, Delivery::Authorize { .. }))
            .unwrap()
            .id;
        f.app.plan_delivery(&f.hosts[0], &message).unwrap();
    }
    let (release, applied_context) = if with_apply {
        // These existing real transaction helpers live in the parent configuration_dispatch_tests module.
        let (material, job, release, change) = prepared_materials(&mut f);
        let tasks = authorize_and_observe(&mut f, &release, &change);
        let ticket = apply_prepared(&mut f, &material, &job, &release, &change, &id());
        f.app.commit_process_change_apply(ticket).unwrap();
        f.configuration = f
            .app
            .inspect_cell(&f.admin, &f.configuration.id)
            .unwrap()
            .1
            .configuration;
        (
            release,
            applied(&tasks[0]).snapshot.cells[0].applied.clone(),
        )
    } else {
        (release_identity(&mut f), None)
    };
    let installation = f.app.installation.id.clone();
    let repository = Shared(
        Arc::new(Mutex::new(f.app.into_repository())),
        Arc::new(AtomicU8::new(0)),
        Arc::new(AtomicU8::new(0)),
    );
    let mut app = Engine::open(
        repository.clone(),
        f.clock.clone(),
        SimulationAuthority,
        installation,
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    let session = app
        .open_evidence_producer(
            &input.host,
            input.snapshot.host_boot.clone(),
            input.snapshot.evidence_journal.clone(),
            Digest::from_bytes([81; 32]),
        )
        .unwrap();
    let host = Identity {
        principal: input.host.clone(),
        session: session.id,
        terminal: None,
    };
    for cell in if cells == 2 {
        vec![
            f.configuration.definition.sha256,
            artifact(12, "rx.cell-definition.v1").sha256,
        ]
    } else {
        vec![f.configuration.definition.sha256]
    } {
        app.negotiate_evidence_cell(&host, cell).unwrap();
    }
    app.register_host_recovery_transport(input.host.clone(), pin())
        .unwrap();
    let actor = Identity {
        principal: release.principal.clone(),
        terminal: release.terminal,
        session: app
            .authenticated_terminal_user_session(
                &release.principal,
                id(),
                Counter(1_000_000_000_000),
                Digest::from_bytes([77; 32]),
            )
            .unwrap()
            .id,
    };
    Test {
        _directory: f._directory,
        app,
        repository,
        clock: f.clock,
        failure: f.failure,
        actor,
        input,
        platform_session: id(),
        applied: applied_context,
    }
}
impl Test {
    fn context(&mut self) -> recovery::Context {
        self.app
            .host_recovery_context(&self.actor, &self.input.host, &self.input.snapshot.cell)
            .unwrap()
    }
    fn evidence(&mut self, c: &recovery::Context, post: bool) -> recovery::ReadEvidence {
        let now = self.clock.now();
        let mut snapshots = BTreeMap::new();
        let mut configuration = configuration_read(&self.input.snapshot);
        configuration.snapshot.cells.clear();
        for cell in &c.host_cells {
            let mut snapshot = self.input.snapshot.clone();
            let cut = &c.cells[cell];
            snapshot.cell = cell.clone();
            snapshot.definition = cut.configuration.definition.sha256;
            snapshot.envelope = cut.configuration.envelope.sha256;
            snapshot.epoch = if post {
                cut.epoch
            } else {
                Counter(cut.epoch.0 - 1)
            };
            snapshot.scopes = cut
                .scopes
                .iter()
                .map(|(s, e)| (s.clone(), if post { *e } else { Counter(e.0 - 1) }))
                .collect();
            snapshot.block_ids = if post {
                cut.blocks
                    .iter()
                    .filter(|b| b.latched)
                    .map(|b| b.id.clone())
                    .collect()
            } else {
                vec![]
            };
            snapshot.captured_at = now.clone();
            for source in &mut snapshot.observations {
                source.acquired_at = now.clone();
                source.evidence_id = id();
            }
            snapshot.pending_operations = c
                .operations
                .values()
                .filter(|o| &o.cell == cell)
                .map(|o| o.operation.clone())
                .collect();
            snapshot.pending_permits = c
                .operations
                .values()
                .filter(|o| &o.cell == cell)
                .map(|o| o.permit.clone())
                .collect();
            snapshot.resource_fences = self
                .repository
                .transact(|tx| {
                    cut.configuration
                        .steps
                        .iter()
                        .flat_map(|s| s.intent.resource_set.iter())
                        .map(|resource| {
                            let (_, value): (_, Counter) = p::load(
                                tx,
                                "host-link-fence",
                                (&c.host, resource),
                                "rx.internal.host-link-fence.v1",
                            )?;
                            Ok((resource.clone(), value))
                        })
                        .collect()
                })
                .unwrap();
            let mut observed_configuration = configuration_read(&snapshot).snapshot.cells.remove(0);
            observed_configuration.applied = self.applied.clone();
            configuration.snapshot.cells.push(observed_configuration);
            snapshots.insert(
                cell.clone(),
                recovery::SnapshotRead {
                    snapshot,
                    started: now.clone(),
                    finished: now.clone(),
                },
            );
        }
        recovery::ReadEvidence {
            platform_session: self.platform_session.clone(),
            transport: pin(),
            configuration,
            configuration_started: now.clone(),
            configuration_finished: now,
            cells: snapshots,
        }
    }
    fn propose(&mut self) -> recovery::Binding {
        let c = self.context();
        assert!(c.blockers.is_empty(), "{:?}", c.blockers);
        let e = self.evidence(&c, false);
        self.app
            .propose_host_recovery(
                &self.actor,
                &id(),
                recovery::Prepare {
                    host: c.host.clone(),
                    origin: c.origin.clone(),
                    expected_context: c.digest().unwrap(),
                    expected_cells: c.expected_cells(),
                },
                verified(e),
            )
            .unwrap()
    }
    fn approve(&mut self, b: &recovery::Binding) -> recovery::Binding {
        self.app
            .approve_host_recovery(
                &self.actor,
                &id(),
                recovery::Approve {
                    id: b.id.clone(),
                    expected_revision: b.revision,
                    proposal_digest: b.proposal_digest().unwrap(),
                    expected_cells: b.context.expected_cells(),
                },
            )
            .unwrap()
    }
    fn ack(&self, t: &recovery::FenceTask, n: u64) -> FenceAcknowledgment {
        FenceAcknowledgment {
            cell: t.cell.clone(),
            invalidation: t.request.clone(),
            epoch: t.epoch,
            scopes: t.scopes.clone(),
            host_boot: self.input.snapshot.host_boot.clone(),
            journal: self.input.snapshot.delivery_journal.clone(),
            sequence: Counter(n),
        }
    }
    fn finish(&mut self, mut b: recovery::Binding) -> recovery::Binding {
        for (index, cell) in b.context.host_cells.clone().iter().enumerate() {
            let evidence = self.evidence(&b.context, false);
            let task = self
                .app
                .plan_host_recovery_fence(&b.id, cell, verified(evidence))
                .unwrap();
            b = self
                .app
                .record_host_recovery_fence(&b.id, cell, self.ack(&task, 100 + index as u64))
                .unwrap();
        }
        let evidence = self.evidence(&b.context, true);
        self.app
            .commit_host_recovery(&b.id, b.revision, verified(evidence))
            .unwrap()
    }
    fn authorities(&mut self) -> Vec<Record> {
        self.repository
            .transact(|tx| {
                let mut rows = Vec::new();
                for prefix in [
                    "cell/",
                    "host/",
                    "host-link-plan/",
                    "permit/",
                    "mandate/",
                    "run/",
                    "resource/",
                    "qualificationhistory/",
                ] {
                    rows.extend(tx.scan(prefix)?);
                }
                rows.sort_by(|a, b| a.key.cmp(&b.key));
                Ok(rows)
            })
            .unwrap()
    }
}
fn verified(e: recovery::ReadEvidence) -> recovery::VerifiedRead {
    recovery::VerifiedRead::new(
        e.platform_session,
        e.transport,
        e.configuration,
        e.configuration_started,
        e.configuration_finished,
        e.cells,
    )
    .unwrap()
}

#[test]
fn recovery_stored_actor_roundtrip_retains_identifiers_without_restoring_session_authority() {
    let mut f = test(1, false, false);
    let proposed = f.propose();
    let approved = f.approve(&proposed);
    let bytes = canonical::bytes(approved.approved_by.as_ref().unwrap()).unwrap();
    let stored: recovery::StoredActor = canonical::decode_json(&bytes).unwrap();
    let identity = stored.identity();
    assert_eq!(identity.principal, f.actor.principal);
    assert_eq!(identity.session, f.actor.session);
    assert_eq!(identity.terminal, f.actor.terminal);
    let mut injected = serde_json::to_value(&stored).unwrap();
    injected["active"] = serde_json::json!(true);
    assert!(serde_json::from_value::<recovery::StoredActor>(injected).is_err());
    f.app.end_user_session(&f.actor).unwrap();
    assert!(matches!(
        f.app.host_recovery(&identity, &approved.id),
        Err(StoreError::Rejected(Rejection::Unauthenticated))
    ));
}

#[test]
fn recovery_initial_unapplied_and_normal_renewal_preserve_dynamic_authority_while_restoring_real_false_fact()
 {
    let mut f = test(1, false, true);
    let before = f.authorities();
    let proposed = f.propose();
    let approved = f.approve(&proposed);
    let ready = f.finish(approved);
    assert_eq!(ready.phase, recovery::Phase::RecoveryOnly);
    assert_eq!(f.authorities(), before);
    let mut evidence = f.evidence(&ready.context, true);
    let source = &mut evidence
        .cells
        .get_mut(&name("cell/a"))
        .unwrap()
        .snapshot
        .observations[0];
    source.value = TypedValue::Boolean(false);
    source.quality_good = false;
    let expected = source.clone();
    let refreshed = f
        .app
        .refresh_host_recovery(&ready.id, verified(evidence))
        .unwrap();
    assert_eq!(refreshed.phase, recovery::Phase::RecoveryOnly);
    let fact = f
        .app
        .inspect_fact(&f.actor, &name("cell/a"), &name("ready"))
        .unwrap();
    assert_eq!(fact.value, TypedValue::Boolean(false));
    assert!(!fact.quality_good);
    assert_eq!(fact.acquired_at, expected.acquired_at);
    assert_eq!(fact.evidence_id, expected.evidence_id);
    assert!(
        !f.app
            .host_recovery(&f.actor, &ready.id)
            .unwrap()
            .operation_authorized
    );
    let mut disputed = f.evidence(&ready.context, true);
    disputed
        .cells
        .get_mut(&name("cell/a"))
        .unwrap()
        .snapshot
        .observations[0]
        .disputed = true;
    let attention = f
        .app
        .refresh_host_recovery(&ready.id, verified(disputed))
        .unwrap();
    assert_eq!(attention.phase, recovery::Phase::Attention);
    assert!(
        f.app
            .inspect_fact(&f.actor, &name("cell/a"), &name("ready"))
            .unwrap()
            .disputed
    );
    assert!(
        f.app
            .inspect_cell(&f.actor, &name("cell/a"))
            .unwrap()
            .1
            .blocks
            .iter()
            .any(|b| b.reason == BlockReason::IntegrityConflict)
    );
}

#[test]
fn recovery_normal_apply_after_initial_none_uses_current_application_proof_instead_of_initial_configuration()
 {
    let mut f = test_complete(1, false, false, true, true);
    let c = f.context();
    let base = f
        .app
        .host_binding_baseline(&c.registrations[&name("cell/a")].plan)
        .unwrap()
        .unwrap();
    assert!(base.applied_context.is_none());
    assert!(f.applied.is_some());
    assert_ne!(
        base.original_configuration_digest,
        c.cells[&name("cell/a")].configuration_digest
    );
    let before = f.authorities();
    let p = f.propose();
    let b = f.approve(&p);
    let ready = f.finish(b);
    assert_eq!(ready.phase, recovery::Phase::RecoveryOnly);
    assert_eq!(f.authorities(), before);
}

#[test]
fn recovery_lost_commits_keep_original_proposal_fence_and_ack_without_replaying_authority() {
    for fault in [1, 2] {
        let mut f = test(1, false, false);
        let c = f.context();
        let e = f.evidence(&c, false);
        let key = id();
        let request = recovery::Prepare {
            host: c.host.clone(),
            origin: c.origin.clone(),
            expected_context: c.digest().unwrap(),
            expected_cells: c.expected_cells(),
        };
        // The first transaction authenticates/caches; inject at the actual proposal commit.
        f.repository.1.store(2, Ordering::SeqCst);
        f.repository.2.store(fault, Ordering::SeqCst);
        assert!(
            f.app
                .propose_host_recovery(&f.actor, &key, request.clone(), verified(e.clone()))
                .is_err()
        );
        let p = f
            .app
            .propose_host_recovery(&f.actor, &key, request.clone(), verified(e.clone()))
            .unwrap();
        assert_eq!(
            f.app
                .propose_host_recovery(&f.actor, &key, request, verified(e.clone()))
                .unwrap()
                .id,
            p.id
        );
        let b = f.approve(&p);
        let cell = name("cell/a");
        f.failure.store(fault, Ordering::SeqCst);
        assert!(
            f.app
                .plan_host_recovery_fence(&b.id, &cell, verified(e.clone()))
                .is_err()
        );
        let task = f
            .app
            .plan_host_recovery_fence(&b.id, &cell, verified(e.clone()))
            .unwrap();
        assert_eq!(task.request, p.fences[&cell].task.request);
        let ack = f.ack(&task, 100);
        f.failure.store(fault, Ordering::SeqCst);
        assert!(
            f.app
                .record_host_recovery_fence(&b.id, &cell, ack.clone())
                .is_err()
        );
        let recorded = f
            .app
            .record_host_recovery_fence(&b.id, &cell, ack.clone())
            .unwrap();
        assert_eq!(
            recorded.fences[&cell]
                .acknowledgment
                .as_ref()
                .unwrap()
                .sequence,
            ack.sequence
        );
        assert_eq!(
            f.app
                .record_host_recovery_fence(&b.id, &cell, ack)
                .unwrap()
                .revision,
            recorded.revision
        );
    }
}

#[test]
fn recovery_proposal_reply_is_recovered_without_fresh_host_data_and_still_checks_key_and_actor() {
    let mut f = test(1, false, false);
    let c = f.context();
    let e = f.evidence(&c, false);
    let key = id();
    let request = recovery::Prepare {
        host: c.host.clone(),
        origin: c.origin.clone(),
        expected_context: c.digest().unwrap(),
        expected_cells: c.expected_cells(),
    };
    assert!(
        f.app
            .lookup_host_recovery_proposal(&f.actor, &key, &request)
            .unwrap()
            .is_none()
    );
    let b = f
        .app
        .propose_host_recovery(&f.actor, &key, request.clone(), verified(e))
        .unwrap();
    f.clock.0.store(1_000_000_000, Ordering::SeqCst);
    assert_eq!(
        f.app
            .lookup_host_recovery_proposal(&f.actor, &key, &request)
            .unwrap()
            .unwrap()
            .id,
        b.id
    );
    let mut different = request.clone();
    different.expected_context = Digest::from_bytes([99; 32]);
    assert!(matches!(
        f.app
            .lookup_host_recovery_proposal(&f.actor, &key, &different),
        Err(StoreError::KeyConflict)
    ));
    f.app.end_user_session(&f.actor).unwrap();
    assert!(
        f.app
            .lookup_host_recovery_proposal(&f.actor, &key, &request)
            .is_err()
    );
}

#[test]
fn recovery_missing_baseline_or_changed_identity_never_autogenerates_continuity() {
    for axis in ["boot", "journal", "auth", "transport"] {
        let mut f = test(1, false, false);
        f.repository
            .transact(|tx| {
                let (revision, mut producer): (_, EvidenceProducer) = p::load(
                    tx,
                    "producer",
                    name("host/0"),
                    "rx.internal.evidence-producer.v1",
                )?;
                match axis {
                    "boot" => producer.peer_boot = id(),
                    "journal" => producer.journal = id(),
                    "auth" => producer.authentication_binding = Digest::from_bytes([99; 32]),
                    _ => {}
                }
                p::save(
                    tx,
                    "producer",
                    name("host/0"),
                    Some(revision),
                    "rx.internal.evidence-producer.v1",
                    &producer,
                )?;
                Ok(())
            })
            .unwrap();
        if axis == "transport" {
            let mut different = pin();
            different.server_leaf_digest = Digest::from_bytes([99; 32]);
            assert!(
                f.app
                    .register_host_recovery_transport(name("host/0"), different)
                    .is_err()
            );
            continue;
        }
        assert!(!f.context().blockers.is_empty(), "{axis}");
    }
    let mut legacy = test_with_baseline(1, false, false, false);
    assert!(
        legacy
            .context()
            .blockers
            .iter()
            .any(|b| matches!(b, recovery::Blocker::BaselineMissing { .. }))
    );
}

#[test]
fn recovery_source_clock_configuration_and_unknown_pending_changes_cannot_be_promoted() {
    for axis in [
        "source",
        "clock",
        "stale",
        "configuration",
        "pending",
        "binding",
    ] {
        let mut f = test(1, false, false);
        let p = f.propose();
        let b = f.approve(&p);
        let mut e = f.evidence(&b.context, false);
        let source = &mut e.cells.get_mut(&name("cell/a")).unwrap().snapshot;
        match axis {
            "source" => source.observations[0].generation = id(),
            "clock" => {
                source.captured_at.clock_id = "other".into();
                source.observations[0].acquired_at.clock_id = "other".into();
            }
            "stale" => {
                f.clock.0.store(100_001_001, Ordering::SeqCst);
            }
            "configuration" => {
                e.configuration.snapshot.cells[0].applied = Some(hc::AppliedContext {
                    cell: name("cell/a"),
                    configuration: Digest::from_bytes([99; 32]),
                    change: id(),
                    request: id(),
                    receipt_sequence: Counter(1),
                    binding_digest: e.configuration.snapshot.binding_digest,
                })
            }
            "pending" => source.pending_operations.push(id()),
            "binding" => e.configuration.snapshot.binding_digest = Digest::from_bytes([99; 32]),
            _ => unreachable!(),
        }
        assert!(
            f.app
                .plan_host_recovery_fence(&b.id, &name("cell/a"), verified(e))
                .is_err(),
            "{axis}"
        );
        let b = f.app.host_recovery(&f.actor, &b.id).unwrap().binding;
        assert_eq!(b.phase, recovery::Phase::Attention, "{axis}");
        assert!(b.last_read.is_some());
        assert_eq!(
            b.fences[&name("cell/a")].phase,
            recovery::FencePhase::Pending
        );
    }
}

#[test]
fn recovery_commit_reply_retry_cannot_hide_a_new_source_generation_after_historical_success() {
    let mut f = test(1, false, false);
    let p = f.propose();
    let b = f.approve(&p);
    let ready = f.finish(b);
    let mut changed = f.evidence(&ready.context, true);
    changed
        .cells
        .get_mut(&name("cell/a"))
        .unwrap()
        .snapshot
        .observations[0]
        .generation = id();
    let result = f
        .app
        .commit_host_recovery(&ready.id, Counter(1), verified(changed))
        .unwrap();
    assert_eq!(result.phase, recovery::Phase::Attention);
    assert!(!f.app.host_recovery(&f.actor, &ready.id).unwrap().current);
}

#[test]
fn recovery_late_ack_survives_all_cell_cas_loss_and_never_changes_registration() {
    let mut f = test(2, false, false);
    let p = f.propose();
    let b = f.approve(&p);
    let before = f.authorities();
    let e = f.evidence(&b.context, false);
    let task = f
        .app
        .plan_host_recovery_fence(&b.id, &name("cell/a"), verified(e))
        .unwrap();
    f.repository
        .transact(|tx| {
            let (revision, cell): (_, Cell) =
                p::load(tx, "cell", name("cell/b"), "rx.internal.cell.v1")?;
            p::save(
                tx,
                "cell",
                name("cell/b"),
                Some(revision),
                "rx.internal.cell.v1",
                &cell,
            )?;
            Ok(())
        })
        .unwrap();
    let b = f
        .app
        .record_host_recovery_fence(&b.id, &name("cell/a"), f.ack(&task, 100))
        .unwrap();
    assert_eq!(b.phase, recovery::Phase::Attention);
    assert!(b.fences[&name("cell/a")].acknowledgment.is_some());
    assert_eq!(
        b.fences[&name("cell/b")].phase,
        recovery::FencePhase::Pending
    );
    let hosts = |rows: Vec<Record>| {
        rows.into_iter()
            .filter(|r| r.document.schema.as_str() == "rx.internal.host-registration.v1")
            .collect::<Vec<_>>()
    };
    assert_eq!(hosts(f.authorities()), hosts(before));
}

#[test]
fn recovery_shared_host_requires_terminal_rights_for_every_affected_cell() {
    let mut f = test(2, false, false);
    f.repository
        .transact(|tx| {
            let (revision, mut terminal): (_, Terminal) = p::load(
                tx,
                "terminal",
                name("panel/main"),
                "rx.internal.terminal.v1",
            )?;
            terminal.cells = BTreeSet::from([name("cell/a")]);
            p::save(
                tx,
                "terminal",
                &terminal.id,
                Some(revision),
                "rx.internal.terminal.v1",
                &terminal,
            )?;
            Ok(())
        })
        .unwrap();
    f.actor.session = f
        .app
        .authenticated_terminal_user_session(
            &f.actor.principal,
            id(),
            Counter(1_000_000_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap()
        .id;
    assert!(matches!(
        f.app
            .host_recovery_context(&f.actor, &name("host/0"), &name("cell/a")),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
}

#[test]
fn recovery_known_unknown_allows_original_lookup_and_receipt_only_without_new_authorize_or_release()
{
    let mut f = test(1, true, false);
    let p = f.propose();
    let b = f.approve(&p);
    let b = f.finish(b);
    let operation = b.context.operations.values().next().unwrap().clone();
    let before = f.repository.pending_outbox(128).unwrap();
    let query = f
        .app
        .host_recovery_query(&b.id, &operation.operation)
        .unwrap();
    assert!(query.lookup_allowed);
    assert!(f.app.host_recovery_query(&b.id, &id()).is_err());
    let receipt = HostReceipt {
        operation: operation.operation.clone(),
        digest: operation.intent_digest,
        invocation: operation.invocation.clone(),
        journal: operation.host_journal.clone(),
        sequence: Counter(101),
        state: ReceiptState::SendEntered,
    };
    let message = operation.messages.last().unwrap();
    let work = f
        .app
        .record_host_recovery_receipt(&b.id, message, receipt.clone())
        .unwrap();
    assert_eq!(
        work.operation.outcome(),
        rx_domain::operation::Outcome::None
    );
    f.app
        .record_host_recovery_receipt(&b.id, message, receipt)
        .unwrap();
    let after = f.repository.pending_outbox(128).unwrap();
    assert!(
        after
            .iter()
            .all(|row| before.iter().any(|old| old.id == row.id)),
        "no new authorization IDs"
    );
    assert!(
        f.authorities()
            .iter()
            .filter(|r| r.document.schema.as_str() == "rx.internal.permit.v1")
            .all(|row| p::decode::<Permit>(row, "rx.internal.permit.v1")
                .unwrap()
                .state
                != PermitState::Issued)
    );
}

#[test]
fn recovery_permanent_void_without_invocation_preserves_known_prepare_identity_and_records_only_not_executed()
 {
    let mut f = test(1, true, false);
    let proposed = f.propose();
    let approved = f.approve(&proposed);
    let binding = f.finish(approved);
    let operation = binding.context.operations.values().next().unwrap().clone();
    assert!(operation.invocation.is_some());
    assert!(operation.messages.contains(&operation.operation));
    let authority = f.authorities();
    let pending = f.repository.pending_outbox(128).unwrap();
    let receipt = HostReceipt {
        operation: operation.operation.clone(),
        digest: operation.intent_digest,
        invocation: None,
        journal: operation.host_journal.clone(),
        sequence: Counter(201),
        state: ReceiptState::VoidedBeforeSend,
    };
    let mut invalid_missing = receipt.clone();
    invalid_missing.state = ReceiptState::Prepared;
    assert!(
        f.app
            .record_host_recovery_receipt(&binding.id, &operation.operation, invalid_missing,)
            .is_err()
    );
    let mut invalid_identity = receipt.clone();
    invalid_identity.invocation = Some(id());
    assert!(
        f.app
            .record_host_recovery_receipt(&binding.id, &operation.operation, invalid_identity,)
            .is_err()
    );
    for _ in 0..2 {
        let work = f
            .app
            .record_host_recovery_receipt(&binding.id, &operation.operation, receipt.clone())
            .unwrap();
        assert_eq!(
            work.operation.outcome(),
            rx_domain::operation::Outcome::NotExecuted
        );
        assert_eq!(work.invocation, operation.invocation);
        assert_ne!(
            work.operation.disposition(),
            rx_domain::operation::Disposition::Released
        );
    }
    assert_eq!(f.repository.pending_outbox(128).unwrap(), pending);
    assert_eq!(f.authorities(), authority);
}

#[test]
fn recovery_sender_reapproval_keeps_original_fence_and_old_runtime_cannot_resume() {
    let mut f = test(1, false, false);
    let p = f.propose();
    let b = f.approve(&p);
    f.app.end_user_session(&f.actor).unwrap();
    let fresh = f
        .app
        .authenticated_terminal_user_session(
            &f.actor.principal,
            id(),
            Counter(1_000_000_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap();
    f.actor.session = fresh.id;
    let e = f.evidence(&b.context, false);
    assert!(
        f.app
            .plan_host_recovery_fence(&b.id, &name("cell/a"), verified(e))
            .is_err()
    );
    let attention = f.app.host_recovery(&f.actor, &b.id).unwrap().binding;
    let again = f.approve(&attention);
    assert_eq!(
        again.fences[&name("cell/a")].task.request,
        p.fences[&name("cell/a")].task.request
    );
    let installation = f.app.installation.id.clone();
    f.app = Engine::open(
        f.repository.clone(),
        f.clock.clone(),
        SimulationAuthority,
        installation,
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    let e = f.evidence(&again.context, false);
    assert!(
        f.app
            .plan_host_recovery_fence(&again.id, &name("cell/a"), verified(e))
            .is_err()
    );
}

#[test]
fn recovery_external_payload_cannot_inject_transport_observations_or_operating_actions() {
    let mut f = test(1, false, false);
    let c = f.context();
    let input = recovery::Prepare {
        host: c.host.clone(),
        origin: c.origin.clone(),
        expected_context: c.digest().unwrap(),
        expected_cells: c.expected_cells(),
    };
    for field in [
        "snapshot",
        "transport",
        "execute",
        "grant",
        "arm",
        "permit",
        "clear_blocks",
    ] {
        let mut value = serde_json::to_value(&input).unwrap();
        value[field] = serde_json::json!(true);
        assert!(
            serde_json::from_value::<recovery::Prepare>(value).is_err(),
            "{field}"
        );
    }
    let p = f.propose();
    let approved = f.approve(&p);
    let evidence = f.evidence(&approved.context, true);
    assert!(
        f.app
            .commit_host_recovery(&approved.id, approved.revision, verified(evidence))
            .is_err()
    );
}

fn current_fence(f: &mut Test) -> rx_ports::OutboxRecord {
    let c = f.context();
    let cut = &c.cells[&name("cell/a")];
    f.repository.pending_outbox(128).unwrap().into_iter().find(|row| {
        let payload:Delivery=serde_json::from_value(row.document.value.clone()).unwrap();
        matches!(payload,Delivery::Fence{cell,host,epoch,scopes,..} if cell==name("cell/a") && host==c.host && epoch==cut.epoch && scopes==cut.scopes)
    }).unwrap()
}

#[test]
fn recovery_adopts_current_original_outbox_and_commits_marker_and_exact_ack_atomically() {
    for mode in [1, 2] {
        let mut f = test(1, false, false);
        let original = current_fence(&mut f);
        let old = id();
        f.repository
            .transact(|tx| {
                tx.enqueue(
                    &old,
                    &p::doc(
                        "rx.internal.delivery.v1",
                        &Delivery::Fence {
                            cell: name("cell/a"),
                            host: name("host/0"),
                            epoch: Counter(1),
                            scopes: [(name("old/scope"), Counter(1))].into(),
                            block_ids: vec![],
                        },
                    )?,
                )
            })
            .unwrap();
        let p = f.propose();
        let b = f.approve(&p);
        let cell = name("cell/a");
        assert_eq!(
            b.fences[&cell].task.originating_message.as_ref(),
            Some(&original.id)
        );
        assert_eq!(b.fences[&cell].task.request, original.id);
        let e = f.evidence(&b.context, false);
        f.failure.store(mode, Ordering::SeqCst);
        assert!(
            f.app
                .plan_host_recovery_fence(&b.id, &cell, verified(e.clone()))
                .is_err()
        );
        let (state, phase) = f
            .repository
            .transact(|tx| {
                let (_, stored): (_, recovery::Binding) =
                    p::load(tx, "host-recovery", &b.id, recovery::SCHEMA)?;
                Ok((
                    tx.outbox(&original.id)?.unwrap().state,
                    stored.fences[&cell].phase,
                ))
            })
            .unwrap();
        assert_eq!(
            state,
            if mode == 1 {
                rx_ports::OutboxState::New
            } else {
                rx_ports::OutboxState::EmitEntered
            }
        );
        assert_eq!(
            phase,
            if mode == 1 {
                recovery::FencePhase::Pending
            } else {
                recovery::FencePhase::SendEntered
            }
        );
        let task = f
            .app
            .plan_host_recovery_fence(&b.id, &cell, verified(e))
            .unwrap();
        let mut wrong = f.ack(&task, 100);
        wrong.invalidation = id();
        assert!(
            f.app
                .record_host_recovery_fence(&b.id, &cell, wrong)
                .is_err()
        );
        let ack = f.ack(&task, 100);
        f.failure.store(mode, Ordering::SeqCst);
        assert!(
            f.app
                .record_host_recovery_fence(&b.id, &cell, ack.clone())
                .is_err()
        );
        let recorded = f.app.record_host_recovery_fence(&b.id, &cell, ack).unwrap();
        assert_eq!(
            recorded.fences[&cell].phase,
            recovery::FencePhase::Acknowledged
        );
        f.repository
            .transact(|tx| {
                assert_eq!(
                    tx.outbox(&original.id)?.unwrap().state,
                    rx_ports::OutboxState::Delivered
                );
                assert_eq!(tx.outbox(&old)?.unwrap().state, rx_ports::OutboxState::New);
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn recovery_ambiguous_current_fences_and_page_overflow_never_select_a_partial_candidate_set() {
    let mut f = test(1, false, false);
    let original = current_fence(&mut f);
    let duplicate = id();
    f.repository
        .transact(|tx| tx.enqueue(&duplicate, &original.document))
        .unwrap();
    let c = f.context();
    let e = f.evidence(&c, false);
    let input = recovery::Prepare {
        host: c.host.clone(),
        origin: c.origin.clone(),
        expected_context: c.digest().unwrap(),
        expected_cells: c.expected_cells(),
    };
    let b = f
        .app
        .propose_host_recovery(&f.actor, &id(), input.clone(), verified(e.clone()))
        .unwrap();
    assert_eq!(b.phase, recovery::Phase::Attention);
    assert_eq!(b.requested_context_digest, input.expected_context);
    assert_ne!(b.context.digest().unwrap(), b.requested_context_digest);
    let mut changed_request = b.clone();
    changed_request.requested_context_digest = Digest::from_bytes([99; 32]);
    assert_ne!(
        changed_request.proposal_digest().unwrap(),
        b.proposal_digest().unwrap()
    );
    assert!(
        b.context
            .blockers
            .iter()
            .any(|b| matches!(b, recovery::Blocker::FenceCandidatesAmbiguous { .. }))
    );
    assert!(
        f.app
            .approve_host_recovery(
                &f.actor,
                &id(),
                recovery::Approve {
                    id: b.id.clone(),
                    expected_revision: b.revision,
                    proposal_digest: b.proposal_digest().unwrap(),
                    expected_cells: b.context.expected_cells()
                }
            )
            .is_err()
    );
    f.repository
        .transact(|tx| {
            assert_eq!(
                tx.outbox(&original.id)?.unwrap().state,
                rx_ports::OutboxState::New
            );
            assert_eq!(
                tx.outbox(&duplicate)?.unwrap().state,
                rx_ports::OutboxState::New
            );
            for _ in 0..recovery::MAX_PENDING_SCAN {
                tx.enqueue(&id(), &original.document)?;
            }
            Ok(())
        })
        .unwrap();
    assert!(
        matches!(f.app.propose_host_recovery(&f.actor,&id(),input,verified(e)),Err(StoreError::Invalid(message)) if message.contains("HOST_RECOVERY_PENDING_SCAN_OVERFLOW"))
    );
    assert_eq!(
        f.repository
            .transact(|tx| tx.scan("host-recovery/"))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn recovery_history_is_discoverable_in_bounded_host_pages_without_partial_cell_access() {
    let mut f = test(2, false, false);
    let expected = BTreeSet::from([f.propose().id, f.propose().id, f.propose().id]);
    let mut seen = BTreeSet::new();
    let mut after = None;
    loop {
        let page = f
            .app
            .list_host_recoveries(&f.actor, &name("host/0"), after.as_ref(), 1)
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert!(!page.items[0].operation_authorized);
        assert!(seen.insert(page.items[0].binding.id.clone()));
        after = page.next;
        if after.is_none() {
            break;
        }
    }
    assert_eq!(seen, expected);
    assert!(
        f.app
            .list_host_recoveries(&f.actor, &name("other/host"), None, 50)
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        f.app
            .list_host_recoveries(&f.actor, &name("host/0"), None, 51)
            .is_err()
    );
    f.repository
        .transact(|tx| {
            let (revision, mut terminal): (_, Terminal) = p::load(
                tx,
                "terminal",
                name("panel/main"),
                "rx.internal.terminal.v1",
            )?;
            terminal.cells = BTreeSet::from([name("cell/a")]);
            p::save(
                tx,
                "terminal",
                &terminal.id,
                Some(revision),
                "rx.internal.terminal.v1",
                &terminal,
            )?;
            Ok(())
        })
        .unwrap();
    f.actor.session = f
        .app
        .authenticated_terminal_user_session(
            &f.actor.principal,
            id(),
            Counter(1_000_000_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap()
        .id;
    assert!(
        f.app
            .list_host_recoveries(&f.actor, &name("host/0"), None, 50)
            .unwrap()
            .items
            .is_empty()
    );
}
