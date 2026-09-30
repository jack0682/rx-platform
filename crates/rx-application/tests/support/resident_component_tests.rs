use super::*;
use rx_application::resident_component as component;
use rx_domain::component::{CatalogReference, Declaration, RegistrationState};

fn empty() -> (tempfile::TempDir, App, ManualClock, Arc<AtomicU8>, Identity) {
    let directory = tempfile::tempdir().unwrap();
    let failure = Arc::new(AtomicU8::new(0));
    let clock = ManualClock(Arc::new(AtomicU64::new(1000)));
    let mut app = Engine::open(
        FaultRepository {
            inner: SqliteRepository::open(directory.path().join("platform.db")).unwrap(),
            mode: failure.clone(),
        },
        clock.clone(),
        SimulationAuthority,
        id(),
        principal(
            "admin",
            &[Role::AccountAdmin, Role::Engineer, Role::Observer],
        ),
    )
    .unwrap();
    let session = app
        .authenticated_session(&name("admin"), id(), expiry(100000))
        .unwrap();
    let admin = Identity {
        principal: name("admin"),
        session: session.id,
        terminal: None,
    };
    (directory, app, clock, failure, admin)
}

fn input(label: &str) -> component::Create {
    component::Create {
        declaration: Declaration {
            label: name(label),
            catalog: CatalogReference {
                program: name("rx/status-http"),
                digest: Digest::from_bytes([73; 32]),
            },
        },
    }
}

#[test]
fn registration_without_a_cell_survives_reply_loss_and_restart_without_authority() {
    let (_directory, mut app, clock, failure, admin) = empty();
    let request = id();
    failure.store(2, Ordering::SeqCst);
    assert!(matches!(
        app.create_component(&admin, request.as_str(), input("camera")),
        Err(StoreError::Unavailable(_))
    ));
    let created = app
        .create_component(&admin, request.as_str(), input("camera"))
        .unwrap();
    assert_eq!(
        created.execution_ownership,
        component::Ownership::NotEstablishedByRegistration
    );
    assert_eq!(
        created.work_use_permission,
        component::WorkUse::NotEvaluated
    );
    assert_eq!(
        created.content_verification,
        component::Verification::NotEstablished
    );
    let installation = app.installation.id.clone();
    let mut app = Engine::open(
        app.into_repository(),
        clock,
        SimulationAuthority,
        installation,
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    assert!(matches!(
        app.component(&admin, &created.record.registration.id, None),
        Err(StoreError::Rejected(Rejection::Unauthenticated))
    ));
    let session = app
        .authenticated_session(&name("admin"), id(), expiry(100000))
        .unwrap();
    let admin = Identity {
        session: session.id,
        ..admin
    };
    assert_eq!(
        app.create_component(&admin, request.as_str(), input("camera"))
            .unwrap(),
        created
    );
    assert_eq!(
        app.component(&admin, &created.record.registration.id, None)
            .unwrap(),
        created
    );
    let (_, records) = app.into_repository().snapshot().unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|r| r.key.as_str().starts_with("component/"))
            .count(),
        1
    );
    for prefix in ["cell/", "work/", "permit/", "resource/", "host/"] {
        assert!(
            !records.iter().any(|r| r.key.as_str().starts_with(prefix)),
            "{prefix}"
        );
    }
}

#[test]
fn declaration_changes_are_revision_checked_historical_and_retirement_is_terminal() {
    let (_directory, mut app, _, failure, admin) = empty();
    let first = app
        .create_component(&admin, id().as_str(), input("camera"))
        .unwrap();
    let component_id = first.record.registration.id.clone();
    let change = component::Update {
        id: component_id.clone(),
        expected_revision: first.revision,
        declaration: input("camera-v2").declaration,
    };
    let request = id();
    failure.store(2, Ordering::SeqCst);
    assert!(matches!(
        app.update_component(&admin, request.as_str(), change.clone()),
        Err(StoreError::Unavailable(_))
    ));
    let changed = app
        .update_component(&admin, request.as_str(), change.clone())
        .unwrap();
    assert_ne!(changed.revision, first.revision);
    assert_eq!(changed.record.registration.id, component_id);
    assert!(matches!(
        app.update_component(&admin, id().as_str(), change),
        Err(StoreError::Rejected(Rejection::StaleRevision))
    ));
    assert_eq!(
        app.component(&admin, &component_id, Some(first.revision))
            .unwrap(),
        first
    );
    let retire_key = id();
    let retire = component::Retire {
        id: component_id.clone(),
        expected_revision: changed.revision,
    };
    let retired = app
        .retire_component(&admin, retire_key.as_str(), retire.clone())
        .unwrap();
    assert_eq!(
        retired.record.registration.state,
        RegistrationState::Retired
    );
    assert_eq!(
        app.retire_component(&admin, retire_key.as_str(), retire)
            .unwrap(),
        retired
    );
    assert_eq!(
        app.component(&admin, &component_id, Some(changed.revision))
            .unwrap(),
        changed
    );
    assert!(
        app.update_component(
            &admin,
            id().as_str(),
            component::Update {
                id: component_id,
                expected_revision: retired.revision,
                declaration: input("resurrect").declaration
            }
        )
        .is_err()
    );
}

#[test]
fn owner_role_and_request_identity_are_checked_before_returning_cached_results() {
    let (_directory, mut app, _, _, admin) = empty();
    let author = add_identity(&mut app, &admin, "author", &[Role::Engineer]);
    let other = add_identity(&mut app, &admin, "other", &[Role::Engineer]);
    let reader = add_identity(&mut app, &admin, "reader", &[Role::Observer]);
    let key = id();
    let first = app
        .create_component(&author, key.as_str(), input("camera"))
        .unwrap();
    assert!(matches!(
        app.component(&other, &first.record.registration.id, None),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    assert!(matches!(
        app.update_component(
            &other,
            id().as_str(),
            component::Update {
                id: first.record.registration.id.clone(),
                expected_revision: first.revision,
                declaration: input("stolen").declaration,
            }
        ),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    assert!(matches!(
        app.retire_component(
            &other,
            id().as_str(),
            component::Retire {
                id: first.record.registration.id.clone(),
                expected_revision: first.revision,
            }
        ),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    assert!(matches!(
        app.create_component(&reader, id().as_str(), input("camera")),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
    assert!(matches!(
        app.create_component(&author, key.as_str(), input("different")),
        Err(StoreError::KeyConflict)
    ));
    // Even an administrator's two accounts sharing a client namespace cannot alias requests.
    let mut alias = principal("alias", &[Role::Engineer]);
    alias.client_namespace = name("client/author");
    app.put_principal(&admin, alias, None).unwrap();
    let session = app
        .authenticated_session(&name("alias"), id(), expiry(100000))
        .unwrap();
    let alias = Identity {
        principal: name("alias"),
        session: session.id,
        terminal: None,
    };
    assert!(matches!(
        app.create_component(&alias, key.as_str(), input("camera")),
        Err(StoreError::KeyConflict)
    ));
    assert_eq!(
        app.component(&admin, &first.record.registration.id, None)
            .unwrap(),
        first
    );
    app.put_principal(
        &admin,
        principal("author", &[Role::Observer]),
        Some(Counter(1)),
    )
    .unwrap();
    assert!(matches!(
        app.create_component(&author, key.as_str(), input("camera")),
        Err(StoreError::Rejected(Rejection::Forbidden))
    ));
}

#[test]
fn failed_transaction_creates_neither_registration_history_nor_cached_success() {
    let (_directory, mut app, _, failure, admin) = empty();
    let key = id();
    failure.store(1, Ordering::SeqCst);
    assert!(matches!(
        app.create_component(&admin, key.as_str(), input("camera")),
        Err(StoreError::Unavailable(_))
    ));
    let mut repo = app.into_repository();
    let (_, records) = repo.snapshot().unwrap();
    assert!(
        !records
            .iter()
            .any(|r| r.key.as_str().starts_with("component/")
                || r.key.as_str().starts_with("componenthistory/"))
    );
    // Reopening must not treat the failed intent as an accepted registration.
    let installation: Installation = repo
        .transact(|tx| {
            let row = tx.get(&name("installation/current"))?.unwrap();
            rx_application::persistence::decode(&row, "rx.internal.installation.v1")
        })
        .unwrap();
    let mut app = Engine::open(
        repo,
        ManualClock(Arc::new(AtomicU64::new(2000))),
        SimulationAuthority,
        installation.id,
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    let session = app
        .authenticated_session(&name("admin"), id(), expiry(100000))
        .unwrap();
    let admin = Identity {
        session: session.id,
        ..admin
    };
    let created = app
        .create_component(&admin, key.as_str(), input("camera"))
        .unwrap();
    assert_eq!(
        created.record.registration.state,
        RegistrationState::Accepted
    );
}
