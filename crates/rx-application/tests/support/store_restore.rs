use super::*;
use rx_application::{persistence as p, runtime_invalidation as origin, store_restore};

fn reopen(f: Fixture) -> (App, ManualClock, Installation, CellConfiguration) {
    let installation = f.app.installation.clone();
    let app = Engine::open(
        f.app.into_repository(),
        f.clock.clone(),
        SimulationAuthority,
        installation.id.clone(),
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    (app, f.clock, installation, f.configuration)
}

#[test]
fn restore_rotates_the_generation_and_retires_prior_restart_provenance() {
    let f = fixture(1, false);
    let (app, clock, previous, configuration) = reopen(f);
    // The restart left a RuntimeRestart restriction with provenance under this generation.
    let restarted = app.installation.clone();
    let mut repository = app.into_repository();
    let block = repository
        .transact(|tx| {
            let (_, cell): (_, Cell) =
                p::load(tx, "cell", configuration.id.clone(), "rx.internal.cell.v1")?;
            let block = cell.blocks[0].id.clone();
            assert!(origin::read_for_cell(tx, &restarted, &configuration.id, &block)?.is_some());
            Ok(block)
        })
        .unwrap();
    assert!(
        store_restore::last_store_restore(&mut repository)
            .unwrap()
            .is_none()
    );

    let wrong = store_restore::restore_store(
        &mut repository,
        &id(),
        Digest::from_bytes([5; 32]),
        clock.now(),
    );
    assert!(matches!(
        wrong,
        Err(StoreError::Rejected(Rejection::InvalidInput))
    ));

    let first = store_restore::restore_store(
        &mut repository,
        &previous.id,
        Digest::from_bytes([5; 32]),
        clock.now(),
    )
    .unwrap();
    assert_eq!(first.installation, previous.id);
    assert_eq!(first.previous_generation, previous.store_generation);
    assert_ne!(first.generation, previous.store_generation);
    assert_eq!(first.previous_runtime_boot, restarted.runtime_boot);
    assert_eq!(
        store_restore::last_store_restore(&mut repository).unwrap(),
        Some(first.clone())
    );

    // The next runtime start carries the new generation; provenance from before the restore
    // is no longer current provenance, and the cell is invalidated again.
    let app = Engine::open(
        repository,
        clock.clone(),
        SimulationAuthority,
        previous.id.clone(),
        principal("admin", &[Role::AccountAdmin]),
    )
    .unwrap();
    let current = app.installation.clone();
    assert_eq!(current.store_generation, first.generation);
    let mut repository = app.into_repository();
    repository
        .transact(|tx| {
            assert!(origin::read_for_cell(tx, &current, &configuration.id, &block).is_err());
            let (_, cell): (_, Cell) =
                p::load(tx, "cell", configuration.id.clone(), "rx.internal.cell.v1")?;
            assert!(cell.blocks.iter().any(|b| b.id != block && b.latched));
            Ok(())
        })
        .unwrap();

    // Restores chain: the second one names the first one's generation as previous.
    let second = store_restore::restore_store(
        &mut repository,
        &previous.id,
        Digest::from_bytes([6; 32]),
        clock.now(),
    )
    .unwrap();
    assert_eq!(second.previous_generation, first.generation);
    assert_ne!(second.id, first.id);
    assert_eq!(
        store_restore::last_store_restore(&mut repository)
            .unwrap()
            .map(|r| r.id),
        Some(second.id)
    );
}
