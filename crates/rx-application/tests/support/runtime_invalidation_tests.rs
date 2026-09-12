use super::*;
use rx_application::{persistence as p, runtime_invalidation as origin};
use rx_domain::canonical;

fn restart(repository: FaultRepository, clock: ManualClock, installation: &Installation) -> App {
    Engine::open(
        repository,
        clock,
        SimulationAuthority,
        installation.id.clone(),
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap()
}
fn cell(tx: &mut dyn rx_ports::Transaction) -> rx_ports::Result<(Counter, Cell)> {
    p::load(tx, "cell", name("cell/a"), "rx.internal.cell.v1")
}

#[test]
fn first_runtime_restart_records_the_exact_before_and_after_boundaries() {
    let mut f = fixture(1, false);
    let before = f.app.inspect_cell(&f.admin, &f.configuration.id).unwrap();
    let previous = f.app.installation.clone();
    let mut repository = f.app.into_repository();
    repository
        .transact(|tx| {
            assert!(tx.scan("runtimeinvalidationorigin/")?.is_empty());
            Ok(())
        })
        .unwrap();
    let app = restart(repository, f.clock, &previous);
    let current = app.installation.clone();
    let mut repository = app.into_repository();
    repository
        .transact(|tx| {
            let (revision, after) = cell(tx)?;
            assert_eq!(after.blocks.len(), 1);
            let block = &after.blocks[0];
            let value =
                origin::read_for_cell(tx, &current, &f.configuration.id, &block.id)?.unwrap();
            assert_eq!(value.installation, previous.id);
            assert_eq!(value.store_generation, previous.store_generation);
            assert_eq!(value.previous_runtime_boot, previous.runtime_boot);
            assert_eq!(value.runtime_boot, current.runtime_boot);
            assert_eq!(value.before.revision, before.0);
            assert_eq!(value.before.epoch, before.1.epoch);
            assert_eq!(value.before.scope_epochs, before.1.scope_epochs);
            assert_eq!(value.after.revision, revision);
            assert_eq!(value.after.epoch, after.epoch);
            assert_eq!(value.after.scope_epochs, after.scope_epochs);
            assert_eq!(
                value.before.configuration_digest,
                origin::configuration_digest(&f.configuration).unwrap()
            );
            assert_eq!(
                value.before.configuration_digest,
                value.after.configuration_digest
            );
            assert_eq!(
                canonical::bytes(&value.block).unwrap(),
                canonical::bytes(block).unwrap()
            );
            assert!(value.digest().is_ok());
            assert!(after.qualification.is_none());
            assert_eq!(after.commissioning, Some(Commissioning::NotCommissioned));
            assert!(tx.scan("changeblockowner/")?.is_empty());
            Ok(())
        })
        .unwrap();
}

#[test]
fn consecutive_restarts_keep_each_immutable_origin_and_old_block_readable() {
    let f = fixture(1, false);
    let mut previous = f.app.installation.clone();
    let mut repository = f.app.into_repository();
    let mut historical = BTreeMap::new();
    for expected in 1..=3 {
        let app = restart(repository, f.clock.clone(), &previous);
        let current = app.installation.clone();
        repository = app.into_repository();
        repository
            .transact(|tx| {
                let (_, after) = cell(tx)?;
                assert_eq!(after.blocks.len(), expected);
                assert_eq!(tx.scan("runtimeinvalidationorigin/")?.len(), expected);
                for block in &after.blocks {
                    let value =
                        origin::read_for_cell(tx, &current, &after.configuration.id, &block.id)?
                            .unwrap();
                    let bytes = canonical::bytes(&value).unwrap();
                    if let Some(old) = historical.get(&block.id) {
                        assert_eq!(old, &bytes);
                    } else {
                        assert_eq!(value.previous_runtime_boot, previous.runtime_boot);
                        assert_eq!(value.runtime_boot, current.runtime_boot);
                        historical.insert(block.id.clone(), bytes);
                    }
                }
                Ok(())
            })
            .unwrap();
        previous = current;
    }
}

#[test]
fn origin_lookup_rejects_wrong_identity_and_does_not_accept_a_changed_block() {
    let f = fixture(1, false);
    let previous = f.app.installation.clone();
    let app = restart(f.app.into_repository(), f.clock, &previous);
    let current = app.installation.clone();
    let mut repository = app.into_repository();
    let original = repository
        .transact(|tx| {
            let (_, after) = cell(tx)?;
            let block = &after.blocks[0].id;
            assert!(origin::load(tx, &id())?.is_none());
            assert!(origin::read_for_cell(tx, &current, &name("cell/b"), block).is_err());
            for replacement in [0, 1, 2] {
                let mut changed = current.clone();
                match replacement {
                    0 => changed.id = id(),
                    1 => changed.store_generation = id(),
                    _ => changed.runtime_boot = id(),
                }
                assert!(
                    origin::read_for_cell(tx, &changed, &after.configuration.id, block).is_err()
                );
            }
            let value = origin::load(tx, block)?.unwrap();
            let wrong = id();
            p::save(
                tx,
                "runtimeinvalidationorigin",
                &wrong,
                None,
                origin::SCHEMA,
                &value,
            )?;
            assert!(origin::load(tx, &wrong).is_err());
            Ok(value)
        })
        .unwrap();
    for field in [0, 1, 2] {
        let result: rx_ports::Result<()> = repository.transact(|tx| {
            let (revision, mut after) = cell(tx)?;
            match field {
                0 => after.blocks[0].reason = BlockReason::OperatorHold,
                1 => after.blocks[0].created_revision = Some(Counter(999)),
                _ => after.scope_epochs.clear(),
            }
            p::save(
                tx,
                "cell",
                &after.configuration.id,
                Some(revision),
                "rx.internal.cell.v1",
                &after,
            )?;
            assert!(
                origin::read_for_cell(tx, &current, &after.configuration.id, &original.block.id)
                    .is_err()
            );
            Err(StoreError::Unavailable(
                "rollback corrupted test state".into(),
            ))
        });
        assert!(result.is_err());
    }
    repository
        .transact(|tx| {
            let value =
                origin::read_for_cell(tx, &current, &original.cell, &original.block.id)?.unwrap();
            assert_eq!(value.digest().unwrap(), original.digest().unwrap());
            Ok(())
        })
        .unwrap();
}

#[test]
fn older_runtime_restart_reasons_and_operator_holds_do_not_acquire_origins() {
    let mut f = fixture(1, false);
    f.app
        .hold(&f.operator, id().as_str(), &f.configuration.id)
        .unwrap();
    let previous = f.app.installation.clone();
    let mut repository = f.app.into_repository();
    let old = repository
        .transact(|tx| {
            let (revision, mut before) = cell(tx)?;
            let legacy = Block {
                id: id(),
                created_revision: Some(revision.increment().unwrap()),
                case_id: None,
                reason: BlockReason::RuntimeRestart,
                latched: true,
                scopes: before.configuration.scopes.clone(),
            };
            before.blocks.push(legacy);
            p::save(
                tx,
                "cell",
                &before.configuration.id,
                Some(revision),
                "rx.internal.cell.v1",
                &before,
            )?;
            Ok(before.blocks)
        })
        .unwrap();
    let app = restart(repository, f.clock, &previous);
    let current = app.installation.clone();
    let mut repository = app.into_repository();
    repository
        .transact(|tx| {
            let (_, after) = cell(tx)?;
            for block in &old {
                assert!(
                    origin::read_for_cell(tx, &current, &after.configuration.id, &block.id)?
                        .is_none()
                );
                let kept = after.blocks.iter().find(|b| b.id == block.id).unwrap();
                assert_eq!(
                    canonical::bytes(kept).unwrap(),
                    canonical::bytes(block).unwrap()
                );
            }
            assert_eq!(tx.scan("runtimeinvalidationorigin/")?.len(), 1);
            assert!(tx.scan("changeblockowner/")?.is_empty());
            assert!(after.qualification.is_none());
            Ok(())
        })
        .unwrap();
}

#[test]
fn origin_recording_preserves_source_facts_and_existing_run_budget_on_restart() {
    let mut f = fixture(1, true);
    let run = start(&mut f, 2);
    activation(&mut f, &run);
    let previous = f.app.installation.clone();
    let before_run = f.app.inspect_run(&f.operator, &run.id).unwrap().1;
    let mut repository = f.app.into_repository();
    let facts = repository.transact(|tx| tx.scan("fact/")).unwrap();
    let app = restart(repository, f.clock, &previous);
    let current = app.installation.clone();
    let mut repository = app.into_repository();
    repository
        .transact(|tx| {
            assert_eq!(tx.scan("fact/")?, facts);
            let (_, after_run): (_, Run) = p::load(tx, "run", &run.id, "rx.internal.run.v1")?;
            assert_eq!(
                canonical::bytes(&after_run.budget).unwrap(),
                canonical::bytes(&before_run.budget).unwrap()
            );
            assert_eq!(after_run.part_ids, before_run.part_ids);
            assert_eq!(after_run.state, RunState::RecoveryRequired);
            let (_, after) = cell(tx)?;
            let block = after
                .blocks
                .iter()
                .find(|b| b.reason == BlockReason::RuntimeRestart)
                .unwrap();
            assert!(
                origin::read_for_cell(tx, &current, &after.configuration.id, &block.id)?.is_some()
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn restart_origin_and_restriction_commit_together_even_when_the_reply_is_lost() {
    for failure in [1, 2] {
        let f = fixture(1, false);
        let previous = f.app.installation.clone();
        let mut repository = f.app.into_repository();
        let before = repository.snapshot().unwrap();
        f.failure.store(failure, Ordering::SeqCst);
        let result = Engine::open(
            repository,
            f.clock,
            SimulationAuthority,
            previous.id,
            principal("admin", &[Role::AccountAdmin]),
        );
        assert!(result.is_err());
        let mut repository =
            SqliteRepository::open(f._directory.path().join("platform.db")).unwrap();
        if failure == 1 {
            assert_eq!(repository.snapshot().unwrap(), before);
        } else {
            repository
                .transact(|tx| {
                    let metadata = tx.get(&p::name("installation/current"))?.unwrap();
                    let current: Installation =
                        p::decode(&metadata, "rx.internal.installation.v1")?;
                    let (_, after) = cell(tx)?;
                    assert_eq!(after.blocks.len(), 1);
                    let block = &after.blocks[0];
                    let value =
                        origin::read_for_cell(tx, &current, &after.configuration.id, &block.id)?
                            .unwrap();
                    assert_eq!(value.runtime_boot, current.runtime_boot);
                    assert_eq!(tx.scan("runtimeinvalidationorigin/")?.len(), 1);
                    Ok(())
                })
                .unwrap();
        }
    }
}

#[test]
fn runtime_restriction_view_checks_current_roles_cell_access_and_preserves_authority_state() {
    let f = fixture(1, false);
    let previous = f.app.installation.clone();
    let mut app = restart(f.app.into_repository(), f.clock, &previous);
    let session = app
        .authenticated_session(&name("admin"), id(), expiry(99_000))
        .unwrap();
    let admin = Identity {
        principal: name("admin"),
        session: session.id,
        terminal: None,
    };
    let before = app.inspect_cell(&admin, &name("cell/a")).unwrap();
    let expected = app.runtime_restrictions(&admin, &name("cell/a")).unwrap();
    assert_eq!(expected.restrictions.len(), 1);
    assert_eq!(expected.revision, before.0);
    assert_eq!(expected.epoch, before.1.epoch);
    for (index, role) in [
        Role::Engineer,
        Role::Verifier,
        Role::ReleaseManager,
        Role::Operator,
        Role::Observer,
        Role::RecoveryLead,
    ]
    .into_iter()
    .enumerate()
    {
        let label = format!("restriction-reader-{index}");
        let mut reader = principal(&label, &[role]);
        reader.cells = [name("cell/a")].into_iter().collect();
        app.put_principal(&admin, reader.clone(), None).unwrap();
        let session = app
            .authenticated_session(&reader.id, id(), expiry(99_000))
            .unwrap();
        let actor = Identity {
            principal: reader.id.clone(),
            session: session.id,
            terminal: None,
        };
        let before_read = app.inspect_cell(&admin, &name("cell/a")).unwrap();
        let expected = app.runtime_restrictions(&admin, &name("cell/a")).unwrap();
        let result = app.runtime_restrictions(&actor, &name("cell/a"));
        if matches!(role, Role::Engineer | Role::Verifier | Role::ReleaseManager) {
            assert_eq!(
                canonical::bytes(&result.unwrap()).unwrap(),
                canonical::bytes(&expected).unwrap()
            );
        } else {
            assert!(matches!(
                result,
                Err(StoreError::Rejected(Rejection::Forbidden))
            ));
        }
        assert!(matches!(
            app.runtime_restrictions(&actor, &name("cell/b")),
            Err(StoreError::Rejected(Rejection::Forbidden))
        ));
        assert_eq!(
            canonical::bytes(&before_read).unwrap(),
            canonical::bytes(&app.inspect_cell(&admin, &name("cell/a")).unwrap()).unwrap()
        );
        reader.active = false;
        app.put_principal(&admin, reader, Some(Counter(1))).unwrap();
        let after_revocation = app.inspect_cell(&admin, &name("cell/a")).unwrap();
        assert!(app.runtime_restrictions(&actor, &name("cell/a")).is_err());
        assert_eq!(
            canonical::bytes(&after_revocation).unwrap(),
            canonical::bytes(&app.inspect_cell(&admin, &name("cell/a")).unwrap()).unwrap()
        );
    }
    let installation = app.installation.clone();
    app.installation.id = id();
    assert!(app.runtime_restrictions(&admin, &name("cell/a")).is_err());
    app.installation = installation;
    let mut repository = app.into_repository();
    repository
        .transact(|tx| {
            assert!(tx.scan("changeblockowner/")?.is_empty());
            assert!(tx.scan("qualificationbatch/")?.is_empty());
            assert_eq!(tx.scan("runtimeinvalidationorigin/")?.len(), 1);
            Ok(())
        })
        .unwrap();
}
