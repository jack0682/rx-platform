//! Authoritative application transactions, independent of wire codecs and SQLite.
// A panic in the single writer faults the whole Runtime; data-dependent failures
// must return errors instead. `expect` stays allowed for documented invariants.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
pub mod checkpoint_artifact;
pub mod control_journal;
pub mod engine;
pub mod model;
pub mod persistence;
pub mod procedure;
pub mod projection;
pub mod resident_component;
pub mod resident_reporting;
mod run_index;
pub use engine::Engine;
pub use model::*;
pub mod intervention;

pub mod closure;

pub mod lifecycle;

pub mod host_binding_baseline;
pub mod host_link;

pub mod observation;

pub mod diagnostics;

pub mod service_health;

pub mod definition_catalog;
pub mod process_draft;

pub mod draft_bindings;

pub mod device_binding;
pub mod device_catalog;
pub mod device_review;
pub mod package_intake;

pub mod process_review;

pub mod process_change;

pub mod configuration_dispatch;

pub mod requalification;

pub mod qualification_activation;

pub mod runtime_invalidation;

pub mod device_invalidation;

pub mod operator_start;

pub mod host_readmission;
pub mod host_recovery;
pub mod store_restore;

pub mod settlement;

pub mod runtime_skill;
pub mod software_skill;

pub mod host_binding_transition;

pub mod component_intake;

pub mod resident_execution;

mod artifact_storage;
pub mod workflow_model;
pub mod workflow_publication;
