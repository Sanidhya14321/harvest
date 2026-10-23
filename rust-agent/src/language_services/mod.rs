//! Native language/debug service conversion. One process owner and tool
//! registry; behavioral, installation and platform parity verification remains
//! deferred.
pub(crate) mod cli;
pub mod configuration;
pub mod dap;
pub mod edits;
pub mod lsp;
mod runtime;
pub use configuration::{DapConfiguration, LspConfiguration, ProcessConfiguration};
pub use runtime::Runtime;
