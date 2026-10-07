//! Runtime scheduling primitives. Device I/O belongs to outbox consumers, not the state writer.
// A panic in the single writer faults the whole Runtime; data-dependent failures
// must return errors instead. `expect` stays allowed for documented invariants.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
pub mod application;
pub mod writer;

mod service_health;

pub mod package_intake;

pub mod requalification;

pub mod host_recovery;

pub mod software_skill;

pub mod component_intake;
