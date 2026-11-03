//! Component model (port of packages/tui `Component`).

use crate::keyparse::Key;

/// A renderable unit. `render` returns lines no wider than `width` visible
/// columns (implementations sanitize through [`crate::sanitize`]).
pub trait Component {
	fn render(&mut self, width: usize) -> Vec<String>;
	/// Return `true` when the key was consumed.
	fn handle_key(&mut self, _key: &Key) -> bool {
		false
	}
	/// Cursor position (row within this component's lines, column) if focused.
	fn cursor(&self, _width: usize) -> Option<(usize, usize)> {
		None
	}
}

/// Text block, wrapped and sanitized, with optional ANSI prefix/suffix.
#[derive(Debug, Clone, Default)]
pub struct Text {
	pub text:   String,
	pub prefix: String,
	pub suffix: String,
}

impl Text {
	#[must_use]
	pub fn new(text: impl Into<String>) -> Self {
		Self { text: text.into(), ..Default::default() }
	}

	#[must_use]
	pub fn styled(text: impl Into<String>, prefix: &str, suffix: &str) -> Self {
		Self { text: text.into(), prefix: prefix.to_owned(), suffix: suffix.to_owned() }
	}
}

impl Component for Text {
	fn render(&mut self, width: usize) -> Vec<String> {
		crate::sanitize::display_lines(&self.text, width)
			.into_iter()
			.map(|line| format!("{}{line}{}", self.prefix, self.suffix))
			.collect()
	}
}

/// Blank spacer lines.
pub struct Spacer(pub usize);
impl Component for Spacer {
	fn render(&mut self, _width: usize) -> Vec<String> {
		vec![String::new(); self.0]
	}
}
