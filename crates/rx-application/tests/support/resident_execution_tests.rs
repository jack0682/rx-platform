use super::*;
use rx_application::{
    resident_component as component, resident_execution::Identity as SupervisorIdentity,
};
use rx_domain::{
    component::{CatalogReference, Declaration},
    resident_execution as execution,
};
fn setup(
    f: &mut Fixture,
) -> (
    component::View,
    execution::Peer,
    SupervisorIdentity,
    execution::View,
) {
    f.app
        .put_principal(&f.admin, principal("supervisor", &[Role::Supervisor]), None)
        .unwrap();
    f.app
        .configure_resident_supervisors(
            [(
                name("supervisor"),
                execution::Enrollment {
                    registry: Digest::from_bytes([44; 32]),
                    releases: [Digest::from_bytes([7; 32])].into(),
                    programs: [(
                        name("status"),
                        execution::CatalogPolicy {
                            digest: Digest::from_bytes([9; 32]),
                            effect: execution::Effect::NonActuating,
                        },
                    )]
                    .into(),
                },
            )]
            .into(),
        )
        .unwrap();
    let peer = f
        .app
        .open_resident_supervisor(
            &name("supervisor"),
            id(),
            Digest::from_bytes([31; 32]),
            Digest::from_bytes([44; 32]),
        )
        .unwrap();
    let actor = SupervisorIdentity {
        principal: peer.principal.clone(),
        session: peer.id.clone(),
        authentication_binding: peer.authentication_binding,
    };
    let c = f
        .app
        .create_component(
            &f.admin,
            id().as_str(),
            component::Create {
                declaration: Declaration {
                    label: name("source"),
                    catalog: CatalogReference {
                        program: name("status"),
                        digest: Digest::from_bytes([9; 32]),
                    },
                },
            },
        )
        .unwrap();
    let v = f
        .app
        .propose_resident_execution(&f.admin, id().as_str(), proposal(&c))
        .unwrap();
    (c, peer, actor, v)
}
fn proposal(c: &component::View) -> execution::Propose {
    execution::Propose {
        supervisor: name("supervisor"),
        environment: execution::Environment::Simulation,
        profiles: vec![],
        selections: [(
            name("main"),
            execution::Selection {
                component: c.record.registration.id.clone(),
                expected_revision: c.revision,
                parameters: BTreeMap::new(),
                depends_on: vec![],
                startup_timeout_ms: Counter(1000),
                shutdown_timeout_ms: Counter(1000),
            },
        )]
        .into(),
    }
}
fn preparation(v: &execution::View, peer: &execution::Peer) -> execution::Preparation {
    execution::Preparation {
        legacy_source: None,
        assignment: v.assignment.intent.id.clone(),
        intent_digest: v.assignment.intent.digest().unwrap(),
        peer: peer.clone(),
        release: Digest::from_bytes([7; 32]),
        plan_digest: Digest::from_bytes([11; 32]),
        programs: v
            .assignment
            .intent
            .nodes
            .iter()
            .map(|(name, n)| {
                (
                    name.clone(),
                    execution::ProgramVerification {
                        catalog: n.registration.declaration.catalog.clone(),
                        effect: execution::Effect::NonActuating,
                    },
                )
            })
            .collect(),
    }
}
fn approve(v: &execution::View, content: &execution::ContentReceipt) -> execution::Approve {
    execution::Approve {
        assignment: v.assignment.intent.id.clone(),
        expected_revision: v.revision,
        preparation_digest: content.preparation.digest().unwrap(),
        start_window_ms: Counter(1000),
    }
}
fn observation(
    v: &execution::View,
    sequence: u64,
    state: execution::ObservationState,
) -> execution::Observation {
    execution::Observation {
        assignment: v.assignment.intent.id.clone(),
        grant: v.assignment.grant.as_ref().unwrap().id.clone(),
        sequence: Counter(sequence),
        reconciliation_required: false,
        nodes: v
            .assignment
            .intent
            .nodes
            .iter()
            .map(|(s, n)| {
                (
                    s.clone(),
                    execution::NodeObservation {
                        instance: n.instance.clone(),
                        state,
                        pid: if state == execution::ObservationState::NotStarted {
                            None
                        } else {
                            Some(1234)
                        },
                        exit_code: if state == execution::ObservationState::Exited {
                            Some(0)
                        } else {
                            None
                        },
                        detail: "test source fact; no actual process".into(),
                    },
                )
            })
            .collect(),
    }
}
#[test]
fn preparation_owner_grant_outcome_and_claims_share_the_authoritative_writer() {
    let mut f = fixture(1, false);
    let (component, peer, actor, v) = setup(&mut f);
    assert_eq!(v.assignment.phase, execution::Phase::Proposed);
    assert!(!v.claims_held);
    assert!(
        f.app
            .authenticated_user_session(&peer.principal, id(), Counter(1000))
            .is_err()
    );
    assert!(
        f.app
            .open_resident_supervisor(
                &f.operator.principal,
                id(),
                Digest::from_bytes([1; 32]),
                Digest::from_bytes([44; 32])
            )
            .is_err()
    );
    let mut forged = preparation(&v, &peer);
    forged.release = Digest::from_bytes([0; 32]);
    assert!(
        f.app
            .prepare_resident_execution(&actor, id().as_str(), forged)
            .is_err()
    );
    let key = id();
    let input = preparation(&v, &peer);
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .prepare_resident_execution(&actor, key.as_str(), input.clone())
            .is_err()
    );
    let content = f
        .app
        .prepare_resident_execution(&actor, key.as_str(), input)
        .unwrap();
    let prepared = f
        .app
        .resident_execution(&f.admin, &v.assignment.intent.id)
        .unwrap();
    let request = approve(&prepared, &content);
    let key = id();
    f.failure.store(1, Ordering::SeqCst);
    assert!(
        f.app
            .approve_resident_execution(&f.admin, key.as_str(), request.clone())
            .is_err()
    );
    assert!(
        !f.app
            .resident_execution(&f.admin, &v.assignment.intent.id)
            .unwrap()
            .claims_held
    );
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .approve_resident_execution(&f.admin, key.as_str(), request.clone())
            .is_err()
    );
    let started = f
        .app
        .approve_resident_execution(&f.admin, key.as_str(), request)
        .unwrap();
    assert!(started.claims_held);
    assert_eq!(started.assignment.phase, execution::Phase::Granted);
    assert!(
        f.app
            .update_component(
                &f.admin,
                id().as_str(),
                component::Update {
                    id: component.record.registration.id.clone(),
                    expected_revision: component.revision,
                    declaration: component.record.registration.declaration.clone()
                }
            )
            .is_err()
    );
    let running = observation(&started, 1, execution::ObservationState::Running);
    let key = id();
    let first = f
        .app
        .observe_resident_execution(&actor, key.as_str(), running.clone())
        .unwrap();
    assert_eq!(
        f.app
            .observe_resident_execution(&actor, key.as_str(), running)
            .unwrap(),
        first
    );
    let mut fake = observation(&started, 2, execution::ObservationState::NotStarted);
    assert!(
        f.app
            .observe_resident_execution(&actor, id().as_str(), fake.clone())
            .is_err()
    );
    fake = observation(&started, 2, execution::ObservationState::Exited);
    let key = id();
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .observe_resident_execution(&actor, key.as_str(), fake.clone())
            .is_err()
    );
    f.app
        .observe_resident_execution(&actor, key.as_str(), fake)
        .unwrap();
    let ended = f
        .app
        .resident_execution(&f.admin, &v.assignment.intent.id)
        .unwrap();
    assert_eq!(ended.assignment.phase, execution::Phase::Exited);
    assert!(!ended.claims_held);
    f.app
        .update_component(
            &f.admin,
            id().as_str(),
            component::Update {
                id: component.record.registration.id,
                expected_revision: component.revision,
                declaration: component.record.registration.declaration,
            },
        )
        .unwrap();
}
#[test]
fn unknown_outcome_and_stale_supervisor_never_release_or_reissue_a_claim() {
    let mut f = fixture(1, false);
    let (c, peer, actor, v) = setup(&mut f);
    let content = f
        .app
        .prepare_resident_execution(&actor, id().as_str(), preparation(&v, &peer))
        .unwrap();
    let prepared = f
        .app
        .resident_execution(&f.admin, &v.assignment.intent.id)
        .unwrap();
    let granted = f
        .app
        .approve_resident_execution(&f.admin, id().as_str(), approve(&prepared, &content))
        .unwrap();
    let mut unknown = observation(&granted, 1, execution::ObservationState::Unknown);
    unknown.reconciliation_required = true;
    f.app
        .observe_resident_execution(&actor, id().as_str(), unknown)
        .unwrap();
    assert!(
        f.app
            .observe_resident_execution(
                &actor,
                id().as_str(),
                observation(&granted, 2, execution::ObservationState::Exited)
            )
            .is_err()
    );
    let old = f
        .app
        .resident_execution(&f.admin, &v.assignment.intent.id)
        .unwrap();
    assert!(old.claims_held);
    let second = f
        .app
        .propose_resident_execution(&f.admin, id().as_str(), proposal(&c))
        .unwrap();
    let second_content = f
        .app
        .prepare_resident_execution(&actor, id().as_str(), preparation(&second, &peer))
        .unwrap();
    let second = f
        .app
        .resident_execution(&f.admin, &second.assignment.intent.id)
        .unwrap();
    assert!(
        f.app
            .approve_resident_execution(&f.admin, id().as_str(), approve(&second, &second_content))
            .is_err()
    );
    let replaced = f
        .app
        .open_resident_supervisor(
            &peer.principal,
            id(),
            peer.authentication_binding,
            peer.registry,
        )
        .unwrap();
    assert_ne!(peer.id, replaced.id);
    assert!(
        f.app
            .inspect_resident_execution(&actor, &v.assignment.intent.id)
            .is_err()
    );
    let view = f
        .app
        .resident_execution(&f.admin, &v.assignment.intent.id)
        .unwrap();
    assert!(!view.peer_current);
    assert!(view.claims_held);
}
#[test]
fn cancelled_preparation_and_changed_declaration_cannot_produce_a_start_grant() {
    let mut f = fixture(1, false);
    let (c, peer, actor, v) = setup(&mut f);
    let content = f
        .app
        .prepare_resident_execution(&actor, id().as_str(), preparation(&v, &peer))
        .unwrap();
    let prepared = f
        .app
        .resident_execution(&f.admin, &v.assignment.intent.id)
        .unwrap();
    f.app
        .update_component(
            &f.admin,
            id().as_str(),
            component::Update {
                id: c.record.registration.id.clone(),
                expected_revision: c.revision,
                declaration: Declaration {
                    label: name("changed"),
                    ..c.record.registration.declaration.clone()
                },
            },
        )
        .unwrap();
    assert!(
        f.app
            .approve_resident_execution(&f.admin, id().as_str(), approve(&prepared, &content))
            .is_err()
    );
    let cancelled = f
        .app
        .stop_resident_execution(
            &f.admin,
            id().as_str(),
            execution::Stop {
                assignment: v.assignment.intent.id,
                expected_revision: prepared.revision,
            },
        )
        .unwrap();
    assert_eq!(cancelled.assignment.phase, execution::Phase::Cancelled);
    assert!(!cancelled.claims_held);
    assert!(
        f.app
            .approve_resident_execution(&f.admin, id().as_str(), approve(&cancelled, &content))
            .is_err()
    );
}

#[test]
fn imported_identity_requires_the_enrolled_original_registry_and_exact_frozen_cut() {
    let mut f = fixture(1, false);
    let (_dir, source, freeze, ids, _) =
        super::component_intake_tests::source(&f.app.installation.id, 1);
    let input = super::component_intake_tests::configure(&mut f, &freeze);
    let mut reader = super::component_intake_tests::reader(&mut f, &id(), input, source);
    super::component_intake_tests::stage(&mut f, &mut reader);
    f.app.finish_component_intake(reader.finish()).unwrap();
    f.app
        .put_principal(&f.admin, principal("supervisor", &[Role::Supervisor]), None)
        .unwrap();
    let mut policy = execution::Enrollment {
        registry: Digest::from_bytes([44; 32]),
        releases: [Digest::from_bytes([7; 32])].into(),
        programs: [(
            name("status"),
            execution::CatalogPolicy {
                digest: Digest::from_bytes([9; 32]),
                effect: execution::Effect::NonActuating,
            },
        )]
        .into(),
    };
    f.app
        .configure_resident_supervisors([(name("supervisor"), policy.clone())].into())
        .unwrap();
    let component = f.app.component(&f.admin, &ids[0], None).unwrap();
    assert!(
        f.app
            .propose_resident_execution(&f.admin, id().as_str(), proposal(&component))
            .is_err()
    );
    policy.registry = Digest::from_bytes([77; 32]);
    f.app
        .configure_resident_supervisors([(name("supervisor"), policy.clone())].into())
        .unwrap();
    let peer = f
        .app
        .open_resident_supervisor(
            &name("supervisor"),
            id(),
            Digest::from_bytes([31; 32]),
            policy.registry,
        )
        .unwrap();
    let actor = SupervisorIdentity {
        principal: peer.principal.clone(),
        session: peer.id.clone(),
        authentication_binding: peer.authentication_binding,
    };
    let v = f
        .app
        .propose_resident_execution(&f.admin, id().as_str(), proposal(&component))
        .unwrap();
    assert_eq!(
        v.assignment.intent.nodes[&name("main")]
            .origin
            .as_ref()
            .unwrap()
            .freeze,
        freeze
    );
    let mut offer = preparation(&v, &peer);
    assert!(
        f.app
            .prepare_resident_execution(&actor, id().as_str(), offer.clone())
            .is_err()
    );
    offer.legacy_source = Some(freeze.clone());
    f.app
        .prepare_resident_execution(&actor, id().as_str(), offer)
        .unwrap();
}

#[test]
fn platform_restart_keeps_issued_claims_and_refuses_old_live_context() {
    let mut f = fixture(1, false);
    let (_component, peer, actor, v) = setup(&mut f);
    let content = f
        .app
        .prepare_resident_execution(&actor, id().as_str(), preparation(&v, &peer))
        .unwrap();
    let prepared = f
        .app
        .resident_execution(&f.admin, &v.assignment.intent.id)
        .unwrap();
    f.app
        .approve_resident_execution(&f.admin, id().as_str(), approve(&prepared, &content))
        .unwrap();
    let installation = f.app.installation.id.clone();
    let mut app = Engine::open(
        f.app.into_repository(),
        f.clock,
        SimulationAuthority,
        installation,
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    assert!(
        app.inspect_resident_execution(&actor, &v.assignment.intent.id)
            .is_err()
    );
    let session = app
        .authenticated_session(
            &name("admin"),
            id(),
            TimePoint {
                clock_id: "test/boottime".into(),
                ticks_ns: Counter(u64::MAX),
            },
        )
        .unwrap();
    let admin = Identity {
        principal: name("admin"),
        session: session.id,
        terminal: None,
    };
    let stored = app
        .resident_execution(&admin, &v.assignment.intent.id)
        .unwrap();
    assert_eq!(stored.assignment.phase, execution::Phase::Granted);
    assert!(stored.claims_held);
    assert!(!stored.peer_current);
    assert!(
        app.approve_resident_execution(&admin, id().as_str(), approve(&stored, &content))
            .is_err()
    );
}

#[test]
fn changing_an_author_into_a_supervisor_invalidates_old_ordinary_session_use() {
    let mut f = fixture(1, false);
    let (component, _peer, _actor, _v) = setup(&mut f);
    f.app
        .put_principal(
            &f.admin,
            principal("former-author", &[Role::Engineer]),
            None,
        )
        .unwrap();
    let session = f
        .app
        .authenticated_user_session(&name("former-author"), id(), Counter(1000))
        .unwrap();
    let identity = Identity {
        principal: name("former-author"),
        session: session.id,
        terminal: None,
    };
    f.app
        .put_principal(
            &f.admin,
            principal("former-author", &[Role::Supervisor, Role::Engineer]),
            Some(Counter(1)),
        )
        .unwrap();
    assert!(
        f.app
            .create_component(
                &identity,
                id().as_str(),
                component::Create {
                    declaration: component.record.registration.declaration
                }
            )
            .is_err()
    );
    assert!(
        f.app
            .authenticated_user_session(&name("former-author"), id(), Counter(1000))
            .is_err()
    );
}
