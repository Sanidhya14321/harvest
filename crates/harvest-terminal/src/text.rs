//! Compatibility facade over the current shared text engine.
use std::borrow::Cow;

pub use harvest_tui::text::{
	DEFAULT_TAB_WIDTH, Ellipsis, WrappedLines, set_hangul_compat_jamo_width_override, visible_width,
	wrap_text_with_ansi,
};
pub fn truncate_to_width(
	text: &[u16],
	width: u32,
	ellipsis: Option<Ellipsis>,
	pad: Option<bool>,
	tab_width: u32,
) -> Cow<'_, [u16]> {
	harvest_tui::text::truncate_to_width(text, width, ellipsis, pad, tab_width)
}
pub fn slice_with_width(
	text: &[u16],
	start: u32,
	width: u32,
	strict: Option<bool>,
	tab_width: u32,
) -> (Vec<u16>, usize) {
	let result = harvest_tui::text::slice_with_width(text, start, width, strict, tab_width);
	(result.text, result.width)
}
pub fn extract_segments(
	text: &[u16],
	before: u32,
	after_start: u32,
	after: u32,
	strict: bool,
	tab_width: u32,
) -> (Vec<u16>, usize, Vec<u16>, usize) {
	let result =
		harvest_tui::text::extract_segments(text, before, after_start, after, strict, tab_width);
	(result.before, result.before_width, result.after, result.after_width)
}
