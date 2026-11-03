//! Display sanitization helpers (port of `replaceTabs`, `truncateToWidth`,
//! `shortenPath` from packages/tui and coding-agent render utils).

use crate::text::{self, DEFAULT_TAB_WIDTH};

/// Spaces substituted for a tab character.
pub const TAB_WIDTH: usize = DEFAULT_TAB_WIDTH as usize;

/// Replace tabs with spaces and neutralize other control characters (except
/// newline) so raw content can never move the cursor or emit escapes.
#[must_use]
pub fn replace_tabs(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	for ch in text.chars() {
		match ch {
			'\t' => out.extend(std::iter::repeat_n(' ', TAB_WIDTH)),
			'\n' => out.push('\n'),
			'\r' => {},
			c if c.is_control() => out.push('\u{FFFD}'),
			c => out.push(c),
		}
	}
	out
}

/// Display width of plain (escape-free) text.
#[must_use]
pub fn visible_width(text: &str) -> usize {
	crate::text::visible_width(&text.encode_utf16().collect::<Vec<_>>(), DEFAULT_TAB_WIDTH)
}

/// Truncate plain text to `width` columns, appending an ellipsis when cut.
#[must_use]
pub fn truncate_to_width(text: &str, width: usize) -> String {
	let units: Vec<_> = text.encode_utf16().collect();
	String::from_utf16_lossy(&crate::text::truncate_to_width(
		&units,
		u32::try_from(width).unwrap_or(u32::MAX),
		None,
		None,
		DEFAULT_TAB_WIDTH,
	))
}

/// Wrap plain text to `width` columns, hard-breaking long runs.
#[must_use]
pub fn wrap(text: &str, width: usize) -> Vec<String> {
	let units: Vec<_> = text.encode_utf16().collect();
	text::wrap_text_with_ansi(
		&units,
		u32::try_from(width.max(1)).unwrap_or(u32::MAX),
		DEFAULT_TAB_WIDTH,
	)
	.into_iter()
	.map(|line| truncate_to_width(&String::from_utf16_lossy(&line), width))
	.collect()
}

/// Replace the home directory prefix with `~`.
#[must_use]
pub fn shorten_path(path: &str) -> String {
	let Some(home) = std::env::var_os("HOME").map(|h| h.to_string_lossy().into_owned()) else {
		return path.to_owned();
	};
	if home.is_empty() || home == "/" {
		return path.to_owned();
	}
	if path == home {
		return "~".to_owned();
	}
	path.replace(&format!("{home}/"), "~/")
}

/// Full display pipeline for one block of untrusted text.
#[must_use]
pub fn display_lines(text: &str, width: usize) -> Vec<String> {
	wrap(&shorten_path(&replace_tabs(text)), width)
}
