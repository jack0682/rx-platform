use super::*;
use rx_application::component_intake::{Preflight, Reader, SourceBinding, Submit};
use rx_domain::{
    component::{CatalogReference, Declaration, Registration, RegistrationState},
    component_transfer::*,
};
use rx_ports::Document;
fn doc(schema: &str, value: &impl serde::Serialize) -> Document {
    Document {
        schema: name(schema),
        value: serde_json::to_value(value).unwrap(),
    }
}
pub(super) fn source(
    target: &Id,
    count: usize,
) -> (
    tempfile::TempDir,
    SqliteRepository,
    FreezeRecord,
    Vec<Id>,
    Vec<StoredEvent>,
) {
    source_with(target, count, None, false)
}
fn source_with(
    target: &Id,
    count: usize,
    first_id: Option<Id>,
    corrupt_history: bool,
) -> (
    tempfile::TempDir,
    SqliteRepository,
    FreezeRecord,
    Vec<Id>,
    Vec<StoredEvent>,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SqliteRepository::open(dir.path().join("source.db")).unwrap();
    let mut ids = Vec::new();
    db.transact(|tx|{
        for i in 0..count {
            let registration=Registration{id:if i==0 {first_id.clone().unwrap_or_else(id)} else {id()},declaration:Declaration{label:name(&format!("source-{i}")),catalog:CatalogReference{program:name("status"),digest:Digest::from_bytes([9;32])}},state:RegistrationState::Accepted};
            let key=name(&format!("{DECLARATIONS}{}",registration.id));
            let first=tx.put(&key,None,&doc(REGISTRATION_SCHEMA,&registration))?;
            tx.append_control(&id(),&first,&doc(HISTORY_SCHEMA,&serde_json::json!({"registration":registration.id,"action":"registered","entity":first})))?;
            let mut changed=registration.clone();changed.declaration.label=name(&format!("revised-{i}"));
            let second=tx.put(&key,Some(Counter(1)),&doc(REGISTRATION_SCHEMA,&changed))?;
            let mut history_entity=second.clone();
            if corrupt_history {history_entity.document.value["declaration"]["label"]=serde_json::json!("forged-history");}
            tx.append_control(&id(),&second,&doc(HISTORY_SCHEMA,&serde_json::json!({"registration":registration.id,"action":"declaration-changed","entity":history_entity})))?;
            ids.push(registration.id);
        }
        let record=tx.put(&name("legacy/unknown"),None,&doc("rx.internal.run.v1",&serde_json::json!({"outcome":"UNKNOWN"})))?;
        tx.append_control(&id(),&record,&record.document)?;Ok(())
    }).unwrap();
    let original = db.control_events_after(Counter(0), 128).unwrap();
    let freeze = db
        .seal_prefixes(&PREFIXES.map(name), |tx| {
            let rows = tx.scan(DECLARATIONS)?;
            let index = tx.get(&name(SELECTIONS))?;
            let freeze = FreezeRecord {
                request: FreezeRequest {
                    id: id(),
                    target_installation: target.clone(),
                },
                declarations_digest: rx_domain::canonical::digest(
                    "RX-REGISTRATION-SOURCE-CUT-v1",
                    &(&rows, &index),
                )
                .unwrap(),
                declaration_count: Counter(count as u64),
                history_head: tx.control_head()?,
            };
            let row = tx.put(&name(FREEZE), None, &doc(FREEZE_SCHEMA, &freeze))?;
            tx.append_control(&id(), &row, &row.document)?;
            Ok(freeze)
        })
        .unwrap();
    (dir, db, freeze, ids, original)
}
pub(super) fn configure(f: &mut Fixture, freeze: &FreezeRecord) -> Submit {
    let binding = SourceBinding {
        owner: f.admin.principal.clone(),
        location: Digest::from_bytes([77; 32]),
    };
    f.app
        .configure_component_sources([(name("source"), binding.clone())].into())
        .unwrap();
    Submit {
        source: name("source"),
        freeze: freeze.request.id.clone(),
        expected_binding: binding.fingerprint(&name("source")).unwrap(),
    }
}
pub(super) fn reader(
    f: &mut Fixture,
    key: &Id,
    input: Submit,
    db: SqliteRepository,
) -> Reader<SqliteRepository> {
    let Preflight::Read(t) = f
        .app
        .prepare_component_intake(&f.admin, key.clone(), input)
        .unwrap()
    else {
        panic!("ticket");
    };
    Reader::open(*t, db).unwrap()
}
pub(super) fn stage(f: &mut Fixture, reader: &mut Reader<SqliteRepository>) {
    let mut p = f.app.begin_component_intake(reader.begin()).unwrap();
    while p.declarations < p.expected_declarations {
        p = f
            .app
            .stage_component_declarations(reader.declarations(p.declarations).unwrap())
            .unwrap();
    }
    while p.history_after < p.history_head {
        p = f
            .app
            .stage_component_history(reader.history(p.history_after).unwrap())
            .unwrap();
    }
}
#[test]
fn original_identity_revisions_and_events_are_atomic_visible_only_after_acceptance() {
    let mut f = fixture(1, false);
    let (_dir, db, freeze, ids, original) = source(&f.app.installation.id, 12);
    let input = configure(&mut f, &freeze);
    let key = id();
    let mut reader = reader(&mut f, &key, input.clone(), db);
    f.app.begin_component_intake(reader.begin()).unwrap();
    f.failure.store(2, Ordering::SeqCst);
    assert!(
        f.app
            .stage_component_declarations(reader.declarations(Counter(0)).unwrap())
            .is_err()
    );
    assert_eq!(
        f.app
            .component_intake_progress(&f.admin, &freeze.request.id)
            .unwrap()
            .declarations,
        Counter(8)
    );
    assert!(f.app.component(&f.admin, &ids[0], None).is_err());
    stage(&mut f, &mut reader);
    f.failure.store(1, Ordering::SeqCst);
    let error = f.app.finish_component_intake(reader.finish()).unwrap_err();
    assert!(matches!(error, StoreError::Unavailable(_)), "{error:?}");
    assert!(f.app.component(&f.admin, &ids[0], None).is_err());
    f.failure.store(2, Ordering::SeqCst);
    let error = f.app.finish_component_intake(reader.finish()).unwrap_err();
    assert!(matches!(error, StoreError::Unavailable(_)), "{error:?}");
    let Preflight::Recorded(receipt) = f
        .app
        .prepare_component_intake(&f.admin, key, input)
        .unwrap()
    else {
        panic!("recovered");
    };
    assert_eq!(
        receipt.work_use_permission,
        rx_application::resident_component::WorkUse::NotEvaluated
    );
    for component in &ids {
        let current = f.app.component(&f.admin, component, None).unwrap();
        assert_eq!(current.revision, Counter(2));
        assert_eq!(&current.record.registration.id, component);
        let old = f
            .app
            .component(&f.admin, component, Some(Counter(1)))
            .unwrap();
        assert_ne!(
            old.record.registration.declaration.label,
            current.record.registration.declaration.label
        );
    }
    let mut archived = Vec::new();
    let mut after = Counter(0);
    loop {
        let page = f
            .app
            .component_intake_history(&f.admin, &freeze.request.id, after)
            .unwrap();
        archived.extend(page.events);
        if let Some(next) = page.next_after {
            after = next
        } else {
            break;
        }
    }
    assert_eq!(archived, original);
    let new = f
        .app
        .update_component(
            &f.admin,
            id().as_str(),
            rx_application::resident_component::Update {
                id: ids[0].clone(),
                expected_revision: Counter(2),
                declaration: Declaration {
                    label: name("p-owned"),
                    catalog: CatalogReference {
                        program: name("status"),
                        digest: Digest::from_bytes([9; 32]),
                    },
                },
            },
        )
        .unwrap();
    assert_eq!(new.revision, Counter(3));
    f.app.configure_component_sources(BTreeMap::new()).unwrap();
    assert_eq!(
        f.app
            .component_intake_receipt(&f.admin, &freeze.request.id)
            .unwrap()
            .id,
        freeze.request.id
    );
}
#[test]
fn source_and_current_authority_failures_cannot_publish_imports() {
    let mut f = fixture(1, false);
    let (_dir, db, freeze, _, _) = source(&id(), 1);
    let input = configure(&mut f, &freeze);
    assert!(
        f.app
            .prepare_component_intake(&f.operator, id(), input.clone())
            .is_err()
    );
    let Preflight::Read(t) = f
        .app
        .prepare_component_intake(&f.admin, id(), input)
        .unwrap()
    else {
        panic!();
    };
    assert!(Reader::open(*t, db).is_err());
    assert!(
        f.app
            .component_intake_progress(&f.admin, &freeze.request.id)
            .is_err()
    );
    let (_dir, db, freeze, ids, _) = source(&f.app.installation.id, 1);
    let input = configure(&mut f, &freeze);
    let key = id();
    let mut r = reader(&mut f, &key, input, db);
    stage(&mut f, &mut r);
    f.app.configure_component_sources(BTreeMap::new()).unwrap();
    assert!(f.app.finish_component_intake(r.finish()).is_err());
    assert!(f.app.component(&f.admin, &ids[0], None).is_err());
}

#[test]
fn a_new_boot_reauthorizes_and_resumes_the_same_partial_intake() {
    let mut f = fixture(1, false);
    let (dir, db, freeze, ids, _) = source(&f.app.installation.id, 12);
    let input = configure(&mut f, &freeze);
    let key = id();
    let r = reader(&mut f, &key, input.clone(), db);
    f.app.begin_component_intake(r.begin()).unwrap();
    f.app
        .stage_component_declarations(r.declarations(Counter(0)).unwrap())
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
        app.stage_component_declarations(r.declarations(Counter(8)).unwrap())
            .is_err()
    );
    drop(r);
    let session = id();
    app.authenticated_session(
        &name("admin"),
        session.clone(),
        TimePoint {
            clock_id: "test/boottime".into(),
            ticks_ns: Counter(u64::MAX),
        },
    )
    .unwrap();
    let admin = Identity {
        principal: name("admin"),
        session,
        terminal: None,
    };
    let binding = SourceBinding {
        owner: name("admin"),
        location: Digest::from_bytes([77; 32]),
    };
    app.configure_component_sources([(name("source"), binding)].into())
        .unwrap();
    let Preflight::Read(t) = app
        .prepare_component_intake(&admin, key.clone(), input.clone())
        .unwrap()
    else {
        panic!();
    };
    let mut r = Reader::open(
        *t,
        SqliteRepository::open_sealed_existing(dir.path().join("source.db")).unwrap(),
    )
    .unwrap();
    let mut p = app.begin_component_intake(r.begin()).unwrap();
    assert_eq!(p.declarations, Counter(8));
    while p.declarations < p.expected_declarations {
        p = app
            .stage_component_declarations(r.declarations(p.declarations).unwrap())
            .unwrap();
    }
    while p.history_after < p.history_head {
        p = app
            .stage_component_history(r.history(p.history_after).unwrap())
            .unwrap();
    }
    app.finish_component_intake(r.finish()).unwrap();
    assert_eq!(
        app.component(&admin, &ids[0], None).unwrap().revision,
        Counter(2)
    );
    assert!(matches!(
        app.prepare_component_intake(&admin, key, input).unwrap(),
        Preflight::Recorded(_)
    ));
}

#[test]
fn identity_collision_and_inconsistent_history_leave_no_usable_new_registration() {
    let mut f = fixture(1, false);
    let existing = f
        .app
        .create_component(
            &f.admin,
            id().as_str(),
            rx_application::resident_component::Create {
                declaration: Declaration {
                    label: name("existing"),
                    catalog: CatalogReference {
                        program: name("status"),
                        digest: Digest::from_bytes([9; 32]),
                    },
                },
            },
        )
        .unwrap();
    let (_dir, db, freeze, _, _) = source_with(
        &f.app.installation.id,
        1,
        Some(existing.record.registration.id.clone()),
        false,
    );
    let input = configure(&mut f, &freeze);
    let key = id();
    let r = reader(&mut f, &key, input, db);
    f.app.begin_component_intake(r.begin()).unwrap();
    assert!(
        f.app
            .stage_component_declarations(r.declarations(Counter(0)).unwrap())
            .is_err()
    );
    assert_eq!(
        f.app
            .component(&f.admin, &existing.record.registration.id, None)
            .unwrap(),
        existing
    );
    let (_dir, db, freeze, ids, _) = source_with(&f.app.installation.id, 1, None, true);
    let input = configure(&mut f, &freeze);
    let key = id();
    let mut r = reader(&mut f, &key, input, db);
    f.app.begin_component_intake(r.begin()).unwrap();
    f.app
        .stage_component_declarations(r.declarations(Counter(0)).unwrap())
        .unwrap();
    assert!(
        f.app
            .stage_component_history(r.history(Counter(0)).unwrap())
            .is_err()
    );
    assert!(f.app.finish_component_intake(r.finish()).is_err());
    assert!(f.app.component(&f.admin, &ids[0], None).is_err());
}

#[test]
fn canonical_import_requires_revoking_diagnostic_alias_and_prevents_recreation() {
    use rx_application::{resident_component as component, resident_reporting as reporting};
    let mut f = fixture(1, false);
    let (_dir, db, freeze, ids, _) = source(&f.app.installation.id, 1);
    let input = configure(&mut f, &freeze);
    f.app
        .put_principal(&f.admin, principal("reporter", &[Role::Observer]), None)
        .unwrap();
    let peer = f
        .app
        .open_resident_reporter(&name("reporter"), id(), Digest::from_bytes([3; 32]))
        .unwrap();
    let alias = f
        .app
        .create_component(
            &f.admin,
            id().as_str(),
            component::Create {
                declaration: Declaration {
                    label: name("alias"),
                    catalog: CatalogReference {
                        program: name("status"),
                        digest: Digest::from_bytes([9; 32]),
                    },
                },
            },
        )
        .unwrap();
    let issue = reporting::Issue {
        component: alias.record.registration.id.clone(),
        expected_component_revision: alias.revision,
        reporter_session: peer.id.clone(),
        source_registration: ids[0].clone(),
        source_revision: Counter(2),
    };
    let scope = f
        .app
        .issue_resident_reporting(&f.admin, id().as_str(), issue.clone())
        .unwrap();
    let mut r = reader(&mut f, &id(), input, db);
    f.app.begin_component_intake(r.begin()).unwrap();
    assert!(
        f.app
            .stage_component_declarations(r.declarations(Counter(0)).unwrap())
            .is_err()
    );
    f.app
        .revoke_resident_reporting(
            &f.admin,
            id().as_str(),
            reporting::Revoke {
                scope: scope.scope.id,
                expected_revision: scope.revision,
            },
        )
        .unwrap();
    f.app
        .stage_component_declarations(r.declarations(Counter(0)).unwrap())
        .unwrap();
    assert!(
        f.app
            .issue_resident_reporting(&f.admin, id().as_str(), issue.clone())
            .is_err(),
        "staging must not allow a new diagnostic alias"
    );
    let canonical = reporting::Issue {
        component: ids[0].clone(),
        expected_component_revision: Counter(2),
        ..issue.clone()
    };
    assert!(
        f.app
            .issue_resident_reporting(&f.admin, id().as_str(), canonical.clone())
            .is_err()
    );
    stage(&mut f, &mut r);
    f.app.finish_component_intake(r.finish()).unwrap();
    assert!(
        f.app
            .issue_resident_reporting(&f.admin, id().as_str(), issue)
            .is_err()
    );
    f.app
        .issue_resident_reporting(&f.admin, id().as_str(), canonical)
        .unwrap();
}

#[test]
fn a_cached_import_receipt_does_not_bypass_current_authorization() {
    let mut f = fixture(1, false);
    f.app
        .put_principal(&f.admin, principal("author", &[Role::Engineer]), None)
        .unwrap();
    let session = f
        .app
        .authenticated_session(
            &name("author"),
            id(),
            TimePoint {
                clock_id: "test/boottime".into(),
                ticks_ns: Counter(u64::MAX),
            },
        )
        .unwrap();
    let author = Identity {
        principal: name("author"),
        session: session.id,
        terminal: None,
    };
    let (_dir, db, freeze, ids, _) = source(&f.app.installation.id, 1);
    let binding = SourceBinding {
        owner: name("author"),
        location: Digest::from_bytes([77; 32]),
    };
    f.app
        .configure_component_sources([(name("source"), binding.clone())].into())
        .unwrap();
    let input = Submit {
        source: name("source"),
        freeze: freeze.request.id.clone(),
        expected_binding: binding.fingerprint(&name("source")).unwrap(),
    };
    let key = id();
    let Preflight::Read(t) = f
        .app
        .prepare_component_intake(&author, key.clone(), input.clone())
        .unwrap()
    else {
        panic!()
    };
    let mut r = Reader::open(*t, db).unwrap();
    stage(&mut f, &mut r);
    f.app.finish_component_intake(r.finish()).unwrap();
    assert!(matches!(
        f.app
            .prepare_component_intake(&author, key.clone(), input.clone())
            .unwrap(),
        Preflight::Recorded(_)
    ));
    f.app
        .put_principal(
            &f.admin,
            principal("author", &[Role::Observer]),
            Some(Counter(1)),
        )
        .unwrap();
    assert!(f.app.prepare_component_intake(&author, key, input).is_err());
    assert!(
        f.app
            .component_intake_receipt(&author, &freeze.request.id)
            .is_err()
    );
    assert!(f.app.component(&author, &ids[0], None).is_err());
    assert!(
        f.app
            .component_intake_receipt(&f.admin, &freeze.request.id)
            .is_ok()
    );
}

#[test]
fn only_a_current_canonical_report_scope_reads_original_target_acceptance() {
    use rx_application::{resident_component as component, resident_reporting as reporting};
    let mut f = fixture(1, false);
    let (_dir, db, freeze, ids, _) = source(&f.app.installation.id, 1);
    let input = configure(&mut f, &freeze);
    let mut r = reader(&mut f, &id(), input, db);
    stage(&mut f, &mut r);
    f.app.finish_component_intake(r.finish()).unwrap();
    f.app
        .put_principal(&f.admin, principal("reporter", &[Role::Observer]), None)
        .unwrap();
    let peer = f
        .app
        .open_resident_reporter(&name("reporter"), id(), Digest::from_bytes([3; 32]))
        .unwrap();
    let actor = reporting::ReporterIdentity {
        principal: peer.principal.clone(),
        session: peer.id.clone(),
        authentication_binding: peer.authentication_binding,
    };
    assert!(
        f.app
            .registration_target_acceptance(&actor, &id(), &freeze.request.id)
            .is_err()
    );
    let scope = f
        .app
        .issue_resident_reporting(
            &f.admin,
            id().as_str(),
            reporting::Issue {
                component: ids[0].clone(),
                expected_component_revision: Counter(2),
                reporter_session: peer.id.clone(),
                source_registration: ids[0].clone(),
                source_revision: Counter(2),
            },
        )
        .unwrap();
    let value = f
        .app
        .registration_target_acceptance(&actor, &scope.scope.id, &freeze.request.id)
        .unwrap();
    assert_eq!(value.original, freeze);
    assert_eq!(value.peer, peer);
    assert_eq!(value.component, ids[0]);
    assert!(
        f.app
            .registration_target_acceptance(&actor, &scope.scope.id, &id())
            .is_err()
    );
    let forged = reporting::ReporterIdentity {
        authentication_binding: Digest::from_bytes([4; 32]),
        ..actor.clone()
    };
    assert!(
        f.app
            .registration_target_acceptance(&forged, &scope.scope.id, &freeze.request.id)
            .is_err()
    );
    f.app
        .update_component(
            &f.admin,
            id().as_str(),
            component::Update {
                id: ids[0].clone(),
                expected_revision: Counter(2),
                declaration: Declaration {
                    label: name("new-p-declaration"),
                    catalog: CatalogReference {
                        program: name("different"),
                        digest: Digest::from_bytes([5; 32]),
                    },
                },
            },
        )
        .unwrap();
    assert_eq!(
        f.app
            .registration_target_acceptance(&actor, &scope.scope.id, &freeze.request.id)
            .unwrap(),
        value
    );
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
            .registration_target_acceptance(&actor, &scope.scope.id, &freeze.request.id)
            .is_err()
    );
    let scope = f
        .app
        .issue_resident_reporting(
            &f.admin,
            id().as_str(),
            reporting::Issue {
                component: ids[0].clone(),
                expected_component_revision: Counter(3),
                reporter_session: peer.id.clone(),
                source_registration: ids[0].clone(),
                source_revision: Counter(2),
            },
        )
        .unwrap();
    assert!(
        f.app
            .registration_target_acceptance(&actor, &scope.scope.id, &freeze.request.id)
            .is_ok()
    );
    f.app
        .open_resident_reporter(&name("reporter"), id(), Digest::from_bytes([3; 32]))
        .unwrap();
    assert!(
        f.app
            .registration_target_acceptance(&actor, &scope.scope.id, &freeze.request.id)
            .is_err()
    );
    let peer = f
        .app
        .open_resident_reporter(&name("reporter"), id(), Digest::from_bytes([3; 32]))
        .unwrap();
    let actor = reporting::ReporterIdentity {
        principal: peer.principal.clone(),
        session: peer.id.clone(),
        authentication_binding: peer.authentication_binding,
    };
    let issue = reporting::Issue {
        component: ids[0].clone(),
        expected_component_revision: Counter(3),
        reporter_session: peer.id,
        source_registration: ids[0].clone(),
        source_revision: Counter(2),
    };
    let scope = f
        .app
        .issue_resident_reporting(&f.admin, id().as_str(), issue.clone())
        .unwrap();
    assert!(
        f.app
            .registration_target_acceptance(&actor, &scope.scope.id, &freeze.request.id)
            .unwrap()
            .same_decision(&value)
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
        app.registration_target_acceptance(&actor, &scope.scope.id, &freeze.request.id)
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
    let peer = app
        .open_resident_reporter(&name("reporter"), id(), Digest::from_bytes([3; 32]))
        .unwrap();
    let actor = reporting::ReporterIdentity {
        principal: peer.principal.clone(),
        session: peer.id.clone(),
        authentication_binding: peer.authentication_binding,
    };
    let scope = app
        .issue_resident_reporting(
            &admin,
            id().as_str(),
            reporting::Issue {
                reporter_session: peer.id,
                ..issue
            },
        )
        .unwrap();
    assert!(
        app.registration_target_acceptance(&actor, &scope.scope.id, &freeze.request.id)
            .unwrap()
            .same_decision(&value)
    );
}
