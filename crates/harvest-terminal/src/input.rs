//! Bounded, resumable terminal escape scanning.
//!
//! Offsets and payloads use UTF-16 units at the transitional host boundary so
//! even incomplete or malformed terminal input is preserved without
//! replacement. Timing, bracketed-paste dispatch and terminal ownership belong
//! to the caller.

use std::ops::Range;

const ESC: u16 = 0x1b;
pub const MAX_CSI_UNITS: usize = 4096;
pub const MAX_STRING_UNITS: usize = 16 * 1024 * 1024;

/// Zero-copy event ranges and state required to resume a fragmented escape.
/// Offsets refer to the input slice and preserve its original UTF-16 payload.
#[derive(Debug, PartialEq, Eq)]
pub struct Scan {
	pub sequences:          Vec<Range<usize>>,
	pub remainder_start:    usize,
	pub resume_search_from: usize,
	/// An over-limit string must be discarded through its terminator, not typed.
	pub discard_from:       Option<usize>,
}

enum End {
	Complete(usize),
	Incomplete,
	Capped,
}

fn sgr_mouse(body: &[u16]) -> bool {
	if body.first() != Some(&0x3c) || !matches!(body.last(), Some(0x4d | 0x6d)) {
		return false;
	}
	let mut fields = 0;
	let mut digits = 0;
	for &unit in &body[1..body.len() - 1] {
		match unit {
			0x30..=0x39 => digits += 1,
			0x3b if digits > 0 && fields < 2 => {
				fields += 1;
				digits = 0;
			},
			_ => return false,
		}
	}
	fields == 2 && digits > 0
}

fn escape_end(input: &[u16], pos: usize, resume: usize) -> End {
	let Some(&next) = input.get(pos + 1) else {
		return End::Incomplete;
	};
	match next {
		ESC => End::Incomplete,
		0x5b => {
			if input.get(pos + 2) == Some(&0x4d) {
				return if pos + 6 <= input.len() {
					End::Complete(pos + 6)
				} else {
					End::Incomplete
				};
			}
			let limit = input.len().min(pos + MAX_CSI_UNITS);
			let mouse = input.get(pos + 2) == Some(&0x3c);
			for index in pos + 2..limit {
				if (0x40..=0x7e).contains(&input[index])
					&& (!mouse || sgr_mouse(&input[pos + 2..=index]))
				{
					return End::Complete(index + 1);
				}
			}
			if input.len() - pos >= MAX_CSI_UNITS {
				End::Capped
			} else {
				End::Incomplete
			}
		},
		0x5d | 0x50 | 0x5f => {
			let limit = input.len().min(pos + MAX_STRING_UNITS);
			let start = (pos + 2).max(resume.saturating_sub(1));
			for index in start..limit {
				if next == 0x5d && input[index] == 7 {
					return End::Complete(index + 1);
				}
				if input[index] == ESC && index + 1 < limit && input[index + 1] == 0x5c {
					return End::Complete(index + 2);
				}
			}
			if input.len() - pos >= MAX_STRING_UNITS {
				End::Capped
			} else {
				End::Incomplete
			}
		},
		0x4f => {
			if pos + 3 <= input.len() {
				End::Complete(pos + 3)
			} else {
				End::Incomplete
			}
		},
		_ => End::Complete(pos + 2),
	}
}

/// Split completed scalar/key/protocol events without rescanning string
/// payloads.
///
/// The host must retain the input from `remainder_start` and pass
/// `resume_search_from` on
/// the next call. A `discard_from` result requires dropping the torn string and
/// its tail.
#[must_use]
pub fn scan(input: &[u16], resume_search_from: usize) -> Scan {
	let mut result = Scan {
		sequences:          Vec::new(),
		remainder_start:    input.len(),
		resume_search_from: 0,
		discard_from:       None,
	};
	let mut pos = 0;
	while pos < input.len() {
		if input[pos] != ESC {
			let pair = (0xd800..=0xdbff).contains(&input[pos])
				&& input
					.get(pos + 1)
					.is_some_and(|unit| (0xdc00..=0xdfff).contains(unit));
			let end = pos + if pair { 2 } else { 1 };
			result.sequences.push(pos..end);
			pos = end;
			continue;
		}
		if input.get(pos + 1) == Some(&ESC) {
			let Some(&third) = input.get(pos + 2) else {
				result.remainder_start = pos;
				return result;
			};
			if !matches!(third, 0x5b | 0x4f) {
				result.sequences.push(pos..pos + 1);
				pos += 1;
				continue;
			}
			match escape_end(input, pos + 1, 0) {
				End::Incomplete => {
					result.remainder_start = pos;
					return result;
				},
				End::Capped => {
					let end = input.len().min(pos + MAX_CSI_UNITS);
					result.sequences.push(pos..end);
					pos = end;
				},
				End::Complete(end) => {
					if third == 0x5b && input.get(pos + 3) == Some(&0x3c) {
						result.sequences.push(pos..pos + 1);
						result.sequences.push(pos + 1..end);
					} else {
						result.sequences.push(pos..end);
					}
					pos = end;
				},
			}
			continue;
		}
		match escape_end(input, pos, if pos == 0 { resume_search_from } else { 0 }) {
			End::Complete(end) => {
				result.sequences.push(pos..end);
				pos = end;
			},
			End::Incomplete => {
				result.remainder_start = pos;
				if pos == 0 && matches!(input.get(pos + 1), Some(0x5d | 0x50 | 0x5f)) {
					result.resume_search_from = input.len();
				}
				return result;
			},
			End::Capped => {
				if matches!(input.get(pos + 1), Some(0x5d | 0x50 | 0x5f)) {
					result.discard_from = Some(pos);
					return result;
				}
				let end = input.len().min(pos + MAX_CSI_UNITS);
				result.sequences.push(pos..end);
				pos = end;
			},
		}
	}
	result
}
