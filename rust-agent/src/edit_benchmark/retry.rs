//! Benchmark recovery uses typed transport errors, never provider text or model
//! prose. Retrying a run after a mutation needs a separate replay contract.
use std::time::{Duration, Instant};

use anyhow::Result;
use harvest_runtime::cancel::CancelToken;
use serde::Serialize;

use crate::provider::ProviderRequestError;

#[derive(Debug, Serialize)]
pub(super) struct Failure {
	pub kind:   &'static str,
	pub status: Option<u16>,
}
impl Failure {
	pub fn classify(error: &anyhow::Error) -> Option<Self> {
		match error
			.chain()
			.find_map(|cause| cause.downcast_ref::<ProviderRequestError>())?
		{
			ProviderRequestError::Status(status @ (401 | 403)) => {
				Some(Self { kind: "auth", status: Some(*status) })
			},
			ProviderRequestError::Status(status @ (408 | 425 | 429 | 500..=599)) => {
				Some(Self { kind: "http", status: Some(*status) })
			},
			ProviderRequestError::Transport(ureq::ErrorKind::Io) => {
				Some(Self { kind: "transport", status: None })
			},
			_ => None,
		}
	}
}
pub(super) fn delay(retry: u32) -> Duration {
	Duration::from_millis((1000_u64 << retry.saturating_sub(1).min(4)).min(10_000))
}
pub(super) fn wait(duration: Duration, cancellation: &CancelToken) -> Result<()> {
	let deadline = Instant::now() + duration;
	loop {
		cancellation.heartbeat()?;
		let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
			return Ok(());
		};
		std::thread::sleep(remaining.min(Duration::from_millis(20)));
	}
}
