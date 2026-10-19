//! Terminal output boundaries and backlog progress policy.
use std::{num::NonZeroUsize, time::Duration};

pub const CONPTY_WRITE_BYTES: usize = 16 * 1024;

/// Borrowed UTF-8 writes, preferring complete lines at each byte boundary.
/// A scalar wider than the configured limit is kept intact in its own chunk.
pub struct ConptyChunks<'a> {
	remaining: &'a str,
	limit:     usize,
}
impl<'a> ConptyChunks<'a> {
	pub const fn new(text: &'a str, limit: NonZeroUsize) -> Self {
		Self { remaining: text, limit: limit.get() }
	}
}
impl<'a> Iterator for ConptyChunks<'a> {
	type Item = &'a str;

	fn next(&mut self) -> Option<Self::Item> {
		if self.remaining.is_empty() {
			return None;
		}
		let mut end = self.remaining.len().min(self.limit);
		if end < self.remaining.len() {
			while end > 0 && !self.remaining.is_char_boundary(end) {
				end -= 1;
			}
			if end == 0 {
				end = self.remaining.chars().next()?.len_utf8();
			}
			if let Some(newline) = self.remaining.as_bytes()[..end]
				.iter()
				.rposition(|&byte| byte == b'\n')
			{
				end = newline + 1;
			}
		}
		let (chunk, rest) = self.remaining.split_at(end);
		self.remaining = rest;
		Some(chunk)
	}
}

/// A draining oversized frame stays alive; a stalled backlog remains watched
/// even after falling below the original arm threshold.
#[derive(Debug)]
pub struct StallWatchdog {
	arm_bytes:   usize,
	clear_bytes: usize,
	stall:       Duration,
	episode:     Option<(usize, Duration)>,
}
impl Default for StallWatchdog {
	fn default() -> Self {
		Self::new(64 * 1024 * 1024, 256 * 1024, Duration::from_secs(2))
	}
}
impl StallWatchdog {
	pub const fn new(arm_bytes: usize, clear_bytes: usize, stall: Duration) -> Self {
		Self { arm_bytes, clear_bytes, stall, episode: None }
	}

	pub const fn armed(&self) -> bool {
		self.episode.is_some()
	}

	pub const fn reset(&mut self) {
		self.episode = None;
	}

	/// Return true when the host must disconnect a never-draining consumer.
	pub fn sample(&mut self, pending: usize, now: Duration) -> bool {
		let Some((low_water, since)) = self.episode else {
			if pending > self.arm_bytes {
				self.episode = Some((pending, now));
			}
			return false;
		};
		if pending <= self.clear_bytes {
			self.reset();
			return false;
		}
		if pending < low_water {
			self.episode = Some((pending, now));
			return false;
		}
		now.saturating_sub(since) >= self.stall
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn chunks(text: &str, limit: usize) -> Vec<&str> {
		ConptyChunks::new(text, NonZeroUsize::new(limit).unwrap()).collect()
	}
	fn ms(value: u64) -> Duration {
		Duration::from_millis(value)
	}
	#[test]
	fn writes_prefer_lines_without_cutting_renderer_escape_sequences() {
		let text = "\x1b[31mred\x1b[0m\r\n\x1b[32mgreen\x1b[0m\r\nlast";
		assert_eq!(chunks(text, 24), ["\x1b[31mred\x1b[0m\r\n", "\x1b[32mgreen\x1b[0m\r\nlast"]);
	}
	#[test]
	fn byte_caps_preserve_non_ascii_scalars_and_hard_cut_long_lines() {
		let text = "字🙂\n界🙂尾";
		let writes = chunks(text, 7);
		assert_eq!(writes, ["字🙂", "\n", "界🙂", "尾"]);
		assert_eq!(writes.concat(), text);
		assert!(writes.iter().all(|chunk| chunk.len() <= 7));
		assert_eq!(chunks("🙂字", 1), ["🙂", "字"]);
		assert!(chunks("", 1).is_empty());
	}
	#[test]
	fn an_oversized_draining_frame_does_not_disconnect() {
		let mut watchdog = StallWatchdog::new(1000, 100, ms(2000));
		for (i, pending) in (101..=5000).rev().step_by(100).enumerate() {
			assert!(!watchdog.sample(pending, ms(i as u64 * 300)));
		}
		assert!(watchdog.armed());
		assert!(!watchdog.sample(100, ms(20000)));
		assert!(!watchdog.armed());
	}
	#[test]
	fn a_wedge_below_the_arm_threshold_still_disconnects_from_last_progress() {
		let mut watchdog = StallWatchdog::new(1000, 100, ms(2000));
		assert!(!watchdog.sample(2000, ms(0)));
		assert!(!watchdog.sample(900, ms(1500)));
		assert!(!watchdog.sample(1100, ms(3499)));
		assert!(watchdog.sample(1000, ms(3500)));
	}
	#[test]
	fn healthy_drain_and_explicit_teardown_start_a_fresh_stall_window() {
		let mut watchdog = StallWatchdog::new(1000, 100, ms(2000));
		assert!(!watchdog.sample(1000, ms(0)));
		assert!(!watchdog.armed());
		assert!(!watchdog.sample(1001, ms(1)));
		assert!(!watchdog.sample(100, ms(2)));
		assert!(!watchdog.sample(1001, ms(10000)));
		assert!(!watchdog.sample(1001, ms(11999)));
		assert!(watchdog.sample(1001, ms(12000)));
		watchdog.reset();
		assert!(!watchdog.sample(1001, ms(20000)));
		assert!(!watchdog.sample(1001, ms(21999)));
	}
}
