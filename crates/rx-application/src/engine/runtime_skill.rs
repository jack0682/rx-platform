use super::*;
use crate::{projection::Versioned, runtime_skill as view};

fn binding(meta: &Installation, cfg: &CellConfiguration) -> Result<Option<view::Binding>> {
    let Some(process) = cfg.process.as_ref() else {
        return Ok(None);
    };
    let configuration = process_change::config_ref(cfg)?;
    let digest = canonical::digest(
        "RX-INSTALLED-PROCESS-SKILL-v1",
        &(&meta.id, &meta.store_generation, &cfg.id, &configuration),
    )
    .map_err(domain_error)?;
    Ok(Some(view::Binding {
        binding_digest: digest,
        name: process.process.clone(),
        cell: cfg.id.clone(),
        environment: cfg.environment,
        recipe: cfg.recipe.clone(),
        envelope: cfg.envelope.clone(),
        source_digest: process.source_digest,
        package_digest: process.package_digest,
        site_config_digest: cfg.site_config_digest,
        maximum_budget: cfg.maximum_budget,
        input_mode: if cfg.execution.is_some() {
            "PUBLISHED_SELECTION_V2"
        } else {
            "BOUND_CONFIGURATION"
        },
    }))
}

impl<R: Repository, C: Clock, A: QualificationAuthority> Engine<R, C, A> {
    pub fn runtime_skill_catalog(&mut self, identity: &Identity) -> Result<view::Catalog> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let principal = authorize_identity(tx, identity, meta, &now)?;
            let mut bindings = vec![];
            for row in tx.scan("cell/")? {
                let cell: Cell = decode(&row, CELL)?;
                if !principal.cells.contains(&cell.configuration.id) {
                    continue;
                }
                if let Some(bound) = binding(meta, &cell.configuration)? {
                    bindings.push(view::Installed {
                        binding: bound,
                        cell_revision: row.revision,
                        epoch: cell.epoch,
                        commissioning: cell.commissioning,
                        mode: cell.mode,
                        blocks: cell.blocks,
                    });
                }
            }
            let truncated = bindings.len() > 128;
            bindings.truncate(128);
            Ok(view::Catalog {
                schema: "rx.runtime-skill-catalog.v1",
                snapshot_id: id(),
                installation: meta.clone(),
                observed_at: now,
                bindings,
                truncated,
            })
        })
    }
    pub fn runtime_skill_result(
        &mut self,
        identity: &Identity,
        run_id: &Id,
    ) -> Result<view::ResultView> {
        let meta = &self.installation;
        let clock = &self.clock;
        self.repository.transact(|tx| {
            let now = clock.now();
            let (revision, run): (_, Run) = load(tx, "run", run_id, RUN)?;
            authorize_read(tx, identity, meta, &now, &run.cell)?;
            let (cell_revision, cell): (_, Cell) = load(tx, "cell", &run.cell, CELL)?;
            let original = run_configuration::read(tx, &run, &cell.configuration)?;
            let bound =
                binding(meta, &original)?.ok_or(StoreError::Rejected(Reject::InvalidInput))?;
            let current_binding_matches = binding(meta, &cell.configuration)?
                .is_some_and(|b| b.binding_digest == bound.binding_digest);
            let mut parts = vec![];
            for part_id in run.part_ids.iter().take(256) {
                let (r, part): (_, PartAttempt) = load(tx, "part", part_id, PART)?;
                if part.run != run.id {
                    return Err(StoreError::Integrity(
                        "skill part/run correlation differs".into(),
                    ));
                }
                parts.push(Versioned {
                    revision: r,
                    value: part,
                });
            }
            let mut work = vec![];
            let mut work_count = 0usize;
            for row in tx.scan("work/")? {
                let value: crate::Work = decode(&row, WORK)?;
                if value.run != run.id {
                    continue;
                }
                if value.cell != run.cell {
                    return Err(StoreError::Integrity("skill work/run cell differs".into()));
                }
                work_count += 1;
                if work.len() < 256 {
                    work.push(view::Work {
                        execution: value.execution,
                        operation: value.operation,
                        part: value.part,
                        slot: value.slot,
                        host: value.host,
                        activation: value.activation,
                        invocation: value.invocation,
                    });
                }
            }
            Ok(view::ResultView {
                schema: "rx.runtime-skill-result.v1",
                snapshot_id: id(),
                installation: meta.clone(),
                observed_at: now,
                binding: bound,
                details_truncated: run.part_ids.len() > 256 || work_count > 256,
                run: Versioned {
                    revision,
                    value: run,
                },
                parts,
                work,
                current_cell_revision: cell_revision,
                current_binding_matches,
                result_owner: "PLATFORM",
            })
        })
    }
}
