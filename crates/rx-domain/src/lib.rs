//! Transport-, storage-, and device-independent RX semantics.
pub mod budget;
pub mod canonical;
pub mod condition;
pub mod epoch;
pub mod fault;
pub mod intent;
pub mod operation;
pub mod types;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("invalid transition: {0}")]
    InvalidTransition(String),
    #[error("conflict: {0}")]
    Conflict(String),
}

pub type Result<T> = std::result::Result<T, DomainError>;

pub mod host_snapshot;

pub mod host_configuration;

pub mod host_qualification;
