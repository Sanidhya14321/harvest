//! Markdown is canonical; indexes and graph edges are derived session caches.

pub mod admission;
pub mod capture;
pub mod evidence;
pub mod identity;
pub mod model;
pub mod retrieval;
pub mod store;

pub use admission::*;
pub use capture::*;
pub use evidence::*;
pub use identity::*;
pub use model::*;
pub use retrieval::*;
pub use store::*;

/// Stable document/receipt paths use forward slashes on every host.
/// On Unix a literal backslash remains part of the filename.
pub(crate) fn document_path(path: &std::path::Path) -> String {
	path
		.to_string_lossy()
		.replace(std::path::MAIN_SEPARATOR, "/")
}
