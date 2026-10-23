//! Native MCP stdio/streamable-HTTP client sharing bounded owners with plugins
//! and providers. Conversion checkpoint: behavioral parity remains unverified.
pub(crate) mod cli;
mod client;
pub mod configuration;
mod transport;
pub use client::{Client, RpcError, ServerInfo, ToolRegistration};
pub use configuration::{
	Configuration, RequestIdFormat, Transport as TransportConfiguration, load_file,
};
pub const PROTOCOL_VERSION: &str = "2025-11-25";
