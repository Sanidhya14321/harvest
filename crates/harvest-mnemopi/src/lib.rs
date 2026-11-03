//! Native port of the `@harvest/mnemopi` memory engine.
//!
//! The crate opens the same SQLite database (`$MNEMOPI_DATA_DIR/mnemopi.db`,
//! default `~/.hermes/mnemopi/data/mnemopi.db`) with the same schema the
//! TypeScript package creates, so both runtimes share one memory store.
//!
//! LLM and embedding access is host-provided through the [`Llm`] and
//! [`Embedder`] traits; this crate performs no network I/O itself.

pub mod aaak;
pub mod api;
pub mod banks;
pub mod beam;
pub mod config;
pub mod db;
pub mod diagnose;
pub mod error;
pub mod extraction;
pub mod graph;
pub mod limits;
pub mod providers;
pub mod schema;
pub mod triples;
pub mod util;

pub use api::*;
pub use beam::BeamMemory;
pub use error::{MnemopiError, Result};
pub use providers::{Embedder, Llm};
