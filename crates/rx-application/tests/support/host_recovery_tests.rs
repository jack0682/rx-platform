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

fn rejoin_fixture(cells: usize) -> (Fixture, host_link::Prepare, Identity) {
    rejoin_fixture_native(cells, false)
}
fn rejoin_fixture_native(
    cells: usize,
    native_result: bool,
) -> (Fixture, host_link::Prepare, Identity) {
    let (mut f, mut input) = link_fixture_native(native_result);
    input.provenance = Some(proof(&input));
    let plan = f.app.prepare_host_link(input.clone()).unwrap();
    let registration = f.app.commit_host_link(link_commit(&f, &plan)).unwrap();
    f.registrations.push(registration);
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
        next.snapshot.cell = second.id;
        next.snapshot.definition = second.definition.sha256;
        next.snapshot.resource_fences = [(name("robot/second"), Counter(0))].into();
        next.snapshot.observations[0].evidence_id = id();
        next.provenance = Some(proof(&next));
        let plan = f.app.prepare_host_link(next).unwrap();
        let mut commit = link_commit(&f, &plan);
        commit.fence_receipt.sequence = Counter(2);
        let registration = f.app.commit_host_link(commit).unwrap();
        f.registrations.push(registration);
    }
    f.app
        .register_host_recovery_transport(input.host.clone(), pin())
        .unwrap();
    let release = release_identity(&mut f);
    (f, input, release)
}
fn replace_host(
    f: &mut Fixture,
    input: &host_link::Prepare,
    boot: Id,
    journal: Id,
    binding: Digest,
) -> Identity {
    let session = f
        .app
        .open_evidence_producer(&input.host, boot, journal, binding)
        .unwrap();
    let who = Identity {
        principal: input.host.clone(),
        session: session.id,
        terminal: None,
    };
    f.app
        .negotiate_evidence_cell(&who, f.configuration.definition.sha256)
        .unwrap();
    who
}
#[test]
fn host_rejoin_origin_is_exact_immutable_and_read_context_creates_no_authority() {
    let (mut f, input, release) = rejoin_fixture(1);
    f.app
        .hold(&f.operator, id().as_str(), &input.snapshot.cell)
        .unwrap();
    let before = f.app.inspect_cell(&f.admin, &input.snapshot.cell).unwrap();
    let boot = id();
    let who = replace_host(
        &mut f,
        &input,
        boot.clone(),
        input.snapshot.evidence_journal.clone(),
        Digest::from_bytes([81; 32]),
    );
    let context = f
        .app
        .host_rejoin_context(&release, &input.host, &input.snapshot.cell)
        .unwrap();
    assert!(
        context.local_prerequisites_current,
        "{:?}",
        context.blockers
    );
    assert!(!context.operation_authorized && context.fresh_host_read_required);
    let origin = context.replacement.as_ref().unwrap();
    assert_eq!(origin.before.boot, input.snapshot.host_boot);
    assert_eq!(origin.after.boot, boot);
    assert_eq!(origin.after.session, who.session);
    let cut = &origin.cells[&input.snapshot.cell];
    assert_eq!(cut.before.revision, before.0);
    assert_eq!(
        cut.prior_block_ids,
        before.1.blocks.iter().map(|b| b.id.clone()).collect()
    );
    assert!(!cut.prior_block_ids.contains(&cut.block.id));
    assert_eq!(cut.block.reason, BlockReason::DeviceRestart);
    assert_eq!(
        f.app
            .open_evidence_producer(
                &input.host,
                boot,
                input.snapshot.evidence_journal.clone(),
                Digest::from_bytes([81; 32])
            )
            .unwrap()
            .id,
        who.session
    );
    let again = f
        .app
        .host_rejoin_context(&release, &input.host, &input.snapshot.cell)
        .unwrap();
    assert_eq!(context.digest().unwrap(), again.digest().unwrap());
    assert_eq!(
        again.cells[&input.snapshot.cell]
            .registration
            .as_ref()
            .unwrap()
            .boot_id,
        input.snapshot.host_boot
    );
    let mut repository = f.app.into_repository();
    let before_head = repository.journal_head().unwrap();
    let stored = repository
        .transact(|tx| rx_application::host_invalidation::load(tx, &who.session))
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.digest().unwrap(),
        context.replacement_digest.unwrap()
    );
    assert_eq!(repository.journal_head().unwrap(), before_head);
}
#[test]
fn host_rejoin_refuses_missing_origin_changed_source_and_advanced_context() {
    for change in 0..4 {
        let (mut f, input, release) = rejoin_fixture(1);
        if change != 0 {
            let journal = if change == 1 {
                id()
            } else {
                input.snapshot.evidence_journal.clone()
            };
            let binding = if change == 2 {
                Digest::from_bytes([91; 32])
            } else {
                Digest::from_bytes([81; 32])
            };
            replace_host(&mut f, &input, id(), journal, binding);
        }
        if change == 3 {
            f.app
                .hold(&f.operator, id().as_str(), &input.snapshot.cell)
                .unwrap();
        }
        let c = f
            .app
            .host_rejoin_context(&release, &input.host, &input.snapshot.cell)
            .unwrap();
        assert!(!c.local_prerequisites_current && !c.operation_authorized);
        assert!(
            c.blockers.iter().any(|b| matches!(
                (change, b),
                (0, rx_application::host_rejoin::Blocker::OriginMissing)
                    | (
                        1 | 2,
                        rx_application::host_rejoin::Blocker::SourceIdentityChanged
                    )
                    | (
                        3,
                        rx_application::host_rejoin::Blocker::ContextAdvanced { .. }
                    )
            )),
            "{change}: {:?}",
            c.blockers
        );
        assert!(
            f.app
                .host_rejoin_context(&f.operator, &input.host, &input.snapshot.cell)
                .is_err()
        );
        let mut no_terminal = release.clone();
        no_terminal.terminal = None;
        assert!(
            f.app
                .host_rejoin_context(&no_terminal, &input.host, &input.snapshot.cell)
                .is_err()
        );
    }
}

#[test]
fn host_rejoin_source_and_session_commit_atomically_and_reply_loss_recovers_origin() {
    for failure in [1, 2] {
        let (mut f, input, release) = rejoin_fixture(1);
        let old = f.app.current_evidence_producer(&input.host).unwrap();
        let before = f.app.inspect_cell(&f.admin, &input.snapshot.cell).unwrap();
        let boot = id();
        f.failure.store(failure, Ordering::SeqCst);
        assert!(
            f.app
                .open_evidence_producer(
                    &input.host,
                    boot.clone(),
                    old.journal.clone(),
                    old.authentication_binding
                )
                .is_err()
        );
        f.failure.store(0, Ordering::SeqCst);
        let observed = f.app.current_evidence_producer(&input.host).unwrap();
        if failure == 1 {
            assert_eq!(observed.session, old.session);
            assert_eq!(
                canonical::bytes(&f.app.inspect_cell(&f.admin, &input.snapshot.cell).unwrap())
                    .unwrap(),
                canonical::bytes(&before).unwrap()
            );
        } else {
            assert_ne!(observed.session, old.session);
        }
        let current = replace_host(
            &mut f,
            &input,
            boot,
            old.journal.clone(),
            old.authentication_binding,
        );
        if failure == 2 {
            assert_eq!(current.session, observed.session);
        }
        let c = f
            .app
            .host_rejoin_context(&release, &input.host, &input.snapshot.cell)
            .unwrap();
        assert!(c.local_prerequisites_current, "{:?}", c.blockers);
        let digest = c.replacement_digest.unwrap();
        let mut repo = f.app.into_repository();
        let list = repo
            .transact(|tx| tx.scan("host-invalidation-origin/"))
            .unwrap();
        assert_eq!(list.len(), 1);
        let o = repo
            .transact(|tx| rx_application::host_invalidation::load(tx, &current.session))
            .unwrap()
            .unwrap();
        assert_eq!(o.digest().unwrap(), digest);
    }
}
#[test]
fn host_rejoin_context_checks_the_whole_shared_host_cohort_before_returning_evidence() {
    let (mut f, input, release) = rejoin_fixture(2);
    let who = replace_host(
        &mut f,
        &input,
        id(),
        input.snapshot.evidence_journal.clone(),
        Digest::from_bytes([81; 32]),
    );
    f.app
        .negotiate_evidence_cell(&who, artifact(12, "rx.cell-definition.v1").sha256)
        .unwrap();
    let c = f
        .app
        .host_rejoin_context(&release, &input.host, &input.snapshot.cell)
        .unwrap();
    assert!(c.local_prerequisites_current, "{:?}", c.blockers);
    assert_eq!(c.cells.len(), 2);
    assert_eq!(c.replacement.as_ref().unwrap().cells.len(), 2);
    let mut account = principal("rejoin-only-a", &[Role::ReleaseManager]);
    account.cells = BTreeSet::from([name("cell/a")]);
    f.app.put_principal(&f.admin, account, None).unwrap();
    let session = f
        .app
        .authenticated_terminal_user_session(
            &name("rejoin-only-a"),
            id(),
            Counter(99_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap();
    let restricted = Identity {
        principal: name("rejoin-only-a"),
        session: session.id,
        terminal: release.terminal.clone(),
    };
    assert!(matches!(
        f.app
            .host_rejoin_context(&restricted, &input.host, &input.snapshot.cell),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
}

fn rejoin_read_fixture() -> (
    Fixture,
    Identity,
    rx_application::host_rejoin::Prepare,
    recovery::ReadEvidence,
) {
    rejoin_read_fixture_with_work(false)
}
fn rejoin_read_fixture_with_work(
    with_work: bool,
) -> (
    Fixture,
    Identity,
    rx_application::host_rejoin::Prepare,
    recovery::ReadEvidence,
) {
    let (f, who, request, read, _) = rejoin_read_fixture_state(if with_work { 1 } else { 0 });
    (f, who, request, read)
}
fn rejoin_read_fixture_state(
    mode: u8,
) -> (
    Fixture,
    Identity,
    rx_application::host_rejoin::Prepare,
    recovery::ReadEvidence,
    Option<NativeEvidence>,
) {
    let mut native = None;
    let (mut f, input, mut release) = rejoin_fixture_native(1, mode == 2);
    release.session = f
        .app
        .authenticated_terminal_user_session(
            &release.principal,
            id(),
            Counter(1_000_000_000_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap()
        .id;
    if mode > 0 {
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
        if mode == 2 {
            let work = f
                .app
                .inspect_work(&f.operator, work.operation.id())
                .unwrap();
            f.app
                .record_host_receipt(
                    &f.hosts[0],
                    &message,
                    HostReceipt {
                        operation: work.operation.id().clone(),
                        digest: work.intent.digest().unwrap(),
                        invocation: work.invocation.clone(),
                        journal: work.host_journal.clone(),
                        sequence: Counter(11),
                        state: ReceiptState::ResultCaptured,
                    },
                )
                .unwrap();
            let evidence = NativeEvidence {
                id: id(),
                operation: work.operation.id().clone(),
                invocation: work.invocation.unwrap(),
                profile_digest: work.intent.profile_digest,
                device_session: id(),
                status_schema: name("rx.sim.completed.v1"),
                status: Integer(0),
                captured_at: f.clock.now(),
                native_details: None,
            };
            f.app
                .ingest_evidence(
                    &f.hosts[0],
                    EvidenceBatch {
                        journal: input.snapshot.evidence_journal.clone(),
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
                rx_domain::operation::Outcome::Succeeded
            );
            native = Some(evidence);
        }
    }
    let who = replace_host(
        &mut f,
        &input,
        id(),
        input.snapshot.evidence_journal.clone(),
        Digest::from_bytes([81; 32]),
    );
    let context = f
        .app
        .host_rejoin_context(&release, &input.host, &input.snapshot.cell)
        .unwrap();
    let mut snapshot = input.snapshot.clone();
    snapshot.host_boot = context.producer.peer_boot.clone();
    snapshot.captured_at = f.clock.now();
    snapshot.resource_fences = f.registrations[0]
        .grant
        .resources
        .iter()
        .map(|r| (r.clone(), f.registrations[0].grant.fence))
        .collect();
    for source in &mut snapshot.observations {
        source.acquired_at = f.clock.now();
        source.evidence_id = id();
        source.value = TypedValue::Boolean(false);
    }
    let configuration = configuration_read(&snapshot);
    let evidence = recovery::ReadEvidence {
        platform_session: id(),
        transport: pin(),
        configuration,
        configuration_started: f.clock.now(),
        configuration_finished: f.clock.now(),
        cells: BTreeMap::from([(
            snapshot.cell.clone(),
            recovery::SnapshotRead {
                snapshot,
                started: f.clock.now(),
                finished: f.clock.now(),
            },
        )]),
    };
    assert_eq!(who.session, context.producer.session);
    let request = rx_application::host_rejoin::Prepare {
        host: context.host.clone(),
        origin: context.origin.clone(),
        expected_context: context.digest().unwrap(),
        expected_cells: context.expected_cells(),
    };
    (f, release, request, evidence, native)
}
#[test]
fn host_rejoin_proposal_persists_fresh_read_without_adoption_and_recovers_commit_reply_loss() {
    for failure in [0, 1, 2] {
        let (mut f, release, request, evidence) = rejoin_read_fixture();
        let before = f.app.inspect_cell(&f.admin, &request.origin).unwrap();
        let key = id();
        f.failure.store(failure, Ordering::SeqCst);
        let result =
            f.app
                .propose_host_rejoin(&release, &key, request.clone(), verified(evidence.clone()));
        f.failure.store(0, Ordering::SeqCst);
        if failure != 0 {
            assert!(result.is_err());
        }
        let saved = f
            .app
            .lookup_host_rejoin_proposal(&release, &key, request.clone())
            .unwrap();
        assert_eq!(saved.is_some(), failure != 1);
        let value = if let Some(v) = saved {
            v
        } else {
            f.app
                .propose_host_rejoin(&release, &key, request.clone(), verified(evidence.clone()))
                .unwrap()
        };
        assert!(value.context_current && value.read_current && !value.operation_authorized);
        assert_eq!(
            value.proposal.read.cells[&request.origin]
                .snapshot
                .observations[0]
                .value,
            TypedValue::Boolean(false)
        );
        assert_eq!(
            canonical::bytes(&f.app.inspect_cell(&f.admin, &request.origin).unwrap()).unwrap(),
            canonical::bytes(&before).unwrap()
        );
        let again = f
            .app
            .host_rejoin_proposal(&release, &value.proposal.id)
            .unwrap();
        assert_eq!(again.proposal_digest, value.proposal_digest);
        f.clock
            .0
            .store(recovery::READ_AGE_NS + 1001, Ordering::SeqCst);
        let aged = f
            .app
            .lookup_host_rejoin_proposal(&release, &key, request)
            .unwrap()
            .unwrap();
        assert!(aged.context_current && !aged.read_current && !aged.operation_authorized);
        assert_eq!(aged.proposal.id, value.proposal.id);
        let mut repo = f.app.into_repository();
        assert_eq!(
            repo.transact(|tx| tx.scan("host-rejoin-proposal/"))
                .unwrap()
                .len(),
            1
        );
    }
}
#[test]
fn host_rejoin_proposal_rejects_untrusted_changed_stale_or_foreign_reads_and_cas() {
    for bad in 0..10 {
        let (mut f, release, request, mut e) = rejoin_read_fixture();
        match bad {
            0 => {
                e.cells.get_mut(&request.origin).unwrap().snapshot.host_boot = id();
            }
            1 => {
                e.configuration.snapshot.delivery_journal = id();
            }
            2 => {
                e.cells
                    .get_mut(&request.origin)
                    .unwrap()
                    .snapshot
                    .observations[0]
                    .generation = id();
            }
            3 => {
                e.transport.server_leaf_digest = Digest::from_bytes([99; 32]);
            }
            4 => {
                f.clock
                    .0
                    .store(recovery::READ_AGE_NS + 1001, Ordering::SeqCst);
            }
            5 => {
                e.cells
                    .get_mut(&request.origin)
                    .unwrap()
                    .snapshot
                    .pending_operations
                    .push(id());
            }
            6 => {
                e.configuration.snapshot.binding_digest = Digest::from_bytes([99; 32]);
            }
            7 => {
                e.cells
                    .get_mut(&request.origin)
                    .unwrap()
                    .snapshot
                    .observations[0]
                    .quality_good = false;
            }
            8 => {
                f.app
                    .hold(&f.operator, id().as_str(), &request.origin)
                    .unwrap();
            }
            _ => {
                let p = principal(release.principal.as_str(), &[Role::Observer]);
                f.app.put_principal(&f.admin, p, Some(Counter(1))).unwrap();
            }
        }
        assert!(
            f.app
                .propose_host_rejoin(&release, &id(), request, verified(e))
                .is_err(),
            "case {bad}"
        );
        let mut repo = f.app.into_repository();
        assert!(
            repo.transact(|tx| tx.scan("host-rejoin-proposal/"))
                .unwrap()
                .is_empty()
        );
    }
}

fn rejoin_approval_input(
    p: &rx_application::host_rejoin::ProposalView,
) -> rx_application::host_rejoin::Approve {
    rx_application::host_rejoin::Approve {
        id: p.proposal.id.clone(),
        expected_revision: p.proposal.revision,
        proposal_digest: p.proposal_digest,
        expected_cells: p.proposal.context.expected_cells(),
    }
}
fn fresh_rejoin_read(
    e: &recovery::ReadEvidence,
    now: TimePoint,
    context: &rx_application::host_rejoin::Context,
    fenced: bool,
) -> recovery::ReadEvidence {
    let mut e = e.clone();
    e.configuration_started = now.clone();
    e.configuration_finished = now.clone();
    for (cell, read) in &mut e.cells {
        read.started = now.clone();
        read.finished = now.clone();
        read.snapshot.captured_at = now.clone();
        for source in &mut read.snapshot.observations {
            source.acquired_at = now.clone();
            source.evidence_id = id();
        }
        if fenced {
            let cut = &context.cells[cell].cell;
            read.snapshot.epoch = cut.epoch;
            read.snapshot.scopes = cut.scope_epochs.clone();
            read.snapshot.block_ids = cut
                .blocks
                .iter()
                .filter(|b| b.latched)
                .map(|b| b.id.clone())
                .collect();
            let config = e
                .configuration
                .snapshot
                .cells
                .iter_mut()
                .find(|c| &c.cell == cell)
                .unwrap();
            config.epoch = cut.epoch;
            config.scopes = cut.scope_epochs.clone();
            config.blocked = read.snapshot.block_ids.clone();
        }
    }
    e
}
#[test]
fn host_rejoin_approval_uses_new_read_and_exact_fence_after_old_read_expires() {
    let (mut f, release, input, e) = rejoin_read_fixture();
    let p = f
        .app
        .propose_host_rejoin(&release, &id(), input, verified(e.clone()))
        .unwrap();
    let before = p.proposal.context.cells[&name("cell/a")]
        .registration
        .clone()
        .unwrap();
    f.clock.0.store(200_001_000, Ordering::SeqCst);
    assert!(
        !f.app
            .host_rejoin_proposal(&release, &p.proposal.id)
            .unwrap()
            .read_current
    );
    let key = id();
    let command = rejoin_approval_input(&p);
    let approved = f
        .app
        .approve_host_rejoin(&release, &key, command.clone())
        .unwrap();
    assert_eq!(approved.binding.phase, recovery::Phase::Fencing);
    assert!(!approved.operation_authorized);
    let same = f.app.approve_host_rejoin(&release, &key, command).unwrap();
    assert_eq!(same.binding.id, approved.binding.id);
    let e = fresh_rejoin_read(&e, f.clock.now(), &p.proposal.context, false);
    let cell = name("cell/a");
    let task = f
        .app
        .plan_host_rejoin_fence(&p.proposal.id, &cell, verified(e.clone()))
        .unwrap();
    let again = f
        .app
        .plan_host_rejoin_fence(&p.proposal.id, &cell, verified(e.clone()))
        .unwrap();
    assert_eq!(task.request, again.request);
    let mut ack = FenceAcknowledgment {
        cell: cell.clone(),
        invalidation: task.request.clone(),
        epoch: task.epoch,
        scopes: task.scopes.clone(),
        host_boot: p.proposal.context.producer.peer_boot.clone(),
        journal: e.configuration.snapshot.delivery_journal.clone(),
        sequence: Counter(500),
    };
    let right_boot = ack.host_boot.clone();
    ack.host_boot = id();
    assert!(
        f.app
            .record_host_rejoin_fence(&p.proposal.id, &cell, ack.clone())
            .is_err()
    );
    ack.host_boot = right_boot;
    f.app
        .record_host_rejoin_fence(&p.proposal.id, &cell, ack)
        .unwrap();
    let fresh = fresh_rejoin_read(&e, f.clock.now(), &p.proposal.context, true);
    f.app
        .refresh_host_rejoin(&p.proposal.id, verified(fresh))
        .unwrap();
    let view = f.app.host_rejoin_binding(&release, &p.proposal.id).unwrap();
    assert_eq!(view.binding.phase, recovery::Phase::RecoveryOnly);
    assert!(view.context_current && view.read_current);
    assert!(!view.operation_authorized);
    assert_eq!(
        canonical::bytes(&before).unwrap(),
        canonical::bytes(
            &view.proposal.context.cells[&cell]
                .registration
                .clone()
                .unwrap()
        )
        .unwrap()
    );
    let current = f
        .app
        .host_rejoin_context(&release, &p.proposal.context.host, &cell)
        .unwrap();
    assert_eq!(
        canonical::bytes(&before).unwrap(),
        canonical::bytes(&current.cells[&cell].registration.clone().unwrap()).unwrap()
    );
    assert_eq!(
        p.proposal_digest,
        f.app
            .host_rejoin_proposal(&release, &p.proposal.id)
            .unwrap()
            .proposal_digest
    );
}
#[test]
fn host_rejoin_stale_read_or_revoked_approval_cannot_enter_fence() {
    for stale in [true, false] {
        let (mut f, release, input, e) = rejoin_read_fixture();
        let p = f
            .app
            .propose_host_rejoin(&release, &id(), input, verified(e.clone()))
            .unwrap();
        f.app
            .approve_host_rejoin(&release, &id(), rejoin_approval_input(&p))
            .unwrap();
        if stale {
            f.clock.0.store(200_001_000, Ordering::SeqCst);
        } else {
            f.app
                .put_principal(
                    &f.admin,
                    principal(release.principal.as_str(), &[Role::Observer]),
                    Some(Counter(1)),
                )
                .unwrap();
        }
        assert!(
            f.app
                .plan_host_rejoin_fence(&p.proposal.id, &name("cell/a"), verified(e))
                .is_err()
        );
        let rows = f
            .app
            .into_repository()
            .transact(|tx| tx.scan("host-rejoin-binding/"))
            .unwrap();
        let b: rx_application::host_rejoin::Binding =
            p::decode(&rows[0], rx_application::host_rejoin::BINDING_SCHEMA).unwrap();
        assert_eq!(b.phase, recovery::Phase::Attention);
        assert!(
            b.fences
                .values()
                .all(|f| f.phase == recovery::FencePhase::Pending)
        );
    }
}

#[test]
fn host_rejoin_approval_commit_loss_recovers_and_context_changes_require_new_proposal() {
    for failure in [0, 1, 2] {
        let (mut f, release, input, e) = rejoin_read_fixture();
        let p = f
            .app
            .propose_host_rejoin(&release, &id(), input, verified(e))
            .unwrap();
        let command = rejoin_approval_input(&p);
        let key = id();
        f.failure.store(failure, Ordering::SeqCst);
        let first = f.app.approve_host_rejoin(&release, &key, command.clone());
        assert_eq!(first.is_ok(), failure == 0);
        let recovered = f.app.approve_host_rejoin(&release, &key, command).unwrap();
        assert_eq!(recovered.binding.id, p.proposal.id);
        let mut repo = f.app.into_repository();
        assert_eq!(
            repo.transact(|tx| tx.scan("host-rejoin-binding/"))
                .unwrap()
                .len(),
            1
        );
    }
    for bad in [0, 1, 2] {
        let (mut f, release, input, e) = rejoin_read_fixture();
        let p = f
            .app
            .propose_host_rejoin(&release, &id(), input, verified(e))
            .unwrap();
        let mut command = rejoin_approval_input(&p);
        match bad {
            0 => command.proposal_digest = Digest::from_bytes([99; 32]),
            1 => {
                f.clock.0.store(30_000_001_000, Ordering::SeqCst);
            }
            _ => {
                f.app
                    .hold(&f.operator, id().as_str(), &name("cell/a"))
                    .unwrap();
            }
        }
        assert!(f.app.approve_host_rejoin(&release, &id(), command).is_err());
        assert!(
            f.app
                .into_repository()
                .transact(|tx| tx.scan("host-rejoin-binding/"))
                .unwrap()
                .is_empty()
        );
    }
}
#[test]
fn host_rejoin_query_is_scoped_and_missing_or_correlated_results_never_promote_work() {
    let (mut f, release, input, e) = rejoin_read_fixture_with_work(true);
    let p = f
        .app
        .propose_host_rejoin(&release, &id(), input, verified(e.clone()))
        .unwrap();
    let allowed = p
        .proposal
        .recovery_scope
        .as_ref()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    f.app
        .approve_host_rejoin(&release, &id(), rejoin_approval_input(&p))
        .unwrap();
    assert!(
        f.app
            .host_rejoin_query_plan(&p.proposal.id, &allowed.operation)
            .is_err()
    );
    let task = f
        .app
        .plan_host_rejoin_fence(&p.proposal.id, &allowed.cell, verified(e.clone()))
        .unwrap();
    f.app
        .record_host_rejoin_fence(
            &p.proposal.id,
            &allowed.cell,
            FenceAcknowledgment {
                cell: allowed.cell.clone(),
                invalidation: task.request,
                epoch: task.epoch,
                scopes: task.scopes,
                host_boot: p.proposal.context.producer.peer_boot.clone(),
                journal: allowed.host_journal.clone(),
                sequence: Counter(500),
            },
        )
        .unwrap();
    f.app
        .refresh_host_rejoin(
            &p.proposal.id,
            verified(fresh_rejoin_read(
                &e,
                f.clock.now(),
                &p.proposal.context,
                true,
            )),
        )
        .unwrap();
    let plan = f
        .app
        .host_rejoin_query_plan(&p.proposal.id, &allowed.operation)
        .unwrap();
    assert!(plan.lookup_allowed);
    assert!(f.app.host_rejoin_query_plan(&p.proposal.id, &id()).is_err());
    let missing = rx_application::host_rejoin::VerifiedQuery::new(
        p.proposal.id.clone(),
        allowed.operation.clone(),
        None,
        None,
        recovery::QueryLookup::Unavailable,
    )
    .unwrap();
    let missing = f.app.record_host_rejoin_query(missing).unwrap();
    assert!(!missing.result.evidence_complete && !missing.result.operation_authorized);
    let mut receipt = plan.original_receipt.unwrap();
    receipt.digest = Digest::from_bytes([99; 32]);
    let wrong = rx_application::host_rejoin::VerifiedQuery::new(
        p.proposal.id.clone(),
        allowed.operation.clone(),
        Some(receipt.clone()),
        None,
        recovery::QueryLookup::NotNeeded,
    )
    .unwrap();
    assert!(f.app.record_host_rejoin_query(wrong).is_err());
    receipt.digest = allowed.intent_digest;
    let valid = rx_application::host_rejoin::VerifiedQuery::new(
        p.proposal.id.clone(),
        allowed.operation.clone(),
        Some(receipt),
        None,
        recovery::QueryLookup::NotNeeded,
    )
    .unwrap();
    f.app
        .hold(&f.operator, id().as_str(), &allowed.cell)
        .unwrap();
    let late = f.app.record_host_rejoin_query(valid).unwrap();
    assert!(!late.context_current_at_record);
    assert!(
        f.app
            .host_rejoin_query_plan(&p.proposal.id, &allowed.operation)
            .is_err()
    );
    let mut repo = f.app.into_repository();
    let work: Work = repo
        .transact(|tx| Ok(p::load(tx, "work", &allowed.operation, "rx.internal.work.v1")?.1))
        .unwrap();
    assert_eq!(
        work.operation.outcome(),
        rx_domain::operation::Outcome::None
    );
    assert_ne!(
        work.operation.disposition(),
        rx_domain::operation::Disposition::Released
    );
}

#[test]
fn host_rejoin_legacy_proposal_encoding_stays_historical_and_owner_cannot_be_stolen() {
    let (mut f, release, input, e) = rejoin_read_fixture();
    let first = f
        .app
        .propose_host_rejoin(&release, &id(), input.clone(), verified(e.clone()))
        .unwrap();
    let mut legacy = serde_json::to_value(&first.proposal).unwrap();
    legacy.as_object_mut().unwrap().remove("recovery_scope");
    let decoded: rx_application::host_rejoin::Proposal =
        serde_json::from_value(legacy.clone()).unwrap();
    assert!(decoded.recovery_scope.is_none());
    assert_eq!(
        canonical::bytes(&decoded).unwrap(),
        canonical::bytes(&legacy).unwrap()
    );
    assert_eq!(
        decoded.digest().unwrap(),
        canonical::digest("RX-HOST-REJOIN-PROPOSAL-v1", &legacy).unwrap()
    );
    let second = f
        .app
        .propose_host_rejoin(&release, &id(), input, verified(e))
        .unwrap();
    f.app
        .approve_host_rejoin(&release, &id(), rejoin_approval_input(&first))
        .unwrap();
    assert!(matches!(
        f.app
            .approve_host_rejoin(&release, &id(), rejoin_approval_input(&second)),
        Err(StoreError::Rejected(Rejection::Busy))
    ));
}
#[test]
fn host_rejoin_late_fence_ack_is_retained_without_current_communication_or_authority() {
    let (mut f, release, input, e) = rejoin_read_fixture();
    let p = f
        .app
        .propose_host_rejoin(&release, &id(), input, verified(e.clone()))
        .unwrap();
    f.app
        .approve_host_rejoin(&release, &id(), rejoin_approval_input(&p))
        .unwrap();
    let cell = name("cell/a");
    let task = f
        .app
        .plan_host_rejoin_fence(&p.proposal.id, &cell, verified(e.clone()))
        .unwrap();
    f.app.hold(&f.operator, id().as_str(), &cell).unwrap();
    let ack = FenceAcknowledgment {
        cell: cell.clone(),
        invalidation: task.request,
        epoch: task.epoch,
        scopes: task.scopes,
        host_boot: p.proposal.context.producer.peer_boot.clone(),
        journal: e.configuration.snapshot.delivery_journal,
        sequence: Counter(500),
    };
    let b = f
        .app
        .record_host_rejoin_fence(&p.proposal.id, &cell, ack.clone())
        .unwrap();
    assert_eq!(b.phase, recovery::Phase::Attention);
    assert!(b.fences[&cell].acknowledgment.is_some());
    assert_eq!(
        b.revision,
        f.app
            .record_host_rejoin_fence(&p.proposal.id, &cell, ack)
            .unwrap()
            .revision
    );
    let v = f.app.host_rejoin_binding(&release, &p.proposal.id).unwrap();
    assert!(!v.context_current && !v.read_current && !v.operation_authorized);
}

fn rejoin_settlement_fixture() -> (
    Fixture,
    Identity,
    rx_application::host_rejoin::ApproveSettlement,
    rx_application::host_rejoin::VerifiedHandover,
) {
    rejoin_settlement_fixture_bad(0)
}
fn rejoin_settlement_fixture_bad(
    bad: u8,
) -> (
    Fixture,
    Identity,
    rx_application::host_rejoin::ApproveSettlement,
    rx_application::host_rejoin::VerifiedHandover,
) {
    let (mut f, release, input, e, native) = rejoin_read_fixture_state(2);
    let native = native.unwrap();
    let mut returned_native = native.clone();
    if bad == 7 {
        returned_native.status = Integer(9);
    }
    let p = f
        .app
        .propose_host_rejoin(&release, &id(), input, verified(e.clone()))
        .unwrap();
    f.app
        .approve_host_rejoin(&release, &id(), rejoin_approval_input(&p))
        .unwrap();
    let cell = name("cell/a");
    let task = f
        .app
        .plan_host_rejoin_fence(&p.proposal.id, &cell, verified(e.clone()))
        .unwrap();
    f.app
        .record_host_rejoin_fence(
            &p.proposal.id,
            &cell,
            FenceAcknowledgment {
                cell: cell.clone(),
                invalidation: task.request,
                epoch: task.epoch,
                scopes: task.scopes,
                host_boot: p.proposal.context.producer.peer_boot.clone(),
                journal: e.configuration.snapshot.delivery_journal.clone(),
                sequence: Counter(500),
            },
        )
        .unwrap();
    let read = fresh_rejoin_read(&e, f.clock.now(), &p.proposal.context, true);
    f.app
        .refresh_host_rejoin(&p.proposal.id, verified(read.clone()))
        .unwrap();
    let plan = f
        .app
        .host_rejoin_query_plan(&p.proposal.id, &native.operation)
        .unwrap();
    let query = rx_application::host_rejoin::VerifiedQuery::new(
        p.proposal.id.clone(),
        native.operation.clone(),
        plan.original_receipt,
        Some(EvidenceBatch {
            journal: p.proposal.context.producer.journal.clone(),
            first: Counter(1),
            records: vec![returned_native],
        }),
        recovery::QueryLookup::PrefixObserved,
    )
    .unwrap();
    let query = f.app.record_host_rejoin_query(query).unwrap();
    let work = f.app.inspect_work(&f.operator, &native.operation).unwrap();
    let mut proof = handover_proof(&mut f, &work, &native);
    for v in &mut proof.observations {
        v.host_boot = p.proposal.context.producer.peer_boot.clone();
    }
    match bad {
        1 => proof.observations[0].host_boot = id(),
        2 => proof.observations[0].device_session = id(),
        3 => proof.observations[2].value = false,
        4 => proof.observations[0].observed_at.ticks_ns = Counter(0),
        5 => proof.observations[1].invocation = id(),
        6 => proof.observations[2].quality_good = false,
        _ => {}
    }
    let command = rx_application::host_rejoin::ApproveSettlement {
        reference: rx_application::host_rejoin::SettlementReference {
            binding: p.proposal.id,
            query: query.id,
        },
        settlement: rx_application::settlement::Approve {
            operation: native.operation,
            expected_operation: work.operation.revision(),
            expected_cell: proof.expected_cell,
            justification: "Known original effect; fresh current handover only".into(),
        },
    };
    (
        f,
        release,
        command,
        rx_application::host_rejoin::VerifiedHandover::new(verified(read), proof.observations)
            .unwrap(),
    )
}
#[test]
fn rejoin_settlement_closes_original_work_without_rebinding_or_restoring_old_authority() {
    for failure in [0, 1, 2] {
        let (mut f, release, command, proof) = rejoin_settlement_fixture();
        let cell_before = f.app.inspect_cell(&f.operator, &name("cell/a")).unwrap().1;
        let old_host = f.registrations[0].clone();
        let key = id();
        let auth = f
            .app
            .approve_rejoin_settlement(&release, &key, command.clone())
            .unwrap();
        f.failure.store(failure, Ordering::SeqCst);
        let result = f.app.apply_rejoin_settlement(&auth.id, proof.clone());
        assert_eq!(result.is_ok(), failure == 0);
        let done = f.app.apply_rejoin_settlement(&auth.id, proof).unwrap();
        assert!(done.applied_at.is_some());
        let receipt = f
            .app
            .approve_rejoin_settlement(&release, &key, command.clone())
            .unwrap();
        assert_eq!(receipt.applied_at, done.applied_at);
        let work = f
            .app
            .inspect_work(&f.operator, &command.settlement.operation)
            .unwrap();
        assert_eq!(
            work.operation.outcome(),
            rx_domain::operation::Outcome::Succeeded
        );
        assert_eq!(
            work.operation.disposition(),
            rx_domain::operation::Disposition::Released
        );
        let context = f
            .app
            .host_rejoin_context(&release, &work.host, &work.cell)
            .unwrap();
        assert_eq!(
            canonical::bytes(&old_host).unwrap(),
            canonical::bytes(context.cells[&work.cell].registration.as_ref().unwrap()).unwrap()
        );
        assert_eq!(
            canonical::bytes(&cell_before).unwrap(),
            canonical::bytes(&context.cells[&work.cell].cell).unwrap()
        );
        let mut repo = f.app.into_repository();
        repo.transact(|tx| {
            let (_, run): (_, Run) = p::load(tx, "run", &work.run, "rx.internal.run.v1")?;
            assert_eq!(run.state, RunState::Completed);
            let (_, permit): (_, Permit) =
                p::load(tx, "permit", &work.permit, "rx.internal.permit.v1")?;
            assert_eq!(permit.host_boot, old_host.boot_id);
            let (_, mandate): (_, Mandate) =
                p::load(tx, "mandate", &permit.mandate, "rx.internal.mandate.v1")?;
            assert_eq!(mandate.state, MandateState::Revoked);
            for r in &work.intent.resource_set {
                let (_, resource): (_, Resource) =
                    p::load(tx, "resource", r, "rx.internal.resource.v1")?;
                assert!(resource.holder.is_none() && !resource.quarantined);
            }
            Ok(())
        })
        .unwrap();
    }
}

#[test]
fn rejoin_settlement_rejects_wrong_result_and_ineligible_current_handover() {
    for bad in 1..=10 {
        let (mut f, release, command, proof) = rejoin_settlement_fixture_bad(bad);
        let key = id();
        let result = f
            .app
            .approve_rejoin_settlement(&release, &key, command.clone());
        if bad == 7 {
            assert!(result.is_err());
            continue;
        }
        let auth = result.unwrap();
        match bad {
            8 => {
                f.app
                    .hold(&f.operator, id().as_str(), &name("cell/a"))
                    .unwrap();
            }
            9 => {
                f.app
                    .put_principal(
                        &f.admin,
                        principal(release.principal.as_str(), &[Role::Observer]),
                        Some(Counter(1)),
                    )
                    .unwrap();
            }
            10 => {
                f.clock.0.store(30_000_001_001, Ordering::SeqCst);
            }
            _ => {}
        }
        assert!(
            f.app.apply_rejoin_settlement(&auth.id, proof).is_err(),
            "case {bad}"
        );
        if bad != 10 {
            assert_eq!(
                f.app
                    .inspect_work(&f.operator, &command.settlement.operation)
                    .unwrap()
                    .operation
                    .disposition(),
                rx_domain::operation::Disposition::Quarantined
            );
        }
        let mut repo = f.app.into_repository();
        repo.transact(|tx| {
            let (_, authorization): (_, rx_application::settlement::Authorization) = p::load(
                tx,
                "settlement",
                &auth.id,
                "rx.internal.settlement-authorization.v1",
            )?;
            assert!(authorization.applied_at.is_none());
            for row in tx.scan("resource/")? {
                let resource: Resource = p::decode(&row, "rx.internal.resource.v1")?;
                assert!(resource.holder.is_some() && resource.quarantined);
            }
            Ok(())
        })
        .unwrap();
    }
}

fn rebind_fixture(
    cells: usize,
) -> (
    Fixture,
    Identity,
    rx_application::host_rejoin::BindingView,
    recovery::ReadEvidence,
) {
    let (mut f, input, mut release) = rejoin_fixture(cells);
    release.session = f
        .app
        .authenticated_terminal_user_session(
            &release.principal,
            id(),
            Counter(1_000_000_000_000),
            Digest::from_bytes([77; 32]),
        )
        .unwrap()
        .id;
    let who = replace_host(
        &mut f,
        &input,
        id(),
        input.snapshot.evidence_journal.clone(),
        Digest::from_bytes([81; 32]),
    );
    if cells == 2 {
        f.app
            .negotiate_evidence_cell(&who, artifact(12, "rx.cell-definition.v1").sha256)
            .unwrap();
    }
    let context = f
        .app
        .host_rejoin_context(&release, &input.host, &input.snapshot.cell)
        .unwrap();
    assert!(
        context.local_prerequisites_current,
        "{:?}",
        context.blockers
    );
    let mut evidence = recovery::ReadEvidence {
        platform_session: id(),
        transport: pin(),
        configuration: configuration_read(&input.snapshot),
        configuration_started: f.clock.now(),
        configuration_finished: f.clock.now(),
        cells: BTreeMap::new(),
    };
    evidence.configuration.snapshot.host_boot = context.producer.peer_boot.clone();
    evidence.configuration.snapshot.cells.clear();
    for (cell, cut) in &context.cells {
        let mut snap = input.snapshot.clone();
        let reg = cut.registration.as_ref().unwrap();
        snap.cell = cell.clone();
        snap.definition = cut.cell.configuration.definition.sha256;
        snap.envelope = cut.cell.configuration.envelope.sha256;
        snap.host_boot = context.producer.peer_boot.clone();
        snap.epoch = reg.epoch;
        snap.scopes = reg.scopes.clone();
        snap.resource_fences = reg
            .grant
            .resources
            .iter()
            .map(|r| (r.clone(), reg.grant.fence))
            .collect();
        evidence
            .configuration
            .snapshot
            .cells
            .push(configuration_read(&snap).snapshot.cells[0].clone());
        evidence.cells.insert(
            cell.clone(),
            recovery::SnapshotRead {
                snapshot: snap,
                started: f.clock.now(),
                finished: f.clock.now(),
            },
        );
    }
    let input = rx_application::host_rejoin::Prepare {
        host: context.host.clone(),
        origin: context.origin.clone(),
        expected_context: context.digest().unwrap(),
        expected_cells: context.expected_cells(),
    };
    let p = f
        .app
        .propose_host_rejoin(&release, &id(), input, verified(evidence.clone()))
        .unwrap();
    f.app
        .approve_host_rejoin(&release, &id(), rejoin_approval_input(&p))
        .unwrap();
    for (i, cell) in context.cells.keys().enumerate() {
        let task = f
            .app
            .plan_host_rejoin_fence(&p.proposal.id, cell, verified(evidence.clone()))
            .unwrap();
        f.app
            .record_host_rejoin_fence(
                &p.proposal.id,
                cell,
                FenceAcknowledgment {
                    cell: cell.clone(),
                    invalidation: task.request,
                    epoch: task.epoch,
                    scopes: task.scopes,
                    host_boot: context.producer.peer_boot.clone(),
                    journal: context.cells[cell]
                        .baseline
                        .as_ref()
                        .unwrap()
                        .delivery_journal
                        .clone(),
                    sequence: Counter(100 + i as u64),
                },
            )
            .unwrap();
        // Actual following reads must show the fence already applied on the affected cell.
        let read = evidence.cells.get_mut(cell).unwrap();
        read.snapshot.epoch = context.cells[cell].cell.epoch;
        read.snapshot.scopes = context.cells[cell].cell.scope_epochs.clone();
        read.snapshot.block_ids = context.cells[cell]
            .cell
            .blocks
            .iter()
            .filter(|b| b.latched)
            .map(|b| b.id.clone())
            .collect();
        let c = evidence
            .configuration
            .snapshot
            .cells
            .iter_mut()
            .find(|c| &c.cell == cell)
            .unwrap();
        c.epoch = read.snapshot.epoch;
        c.scopes = read.snapshot.scopes.clone();
        c.blocked = read.snapshot.block_ids.clone();
    }
    f.app
        .refresh_host_rejoin(&p.proposal.id, verified(evidence.clone()))
        .unwrap();
    let view = f.app.host_rejoin_binding(&release, &p.proposal.id).unwrap();
    (f, release, view, evidence)
}
fn rebind_approval(
    view: &rx_application::host_rejoin::BindingView,
) -> rx_application::host_rejoin::ApproveRebind {
    rx_application::host_rejoin::ApproveRebind {
        binding: view.binding.id.clone(),
        expected_binding_revision: view.binding.revision,
        proposal_digest: view.proposal.digest().unwrap(),
        expected_cells: view.proposal.context.expected_cells(),
    }
}
#[test]
fn host_rebind_commits_all_cells_together_and_preserves_old_baselines() {
    for failure in [0, 1, 2] {
        let (mut f, release, parent, mut evidence) = rebind_fixture(2);
        let who = &parent.proposal.context.producer;
        let old_session = parent
            .proposal
            .context
            .replacement
            .as_ref()
            .unwrap()
            .before
            .session
            .clone();
        let approval = f
            .app
            .approve_host_rebind(&release, &id(), rebind_approval(&parent))
            .unwrap();
        assert!(!approval.production_authorized);
        let ttls = parent
            .proposal
            .context
            .cells
            .keys()
            .map(|c| (c.clone(), Counter(1000)))
            .collect();
        let mut prepared = f
            .app
            .prepare_host_rebind(&approval.rebind.id, verified(evidence.clone()), ttls)
            .unwrap();
        assert_eq!(prepared.steps.len(), 2);
        for (index, cell) in parent.proposal.context.cells.keys().enumerate() {
            assert!(
                f.app
                    .ready_host_rebind(&who.principal, cell, &old_session)
                    .unwrap()
                    .is_none()
            );
            let plan = f
                .app
                .plan_host_rebind_grant(&prepared.id, cell, verified(evidence.clone()))
                .unwrap();
            let retry = f
                .app
                .plan_host_rebind_grant(&prepared.id, cell, verified(evidence.clone()))
                .unwrap();
            assert_eq!(plan.grant_request, retry.grant_request);
            let mut reply = link_commit(&f, &plan);
            reply.fence_receipt = prepared.steps[cell].fence.clone();
            reply.grant_sent_at = plan.prepared_at.clone();
            assert!(
                f.app.commit_host_link(reply.clone()).is_err(),
                "ordinary commit must not adopt changed Host"
            );
            for r in &plan.resources {
                evidence
                    .cells
                    .get_mut(cell)
                    .unwrap()
                    .snapshot
                    .resource_fences
                    .insert(r.clone(), plan.fence);
            }
            prepared = f
                .app
                .record_host_rebind_grant(&prepared.id, cell, reply, &plan.host_boot)
                .unwrap();
            if index == 0 {
                assert!(
                    f.app
                        .commit_host_rebind(&prepared.id, verified(evidence.clone()))
                        .is_err()
                );
            }
        }
        f.failure.store(failure, Ordering::SeqCst);
        let first = f
            .app
            .commit_host_rebind(&prepared.id, verified(evidence.clone()));
        assert_eq!(first.is_ok(), failure == 0);
        let bound = f
            .app
            .commit_host_rebind(&prepared.id, verified(evidence.clone()))
            .unwrap();
        assert_eq!(bound.phase, rx_application::host_rejoin::RebindPhase::Bound);
        for (cell, step) in &bound.steps {
            assert_eq!(
                f.app
                    .ready_host_rebind(&who.principal, cell, &old_session)
                    .unwrap(),
                Some(bound.id.clone())
            );
            let actual = f.app.bound_host_link(&step.plan.id).unwrap();
            assert_eq!(actual.boot_id, who.peer_boot);
            assert_eq!(actual.grant.id, step.commit.as_ref().unwrap().grant.id);
            assert!(
                f.app
                    .host_binding_baseline(&step.plan.id)
                    .unwrap()
                    .is_some()
            );
            assert!(
                f.app
                    .host_binding_baseline(
                        &parent.proposal.context.cells[cell]
                            .baseline
                            .as_ref()
                            .unwrap()
                            .plan
                    )
                    .unwrap()
                    .is_some()
            );
        }
        assert!(
            !f.app
                .host_rebind(&release, &bound.id)
                .unwrap()
                .production_authorized
        );
    }
}
#[test]
fn host_rebind_requires_closed_work_and_exact_current_approval_and_read() {
    let (mut f, release, command, _) = rejoin_settlement_fixture();
    let parent = f
        .app
        .host_rejoin_binding(&release, &command.reference.binding)
        .unwrap();
    assert!(
        f.app
            .approve_host_rebind(&release, &id(), rebind_approval(&parent))
            .is_err()
    );
    for bad in 0..6 {
        let (mut f, release, parent, mut read) = rebind_fixture(1);
        let mut request = rebind_approval(&parent);
        if bad == 0 {
            request.proposal_digest = Digest::from_bytes([99; 32]);
            assert!(f.app.approve_host_rebind(&release, &id(), request).is_err());
            continue;
        }
        let approved = f.app.approve_host_rebind(&release, &id(), request).unwrap();
        match bad {
            1 => {
                read.cells
                    .get_mut(&name("cell/a"))
                    .unwrap()
                    .snapshot
                    .host_boot = id()
            }
            2 => read
                .cells
                .get_mut(&name("cell/a"))
                .unwrap()
                .snapshot
                .pending_operations
                .push(id()),
            3 => {
                f.app
                    .hold(&f.operator, id().as_str(), &name("cell/a"))
                    .unwrap();
            }
            4 => {
                f.app
                    .put_principal(
                        &f.admin,
                        principal(release.principal.as_str(), &[Role::Observer]),
                        Some(Counter(1)),
                    )
                    .unwrap();
            }
            5 => {
                f.clock.0.store(100_001_001, Ordering::SeqCst);
            }
            _ => {}
        }
        assert!(
            f.app
                .prepare_host_rebind(
                    &approved.rebind.id,
                    verified(read),
                    BTreeMap::from([(name("cell/a"), Counter(1000))])
                )
                .is_err(),
            "case {bad}"
        );
    }
}

#[test]
fn host_rebind_rejects_wrong_grant_and_retains_late_reply_without_adoption() {
    for bad in 0..4 {
        let (mut f, release, parent, read) = rebind_fixture(1);
        let approved = f
            .app
            .approve_host_rebind(&release, &id(), rebind_approval(&parent))
            .unwrap();
        let prepared = f
            .app
            .prepare_host_rebind(
                &approved.rebind.id,
                verified(read.clone()),
                BTreeMap::from([(name("cell/a"), Counter(1000))]),
            )
            .unwrap();
        let cell = name("cell/a");
        let plan = f
            .app
            .plan_host_rebind_grant(&prepared.id, &cell, verified(read))
            .unwrap();
        let mut reply = link_commit(&f, &plan);
        reply.fence_receipt = prepared.steps[&cell].fence.clone();
        let mut boot = plan.host_boot.clone();
        match bad {
            0 => reply.grant.fence = Counter(900),
            1 => reply.grant.owner = name("another/platform"),
            2 => boot = id(),
            _ => {
                f.app.hold(&f.operator, id().as_str(), &cell).unwrap();
            }
        }
        let result = f
            .app
            .record_host_rebind_grant(&prepared.id, &cell, reply, &boot);
        if bad < 3 {
            assert!(result.is_err());
        } else {
            let b = result.unwrap();
            assert_eq!(b.phase, rx_application::host_rejoin::RebindPhase::Attention);
            assert!(b.steps[&cell].commit.is_some());
        }
        let mut repo = f.app.into_repository();
        repo.transact(|tx| {
            let (_, host): (_, HostRegistration) = p::load(
                tx,
                "host",
                (&cell, &parent.proposal.context.host),
                "rx.internal.host-registration.v1",
            )?;
            assert_eq!(
                host.boot_id,
                parent.proposal.context.cells[&cell]
                    .registration
                    .as_ref()
                    .unwrap()
                    .boot_id
            );
            assert!(tx.scan("host-rebind-configuration-origin/")?.is_empty());
            Ok(())
        })
        .unwrap();
    }
}

#[path = "host_rebind_qualification_tests.rs"]
mod host_rebind_qualification_tests;
