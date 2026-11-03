//! Fixed host safety ceilings, independent of environment-tunable scoring
//! policy.

use crate::{MnemopiError, Result};

pub const MAX_CONTENT_BYTES: usize = 1024 * 1024;
pub const MAX_QUERY_BYTES: usize = 64 * 1024;
pub const MAX_METADATA_BYTES: usize = 64 * 1024;
pub const MAX_BATCH_ITEMS: usize = 5000;
pub const MAX_ROWS: usize = 10_000;
pub const MAX_RESULT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_RECALL_RESULTS: usize = 100;
pub const MAX_VECTOR_DIMENSIONS: usize = 16_384;

pub(crate) fn check_text(value: &str, limit: usize, name: &str) -> Result<()> {
	if value.len() > limit {
		return Err(MnemopiError::Invalid(format!("{name} exceeds {limit} bytes")));
	}
	Ok(())
}

pub(crate) fn importance(value: Option<f64>) -> Result<()> {
	if value.is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v)) {
		return Err(MnemopiError::Invalid("importance must be finite and between 0 and 1".into()));
	}
	Ok(())
}

pub(crate) fn serialized_size(value: &impl serde::Serialize, limit: usize) -> Result<usize> {
	let mut output = harvest_runtime::BoundedWriter::new(limit);
	serde_json::to_writer(&mut output, value)?;
	Ok(output.into_inner().len())
}
