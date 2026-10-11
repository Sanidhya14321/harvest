//! Experimental Rust application runtime. Legacy capability parity is not yet
//! met.
pub mod agent;
mod brain_cli;
pub mod chat;
mod cli;
pub mod config;
pub mod provider;
pub mod session;
mod shell;
pub mod tools;

/// Execute the Rust command line without installation side effects.
///
/// # Errors
/// Returns parsing, filesystem, provider or output errors to the host.
pub fn execute_cli() -> anyhow::Result<u8> {
	cli::execute()
}
