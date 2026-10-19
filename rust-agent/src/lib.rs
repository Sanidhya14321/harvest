//! Experimental Rust application runtime. Legacy capability parity is not yet
//! met.
pub mod agent;
pub mod agent_session;
mod brain_cli;
pub mod chat;
mod cli;
pub mod config;
pub mod durable_session;
mod edit;
pub mod provider;
mod search;
mod session_context;
pub mod session_format;
mod shell;
mod streaming;
pub mod tools;

/// Document conversion and SVG previews shared with the native addon.
pub use harvest_media as media;
/// Offline tokenizers shared with the legacy host, with unchanged vocabularies.
pub use harvest_tokenizer as tokenizer;
/// Terminal protocol primitives usable without an embedded JavaScript runtime.
pub use harvest_tui as tui;

/// Execute the Rust command line without installation side effects.
///
/// # Errors
/// Returns parsing, filesystem, provider or output errors to the host.
pub fn execute_cli() -> anyhow::Result<u8> {
	cli::execute()
}
pub mod usage;
