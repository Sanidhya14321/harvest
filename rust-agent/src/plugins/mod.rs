//! Executable plugins share the native host tools and approval owner.
//! Conversion checkpoint: behavioral/installed verification is pending.
mod host;
pub mod protocol;
pub mod sdk;
pub use host::{Configuration, Plugin};
