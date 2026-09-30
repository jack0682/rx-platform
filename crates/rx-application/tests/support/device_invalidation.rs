use super::*;
use rx_application::device_invalidation as device;
use rx_application::observation::Disposition;

fn loss_fact(cell: &Name, host: &Name, generation: &Id) -> FactRecord {
    FactRecord {
        cell: cell.clone(),
        id: name("ready"),
        source_host: host.clone(),
        source_generation: generation.clone(),
        schema: name("boolean/v1"),
        unit: name("unitless"),
        acquired_at: expiry(2000),
        maximum_age_ns: Counter(20000),
        acquisition_uncertainty_ns: Counter(0),
        quality_good: true,
        origin_age_bounded: true,
        disputed: false,
        value: TypedValue::Boolean(true),
        evidence_id: id(),
    }
}

/// Applied change + configured policy + a raised DeviceRestart block from a source-generation
/// loss on the registered Host. Returns the block id, its recorded origin digest and the new
/// source generation the Host must present to prove re-link.
fn armed() -> (
    Fixture,
    Identity,
    process_change::Change,
    qsupport::Fixture,
    review_support::Fixture,
    Id,
    Digest,
    Id,
) {
    let (mut f, release, c, p, source) = setup_sources();
    let generation = id();
    let receipt = f
        .app
        .report_facts(
            &f.hosts[0],
            vec![loss_fact(&c.cell, &f.hosts[0].principal, &generation)],
        )
        .unwrap();
    assert_eq!(
        receipt.entries[0].disposition,
        Disposition::GenerationChanged
    );
    let view = f.app.device_restrictions(&f.admin, &c.cell).unwrap();
    assert_eq!(view.restrictions.len(), 1);
    let block = view.restrictions[0].block.id.clone();
    let digest = view.restrictions[0].origin_digest.unwrap();
    (f, release, c, p, source, block, digest, generation)
}

fn continuity_unproven<T: std::fmt::Debug>(result: Result<T, StoreError>) {
    assert!(
        matches!(
            result,
            Err(StoreError::Rejected(Rejection::ContinuityUnproven))
        ),
        "{result:?}"
    );
}

fn device_input(
    f: &mut Fixture,
    c: &process_change::Change,
    p: &qsupport::Fixture,
    block: &Id,
    digest: Digest,
) -> q::Begin {
    let mut input = begin_input(f, c, p);
    input.device_restrictions = std::collections::BTreeMap::from([(block.clone(), digest)]);
    input
}

#[test]
fn source_generation_loss_records_one_origin_with_the_superseded_generation() {
    let (f, _release, c, _p, _source, block, _digest, generation) = armed();
    let previous = f.registrations[0].source_sessions[&name("ready")].clone();
    assert_ne!(previous, generation);
    let installation = f.app.installation.clone();
    let mut repository = f.app.into_repository();
    repository
        .transact(|tx| {
            assert_eq!(tx.scan("deviceinvalidationorigin/")?.len(), 1);
            let origin = device::read_for_cell(tx, &installation, &c.cell, &block)?.unwrap();
            let device::DeviceInvalidationCause::SourceGenerationChanged {
                source,
                previous_generation,
                generation: recorded,
            } = &origin.cause
            else {
                panic!("expected SourceGenerationChanged");
            };
            assert_eq!(*source, name("ready"));
            assert_eq!(previous_generation.as_ref(), Some(&previous));
            assert_eq!(*recorded, generation);
            assert_eq!(origin.host, name("host/0"));
            assert_eq!(origin.registration_cell, c.cell);
            assert_eq!(origin.registration_epoch, origin.after.epoch);
            assert!(origin.digest().is_ok());
            // read_for_cell binds the exact cell: another cell's read of the same block fails.
            assert!(device::read_for_cell(tx, &installation, &name("cell/b"), &block).is_err());
            Ok(())
        })
        .unwrap();
}

#[test]
fn device_restriction_selection_before_relink_is_continuity_unproven() {
    // The registered Host still presents the superseded generation, so the release precondition
    // is not met: provenance exists but is not permission.
    let (mut f, release, c, p, _source, block, digest, _generation) = armed();
    let input = device_input(&mut f, &c, &p, &block, digest);
    continuity_unproven(f.app.begin_requalification(&release, &id(), input));
    assert!(
        f.app
            .inspect_cell(&f.admin, &c.cell)
            .unwrap()
            .1
            .blocks
            .iter()
            .any(|b| b.id == block)
    );
}

#[test]
fn device_restriction_selection_with_a_stale_digest_is_rejected() {
    let (mut f, release, c, p, _source, block, _digest, _generation) = armed();
    let input = device_input(&mut f, &c, &p, &block, Digest::from_bytes([1; 32]));
    assert!(matches!(
        f.app.begin_requalification(&release, &id(), input),
        Err(StoreError::Rejected(Rejection::StaleRevision))
    ));
}

#[test]
fn a_device_block_without_provenance_can_never_be_selected() {
    // A block id with no recorded origin (a legacy DeviceRestart, or none at all) is refused as
    // ContinuityUnproven before any digest or precondition check.
    let (mut f, release, c, p, _source, _block, _digest, _generation) = armed();
    let input = device_input(&mut f, &c, &p, &id(), Digest::from_bytes([0; 32]));
    continuity_unproven(f.app.begin_requalification(&release, &id(), input));
}

#[test]
fn a_v2_request_may_not_carry_device_restrictions() {
    let (mut f, release, c, p, _source, _block, _digest, _generation) = armed();
    let origin = f
        .app
        .device_restrictions(&f.admin, &c.cell)
        .unwrap()
        .restrictions
        .into_iter()
        .find_map(|r| r.origin)
        .unwrap();
    // A requalification with no device selection is a v2 request.
    let input = begin_input(&mut f, &c, &p);
    let j = f.app.begin_requalification(&release, &id(), input).unwrap();
    assert_eq!(j.request.schema, name("rx.requalification-request.v2"));
    assert!(j.request.device_restrictions.is_empty());
    // Attaching a device restriction to a v2 request is rejected by the digest schema rules.
    let mut request = j.request.clone();
    request.device_restrictions = vec![origin];
    assert!(request.digest().is_err());
}

#[test]
fn a_neighbouring_cell_names_the_registration_cell_it_is_released_through() {
    let (mut f, _release, c, _p, _source) = setup_sources();
    // cell/b shares cell/a's scope, so a loss on cell/a's Host reaches it through the closure.
    let mut other = f
        .app
        .inspect_cell(&f.admin, &c.cell)
        .unwrap()
        .1
        .configuration;
    other.id = name("cell/b");
    other.process = None;
    other.hosts = vec![name("host/b")];
    for s in &mut other.steps {
        s.host = name("host/b");
    }
    for spec in &mut other.fact_specs {
        spec.host = name("host/b");
    }
    f.app.install_cell(&f.admin, other).unwrap();
    f.app
        .report_facts(
            &f.hosts[0],
            vec![loss_fact(&c.cell, &f.hosts[0].principal, &id())],
        )
        .unwrap();
    let own = f.app.device_restrictions(&f.admin, &c.cell).unwrap();
    let neighbour = f
        .app
        .device_restrictions(&f.admin, &name("cell/b"))
        .unwrap();
    assert_eq!(own.restrictions.len(), 1);
    assert_eq!(neighbour.restrictions.len(), 1);
    let origin = neighbour.restrictions[0].origin.as_ref().unwrap();
    assert_eq!(origin.cell, name("cell/b"));
    assert_eq!(origin.host, name("host/0"));
    // host/0 has no registration on cell/b: its release goes through the cell/a registration,
    // re-linked at or after the epoch this invalidation left cell/a at.
    assert_eq!(origin.registration_cell, c.cell);
    assert_eq!(origin.registration_epoch, own.epoch);
}

/// Open host/0's evidence producer for `boot` with the kept evidence journal.
fn open_producer(f: &mut Fixture, boot: &Id, evidence_journal: &Id) {
    let session = f
        .app
        .open_evidence_producer(
            &name("host/0"),
            boot.clone(),
            evidence_journal.clone(),
            Digest::from_bytes([81; 32]),
        )
        .unwrap();
    f.hosts[0].session = session.id;
    let definition = f
        .app
        .inspect_cell(&f.admin, &f.configuration.id)
        .unwrap()
        .1
        .configuration
        .definition
        .sha256;
    f.app
        .negotiate_evidence_cell(&f.hosts[0], definition)
        .unwrap();
}

/// Link host/0's open producer to the cell as in operation: a snapshot of the current cell
/// observing `ready` at `generation`, a fence receipt at `sequence` on the delivery journal.
fn commit_link(
    f: &mut Fixture,
    boot: &Id,
    delivery_journal: &Id,
    evidence_journal: &Id,
    generation: &Id,
    sequence: u64,
) -> HostRegistration {
    use rx_domain::host_snapshot::*;
    let host = name("host/0");
    let (_, cell) = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap();
    let c = &cell.configuration;
    let snapshot = HostSnapshot {
        schema: name("rx.host-snapshot.v1"),
        host: host.clone(),
        host_boot: boot.clone(),
        delivery_journal: delivery_journal.clone(),
        evidence_journal: evidence_journal.clone(),
        cell: c.id.clone(),
        definition: c.definition.sha256,
        envelope: c.envelope.sha256,
        environment: name("SIMULATION"),
        epoch: cell.epoch,
        scopes: cell.scope_epochs.clone(),
        resource_fences: c
            .steps
            .iter()
            .filter(|s| s.host == host)
            .flat_map(|s| {
                s.intent
                    .resource_set
                    .iter()
                    .map(|r| (r.clone(), Counter(0)))
            })
            .collect(),
        captured_at: f.clock.now(),
        sources_available: true,
        observations: c
            .fact_specs
            .iter()
            .filter(|s| s.host == host)
            .map(|s| SourceObservation {
                source: s.id.clone(),
                generation: generation.clone(),
                schema: s.schema.clone(),
                unit: s.unit.clone(),
                value: TypedValue::Boolean(true),
                acquired_at: f.clock.now(),
                uncertainty_ns: Counter(0),
                quality_good: true,
                origin_age_bounded: true,
                disputed: false,
                evidence_id: id(),
            })
            .collect(),
        block_ids: cell
            .blocks
            .iter()
            .filter(|b| b.latched)
            .map(|b| b.id.clone())
            .collect(),
        pending_operations: vec![],
        pending_permits: vec![],
    };
    let plan = f
        .app
        .prepare_host_link(rx_application::host_link::Prepare {
            host,
            platform_session: id(),
            snapshot,
            read_started: f.clock.now(),
            ttl_ms: Counter(1000),
            provenance: None,
        })
        .unwrap();
    let mut commit = link_commit(f, &plan);
    commit.fence_receipt.sequence = Counter(sequence);
    commit.grant.valid_until.ticks_ns =
        Counter(plan.prepared_at.ticks_ns.0 + plan.ttl_ms.0 * 1_000_000);
    f.app.commit_host_link(commit).unwrap()
}

/// The requalification fixture with host/0 registered through a bound link, as in operation,
/// so that a restarted Host can be re-admitted and re-linked. Returns the kept evidence journal.
fn linked_applied() -> (
    Fixture,
    Identity,
    process_change::Change,
    qsupport::Fixture,
    review_support::Fixture,
    Id,
) {
    let mut source = None;
    let mut f = fixture_configured((1, false, false, false, None, false, false), |cfg| {
        let (cfg, bytes) = qsupport::configure(cfg);
        source = Some(bytes);
        cfg
    });
    let (boot, delivery, evidence) = (id(), id(), id());
    open_producer(&mut f, &boot, &evidence);
    f.registrations = vec![commit_link(&mut f, &boot, &delivery, &evidence, &id(), 1)];
    report_ready(
        &mut f.app,
        &f.hosts[0],
        &f.configuration,
        &f.registrations[0],
    );
    let (p, job, release, c) = prepared_materials(&mut f);
    authorize_and_observe(&mut f, &release, &c);
    let t = apply_prepared(&mut f, &p, &job, &release, &c, &id());
    let applied = f.app.commit_process_change_apply(t).unwrap();
    let cfg = f
        .app
        .inspect_cell(&f.admin, &c.cell)
        .unwrap()
        .1
        .configuration;
    let proof = qsupport::policy(&cfg, source.unwrap());
    f.app
        .configure_requalification(Some(proof.policy.clone()))
        .unwrap();
    (f, release, applied, proof, p, evidence)
}

#[test]
fn a_relinked_host_releases_its_device_restrictions_only_at_requalification_activation() {
    let (mut f, release, c, p, source, evidence) = linked_applied();
    let previous = f.registrations[0].clone();
    // The source generation changes under the registered Host, then the Host restarts on the
    // same storage: one SourceGenerationChanged and one ProducerReplaced restriction.
    let generation = id();
    f.app
        .report_facts(
            &f.hosts[0],
            vec![loss_fact(&c.cell, &previous.id, &generation)],
        )
        .unwrap();
    let boot = id();
    open_producer(&mut f, &boot, &evidence);
    let view = f.app.device_restrictions(&f.admin, &c.cell).unwrap();
    assert_eq!(view.restrictions.len(), 2);
    let selected: std::collections::BTreeMap<_, _> = view
        .restrictions
        .iter()
        .map(|r| (r.block.id.clone(), r.origin_digest.unwrap()))
        .collect();
    let mut input = begin_input(&mut f, &c, &p);
    input.device_restrictions = selected.clone();
    // Recorded provenance is not a re-link.
    continuity_unproven(f.app.begin_requalification(&release, &id(), input));

    // Explicit re-admission of the replaced generation, then a link of the new boot that reads
    // the source afresh.
    f.app
        .approve_host_readmission(
            &release,
            &id(),
            rx_application::host_readmission::Approve {
                host: previous.id.clone(),
                previous_boot: previous.boot_id.clone(),
                delivery_journal: previous.delivery_journal.clone(),
                evidence_journal: evidence.clone(),
                binding_intent: None,
            },
        )
        .unwrap();
    let relinked = commit_link(
        &mut f,
        &boot,
        &previous.delivery_journal,
        &evidence,
        &generation,
        4242,
    );
    assert_eq!(relinked.boot_id, boot);
    f.registrations[0] = relinked;
    // Re-linking releases nothing by itself.
    let cell = f.app.inspect_cell(&f.admin, &c.cell).unwrap().1;
    assert!(
        selected
            .keys()
            .all(|b| cell.blocks.iter().any(|x| &x.id == b))
    );

    let mut input = begin_input(&mut f, &c, &p);
    input.device_restrictions = selected.clone();
    let j = f.app.begin_requalification(&release, &id(), input).unwrap();
    assert_eq!(j.request.schema, name("rx.requalification-request.v3"));
    assert_eq!(j.request.device_restrictions.len(), 2);
    ack(&mut f, &j);
    let report = prepared_report(&mut f, &j, &p, &id(), None);
    let v = f.app.commit_requalification_report(report).unwrap();
    let reviewer = add_identity(&mut f.app, &f.admin, "device-reviewer", &[Role::Verifier]);
    let q::DecisionPreflight::Verify(t) = f
        .app
        .prepare_requalification_decision(&reviewer, &id(), decision(&v, &j))
        .unwrap()
    else {
        panic!("decision")
    };
    let d = f
        .app
        .commit_requalification_decision(t.verify(&p.policy).unwrap())
        .unwrap();
    // Begin advanced the cell epoch past the link; the re-link still proves continuity.
    let batch = issue(&mut f, &release, &j, &v, &d, &p, &source);
    let cell = f.app.inspect_cell(&f.admin, &c.cell).unwrap().1;
    assert!(
        selected
            .keys()
            .all(|b| cell.blocks.iter().any(|x| &x.id == b))
    );
    confirm(&mut f, &batch);
    activate(&mut f, &release, &batch, &p, &source);
    let cell = f.app.inspect_cell(&f.admin, &c.cell).unwrap().1;
    assert!(
        selected
            .keys()
            .all(|b| !cell.blocks.iter().any(|x| &x.id == b))
    );
    assert!(cell.qualification.is_some());
}

#[test]
fn a_fence_acknowledgement_is_not_a_relink_even_when_the_generation_is_unchanged() {
    let (mut f, release, c, p, _source, _evidence) = linked_applied();
    let registered = f.registrations[0].clone();
    // The same source generation read against another clock cannot be ordered, so it is a
    // generation loss although the generation id itself is unchanged.
    let mut fact = loss_fact(
        &c.cell,
        &registered.id,
        &registered.source_sessions[&name("ready")],
    );
    fact.acquired_at.clock_id = "test/other-clock".into();
    let receipt = f.app.report_facts(&f.hosts[0], vec![fact]).unwrap();
    assert_eq!(
        receipt.entries[0].disposition,
        Disposition::GenerationChanged
    );
    // The Host acknowledges the invalidation fence with its unchanged boot, journal and
    // session. That advances its registration to the cell epoch without any new link.
    let cell = f.app.inspect_cell(&f.admin, &c.cell).unwrap().1;
    let delivery = f
        .app
        .pending_deliveries(128)
        .unwrap()
        .into_iter()
        .find(|d| matches!(&d.payload, Delivery::Fence { epoch, .. } if *epoch == cell.epoch))
        .unwrap();
    f.app.plan_delivery(&f.hosts[0], &delivery.id).unwrap();
    f.app
        .finish_fence_delivery(
            &f.hosts[0],
            &delivery.id,
            FenceAcknowledgment {
                cell: c.cell.clone(),
                invalidation: delivery.id.clone(),
                epoch: cell.epoch,
                scopes: cell.scope_epochs.clone(),
                host_boot: registered.boot_id.clone(),
                journal: registered.delivery_journal.clone(),
                sequence: Counter(700),
            },
        )
        .unwrap();
    let view = f.app.device_restrictions(&f.admin, &c.cell).unwrap();
    assert_eq!(view.restrictions.len(), 1);
    let mut input = begin_input(&mut f, &c, &p);
    input.device_restrictions = std::collections::BTreeMap::from([(
        view.restrictions[0].block.id.clone(),
        view.restrictions[0].origin_digest.unwrap(),
    )]);
    continuity_unproven(f.app.begin_requalification(&release, &id(), input));
    // The premise holds: the registration now carries the cell epoch and the unchanged
    // generation, so an epoch-and-generation check alone would have accepted it.
    let mut repository = f.app.into_repository();
    repository
        .transact(|tx| {
            let (_, now): (_, HostRegistration) = rx_application::persistence::load(
                tx,
                "host",
                (&c.cell, &registered.id),
                "rx.internal.host-registration.v1",
            )?;
            assert_eq!(now.epoch, cell.epoch);
            assert_eq!(now.source_sessions, registered.source_sessions);
            assert_eq!(now.boot_id, registered.boot_id);
            Ok(())
        })
        .unwrap();
}
