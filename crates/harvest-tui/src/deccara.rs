//! Conservative rectangular background fills over final UTF-16 terminal rows.
use std::borrow::Cow;

use crate::{
	MAX_CSI_BYTES,
	text::{DEFAULT_TAB_WIDTH, visible_width},
};

pub const DECSACE_RECT: &str = "\x1b[2*x";
pub const DECSACE_DEFAULT: &str = "\x1b[*x";
pub const MAX_PLAN_LINES: usize = 16_384;
pub const MAX_PLAN_UNITS: usize = 16 * 1024 * 1024;
const RESET: &[u16] = &[0x1b, 0x5b, 0x30, 0x6d];

/// UTF-16 cutoff and terminal column immediately after the last content glyph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FillAnalysis {
	pub cut:      usize,
	pub left_col: usize,
	pub bg:       String,
}

/// Only changed row cutoffs are materialized; an empty list means no changes.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FillPlan {
	pub cuts:     Vec<Option<usize>>,
	pub sequence: String,
}

impl FillPlan {
	/// Preserve unchanged input without copying, and reset attributes on a cut.
	pub fn render_line<'a>(&self, index: usize, line: &'a [u16]) -> Cow<'a, [u16]> {
		match self.cuts.get(index).copied().flatten() {
			Some(0) => Cow::Borrowed(&[]),
			Some(cut) if cut <= line.len() => {
				let mut result = line[..cut].to_vec();
				result.extend_from_slice(RESET);
				Cow::Owned(result)
			},
			_ => Cow::Borrowed(line),
		}
	}
}

/// Encode a valid 1-based inclusive rectangle without admitting control bytes.
pub fn encode_rectangle(top: u32, left: u32, bottom: u32, right: u32, sgr: &str) -> Option<String> {
	if top == 0
		|| left == 0
		|| top > bottom
		|| left > right
		|| sgr.len() > MAX_CSI_BYTES
		|| !sgr
			.bytes()
			.all(|byte| byte.is_ascii_digit() || b";:".contains(&byte))
	{
		return None;
	}
	let sequence = format!("\x1b[{top};{left};{bottom};{right};{sgr}$r");
	(sequence.len() <= MAX_CSI_BYTES).then_some(sequence)
}

fn ascii(units: &[u16]) -> String {
	units.iter().map(|unit| char::from(*unit as u8)).collect()
}

fn parameter(units: &[u16]) -> Option<u32> {
	if units.is_empty() {
		return Some(0);
	}
	units.iter().try_fold(0u32, |value, unit| {
		let digit = unit.checked_sub(u16::from(b'0'))?;
		if digit > 9 {
			return None;
		}
		value.checked_mul(10)?.checked_add(u32::from(digit))
	})
}

struct UnsupportedBackground;

// A default background is valid; malformed metadata is a separate error.
fn background(
	mut current: Option<String>,
	params: &[u16],
) -> Result<Option<String>, UnsupportedBackground> {
	if params.is_empty() {
		return Ok(None);
	}
	if !params
		.iter()
		.all(|unit| (0x30..=0x39).contains(unit) || *unit == u16::from(b';'))
	{
		return Err(UnsupportedBackground);
	}
	let tokens: Vec<_> = params.split(|unit| *unit == u16::from(b';')).collect();
	let mut index = 0;
	while index < tokens.len() {
		let number = parameter(tokens[index]).ok_or(UnsupportedBackground)?;
		match number {
			0 | 49 => current = None,
			40..=47 | 100..=107 => current = Some(ascii(tokens[index])),
			38 | 48 => {
				let mode = *tokens.get(index + 1).ok_or(UnsupportedBackground)?;
				let count = match mode {
					[0x35] => 1,
					[0x32] => 3,
					_ => return Err(UnsupportedBackground),
				};
				let channels = tokens
					.get(index + 2..index + 2 + count)
					.ok_or(UnsupportedBackground)?;
				for channel in channels {
					if parameter(channel).ok_or(UnsupportedBackground)? > 255 {
						return Err(UnsupportedBackground);
					}
				}
				if number == 48 {
					let mut color = format!("48;{}", ascii(mode));
					for channel in channels {
						color.push(';');
						color.push_str(&ascii(channel));
					}
					current = Some(color);
				}
				index += 1 + count;
			},
			// Foreground colors, emphasis and their resets do not draw on blank
			// cells. Decorations, inverse video, font selectors and unknown SGRs
			// stay in the original row instead of being reduced to background.
			1..=3 | 5 | 6 | 8 | 22..=25 | 27..=39 | 90..=97 => {},
			_ => return Err(UnsupportedBackground),
		}
		index += 1;
	}
	Ok(current)
}

/// Refuse unsupported sequences, partial rows and mixed/default trailing fills.
/// Malformed or noncanonical SGR stays in its original row, never a rectangle.
pub fn analyze_line(line: &[u16], width: u32) -> Option<FillAnalysis> {
	if width == 0 || line.is_empty() || line.len() > MAX_PLAN_UNITS {
		return None;
	}
	let mut offset = 0;
	let mut column = 0usize;
	let mut bg = None;
	let mut non_space_end = 0;
	let mut non_space_column = 0;
	let mut trail_bg = None;
	let mut trail_started = false;
	let mut consistent = true;
	while offset < line.len() {
		if line[offset] == 0x1b {
			if line.get(offset + 1) != Some(&0x5b) {
				return None;
			}
			let mut end = offset + 2;
			while end < line.len() && !(0x40..=0x7e).contains(&line[end]) {
				if end - offset >= MAX_CSI_BYTES {
					return None;
				}
				end += 1;
			}
			if line.get(end) != Some(&0x6d) || end - offset >= MAX_CSI_BYTES {
				return None;
			}
			bg = background(bg, &line[offset + 2..end]).ok()?;
			offset = end + 1;
			continue;
		}
		let end = line[offset..]
			.iter()
			.position(|unit| *unit == 0x1b)
			.map_or(line.len(), |at| offset + at);
		let text = &line[offset..end];
		if text
			.iter()
			.any(|unit| *unit < 0x20 || (0x7f..=0x9f).contains(unit))
		{
			return None;
		}
		let non_space_len = text
			.iter()
			.rposition(|unit| *unit != 0x20)
			.map_or(0, |at| at + 1);
		if non_space_len > 0 {
			non_space_end = offset + non_space_len;
			non_space_column =
				column.checked_add(visible_width(&text[..non_space_len], DEFAULT_TAB_WIDTH))?;
			trail_started = non_space_len < text.len();
			trail_bg = if trail_started { bg.clone() } else { None };
			consistent = true;
		} else if !text.is_empty() {
			if !trail_started {
				trail_bg.clone_from(&bg);
				trail_started = true;
			} else if bg != trail_bg {
				consistent = false;
			}
		}
		column = column.checked_add(visible_width(text, DEFAULT_TAB_WIDTH))?;
		offset = end;
	}
	if column != width as usize || non_space_column >= column || !trail_started || !consistent {
		return None;
	}
	Some(FillAnalysis { cut: non_space_end, left_col: non_space_column, bg: trail_bg? })
}

struct Candidate {
	analysis: FillAnalysis,
	removed:  i128,
}
struct Group {
	start:     usize,
	end:       usize,
	rectangle: String,
}

/// Coalesce only adjacent identical spans, charging the wrapper once per frame.
/// Identity plans retain the exact rows, including unsupported control
/// payloads.
pub fn plan_fills(lines: &[Vec<u16>], width: u32, first_screen_row: u32) -> FillPlan {
	if lines.len() > MAX_PLAN_LINES
		|| lines
			.iter()
			.try_fold(0usize, |sum, line| sum.checked_add(line.len()))
			.is_none_or(|units| units > MAX_PLAN_UNITS)
	{
		return FillPlan::default();
	}
	let candidates: Vec<_> = lines
		.iter()
		.map(|line| {
			analyze_line(line, width).map(|analysis| {
				let short_len = if analysis.cut == 0 {
					0
				} else {
					analysis.cut + RESET.len()
				};
				let removed = line.len() as i128 - short_len as i128;
				Candidate { analysis, removed }
			})
		})
		.collect();
	let mut groups = Vec::new();
	let mut removed_total = 0i128;
	let mut rectangle_total = 0usize;
	let mut row = 0;
	while row < lines.len() {
		let Some(head) = &candidates[row] else {
			row += 1;
			continue;
		};
		let mut end = row;
		while let Some(Some(next)) = candidates.get(end + 1) {
			if next.analysis.left_col != head.analysis.left_col || next.analysis.bg != head.analysis.bg
			{
				break;
			}
			end += 1;
		}
		let coordinates = u32::try_from(row)
			.ok()
			.and_then(|row| first_screen_row.checked_add(row)?.checked_add(1))
			.zip(
				u32::try_from(end)
					.ok()
					.and_then(|end| first_screen_row.checked_add(end)?.checked_add(1)),
			);
		let Some((top, bottom)) = coordinates else {
			return FillPlan::default();
		};
		let Some(rectangle) =
			encode_rectangle(top, head.analysis.left_col as u32 + 1, bottom, width, &head.analysis.bg)
		else {
			return FillPlan::default();
		};
		let removed: i128 = candidates[row..=end]
			.iter()
			.flatten()
			.map(|value| value.removed)
			.sum();
		if removed > rectangle.len() as i128 {
			removed_total += removed;
			rectangle_total += rectangle.len();
			groups.push(Group { start: row, end, rectangle });
		}
		row = end + 1;
	}
	if groups.is_empty()
		|| removed_total - rectangle_total as i128
			<= (DECSACE_RECT.len() + DECSACE_DEFAULT.len()) as i128
	{
		return FillPlan::default();
	}
	let mut plan = FillPlan { cuts: vec![None; lines.len()], sequence: DECSACE_RECT.to_owned() };
	for group in groups {
		for (index, candidate) in candidates
			.iter()
			.enumerate()
			.take(group.end + 1)
			.skip(group.start)
		{
			if let Some(candidate) = candidate {
				plan.cuts[index] = Some(candidate.analysis.cut);
			}
		}
		plan.sequence.push_str(&group.rectangle);
	}
	plan.sequence.push_str(DECSACE_DEFAULT);
	plan
}

#[cfg(test)]
mod tests {
	use super::*;
	fn units(text: &str) -> Vec<u16> {
		text.encode_utf16().collect()
	}
	fn blank(width: usize, sgr: &str) -> Vec<u16> {
		units(&format!("\x1b[{sgr}m{}\x1b[49m\x1b[0m", " ".repeat(width)))
	}
	#[test]
	fn rectangle_encoding_preserves_the_wire_form_and_rejects_injection_or_reversed_regions() {
		assert_eq!(encode_rectangle(4, 3, 11, 10, "44").as_deref(), Some("\x1b[4;3;11;10;44$r"));
		assert!(encode_rectangle(0, 1, 1, 2, "41").is_none());
		assert!(encode_rectangle(3, 1, 2, 2, "41").is_none());
		assert!(encode_rectangle(1, 3, 2, 2, "41").is_none());
		assert!(encode_rectangle(1, 1, 2, 2, "41\x1b[2J").is_none());
		assert!(encode_rectangle(1, 1, 2, 2, &"0;".repeat(MAX_CSI_BYTES)).is_none());
	}
	#[test]
	fn whole_row_fills_support_distinct_basic_palette_and_rgb_background_forms() {
		for sgr in ["41", "101", "48;5;4", "48;2;10;20;30"] {
			assert_eq!(
				analyze_line(&blank(10, sgr), 10),
				Some(FillAnalysis { cut: 0, left_col: 0, bg: sgr.to_owned() })
			);
		}
	}
	#[test]
	fn content_cutoffs_preserve_utf16_units_and_measure_wide_terminal_cells() {
		let prefix = units("\x1b[41m👩‍💻");
		let mut line = prefix.clone();
		line.extend(units(&format!("{}\x1b[0m", " ".repeat(38))));
		let analysis = analyze_line(&line, 40).unwrap();
		assert_eq!(analysis.cut, prefix.len());
		assert_eq!(analysis.left_col, 2);
		let plan = plan_fills(std::slice::from_ref(&line), 40, 0);
		let mut expected = prefix;
		expected.extend_from_slice(RESET);
		assert_eq!(plan.render_line(0, &line).as_ref(), expected);
		assert_eq!(plan.sequence, "\x1b[2*x\x1b[1;3;1;40;41$r\x1b[*x");
	}
	#[test]
	fn default_gaps_and_changed_trailing_backgrounds_cannot_be_painted_as_one_span() {
		for text in ["  \x1b[41m        \x1b[0m", "\x1b[41mab\x1b[49m   \x1b[42m     \x1b[0m"] {
			assert!(analyze_line(&units(text), 10).is_none());
		}
		assert_eq!(
			analyze_line(&units("X\x1b[41m         \x1b[0m"), 10),
			Some(FillAnalysis { cut: 1, left_col: 1, bg: "41".to_owned() })
		);
	}
	#[test]
	fn unsupported_sequences_and_malformed_colors_keep_the_original_rows() {
		for text in [
			"\x1b[41m\x1b]8;;x\x07x\x1b]8;;\x07         \x1b[0m",
			"\x1b[41m\x1b[2J          \x1b[0m",
			"\x1b[48:2:1:2:3m          \x1b[0m",
			"\x1b[48;5;256m          \x1b[0m",
			"\x1b[48;2;1;2m          \x1b[0m",
			"\x1b[41m\r          \x1b[0m",
			"\x1b[41m\u{9b}          \x1b[0m",
			"\x1b[41m         \x1b[",
		] {
			let line = units(text);
			assert!(analyze_line(&line, 10).is_none());
			let plan = plan_fills(std::slice::from_ref(&line), 10, 0);
			assert_eq!(plan.sequence, "");
			assert_eq!(plan.render_line(0, &line).as_ref(), line);
		}
	}
	#[test]
	fn visible_attributes_on_padding_are_never_replaced_by_background_only_rectangles() {
		for sgr in ["4;41", "7;41", "9;41", "53;41", "60;41"] {
			let line = blank(40, sgr);
			assert!(analyze_line(&line, 40).is_none());
			let plan = plan_fills(std::slice::from_ref(&line), 40, 0);
			assert_eq!(plan.sequence, "");
			assert_eq!(plan.render_line(0, &line).as_ref(), line);
		}
	}

	#[test]
	fn adjacent_matching_rows_coalesce_with_the_screen_offset() {
		let lines = vec![blank(10, "48;2;10;20;30"); 3];
		let plan = plan_fills(&lines, 10, 3);
		assert_eq!(plan.sequence, "\x1b[2*x\x1b[4;1;6;10;48;2;10;20;30$r\x1b[*x");
		assert_eq!(plan.cuts, vec![Some(0); 3]);
		for (index, line) in lines.iter().enumerate() {
			assert_eq!(plan.render_line(index, line).as_ref(), &[] as &[u16]);
		}
	}
	#[test]
	fn nonadjacent_fills_and_different_backgrounds_remain_separate_rectangles() {
		let lines = vec![blank(40, "41"), units("unchanged"), blank(40, "41"), blank(40, "42")];
		let plan = plan_fills(&lines, 40, 0);
		assert_eq!(
			plan.sequence,
			"\x1b[2*x\x1b[1;1;1;40;41$r\x1b[3;1;3;40;41$r\x1b[4;1;4;40;42$r\x1b[*x"
		);
		assert_eq!(plan.render_line(1, &lines[1]).as_ref(), lines[1]);
	}
	#[test]
	fn the_frame_wrapper_must_be_paid_before_any_rows_change() {
		let tight = units(&format!("\x1b[41m{}\x1b[0m", " ".repeat(15)));
		let paying = units(&format!("\x1b[41m{}\x1b[0m", " ".repeat(16)));
		let rejected = plan_fills(std::slice::from_ref(&tight), 15, 0);
		assert_eq!(rejected.sequence, "");
		assert_eq!(rejected.render_line(0, &tight).as_ref(), tight);
		assert_eq!(plan_fills(&[paying], 16, 0).sequence, "\x1b[2*x\x1b[1;1;1;16;41$r\x1b[*x");
	}
	#[test]
	fn a_reset_that_lengthens_one_row_is_charged_against_the_entire_group() {
		let first = units("\x1b[41mX ");
		let second = units(&format!("\x1b[41mX {}", "\x1b[0m".repeat(7)));
		let lines = vec![first, second];
		let plan = plan_fills(&lines, 2, 0);
		assert_eq!(plan.sequence, "");
		for (index, line) in lines.iter().enumerate() {
			assert_eq!(plan.render_line(index, line).as_ref(), line);
		}
	}
	#[test]
	fn oversized_frames_and_coordinate_overflow_disable_the_optimization() {
		let line = blank(40, "41");
		let lines = vec![line.clone(); MAX_PLAN_LINES + 1];
		let oversized = plan_fills(&lines, 40, 0);
		assert_eq!(oversized.sequence, "");
		assert_eq!(oversized.render_line(0, &line).as_ref(), line);
		let overflow = plan_fills(std::slice::from_ref(&line), 40, u32::MAX);
		assert_eq!(overflow.sequence, "");
		assert_eq!(overflow.render_line(0, &line).as_ref(), line);
		let long = units(&format!("\x1b[{}m          ", "0;".repeat(MAX_CSI_BYTES)));
		assert!(analyze_line(&long, 10).is_none());
	}
}
