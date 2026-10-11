//! Session-local extraction scheduling. New input consumes/cancels the attempt.

use std::time::{Duration, Instant};

pub const MAX_EXTRACTION_INPUT_TOKENS: usize = 3000;
pub const MAX_EXTRACTION_OUTPUT_TOKENS: usize = 1000;
pub const MAX_EXTRACTION_CANDIDATES: usize = 3;

#[derive(Debug, Default)]
pub struct CaptureSchedule {
	eligible_since: Option<Instant>,
	attempted:      bool,
	generation:     u64,
}

impl CaptureSchedule {
	pub const fn task_completed(
		&mut self,
		now: Instant,
		substantive: bool,
		aborted: bool,
		evaluation_child: bool,
	) {
		self.eligible_since = if substantive && !aborted && !evaluation_child {
			Some(now)
		} else {
			None
		};
	}

	pub const fn input_received(&mut self) {
		self.generation = self.generation.wrapping_add(1);
		self.eligible_since = None;
	}

	pub fn begin_if_idle(&mut self, now: Instant) -> Option<u64> {
		if self.attempted
			|| self
				.eligible_since
				.is_none_or(|since| now.saturating_duration_since(since) < Duration::from_secs(30))
		{
			return None;
		}
		self.attempted = true;
		Some(self.generation)
	}

	pub const fn still_current(&self, generation: u64) -> bool {
		self.generation == generation
	}
}
