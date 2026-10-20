//! UTF-16 output transforms preserve even unpaired JavaScript surrogates.
use std::{ops::Range, sync::OnceLock};

use regex::Regex;

const ESC: u16 = 0x1b;
const PARAM_CAP: usize = 16;

fn sgr_end(line: &[u16], at: usize) -> Option<usize> {
	if line.get(at..at + 2)? != [ESC, 91] {
		return None;
	}
	let mut end = at + 2;
	while line
		.get(end)
		.is_some_and(|unit| (48..=57).contains(unit) || *unit == 59 || *unit == 58)
	{
		end += 1;
	}
	(line.get(end) == Some(&109)).then_some(end)
}

/// Whether prepending ASCII spaces preserves additive cell widths.
///
/// Combining marks, joiners and Thai/Lao AM can attach to the new prefix.
/// Unknown control prefixes conservatively require measuring the combined row.
pub fn can_prefix_width(line: &[u16]) -> bool {
	let mut at = 0;
	while line.get(at) == Some(&ESC) {
		let Some(end) = sgr_end(line, at) else {
			return false;
		};
		at = end + 1;
	}
	let Some(first) = char::decode_utf16(line[at..].iter().copied()).next() else {
		return true;
	};
	let Ok(first) = first else {
		return false;
	};
	if matches!(first, '\u{200d}' | '\u{0e33}' | '\u{0eb3}') {
		return false;
	}
	static MARK: OnceLock<Regex> = OnceLock::new();
	let mark = MARK.get_or_init(|| Regex::new(r"^\p{M}$").expect("static Unicode mark expression"));
	let mut bytes = [0_u8; 4];
	!mark.is_match(first.encode_utf8(&mut bytes))
}

fn incomplete_color(params: &[u16]) -> bool {
	let tokens: Vec<&[u16]> = params.split(|unit| *unit == 59).collect();
	let mut at = 0;
	while at < tokens.len() {
		if matches!(tokens[at], [51..=53, 56]) {
			match tokens.get(at + 1).copied() {
				None => return true,
				Some([50]) => {
					if at + 4 >= tokens.len() {
						return true;
					}
					at += 5;
					continue;
				},
				Some([53]) => {
					if at + 2 >= tokens.len() {
						return true;
					}
					at += 3;
					continue;
				},
				_ => {},
			}
		}
		at += 1;
	}
	false
}

fn emit_group(output: &mut Vec<u16>, group: &[u16]) {
	output.extend_from_slice(&[ESC, 91]);
	output.extend_from_slice(group);
	output.push(109);
}

/// Return no allocation when no adjacent SGR run needs rewriting. Only the
/// exact CSI parameter grammar is merged; other payloads remain verbatim.
#[must_use]
pub fn coalesce_sgr(line: &[u16]) -> Option<Vec<u16>> {
	let mut output = Vec::new();
	let mut copied = 0;
	let mut at = 0;
	while at < line.len() {
		let Some(end) = sgr_end(line, at) else {
			at += 1;
			continue;
		};
		let mut params = vec![&line[at + 2..end]];
		let mut next = end + 1;
		while let Some(end) = sgr_end(line, next) {
			params.push(&line[next + 2..end]);
			next = end + 1;
		}
		if params.len() > 1 {
			output.extend_from_slice(&line[copied..at]);
			let mut group = Vec::new();
			let mut count = 0;
			let mut safe = true;
			for params in params {
				let normalized = if params.is_empty() { &[48][..] } else { params };
				let tokens = 1
					+ normalized
						.iter()
						.filter(|unit| **unit == 59 || **unit == 58)
						.count();
				if count > 0 && (!safe || count + tokens > PARAM_CAP) {
					emit_group(&mut output, &group);
					group.clear();
					count = 0;
				}
				if !group.is_empty() {
					group.push(59);
				}
				group.extend_from_slice(normalized);
				count += tokens;
				safe = !incomplete_color(normalized);
			}
			emit_group(&mut output, &group);
			copied = next;
		}
		at = next;
	}
	if copied == 0 {
		return None;
	}
	output.extend_from_slice(&line[copied..]);
	Some(output)
}

/// UTF-16 boundaries retain unmatched code units in the caller's original text.
///
/// Width is measured after the terminal's replacement encoding, using the
/// current shared output engine rather than a second chunking implementation.
#[must_use]
pub fn conpty_chunks(data: &[u16], max_bytes: f64) -> Vec<Range<usize>> {
	conpty_utf8_chunks(&String::from_utf16_lossy(data), max_bytes)
}

/// UTF-16 offsets for borrowed UTF-8 chunks from the current shared engine.
#[must_use]
#[allow(
	clippy::cast_possible_truncation,
	clippy::cast_sign_loss,
	reason = "positive caps saturate and fractions round down"
)]
pub fn conpty_utf8_chunks(text: &str, max_bytes: f64) -> Vec<Range<usize>> {
	if text.is_empty() || max_bytes.is_nan() || max_bytes == f64::INFINITY {
		return std::iter::once(0..text.encode_utf16().count()).collect();
	}
	let cap = std::num::NonZeroUsize::new((max_bytes as usize).max(1)).expect("positive cap");
	let mut offset = 0;
	harvest_tui::output::ConptyChunks::new(text, cap)
		.map(|chunk| {
			let start = offset;
			offset += chunk.encode_utf16().count();
			start..offset
		})
		.collect()
}
