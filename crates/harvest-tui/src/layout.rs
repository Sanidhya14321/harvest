//! Vertical container (port of packages/tui `Container`).

use crate::{component::Component, keyparse::Key};

#[derive(Default)]
pub struct Container {
	pub children: Vec<Box<dyn Component + Send>>,
	pub focus:    Option<usize>,
}

impl Container {
	pub fn push(&mut self, child: Box<dyn Component + Send>) -> usize {
		self.children.push(child);
		self.children.len() - 1
	}

	pub fn clear(&mut self) {
		self.children.clear();
		self.focus = None;
	}
}

impl Component for Container {
	fn render(&mut self, width: usize) -> Vec<String> {
		self
			.children
			.iter_mut()
			.flat_map(|c| c.render(width))
			.collect()
	}

	fn handle_key(&mut self, key: &Key) -> bool {
		self
			.focus
			.and_then(|i| self.children.get_mut(i))
			.is_some_and(|c| c.handle_key(key))
	}
}

/// Clip `lines` to at most `height`, ending `scroll` lines above the tail.
#[must_use]
pub fn viewport(lines: &[String], height: usize, scroll: usize) -> Vec<String> {
	let end = lines.len().saturating_sub(scroll);
	let start = end.saturating_sub(height);
	lines[start..end].to_vec()
}
