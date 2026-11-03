//! Experimental Rust application runtime. Legacy capability parity is not yet
//! met.
pub mod agent;
pub mod agent_session;
pub mod anthropic;
mod ast;
pub mod auth_storage;
mod brain_cli;
pub mod catalog;
pub mod chat;
mod cli;
mod commands;
pub mod config;
pub mod configuration;
pub mod conflict;
pub mod core_specs;
pub mod core_tools;
pub mod durable_session;
pub mod edit;
pub mod edit_benchmark;
mod glob;
mod interactive;
pub mod language_services;
pub mod mcp;
pub mod output;
pub mod plugins;
mod process_transport;
pub mod provider;
mod read;
pub mod replay_cli;
pub mod rpc;
mod search;
mod session_context;
pub mod session_engine;
pub mod session_format;
pub mod settings;
mod sqlite;
mod sse;
mod transport;
pub use harvest_protocol as protocol;
pub use harvest_terminal as terminal;
mod agent_tools;
mod streaming;
mod task_tool;
pub mod todo;
mod tool_cli;
pub mod tools;
mod tree;
mod web_search;

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
