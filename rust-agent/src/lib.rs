//! Experimental Rust application runtime. Legacy capability parity is not yet
//! met.
pub mod agent;
pub mod agent_session;
mod brain_cli;
pub mod chat;
mod cli;
pub mod config;
pub mod durable_session;
pub mod provider;
pub mod replay_cli;
pub mod rpc;
pub mod session_engine;
pub use harvest_protocol::session as session_format;
mod shell;
mod streaming;
pub mod tools;
mod transport;

/// Execute the Rust command line without installation side effects.
///
/// # Errors
/// Returns parsing, filesystem, provider or output errors to the host.
pub fn execute_cli() -> anyhow::Result<u8> {
	cli::execute()
}
