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
pub mod session_format;
mod shell;
mod streaming;
pub mod tools;

/// Execute the Rust command line without installation side effects.
///
/// # Errors
/// Returns parsing, filesystem, provider or output errors to the host.
pub fn execute_cli() -> anyhow::Result<u8> {
	cli::execute()
}
