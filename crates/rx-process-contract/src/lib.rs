//! Shared declarative process semantics. No BT engine, transport, filesystem or execution authority.
pub mod execution;
pub mod execution_validation;
pub mod frontier;
pub mod model;
pub mod production;
pub mod validation;
pub use model::*;
pub mod checkpoint_change;

pub mod source_validation;

pub mod compile_input;

pub mod package_review;

pub mod device_catalog;
pub mod device_review;
pub mod host_binding_plan;
pub mod native_outcome;
pub mod source_link;

pub mod assignment;
