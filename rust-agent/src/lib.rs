//! Experimental Rust application runtime. Legacy capability parity is not yet
//! met.
pub mod agent;
pub mod agent_session;
mod anthropic;
mod brain_cli;
pub mod chat;
mod cli;
pub mod config;
pub mod configuration;
pub mod durable_session;
pub mod edit;
pub mod edit_benchmark;
pub mod language_services;
pub mod mcp;
pub mod plugins;
mod process_transport;
pub mod provider;
pub mod replay_cli;
pub mod rpc;
pub mod session_engine;
mod sse;
mod transport;
pub use harvest_protocol as protocol;
pub use harvest_terminal as terminal;
mod search;
mod session_context;
pub mod session_format;
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

/// Dedicated-profile native browser automation, shared by CLI and SDK
/// consumers.
pub use harvest_browser as browser;

mod browser_cli;

/// Ordinary document conversion API, without JavaScript or native bindings.
pub use harvest_media::docx;

pub mod cleanup_scan;
mod source_text;

pub mod trace_report;
