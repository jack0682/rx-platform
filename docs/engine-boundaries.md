# Engine boundaries

The [machine-readable map](engine-boundaries.json) assigns every module declared from [engine/mod.rs](../crates/rx-application/src/engine/mod.rs) to one of three groups and records the references that currently cross a group boundary in the wrong direction. The engine is one `impl Engine` split over about sixty files that all start with `use super::*`, so nothing in the Rust module system separates the generic runtime from the cell domain. This map and its checker are that separation until the code carries it.

No source file moves and no behavior changes with this map. It names where the seams are.

## The three groups

| Group | Contents | May reference |
|---|---|---|
| `core` | Generic runtime: admission, identity and access, lifecycle, evidence and delivery, dispatch, reconciliation and settlement, invalidation, observation, host link, recovery and re-admission, executor and operator peers, requests, queries, diagnostics, pause, runtime and device restriction reads, store restore, and `mod.rs` itself | `core` |
| `cell` | Cell and production domain: workflow, production, handover, operator start, assignment, run configuration, closure, intervention, procedure, process execution and its transitions, checkpoint change, execution read, executor requests, runtime skills, cell configuration | `cell`, `core` |
| `change_control` | Change control and qualification: package intake, process drafts and draft bindings, device binding plans, device and process review, process change and apply, host configuration dispatch, host binding intents, requalification, qualification activation | `change_control`, `core` |

A submodule file (`host_recovery/reads.rs`, `qualification_activation/issuance.rs`, ...) belongs to its top-level module's group. A new module declared anywhere under `engine/` must be added to exactly one group; the checker fails until it is.

Four modules sit in a different group than their names suggest, because their references decide it: `device_binding` (device binding plans and their review; only reads and is read by change-control modules), `configuration_dispatch` (host configuration tasks for a process change; called from apply, change and activation), `host_binding_transition` (host binding intents issued for a process change; eleven references each way with `process_change`) are change control, and `process_transition` (checkpoint successors from the process contract; used only by `process` and `checkpoint_change`) is cell domain.

## The rule

A file may reference engine modules only from the groups its own group lists under `may_reference`. `core` references nothing outside itself; `cell` and `change_control` may reference `core` and themselves. Every other reference is a **seam**: it must appear in `allowed_violations` with its file, the referenced module, and the exact current count.

The seam list is a record of debt, not a permission. Entries and counts may only decrease. Adding a new entry, raising a count, or moving a module to a group where its references stop counting all require the same review as the refactor that would remove the seam. Removing a seam from the code requires removing its entry, because a stale entry fails the check as well.

## What the checker establishes

```sh
python3 tools/check_engine_boundaries.py          # report and exit status
python3 tools/check_engine_boundaries.py --graph  # also print every module's references
```

The required repository CI job runs the first form. It reads the module tree from the `mod x;` declarations, masks comments and literals, drops every item behind `#[cfg(test)]`, and resolves three path forms to an engine module: `super::x` (and `super::super::x` from a submodule), `crate::engine::x`, and a bare `x::` where `x` is an engine module reachable through the `use super::*` chain. `crate::x` never counts, because the application crate has top-level modules with the same names as engine modules (`crate::process_change` holds the types, `engine::process_change` the transactions). A reference to a submodule counts for its top-level module; a module's references to itself do not count.

**The checker does not type-check.** It cannot see a reference that arrives through a re-export, a type alias, a trait method, or a macro, and it cannot tell whether an allowed reference is a good design. It counts textual paths. A green check means the counted seams did not grow; it does not mean the engine is layered.

## Current seams

Eighteen file-to-module pairs, 31 references, at the revision that introduced this map. The direction column says which rule each one breaks.

| Direction | File | Referenced module | Count | What crosses |
|---|---|---|---|---|
| core → cell | `admission.rs` | `run_configuration` | 1 | run admission requires a current bound configuration |
| core → cell | `dispatch.rs` | `process` | 1 | node eligibility when the cell carries a process |
| core → cell | `settlement.rs` | `handover` | 2 | release confirmed work, complete part transitions |
| core → cell | `settlement.rs` | `run_configuration` | 1 | compare bound and cell configuration |
| core → change_control | `admission.rs` | `qualification_activation` | 2 | readiness and purpose checks |
| core → change_control | `delivery.rs` | `qualification_activation` | 1 | armed clear blocks |
| core → change_control | `executor_peer.rs` | `qualification_activation` | 1 | suspend on executor replacement |
| core → change_control | `host_readmission.rs` | `host_binding_transition` | 1 | read the named binding intent |
| core → change_control | `host_readmission.rs` | `process_change` | 3 | expected configuration from a staged change |
| core → change_control | `host_recovery/context.rs` | `process_change` | 2 | prospective impact on the host |
| core → change_control | `host_recovery/reads.rs` | `configuration_dispatch` | 1 | continuity proof reads the task |
| core → change_control | `host_recovery/reads.rs` | `process_change` | 1 | state of the applied change |
| core → change_control | `mod.rs` | `qualification_activation` | 3 | restart handling in `Engine::open` |
| cell → change_control | `assignment.rs` | `process_change` | 2 | `config_ref` comparison |
| cell → change_control | `operator_start.rs` | `qualification_activation` | 2 | purpose check, armed clear blocks |
| cell → change_control | `run_configuration.rs` | `process_change` | 5 | `read_config`, `store_config`, `config_ref` |
| cell → change_control | `runtime_skill.rs` | `process_change` | 1 | `config_ref` |
| change_control → cell | `process_apply.rs` | `run_configuration` | 1 | bind the applied configuration to affected runs |

The four `core → cell` rows are the entry points for a cell-domain abstraction: admission, dispatch and settlement each call into the cell domain at one or two places. The `cell → change_control` rows all go through `process_change`'s configuration reference helpers, which is one seam with four callers rather than four seams. The `core → change_control` rows are dominated by `qualification_activation` and `process_change`; `host_readmission` and `host_recovery` need the staged change of the host they re-admit.

## Maintenance

When a reference in the table is removed, delete its entry or lower its count in the same change. When a module is added, assign it. When a module changes purpose, move it and re-run the checker; the map must describe the code as it is, and the `may_reference` lists are the only policy in it. `configuration` (cell installation, `qualify`, `register_host`) has no cross-group references, so the graph does not decide its group; it stays in `cell` from the original draft and can move without changing any seam.
