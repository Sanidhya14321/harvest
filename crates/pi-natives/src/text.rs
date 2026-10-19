//! Thin UTF-16 N-API bindings over the shared Rust terminal text engine.
use std::borrow::Cow;

use harvest_tui::text as core;
pub const DEFAULT_TAB_WIDTH: usize = core::DEFAULT_TAB_WIDTH as usize;
use napi::{JsString, bindgen_prelude::*};
use napi_derive::napi;

use crate::{js, utils::clamp_u32};

/// Ellipsis strategy for [`truncate_to_width`].
#[napi]
pub enum Ellipsis {
	/// Use a single Unicode ellipsis character ("…").
	Unicode = 0,
	/// Use three ASCII dots ("...").
	Ascii   = 1,
	/// Omit ellipsis entirely.
	Omit    = 2,
}

/// Visible slice of a line after ANSI-aware column selection
/// (`sliceWithWidth`).
#[napi(object)]
pub struct SliceResult {
	/// UTF-16 slice containing the selected text.
	pub text:  Utf16String,
	/// Visible width of the slice in terminal cells.
	pub width: u32,
}

/// Before/after UTF-16 segments around an overlay region, with measured widths.
#[napi(object)]
pub struct ExtractSegmentsResult {
	/// UTF-16 content before the overlay region.
	pub before:       Utf16String,
	/// Visible width of the `before` segment.
	pub before_width: u32,
	/// UTF-16 content after the overlay region.
	pub after:        Utf16String,
	/// Visible width of the `after` segment.
	pub after_width:  u32,
}

fn build_utf16_string(mut data: Vec<u16>) -> Utf16String {
	// Keep the existing bridge contract; the Rust engine preserves all units.
	while data.last() == Some(&0) {
		data.pop();
	}
	Utf16String::from(data)
}

#[napi]
pub fn set_hangul_compat_jamo_width_override(value: u8) {
	core::set_hangul_compat_jamo_width_override(value);
}

/// Wrap text to a visible width, preserving ANSI escape codes across line
/// breaks.
///
/// Returns UTF-16 lines with active SGR codes carried across line boundaries.
#[napi]
pub fn wrap_text_with_ansi(text: JsString, width: u32, tab_width: u32) -> Result<Vec<Utf16String>> {
	let units = js::utf16(text)?;
	Ok(core::wrap_text_with_ansi(&units, width, tab_width)
		.into_iter()
		.map(build_utf16_string)
		.collect())
}

/// Truncate text to a visible width, preserving ANSI codes.
///
/// Pads with spaces when requested.
#[napi]
pub fn truncate_to_width(
	text: JsString<'_>,
	max_width: u32,
	ellipsis_kind: Option<Ellipsis>,
	pad: Option<bool>,
	tab_width: u32,
) -> Result<Either<JsString<'_>, Utf16String>> {
	let units = js::utf16(text)?;
	let ellipsis = ellipsis_kind.map(|value| match value {
		Ellipsis::Unicode => core::Ellipsis::Unicode,
		Ellipsis::Ascii => core::Ellipsis::Ascii,
		Ellipsis::Omit => core::Ellipsis::Omit,
	});
	Ok(match core::truncate_to_width(&units, max_width, ellipsis, pad, tab_width) {
		Cow::Borrowed(_) => Either::A(text),
		Cow::Owned(output) => Either::B(build_utf16_string(output)),
	})
}

/// Slice a range of visible columns from a line.
///
/// Counts terminal cells, skipping ANSI escapes, and optionally enforces strict
/// width.
#[napi]
pub fn slice_with_width(
	line: JsString,
	start_col: u32,
	length: u32,
	strict: Option<bool>,
	tab_width: u32,
) -> Result<SliceResult> {
	// Preserve the old zero-length fast path, including no string read.
	if length == 0 {
		return Ok(SliceResult { text: build_utf16_string(Vec::new()), width: 0 });
	}
	let units = js::utf16(line)?;
	let result = core::slice_with_width(&units, start_col, length, strict, tab_width);
	Ok(SliceResult { text: build_utf16_string(result.text), width: clamp_u32(result.width as u64) })
}

/// Extract the before/after slices around an overlay region.
///
/// Preserves ANSI state so the `after` segment renders correctly after
/// truncation.
#[napi]
pub fn extract_segments(
	line: JsString,
	before_end: u32,
	after_start: u32,
	after_len: u32,
	strict_after: bool,
	tab_width: u32,
) -> Result<ExtractSegmentsResult> {
	let units = js::utf16(line)?;
	let result =
		core::extract_segments(&units, before_end, after_start, after_len, strict_after, tab_width);
	Ok(ExtractSegmentsResult {
		before:       build_utf16_string(result.before),
		before_width: clamp_u32(result.before_width as u64),
		after:        build_utf16_string(result.after),
		after_width:  clamp_u32(result.after_width as u64),
	})
}

/// Calculate visible width of text, excluding ANSI escape sequences.
///
/// Tabs count as a fixed-width cell.
#[napi]
pub fn visible_width(text: JsString, tab_width: u32) -> Result<u32> {
	let units = js::utf16(text)?;
	Ok(clamp_u32(core::visible_width(&units, tab_width) as u64))
}
