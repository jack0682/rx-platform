use super::*;
use rx_application::process_draft::{Filter, Library, PreparedSave, Save};

fn command(f: &Fixture, draft: Id) -> Save {
    Save {
        id: draft,
        cell: f.configuration.id.clone(),
        expected: None,
        title: "Material supply".into(),
        document: draft_document(),
        presentation: None,
        library: None,
    }
}

#[test]
fn archive_and_restore_are_atomic_versioned_and_do_not_change_execution() {
    for failure in [1, 2] {
        let mut f = fixture(1, true);
        let cell_before = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap();
        let mut save = command(&f, id());
        let first = f
            .app
            .save_process_draft(
                &f.admin,
                &id(),
                PreparedSave::prepare(save.clone()).unwrap(),
            )
            .unwrap();
        save.expected = Some(Counter(1));
        save.library = Some(Library {
            site: Some("site/0f".into()),
            service: Some("service/heat-treatment".into()),
            archived: true,
        });
        let key = id();
        f.failure.store(failure, Ordering::SeqCst);
        assert!(
            f.app
                .save_process_draft(&f.admin, &key, PreparedSave::prepare(save.clone()).unwrap())
                .is_err()
        );
        let after_loss = f
            .app
            .process_draft(&f.admin, &save.cell, &save.id, None)
            .unwrap();
        assert_eq!(
            after_loss.version.revision,
            Counter(if failure == 1 { 1 } else { 2 })
        );
        let archived = f
            .app
            .save_process_draft(&f.admin, &key, PreparedSave::prepare(save.clone()).unwrap())
            .unwrap();
        assert_eq!(archived.version.revision, Counter(2));
        assert_eq!(
            archived.version.document_digest,
            first.version.document_digest
        );
        assert!(archived.version.library.as_ref().unwrap().archived);
        let mut changed_key_body = save.clone();
        changed_key_body.library.as_mut().unwrap().archived = false;
        assert!(matches!(
            f.app.save_process_draft(
                &f.admin,
                &key,
                PreparedSave::prepare(changed_key_body).unwrap()
            ),
            Err(StoreError::KeyConflict)
        ));
        save.expected = Some(Counter(2));
        let mut legacy = save.clone();
        legacy.library = None;
        assert!(matches!(
            f.app
                .save_process_draft(&f.admin, &id(), PreparedSave::prepare(legacy).unwrap()),
            Err(StoreError::Rejected(rx_domain::fault::Rejection::Busy))
        ));
        save.library.as_mut().unwrap().archived = false;
        let mut hidden_edit = save.clone();
        hidden_edit.title = "edited while archived".into();
        assert!(matches!(
            f.app
                .save_process_draft(&f.admin, &id(), PreparedSave::prepare(hidden_edit).unwrap()),
            Err(StoreError::Rejected(
                rx_domain::fault::Rejection::InvalidInput
            ))
        ));
        let restored = f
            .app
            .save_process_draft(
                &f.admin,
                &id(),
                PreparedSave::prepare(save.clone()).unwrap(),
            )
            .unwrap();
        assert_eq!(restored.version.revision, Counter(3));
        assert!(!restored.version.library.unwrap().archived);
        save.expected = Some(Counter(3));
        save.library = None;
        save.title = "Legacy client title edit".into();
        let edited = f
            .app
            .save_process_draft(
                &f.admin,
                &id(),
                PreparedSave::prepare(save.clone()).unwrap(),
            )
            .unwrap();
        assert_eq!(edited.version.library.unwrap().site, Some("site/0f".into()));
        let history = f
            .app
            .process_draft_history(&f.admin, &save.cell, &save.id, None)
            .unwrap();
        assert_eq!(
            history
                .versions
                .iter()
                .map(|v| v.revision.0)
                .collect::<Vec<_>>(),
            vec![4, 3, 2, 1]
        );
        assert!(history.versions[2].library.as_ref().unwrap().archived);
        assert!(history.versions[3].library.is_none());
        let old = f
            .app
            .process_draft(&f.admin, &save.cell, &save.id, Some(Counter(1)))
            .unwrap();
        assert_eq!(old.document, first.document);
        assert_eq!(
            f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap().0,
            cell_before.0
        );
        assert!(f.app.pending_deliveries(128).unwrap().is_empty());
    }
}

#[test]
fn library_filters_apply_before_pagination_and_keep_current_read_authority() {
    let mut f = fixture(1, false);
    for i in 0..60 {
        let draft = Id::new(format!("00000000-0000-4000-8000-{i:012}")).unwrap();
        let mut save = command(&f, draft);
        save.library = Some(Library {
            site: Some((if i == 59 { "site/laser" } else { "site/other" }).into()),
            service: Some("service/process".into()),
            archived: i == 59,
        });
        if i == 59 {
            save.title = "Laser alignment".into();
        }
        f.app
            .save_process_draft(&f.admin, &id(), PreparedSave::prepare(save).unwrap())
            .unwrap();
    }
    let all = f
        .app
        .process_drafts(&f.admin, &f.configuration.id, None, &Filter::default())
        .unwrap();
    assert_eq!(all.drafts.len(), 50);
    assert!(all.next.is_some());
    assert!(!all.drafts.iter().any(|v| v.title.contains("Laser")));
    let filter = Filter {
        query: " lAsEr ".into(),
        site: Some("site/laser".into()),
        service: Some("service/process".into()),
        archived: Some(true),
    };
    let found = f
        .app
        .process_drafts(&f.admin, &f.configuration.id, None, &filter)
        .unwrap();
    assert_eq!(found.drafts.len(), 1);
    assert_eq!(found.drafts[0].title, "Laser alignment");
    assert!(found.next.is_none());
    assert!(
        f.app
            .process_drafts(
                &f.admin,
                &f.configuration.id,
                None,
                &Filter {
                    archived: Some(false),
                    ..filter.clone()
                }
            )
            .unwrap()
            .drafts
            .is_empty()
    );
    assert!(
        f.app
            .process_drafts(&f.operator, &f.configuration.id, None, &filter)
            .is_err()
    );
    assert!(
        f.app
            .process_drafts(&f.admin, &name("cell/not-permitted"), None, &filter)
            .is_err()
    );
    assert!(
        f.app
            .process_draft_history(&f.operator, &f.configuration.id, &found.drafts[0].id, None)
            .is_err()
    );
    assert!(
        f.app
            .process_draft_history(&f.admin, &name("cell/b"), &found.drafts[0].id, None)
            .is_err()
    );
    assert!(
        f.app
            .process_drafts(
                &f.admin,
                &f.configuration.id,
                None,
                &Filter {
                    query: "a".repeat(121),
                    ..Default::default()
                }
            )
            .is_err()
    );
}

#[test]
fn history_pages_are_bounded_and_do_not_repeat_revisions() {
    let mut f = fixture(1, false);
    let mut save = command(&f, id());
    for revision in 1..=54 {
        save.expected = (revision > 1).then_some(Counter(revision - 1));
        save.title = format!("Version {revision}");
        f.app
            .save_process_draft(
                &f.admin,
                &id(),
                PreparedSave::prepare(save.clone()).unwrap(),
            )
            .unwrap();
    }
    let page = f
        .app
        .process_draft_history(&f.admin, &save.cell, &save.id, None)
        .unwrap();
    assert_eq!(page.versions.len(), 50);
    assert_eq!(page.versions.first().unwrap().revision, Counter(54));
    assert_eq!(page.versions.last().unwrap().revision, Counter(5));
    assert_eq!(page.next, Some(Counter(5)));
    let older = f
        .app
        .process_draft_history(&f.admin, &save.cell, &save.id, page.next)
        .unwrap();
    assert_eq!(
        older
            .versions
            .iter()
            .map(|v| v.revision.0)
            .collect::<Vec<_>>(),
        vec![4, 3, 2, 1]
    );
    assert_eq!(older.next, None);
    assert!(
        f.app
            .process_draft_history(&f.admin, &save.cell, &save.id, Some(Counter(1)))
            .unwrap()
            .versions
            .is_empty()
    );
    assert!(
        f.app
            .process_draft_history(&f.admin, &save.cell, &save.id, Some(Counter(0)))
            .is_err()
    );
}
