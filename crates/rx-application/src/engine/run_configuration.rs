//! Immutable Run-owned configuration. Legacy inference is permitted only before any replacement.
use super::*;
const BINDING: &str = "rx.run-configuration-binding.v1";
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    run: Id,
    cell: Name,
    configuration: ArtifactRef,
}
fn matches_run(run: &Run, cfg: &CellConfiguration) -> Result<()> {
    if run.cell != cfg.id
        || run.recipe_digest != cfg.recipe.sha256
        || run.envelope_digest != cfg.envelope.sha256
    {
        return Err(StoreError::Integrity(
            "Run configuration identity differs".into(),
        ));
    }
    Ok(())
}
pub(super) fn bind(tx: &mut dyn Transaction, run: &Run, cfg: &CellConfiguration) -> Result<()> {
    if let Some(row) = tx.get(&key("runconfiguration", &run.id))? {
        let b: Binding = decode(&row, BINDING)?;
        if b.run != run.id || b.cell != run.cell {
            return Err(StoreError::Integrity("Run binding identity differs".into()));
        }
        let existing = process_change::read_config(tx, &b.configuration)?;
        matches_run(run, &existing)?;
        // An existing historic binding is never rewritten using the current cell configuration.
        return Ok(());
    }
    if tx.get(&key("configurationselection", &run.cell))?.is_some() {
        return Err(StoreError::Integrity(
            "Run configuration binding missing after replacement".into(),
        ));
    }
    create(tx, run, cfg)
}
pub(super) fn create(tx: &mut dyn Transaction, run: &Run, cfg: &CellConfiguration) -> Result<()> {
    matches_run(run, cfg)?;
    let reference = process_change::store_config(tx, cfg)?;
    let binding = Binding {
        run: run.id.clone(),
        cell: run.cell.clone(),
        configuration: reference,
    };
    save(tx, "runconfiguration", &run.id, None, BINDING, &binding)?;
    Ok(())
}
pub(super) fn read(
    tx: &mut dyn Transaction,
    run: &Run,
    current: &CellConfiguration,
) -> Result<CellConfiguration> {
    if let Some(row) = tx.get(&key("runconfiguration", &run.id))? {
        let b: Binding = decode(&row, BINDING)?;
        if b.run != run.id || b.cell != run.cell {
            return Err(StoreError::Integrity("Run binding identity differs".into()));
        }
        let cfg = process_change::read_config(tx, &b.configuration)?;
        matches_run(run, &cfg)?;
        return Ok(cfg);
    }
    if tx.get(&key("configurationselection", &run.cell))?.is_some() {
        return Err(StoreError::Integrity(
            "Historical Run configuration unavailable".into(),
        ));
    }
    matches_run(run, current)?;
    Ok(current.clone())
}
pub(super) fn require_current(tx: &mut dyn Transaction, run: &Run, cell: &Cell) -> Result<()> {
    if process_change::config_ref(&read(tx, run, &cell.configuration)?)?
        != process_change::config_ref(&cell.configuration)?
    {
        return reject(Reject::StaleRevision);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_configuration_is_inferred_only_before_replacement_and_bound_history_never_drifts() {
        let dir = tempfile::tempdir().unwrap();
        let mut repository =
            rx_storage::SqliteRepository::open(dir.path().join("history.db")).unwrap();
        let artifact = |n| ArtifactRef {
            sha256: Digest::from_bytes([n; 32]),
            schema_id: name("test/artifact"),
            size_bytes: Counter(1),
        };
        let cfg = CellConfiguration {
            process: None,
            id: name("cell/a"),
            environment: Environment::Simulation,
            definition: artifact(1),
            envelope: artifact(2),
            recipe: artifact(3),
            site_config_digest: Digest::from_bytes([4; 32]),
            scopes: vec![name("zone/a")],
            hosts: vec![name("host/a")],
            executor: name("executor"),
            maximum_budget: Counter(1),
            permit_ttl_ns: Counter(1),
            start_timeout_ns: Counter(1),
            start_conditions: vec![],
            steps: vec![],
            maintained_conditions: vec![],
            fact_specs: vec![],
        };
        let run = Run {
            id: id(),
            cell: cfg.id.clone(),
            recipe_digest: cfg.recipe.sha256,
            envelope_digest: cfg.envelope.sha256,
            purpose: None,
            state: RunState::Completed,
            budget: None,
            executor_session: None,
            mandate: None,
            part_ids: vec![],
            pending_attempt: None,
        };
        repository
            .transact(|tx| {
                assert_eq!(
                    process_change::config_ref(&read(tx, &run, &cfg)?)?,
                    process_change::config_ref(&cfg)?
                );
                bind(tx, &run, &cfg)?;
                tx.put(
                    &key("configurationselection", &cfg.id),
                    None,
                    &doc("test/selection", &true)?,
                )?;
                let mut next = cfg.clone();
                next.maximum_budget = Counter(2);
                assert_eq!(read(tx, &run, &next)?.maximum_budget, Counter(1));
                let missing = Run {
                    id: id(),
                    ..run.clone()
                };
                assert!(matches!(
                    read(tx, &missing, &next),
                    Err(StoreError::Integrity(_))
                ));
                assert!(matches!(
                    bind(tx, &missing, &next),
                    Err(StoreError::Integrity(_))
                ));
                let new_run = Run {
                    id: id(),
                    ..run.clone()
                };
                create(tx, &new_run, &next)?;
                assert_eq!(read(tx, &new_run, &next)?.maximum_budget, Counter(2));
                Ok(())
            })
            .unwrap();
    }
}
