//! P activation integration with test evidence and simulated Host acknowledgements.
use super::*;
use crate::configuration_dispatch_tests::requalification_tests as review;
use review::qualification_activation_tests as activation;
use rx_application::{qualification_activation as a, requalification as q};
use rx_process_contract::execution_v2::host_qualification as wire;

fn blob(blobs: &mut BTreeMap<Digest, Vec<u8>>, schema: &str, bytes: Vec<u8>) -> ArtifactRef {
    let reference = signed_package::artifact(schema, &bytes);
    blobs.insert(reference.sha256, bytes);
    reference
}
#[allow(clippy::too_many_arguments)] // These are separate signed/stored sources, not interchangeable authority.
pub(super) fn activate_domain(
    f: &mut Fixture,
    release: &Identity,
    change: &process_change::Change,
    source: &review_support::Fixture,
    target: &CellConfiguration,
    published: &publication::Publication,
    inputs: &v2::InputClosure,
    package: &signed_package::Fixture,
    configured: &v2::host_configuration::Receipt,
    test_case: u8,
) {
    let expiry_control = test_case == 4;
    let release = &Identity {
        session: f
            .app
            .authenticated_terminal_user_session(
                &release.principal,
                id(),
                Counter(2_000_000_000_000),
                Digest::from_bytes([77; 32]),
            )
            .unwrap()
            .id,
        ..release.clone()
    };
    let saved = f
        .app
        .execution_preview(&f.admin, &published.preview)
        .unwrap();
    let policy = saved.policy().clone();
    let report_count = Counter((policy.candidates.len() * policy.slot_order.len()) as u64);
    let index = v2::ReportIndex {
        schema: name(v2::INDEX_SCHEMA),
        entries: (0..policy.candidates.len() as u8)
            .flat_map(|candidate| {
                let saved = &saved;
                (0..saved.policy().slot_order.len() as u16).map(move |slot| {
                    (
                        candidate,
                        slot,
                        saved.report(candidate, slot).unwrap().report_digest(),
                    )
                })
            })
            .collect(),
    };
    let mut blobs = BTreeMap::new();
    let input_ref = blob(
        &mut blobs,
        "rx.execution-input-closure.v2",
        canonical::bytes(inputs).unwrap(),
    );
    assert_eq!(input_ref, policy.definition_closure);
    let index_ref = blob(
        &mut blobs,
        v2::INDEX_SCHEMA,
        canonical::bytes(&index).unwrap(),
    );
    assert_eq!(index_ref, policy.report_index);
    let policy_ref = blob(
        &mut blobs,
        v2::POLICY_SCHEMA,
        canonical::bytes(&policy).unwrap(),
    );
    assert_eq!(policy_ref, published.policy);
    let publication_ref = blob(
        &mut blobs,
        "rx.workflow-publication.v2",
        canonical::bytes(published).unwrap(),
    );
    let plan = v2::Plan {
        schema: name(v2::PLAN_SCHEMA),
        binding: (**target.execution.as_ref().unwrap()).clone(),
        process: (**target.process.as_ref().unwrap()).clone(),
    };
    assert_eq!(
        blob(
            &mut blobs,
            v2::PLAN_SCHEMA,
            canonical::bytes(&plan).unwrap()
        ),
        target.recipe
    );
    assert_eq!(
        blob(&mut blobs, "test.definition.v1", b"definition".to_vec()),
        target.definition
    );
    assert_eq!(
        blob(&mut blobs, "test.envelope.v1", b"envelope".to_vec()),
        target.envelope
    );
    let site = blob(&mut blobs, "test.site.v1", b"site".to_vec());
    assert_eq!(site.sha256, target.site_config_digest);
    let configuration = blob(
        &mut blobs,
        target.schema(),
        canonical::bytes(target).unwrap(),
    );
    let mut dependencies = vec![
        policy_ref,
        input_ref,
        index_ref,
        publication_ref,
        target.recipe.clone(),
        site,
    ];
    for proof in published.packages.values() {
        dependencies.extend(proof.dependencies.clone());
    }
    for bytes in package.files.values() {
        blobs.insert(rx_package::content_digest(bytes), bytes.clone());
    }
    blobs.insert(
        rx_package::content_digest(&package.manifest),
        package.manifest.clone(),
    );
    blobs.insert(
        rx_package::content_digest(&package.signature),
        package.signature.clone(),
    );
    dependencies.sort_by_key(|r| (r.sha256, r.schema_id.clone()));
    dependencies.dedup();
    let specification = blob(
        &mut blobs,
        "test.criterion.v1",
        b"SIMULATION_TEST_ONLY".to_vec(),
    );
    let policy = q::Policy {
        schema: name(q::DERIVED_POLICY),
        profiles: vec![q::Profile {
            purposes: [name("PRODUCTION"), name("SETUP")].into(),
            cell: target.id.clone(),
            configuration,
            envelope: target.envelope.clone(),
            definition: target.definition.clone(),
            environment: Environment::Simulation,
            acceptance_plan: specification.clone(),
            limitations: specification.clone(),
            dependencies,
            criteria: [
                q::Area::Software,
                q::Area::Equipment,
                q::Area::CellIntegration,
                q::Area::Recovery,
                q::Area::Protection,
                q::Area::Operations,
            ]
            .into_iter()
            .enumerate()
            .map(|(i, area)| q::Criterion {
                id: name(&format!("test/{i}")),
                area,
                specification: specification.clone(),
                evidence_schema: name("rx.test.qualification-result.v1"),
            })
            .collect(),
        }],
        keys: vec![q::Key {
            id: name("test-qualification-signer"),
            public_key: Digest::from_bytes(
                ed25519_dalek::SigningKey::from_bytes(&[83; 32])
                    .verifying_key()
                    .to_bytes(),
            ),
            validators: [Digest::from_bytes([84; 32])].into(),
            environments: [name("SIMULATION")].into(),
        }],
    };
    let proof = review::qsupport::Fixture { policy, blobs };
    f.app
        .configure_requalification(Some(proof.policy.clone()))
        .unwrap();
    let begin = review::begin_input(f, change, &proof);
    let job = f.app.begin_requalification(release, &id(), begin).unwrap();
    review::ack(f, &job);
    let prepared = review::prepared_report(f, &job, &proof, &id(), None);
    let version = f.app.commit_requalification_report(prepared).unwrap();
    assert_eq!(version.derived.len(), 1);
    assert_eq!(version.derived[0].reports_checked, report_count);
    let verifier = add_identity(&mut f.app, &f.admin, "domain-reviewer", &[Role::Verifier]);
    let q::DecisionPreflight::Verify(ticket) = f
        .app
        .prepare_requalification_decision(&verifier, &id(), review::decision(&version, &job))
        .unwrap()
    else {
        panic!("review decision ticket")
    };
    let decision = f
        .app
        .commit_requalification_decision(ticket.verify(&proof.policy).unwrap())
        .unwrap();
    let issue = activation::issue_input(f, &job, &version, &decision);
    let a::Preflight::Verify(ticket) = f
        .app
        .prepare_qualification_issue(release, &id(), issue.clone())
        .unwrap()
    else {
        panic!("issue ticket")
    };
    let prepared = a::Prepared::verify(
        *ticket,
        &proof.policy,
        source
            .store
            .verify_owned(&source.object, &source.policy)
            .unwrap(),
    )
    .unwrap();
    f.clock.0.fetch_add(
        if expiry_control {
            q::DERIVED_TICKET_TTL_NS
        } else {
            31_000_000_000
        },
        Ordering::SeqCst,
    );
    let batch = if expiry_control {
        assert!(matches!(
            f.app.commit_qualification_issue(prepared),
            Err(StoreError::Rejected(Rejection::StaleRevision))
        ));
        assert!(
            f.app
                .qualification_tasks(&f.hosts[0], None)
                .unwrap()
                .is_empty()
        );
        let a::Preflight::Verify(ticket) = f
            .app
            .prepare_qualification_issue(release, &id(), issue)
            .unwrap()
        else {
            panic!("fresh issue ticket")
        };
        let prepared = a::Prepared::verify(
            *ticket,
            &proof.policy,
            source
                .store
                .verify_owned(&source.object, &source.policy)
                .unwrap(),
        )
        .unwrap();
        f.app.commit_qualification_issue(prepared).unwrap()
    } else {
        f.app.commit_qualification_issue(prepared).unwrap()
    };
    let task = f
        .app
        .qualification_tasks(&f.hosts[0], None)
        .unwrap()
        .into_iter()
        .find(|t| t.batch == batch.id)
        .unwrap();
    assert!(!task.execution_policies.is_empty());
    let legacy = activation::inspect(f, &batch, &task);
    assert!(
        f.app
            .bind_qualification_request(&f.hosts[0], &task.id, legacy.clone(), f.clock.now())
            .is_err()
    );
    let configured_stamps = configured
        .policies
        .iter()
        .map(|(cell, p)| {
            (
                cell.clone(),
                wire::ConfiguredPolicy {
                    context: p.clone(),
                    request_digest: configured.request_digest,
                    receipt_digest: configured.digest().unwrap(),
                },
            )
        })
        .collect();
    let initial = wire::Observation {
        schema: name(wire::OBSERVATION_SCHEMA),
        snapshot: legacy.snapshot.clone(),
        configured: configured_stamps,
        accepted: vec![],
        policies: BTreeMap::new(),
        receipt: None,
        receipt_matches_current_host: false,
        activation_authorized: false,
    };
    let task = f
        .app
        .bind_qualification_request(&f.hosts[0], &task.id, initial.clone(), f.clock.now())
        .unwrap();
    let a::Emission::Send { request } = f
        .app
        .enter_qualification_send(&f.hosts[0], &task.id, false)
        .unwrap()
    else {
        panic!("qualification send")
    };
    let a::Request::V2(request) = *request else {
        panic!("no v1 qualification fallback")
    };
    request.matches_configuration(configured).unwrap();
    assert!(
        matches!(f.app.enter_qualification_send(&f.hosts[0], &task.id, false).unwrap(), a::Emission::Lookup { execution_v2: true, request } if request == task.id)
    );
    assert!(
        f.app
            .prepare_qualification_activation(release, &id(), activation::finalize(&batch))
            .is_err(),
        "missing v2 acceptance cannot activate"
    );
    let legacy = activation::accepted(legacy, &task);
    assert!(
        f.app
            .record_qualification_observation(&f.hosts[0], &task.id, legacy.clone(), f.clock.now())
            .is_err()
    );
    let mut context = legacy.receipt.unwrap();
    context.recorded_at = f.clock.now();
    context.quiescence.as_mut().unwrap().observed_at = f.clock.now();
    let policies = request
        .policies
        .iter()
        .map(|(cell, binding)| {
            let t = request
                .context
                .cells
                .iter()
                .find(|t| &t.cell == cell)
                .unwrap();
            (
                cell.clone(),
                wire::AcceptedPolicy {
                    binding: binding.clone(),
                    qualification: t.qualification.clone(),
                    qualification_revision: t.qualification_revision,
                    request: task.id.clone(),
                    acceptance_sequence: context.sequence,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let observation = wire::Observation {
        receipt: Some(wire::Receipt {
            schema: name(wire::RECEIPT_SCHEMA),
            request_digest: request.digest().unwrap(),
            request: *request,
            context,
            policies: policies.clone(),
        }),
        accepted: legacy.accepted,
        policies,
        receipt_matches_current_host: true,
        ..initial
    };
    f.app
        .record_qualification_observation(&f.hosts[0], &task.id, observation.clone(), f.clock.now())
        .unwrap();
    let key = id();
    let input = activation::finalize(&batch);
    let a::Preflight::Verify(ticket) = f
        .app
        .prepare_qualification_activation(release, &key, input.clone())
        .unwrap()
    else {
        panic!("activation ticket")
    };
    let prepared = a::Prepared::verify(
        *ticket,
        &proof.policy,
        source
            .store
            .verify_owned(&source.object, &source.policy)
            .unwrap(),
    )
    .unwrap();
    f.clock.0.fetch_add(
        if expiry_control {
            q::DERIVED_TICKET_TTL_NS
        } else {
            599_000_000_000
        },
        Ordering::SeqCst,
    );
    // A computation ticket never extends the separate three-second Host-read lease.
    f.app
        .record_qualification_observation(&f.hosts[0], &task.id, observation.clone(), f.clock.now())
        .unwrap();
    let prepared = if expiry_control {
        assert!(matches!(
            f.app.commit_qualification_activation(prepared),
            Err(StoreError::Rejected(Rejection::StaleRevision))
        ));
        assert!(
            f.app
                .inspect_cell(&f.admin, &target.id)
                .unwrap()
                .1
                .qualification
                .is_none()
        );
        let a::Preflight::Verify(ticket) = f
            .app
            .prepare_qualification_activation(release, &key, input.clone())
            .unwrap()
        else {
            panic!("fresh activation ticket")
        };
        a::Prepared::verify(
            *ticket,
            &proof.policy,
            source
                .store
                .verify_owned(&source.object, &source.policy)
                .unwrap(),
        )
        .unwrap()
    } else {
        prepared
    };
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app.commit_qualification_activation(prepared).is_err(),
        "lost activation response"
    );
    let a::Preflight::Recorded(active) = f
        .app
        .prepare_qualification_activation(release, &key, input)
        .unwrap()
    else {
        panic!("original activation recovery")
    };
    assert_eq!(active.state, a::State::Active);
    let (revision, cell) = f.app.inspect_cell(&f.admin, &target.id).unwrap();
    assert!(
        matches!(
            f.app.create_run(
                &f.operator,
                id().as_str(),
                CreateRun {
                    cell: target.id.clone(),
                    recipe_digest: target.recipe.sha256,
                    site_config_digest: target.site_config_digest,
                    expected_cell: revision,
                }
            ),
            Err(StoreError::Rejected(Rejection::UnsupportedSchema))
        ),
        "qualified v2 domain cannot become a legacy Run"
    );
    assert!(cell.qualification.is_some());
    assert_eq!(cell.commissioning, Some(Commissioning::Commissioned));
    let view = f
        .app
        .qualification_batch(&f.admin, &target.id, &batch.id)
        .unwrap();
    assert!(view.current && !view.operation_authorized);
    if test_case == 16 {
        return;
    }
    if test_case >= 8 {
        start_part_tests::exercise(f, target, published, inputs, saved.policy(), test_case);
        return;
    }
    if test_case >= 6 {
        inventory_tests::exercise(f, target, published, inputs, saved.policy(), test_case == 7);
        return;
    }
    if test_case == 5 {
        let definition = &inputs.definitions[0];
        let mut update = save(&definition.reference.catalog, definition.body.clone());
        update.id = definition.reference.id.clone();
        update.expected = Some(definition.reference.revision);
        update.label = "Same values but revised after qualification activation".into();
        put(&mut f.app, &f.admin, update);
        assert!(
            !f.app
                .qualification_batch(&f.admin, &target.id, &batch.id)
                .unwrap()
                .current
        );
        assert!(
            matches!(f.app.enter_qualification_send(&f.hosts[0], &task.id, false).unwrap(), a::Emission::Lookup { execution_v2: true, request } if request == task.id)
        );
        return;
    }
    // The original acceptance remains a fact; loss of matching current policy suspends new rights.
    let mut stale = observation;
    stale.configured.get_mut(&target.id).unwrap().receipt_digest = Digest::from_bytes([99; 32]);
    stale.receipt_matches_current_host = false;
    let recorded = f
        .app
        .record_qualification_observation(&f.hosts[0], &task.id, stale, f.clock.now())
        .unwrap();
    assert!(recorded.receipt.is_some());
    assert_eq!(
        f.app
            .qualification_batch(&f.admin, &target.id, &batch.id)
            .unwrap()
            .batch
            .state,
        a::State::Suspended
    );
}

#[path = "execution_inventory_tests.rs"]
mod inventory_tests;

#[path = "execution_start_part_tests.rs"]
mod start_part_tests;
