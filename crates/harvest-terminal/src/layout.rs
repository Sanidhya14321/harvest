//! Safe plain-text layout backed by the shared Unicode/ANSI engine.
use crate::{
	frame::TextLayout,
	text::{self, Ellipsis},
};

/// Render untrusted content as visible plain text. Escape/control characters
/// cannot issue terminal commands; styling belongs to trusted UI components.
#[derive(Debug, Clone, Copy)]
pub struct PlainTextLayout {
	tab_width: u32,
}

impl Default for PlainTextLayout {
	fn default() -> Self {
		Self::new(text::DEFAULT_TAB_WIDTH)
	}
}

impl PlainTextLayout {
	pub fn new(tab_width: u32) -> Self {
		Self { tab_width: tab_width.clamp(1, 16) }
	}

	fn sanitize(self, value: &str, multiline: bool) -> String {
		let mut output = String::with_capacity(value.len());
		for character in value.chars() {
			match character {
				'\t' => output.extend(std::iter::repeat_n(' ', self.tab_width as usize)),
				'\n' if multiline => output.push('\n'),
				'\r' | '\n' => output.push(' '),
				'\x1b' => output.push('\u{241b}'),
				character if character.is_control() => output.push('\u{fffd}'),
				character => output.push(character),
			}
		}
		output
	}
}

impl TextLayout for PlainTextLayout {
	fn fit_row(&mut self, row: &str, columns: usize) -> String {
		let safe: Vec<u16> = self.sanitize(row, false).encode_utf16().collect();
		let fitted = text::truncate_to_width(
			&safe,
			columns.min(u32::MAX as usize) as u32,
			Some(Ellipsis::Omit),
			Some(false),
			self.tab_width,
		);
		String::from_utf16_lossy(&fitted)
	}

	fn wrap_text(&mut self, value: &str, columns: usize) -> Vec<String> {
		let safe: Vec<u16> = self.sanitize(value, true).encode_utf16().collect();
		text::wrap_text_with_ansi(&safe, columns.clamp(1, u32::MAX as usize) as u32, self.tab_width)
			.into_iter()
			.map(|row| self.fit_row(&String::from_utf16_lossy(&row), columns))
			.collect()
	}
}
