use super::*;
use rx_application::{resident_component as component, resident_reporting as reporting};
use rx_domain::{
    component::{Binding, CatalogReference, Declaration},
    resident_reporting::*,
};

fn setup(
    f: &mut Fixture,
) -> (
    component::View,
    Peer,
    reporting::ScopeView,
    reporting::ReporterIdentity,
) {
    f.app
        .put_principal(&f.admin, principal("reporter", &[Role::Observer]), None)
        .unwrap();
    let peer = f
        .app
        .open_resident_reporter(&name("reporter"), id(), Digest::from_bytes([19; 32]))
        .unwrap();
    let component = f
        .app
        .create_component(
            &f.admin,
            id().as_str(),
            component::Create {
                declaration: Declaration {
                    label: name("observation"),
                    catalog: CatalogReference {
                        program: name("rx/status-http"),
                        digest: Digest::from_bytes([23; 32]),
                    },
                },
            },
        )
        .unwrap();
    let scope = f
        .app
        .issue_resident_reporting(
            &f.admin,
            id().as_str(),
            reporting::Issue {
                component: component.record.registration.id.clone(),
                expected_component_revision: component.revision,
                reporter_session: peer.id.clone(),
                source_registration: id(),
                source_revision: Counter(7),
            },
        )
        .unwrap();
    let identity = reporting::ReporterIdentity {
        principal: peer.principal.clone(),
        session: peer.id.clone(),
        authentication_binding: peer.authentication_binding,
    };
    (component, peer, scope, identity)
}
fn report(scope: &Scope) -> Report {
    Report {
        scope: scope.id.clone(),
        source: Binding {
            registration: scope.source_registration.clone(),
            registration_revision: scope.source_revision,
            catalog: scope.catalog.clone(),
            run: id(),
            selection: name("status"),
            instance: id(),
        },
        sequence: Counter(1),
        state: ExecutionState::Running,
        pid: Some(4242),
        exit_code: None,
        detail: "recorded registry observation".into(),
    }
}

#[test]
fn reporter_replay_is_attributed_sequenced_and_not_work_authority() {
    let mut f = fixture(1, false);
    let (component, peer, scope, identity) = setup(&mut f);
    assert_eq!(
        f.app
            .open_resident_reporter(
                &peer.principal,
                peer.peer_boot.clone(),
                peer.authentication_binding
            )
            .unwrap(),
        peer
    );
    let first = report(&scope.scope);
    let key = id();
    f.failure.store(2, Ordering::SeqCst);
    assert!(matches!(
        f.app
            .publish_resident_report(&identity, key.as_str(), first.clone()),
        Err(StoreError::Unavailable(_))
    ));
    let receipt = f
        .app
        .publish_resident_report(&identity, key.as_str(), first.clone())
        .unwrap();
    assert_eq!(receipt.basis, Basis::ReportedRegistrySnapshot);
    assert_eq!(
        receipt.execution_ownership,
        Ownership::NotEstablishedByReport
    );
    assert_eq!(receipt.work_use_permission, WorkUse::NotEvaluated);
    assert_eq!(
        f.app
            .publish_resident_report(&identity, key.as_str(), first.clone())
            .unwrap(),
        receipt
    );
    let mut second = first.clone();
    second.sequence = Counter(2);
    second.state = ExecutionState::Exited;
    second.exit_code = Some(0);
    assert!(matches!(
        f.app
            .publish_resident_report(&identity, key.as_str(), second.clone()),
        Err(StoreError::KeyConflict)
    ));
    let terminal = f
        .app
        .publish_resident_report(&identity, id().as_str(), second.clone())
        .unwrap();
    let mut resurrection = second.clone();
    resurrection.sequence = Counter(3);
    resurrection.state = ExecutionState::Running;
    assert!(
        f.app
            .publish_resident_report(&identity, id().as_str(), resurrection)
            .is_err()
    );
    assert!(
        f.app
            .publish_resident_report(&identity, id().as_str(), first.clone())
            .is_err()
    );
    let view = f
        .app
        .resident_report(
            &f.admin,
            &component.record.registration.id,
            &first.source.instance,
        )
        .unwrap();
    assert_eq!(view.receipt, terminal);
    assert!(view.reporter_session_current && view.registration_revision_current);
    // A reporter session is not a user/Host/Executor session.
    let forged = Identity {
        principal: peer.principal,
        session: peer.id,
        terminal: None,
    };
    assert!(matches!(
        f.app
            .component(&forged, &component.record.registration.id, None),
        Err(StoreError::Rejected(Rejection::Unauthenticated))
    ));
}

#[test]
fn reporting_scope_rejects_wrong_role_certificate_source_and_revocation() {
    let mut f = fixture(1, false);
    let (_, _, scope, identity) = setup(&mut f);
    assert!(
        f.app
            .open_resident_reporter(&f.admin.principal, id(), Digest::from_bytes([1; 32]))
            .is_err()
    );
    let mut broad = principal("broad", &[Role::Observer, Role::Engineer]);
    broad.cells.clear();
    f.app.put_principal(&f.admin, broad, None).unwrap();
    assert!(
        f.app
            .open_resident_reporter(&name("broad"), id(), Digest::from_bytes([1; 32]))
            .is_err()
    );
    let wrong = reporting::ReporterIdentity {
        authentication_binding: Digest::from_bytes([99; 32]),
        ..identity.clone()
    };
    assert!(
        f.app
            .resident_reporting_scope(&wrong, &scope.scope.id)
            .is_err()
    );
    let mut forged = report(&scope.scope);
    forged.source.registration = id();
    assert!(
        f.app
            .publish_resident_report(&identity, id().as_str(), forged)
            .is_err()
    );
    let original = report(&scope.scope);
    let key = id();
    let receipt = f
        .app
        .publish_resident_report(&identity, key.as_str(), original.clone())
        .unwrap();
    f.app
        .revoke_resident_reporting(
            &f.admin,
            id().as_str(),
            reporting::Revoke {
                scope: scope.scope.id.clone(),
                expected_revision: scope.revision,
            },
        )
        .unwrap();
    assert!(
        f.app
            .publish_resident_report(&identity, key.as_str(), original.clone())
            .is_err()
    );
    let view = f
        .app
        .resident_report(&f.admin, &scope.scope.component, &original.source.instance)
        .unwrap();
    assert_eq!(view.receipt, receipt);
    assert!(!view.reporter_session_current);
}

#[test]
fn source_restart_and_p_restart_do_not_promote_historical_observations() {
    let mut f = fixture(1, false);
    let (component, peer, scope, identity) = setup(&mut f);
    let observation = report(&scope.scope);
    f.app
        .publish_resident_report(&identity, id().as_str(), observation.clone())
        .unwrap();
    let replacement = f
        .app
        .open_resident_reporter(&peer.principal, id(), peer.authentication_binding)
        .unwrap();
    assert_ne!(replacement.id, peer.id);
    assert!(
        f.app
            .resident_reporting_scope(&identity, &scope.scope.id)
            .is_err()
    );
    assert!(
        !f.app
            .resident_report(
                &f.admin,
                &component.record.registration.id,
                &observation.source.instance
            )
            .unwrap()
            .reporter_session_current
    );
    let replacement_scope = f
        .app
        .issue_resident_reporting(
            &f.admin,
            id().as_str(),
            reporting::Issue {
                component: component.record.registration.id.clone(),
                expected_component_revision: component.revision,
                reporter_session: replacement.id.clone(),
                source_registration: scope.scope.source_registration.clone(),
                source_revision: scope.scope.source_revision,
            },
        )
        .unwrap();
    let replacement_identity = reporting::ReporterIdentity {
        principal: replacement.principal,
        session: replacement.id,
        authentication_binding: replacement.authentication_binding,
    };
    // Positive control immediately before restart: this exact scope/session works.
    assert_eq!(
        f.app
            .resident_reporting_scope(&replacement_identity, &replacement_scope.scope.id)
            .unwrap(),
        replacement_scope.scope
    );
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
        app.resident_reporting_scope(&replacement_identity, &replacement_scope.scope.id)
            .is_err()
    );
}

#[test]
fn report_storage_failure_rolls_back_and_current_role_is_checked_before_cached_reply() {
    let mut f = fixture(1, false);
    let (_, _, scope, identity) = setup(&mut f);
    let observation = report(&scope.scope);
    let key = id();
    f.failure.store(1, Ordering::SeqCst);
    assert!(matches!(
        f.app
            .publish_resident_report(&identity, key.as_str(), observation.clone()),
        Err(StoreError::Unavailable(_))
    ));
    assert!(matches!(
        f.app.resident_report(
            &f.admin,
            &scope.scope.component,
            &observation.source.instance
        ),
        Err(StoreError::Rejected(Rejection::NotFound))
    ));
    let receipt = f
        .app
        .publish_resident_report(&identity, key.as_str(), observation.clone())
        .unwrap();
    assert_eq!(receipt.report.sequence, Counter(1));
    let mut revoked = principal("reporter", &[Role::Observer]);
    revoked.active = false;
    f.app
        .put_principal(&f.admin, revoked, Some(Counter(1)))
        .unwrap();
    assert!(matches!(
        f.app
            .publish_resident_report(&identity, key.as_str(), observation.clone()),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    let history = f
        .app
        .resident_report(
            &f.admin,
            &scope.scope.component,
            &observation.source.instance,
        )
        .unwrap();
    assert!(!history.reporter_session_current);
    assert_eq!(history.receipt, receipt);
}

#[test]
fn declaration_retirement_preserves_diagnostic_reports_but_never_marks_the_revision_current() {
    let mut f = fixture(1, false);
    let (component, _, scope, identity) = setup(&mut f);
    f.app
        .retire_component(
            &f.admin,
            id().as_str(),
            component::Retire {
                id: component.record.registration.id.clone(),
                expected_revision: component.revision,
            },
        )
        .unwrap();
    let observation = report(&scope.scope);
    f.app
        .publish_resident_report(&identity, id().as_str(), observation.clone())
        .unwrap();
    let view = f
        .app
        .resident_report(
            &f.admin,
            &component.record.registration.id,
            &observation.source.instance,
        )
        .unwrap();
    assert!(!view.registration_revision_current);
    assert!(view.reporter_session_current);
    assert_eq!(view.receipt.work_use_permission, WorkUse::NotEvaluated);
}
