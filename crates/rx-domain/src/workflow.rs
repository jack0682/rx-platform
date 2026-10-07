//! Pure, bounded authoring resolution. No device access or execution authority.
mod math;
mod model;
mod resolve;
pub use model::*;
pub use resolve::resolve;
