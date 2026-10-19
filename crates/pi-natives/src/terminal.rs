//! Numeric validation shared by terminal protocol bridges.
use std::time::Duration;

use napi::{Error, Result};

pub(crate) fn byte_count(value: f64) -> Result<usize> {
	let maximum = (usize::MAX as f64).min(9_007_199_254_740_991.0);
	if !value.is_finite() || !(0.0..=maximum).contains(&value) {
		return Err(Error::from_reason(
			"terminal byte count must be finite, nonnegative and representable",
		));
	}
	Ok(value as usize)
}

pub(crate) fn milliseconds(value: f64, maximum: f64) -> Result<Duration> {
	if !value.is_finite() || !(0.0..=maximum).contains(&value) {
		return Err(Error::from_reason("terminal timeout must be within the finite timer range"));
	}
	Duration::try_from_secs_f64(value / 1000.0)
		.map_err(|_| Error::from_reason("terminal timeout is not representable"))
}
