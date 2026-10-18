//! ANSI-aware text measurement and slicing utilities.
//!
//! Thin N-API adapters for the shared Rust UTF-16 terminal engine.
//! - Single-pass ANSI scanning (no O(n²) `next_ansi` rescans)
//! - ASCII fast-path (no grapheme segmentation, no UTF-8 conversion)
//! - Non-ASCII uses a reused scratch String for grapheme segmentation
//! - Width checks early-exit
//! - Ellipsis decoded lazily
//! - truncateToWidth returns the original `JsString` when possible

use std::borrow::Cow;

use harvest_terminal::text as engine;
use napi::{JsString, bindgen_prelude::*};
use napi_derive::napi;

use crate::js;

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

fn build_utf16_string(mut data: Vec<u16>) -> Utf16String {
	while data.last() == Some(&0) {
		data.pop();
	}
	Utf16String::from(data)
}

// ============================================================================
// Results
// ============================================================================

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

#[napi]
pub fn set_hangul_compat_jamo_width_override(value: u8) {
	engine::set_hangul_compat_jamo_width_override(value);
}

#[napi]
pub fn wrap_text_with_ansi(text: JsString, width: u32, tab_width: u32) -> Result<Vec<Utf16String>> {
	let text = js::utf16(text)?;
	Ok(engine::wrap_text_with_ansi(&text, width, tab_width)
		.into_iter()
		.map(build_utf16_string)
		.collect())
}

#[napi]
pub fn truncate_to_width(
	text: JsString<'_>,
	max_width: u32,
	ellipsis_kind: Option<Ellipsis>,
	pad: Option<bool>,
	tab_width: u32,
) -> Result<Either<JsString<'_>, Utf16String>> {
	let input = js::utf16(text)?;
	let ellipsis = ellipsis_kind.map(|kind| match kind {
		Ellipsis::Unicode => engine::Ellipsis::Unicode,
		Ellipsis::Ascii => engine::Ellipsis::Ascii,
		Ellipsis::Omit => engine::Ellipsis::Omit,
	});
	Ok(match engine::truncate_to_width(&input, max_width, ellipsis, pad, tab_width) {
		Cow::Borrowed(_) => Either::A(text),
		Cow::Owned(output) => Either::B(build_utf16_string(output)),
	})
}

#[napi]
pub fn slice_with_width(
	line: JsString,
	start_col: u32,
	length: u32,
	strict: Option<bool>,
	tab_width: u32,
) -> Result<SliceResult> {
	// Preserve the old zero-length fast path without coercing the JS input.
	if length == 0 {
		return Ok(SliceResult { text: build_utf16_string(Vec::new()), width: 0 });
	}
	let line = js::utf16(line)?;
	let (out, width) = engine::slice_with_width(&line, start_col, length, strict, tab_width);
	Ok(SliceResult { text: build_utf16_string(out), width: crate::utils::clamp_u32(width as u64) })
}

#[napi]
pub fn extract_segments(
	line: JsString,
	before_end: u32,
	after_start: u32,
	after_len: u32,
	strict_after: bool,
	tab_width: u32,
) -> Result<ExtractSegmentsResult> {
	let line = js::utf16(line)?;
	let (before, before_width, after, after_width) =
		engine::extract_segments(&line, before_end, after_start, after_len, strict_after, tab_width);
	Ok(ExtractSegmentsResult {
		before:       build_utf16_string(before),
		before_width: crate::utils::clamp_u32(before_width as u64),
		after:        build_utf16_string(after),
		after_width:  crate::utils::clamp_u32(after_width as u64),
	})
}

#[napi]
pub fn visible_width(text: JsString, tab_width: u32) -> Result<u32> {
	let text = js::utf16(text)?;
	Ok(crate::utils::clamp_u32(engine::visible_width(&text, tab_width) as u64))
}
