//! Portable session records and deterministic display projections.
//!
//! Original/provider replay data stays in [`session::SessionDocument`]. The
//! separate [`display`] projection contains only explicitly selected display
//! fields; it must never be written back as provider history. Rendering hosts
//! remain responsible for terminal sanitization and safe DOM output.
pub mod digest;
pub mod display;
pub mod session;
