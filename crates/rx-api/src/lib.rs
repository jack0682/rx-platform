//! Versioned browser BFF. Business decisions remain in rx-application's dedicated writer.
pub mod auth;
pub mod error;
pub mod grpc;
mod routes;
pub use routes::{LocalPolicy, router, router_with_package_intake};

pub mod terminal_https;
