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

fn utf8_unit_width(data: &[u16], at: usize) -> (usize, usize) {
	match data[at] {
		0..=0x7f => (1, 1),
		0x80..=0x7ff => (1, 2),
		0xd800..=0xdbff
			if data
				.get(at + 1)
				.is_some_and(|unit| (0xdc00..=0xdfff).contains(unit)) =>
		{
			(2, 4)
		},
		_ => (1, 3),
	}
}

/// Newline-preferred Windows writes, measured in encoded bytes. A scalar larger
/// than the requested cap remains intact (the existing caller contract).
#[must_use]
#[allow(
	clippy::cast_possible_truncation,
	clippy::cast_sign_loss,
	reason = "positive byte caps saturate on conversion; fractions round down"
)]
pub fn conpty_chunks(data: &[u16], max_bytes: f64) -> Vec<Range<usize>> {
	if data.is_empty() || max_bytes.is_nan() {
		return std::iter::once(0..data.len()).collect();
	}
	let cap = if max_bytes < 1.0 {
		0
	} else {
		max_bytes as usize
	};
	let mut chunks = Vec::new();
	let mut start = 0;
	while start < data.len() {
		let mut bytes = 0;
		let mut newline = None;
		let mut at = start;
		while at < data.len() {
			let (units, encoded_bytes) = utf8_unit_width(data, at);
			if bytes + encoded_bytes > cap && at > start {
				let end = newline.unwrap_or(at);
				chunks.push(start..end);
				start = end;
				break;
			}
			bytes += encoded_bytes;
			if data[at] == 10 {
				newline = Some(at + units);
			}
			at += units;
		}
		if at == data.len() {
			chunks.push(start..at);
			start = at;
		}
	}
	chunks
}

/// Chunk an already encoded UTF-8 packet without expanding it to UTF-16.
///
/// Returned offsets address the original host UTF-16 string. UTF-8 replacement
/// of unmatched surrogates preserves both their unit count and encoded width.
#[must_use]
#[allow(
	clippy::cast_possible_truncation,
	clippy::cast_sign_loss,
	reason = "positive caps are explicitly clamped before conversion"
)]
pub fn conpty_utf8_chunks(text: &str, max_bytes: f64) -> Vec<Range<usize>> {
	fn units(bytes: &[u8]) -> usize {
		// Valid UTF-8: a continuation has high bits 10, and a four-byte
		// scalar has leading bits 1111. Count eight bytes per portable word;
		// popcount avoids a scalar decoder without requiring WASM SIMD.
		let (words, remainder) = bytes.as_chunks::<8>();
		let mut count = 0;
		for bytes in words {
			let word = u64::from_le_bytes(*bytes);
			let continuation = word & 0x8080_8080_8080_8080 & !(word << 1);
			let four_byte = word & (word << 1) & (word << 2) & (word << 3) & 0x8080_8080_8080_8080;
			count += 8 - continuation.count_ones() as usize + four_byte.count_ones() as usize;
		}
		remainder.iter().fold(count, |count, byte| {
			count + usize::from(byte & 0xc0 != 0x80) + usize::from(*byte >= 0xf0)
		})
	}
	if text.is_empty() || max_bytes.is_nan() || max_bytes == f64::INFINITY {
		return std::iter::once(0..units(text.as_bytes())).collect();
	}
	let cap = if max_bytes < 1.0 {
		0
	} else {
		max_bytes as usize
	};
	let mut ranges = Vec::new();
	let mut start = 0;
	let mut start_units = 0;
	while start < text.len() {
		let mut end = start.saturating_add(cap).min(text.len());
		while !text.is_char_boundary(end) {
			end -= 1;
		}
		if end == start {
			end += text[start..]
				.chars()
				.next()
				.expect("remaining scalar")
				.len_utf8();
		}
		if end < text.len()
			&& let Some(newline) = text[start..end].rfind('\n')
		{
			end = start + newline + 1;
		}
		let end_units = start_units + units(&text.as_bytes()[start..end]);
		ranges.push(start_units..end_units);
		start = end;
		start_units = end_units;
	}
	ranges
}
