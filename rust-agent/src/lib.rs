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
pub mod replay_cli;
pub mod rpc;
pub mod session_engine;
mod transport;
pub use harvest_protocol as protocol;
pub use harvest_terminal as terminal;
mod search;
mod session_context;
pub mod session_format;
mod shell;
mod streaming;
pub mod tools;

/// Document conversion, SVG previews and bounded SIXEL shared with the addon.
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
