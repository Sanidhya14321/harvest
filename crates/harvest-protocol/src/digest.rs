//! Stable portable fingerprints shared by persistence and legacy imports.
use sha2::{Digest, Sha256};

/// Stable content fingerprints; never use process-dependent runtime hashes.
pub fn content_hash(bytes: &[u8]) -> String {
	format!("{:x}", Sha256::digest(bytes))
}

/// Incremental fingerprints avoid retaining a second copy of session history.
#[derive(Default)]
pub struct ContentDigest(Sha256);
impl ContentDigest {
	pub fn update(&mut self, bytes: &[u8]) {
		self.0.update(bytes);
	}

	pub fn hash(&self) -> String {
		format!("{:x}", self.0.clone().finalize())
	}
}
