//! Multi-line input editor (port of packages/tui `Editor`, core subset).

use crate::{component::Component, keyparse::Key, sanitize};

pub const MAX_EDITOR_BYTES: usize = 512 * 1024;
const MAX_HISTORY_ENTRIES: usize = 100;
const MAX_HISTORY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub struct Editor {
	lines:       Vec<Vec<char>>,
	row:         usize,
	col:         usize,
	history:     Vec<String>,
	history_pos: Option<usize>,
	bytes:       usize,
	pub prompt:  String,
}

impl Default for Editor {
	fn default() -> Self {
		Self {
			lines:       vec![Vec::new()],
			row:         0,
			col:         0,
			history:     Vec::new(),
			history_pos: None,
			bytes:       0,
			prompt:      String::new(),
		}
	}
}

/// Result of feeding a key to the editor.
#[derive(Debug, PartialEq, Eq)]
pub enum EditorAction {
	None,
	Submit(String),
	Rejected,
}

impl Editor {
	#[must_use]
	pub fn new(prompt: &str) -> Self {
		Self { lines: vec![Vec::new()], prompt: prompt.to_owned(), ..Default::default() }
	}

	#[must_use]
	pub fn text(&self) -> String {
		self
			.lines
			.iter()
			.map(|l| l.iter().collect::<String>())
			.collect::<Vec<_>>()
			.join("\n")
	}

	/// Replace the draft atomically; oversized input leaves the current draft
	/// intact.
	pub fn set_text(&mut self, text: &str) -> bool {
		let Some(text) = normalize_input(text, MAX_EDITOR_BYTES) else {
			return false;
		};
		self.bytes = text.len();
		self.lines = text.split('\n').map(|l| l.chars().collect()).collect();
		self.row = self.lines.len() - 1;
		self.col = self.lines[self.row].len();
		self.history_pos = None;
		true
	}

	fn insert(&mut self, ch: char) -> bool {
		let added = if ch == '\t' {
			sanitize::TAB_WIDTH
		} else {
			ch.len_utf8()
		};
		if added > MAX_EDITOR_BYTES.saturating_sub(self.bytes) {
			return false;
		}
		if ch == '\n' {
			let rest = self.lines[self.row].split_off(self.col);
			self.row += 1;
			self.lines.insert(self.row, rest);
			self.col = 0;
		} else if ch == '\t' {
			for _ in 0..sanitize::TAB_WIDTH {
				self.insert(' ');
			}
		} else if !ch.is_control() {
			self.lines[self.row].insert(self.col, ch);
			self.col += 1;
		} else {
			return true;
		}
		if ch != '\t' {
			self.bytes += added;
		}
		true
	}

	fn backspace(&mut self) {
		if self.col > 0 {
			self.col -= 1;
			self.lines[self.row].remove(self.col);
		} else if self.row > 0 {
			let line = self.lines.remove(self.row);
			self.row -= 1;
			self.col = self.lines[self.row].len();
			self.lines[self.row].extend(line);
		}
	}

	fn word_left(&mut self) {
		let line = &self.lines[self.row];
		while self.col > 0 && line[self.col - 1].is_whitespace() {
			self.col -= 1;
		}
		while self.col > 0 && !line[self.col - 1].is_whitespace() {
			self.col -= 1;
		}
	}

	fn word_right(&mut self) {
		let line = &self.lines[self.row];
		while self.col < line.len() && line[self.col].is_whitespace() {
			self.col += 1;
		}
		while self.col < line.len() && !line[self.col].is_whitespace() {
			self.col += 1;
		}
	}

	fn recall(&mut self, older: bool) {
		if self.history.is_empty() {
			return;
		}
		let next = match (self.history_pos, older) {
			(None, true) => Some(self.history.len() - 1),
			(Some(i), true) => Some(i.saturating_sub(1)),
			(Some(i), false) if i + 1 < self.history.len() => Some(i + 1),
			_ => None,
		};
		let text = next.map(|i| self.history[i].clone()).unwrap_or_default();
		self.set_text(&text);
		self.history_pos = next;
	}

	/// Apply one key; Enter submits non-empty text.
	pub fn apply(&mut self, key: &Key) -> EditorAction {
		match key {
			Key::Char(c) => {
				if !self.insert(*c) {
					return EditorAction::Rejected;
				}
			},
			Key::Tab => {
				if !self.insert('\t') {
					return EditorAction::Rejected;
				}
			},
			Key::Paste(text) => {
				let Some(text) = normalize_input(text, MAX_EDITOR_BYTES.saturating_sub(self.bytes))
				else {
					return EditorAction::Rejected;
				};
				for ch in text.chars() {
					self.insert(ch);
				}
			},
			Key::ShiftEnter | Key::Ctrl('j') => {
				if !self.insert('\n') {
					return EditorAction::Rejected;
				}
			},
			Key::Enter => {
				let text = self.text();
				if text.trim().is_empty() {
					return EditorAction::None;
				}
				if self.history.last() != Some(&text) {
					self.history.push(text.clone());
				}
				while self.history.len() > MAX_HISTORY_ENTRIES
					|| self.history.iter().map(String::len).sum::<usize>() > MAX_HISTORY_BYTES
				{
					self.history.remove(0);
				}
				self.history_pos = None;
				self.set_text("");
				return EditorAction::Submit(text);
			},
			Key::Backspace => self.backspace(),
			Key::Delete => {
				if self.col < self.lines[self.row].len() {
					self.lines[self.row].remove(self.col);
				} else if self.row + 1 < self.lines.len() {
					let next = self.lines.remove(self.row + 1);
					self.lines[self.row].extend(next);
				}
			},
			Key::Left => {
				if self.col > 0 {
					self.col -= 1;
				} else if self.row > 0 {
					self.row -= 1;
					self.col = self.lines[self.row].len();
				}
			},
			Key::Right => {
				if self.col < self.lines[self.row].len() {
					self.col += 1;
				} else if self.row + 1 < self.lines.len() {
					self.row += 1;
					self.col = 0;
				}
			},
			Key::Up if self.row > 0 => {
				self.row -= 1;
				self.col = self.col.min(self.lines[self.row].len());
			},
			Key::Up => self.recall(true),
			Key::Down if self.row + 1 < self.lines.len() => {
				self.row += 1;
				self.col = self.col.min(self.lines[self.row].len());
			},
			Key::Down => self.recall(false),
			Key::Home | Key::Ctrl('a') => self.col = 0,
			Key::End | Key::Ctrl('e') => self.col = self.lines[self.row].len(),
			Key::WordLeft | Key::Alt('b') => self.word_left(),
			Key::WordRight | Key::Alt('f') => self.word_right(),
			Key::Ctrl('u') => {
				self.lines[self.row].drain(..self.col);
				self.col = 0;
			},
			Key::Ctrl('k') => self.lines[self.row].truncate(self.col),
			Key::Ctrl('w') => {
				let end = self.col;
				self.word_left();
				self.lines[self.row].drain(self.col..end);
			},
			_ => {},
		}
		self.bytes = self
			.lines
			.iter()
			.flatten()
			.map(|ch| ch.len_utf8())
			.sum::<usize>()
			+ self.lines.len().saturating_sub(1);
		EditorAction::None
	}

	/// Visual rows: (text, logical row, start col), hard-wrapped.
	fn visual(&self, width: usize) -> Vec<(String, usize, usize)> {
		let prompt = sanitize::truncate_to_width(
			&sanitize::replace_tabs(&self.prompt).replace('\n', " "),
			width.saturating_sub(1),
		);
		let avail = width
			.saturating_sub(sanitize::visible_width(&prompt))
			.max(1);
		let mut rows = Vec::new();
		for (r, line) in self.lines.iter().enumerate() {
			let mut start = 0;
			let mut cur = String::new();
			let mut used = 0;
			let value: String = line.iter().collect();
			let mut i = 0;
			for grapheme in xutf::graphemes_str(&value) {
				let w = sanitize::visible_width(grapheme);
				if used + w > avail && !cur.is_empty() {
					rows.push((std::mem::take(&mut cur), r, start));
					start = i;
					used = 0;
				}
				cur.push_str(grapheme);
				used += w;
				i += grapheme.chars().count();
			}
			rows.push((cur, r, start));
			if used >= avail && self.row == r && self.col == line.len() {
				rows.push((String::new(), r, line.len()));
			}
		}
		rows
	}
}

impl Component for Editor {
	fn render(&mut self, width: usize) -> Vec<String> {
		let prompt = sanitize::truncate_to_width(
			&sanitize::replace_tabs(&self.prompt).replace('\n', " "),
			width.saturating_sub(1),
		);
		let pad = " ".repeat(sanitize::visible_width(&prompt));
		self
			.visual(width)
			.into_iter()
			.enumerate()
			.map(|(i, (text, ..))| {
				sanitize::truncate_to_width(
					&format!("{}{text}", if i == 0 { &prompt } else { &pad }),
					width,
				)
			})
			.collect()
	}

	fn handle_key(&mut self, key: &Key) -> bool {
		self.apply(key);
		true
	}

	fn cursor(&self, width: usize) -> Option<(usize, usize)> {
		let prompt = sanitize::truncate_to_width(
			&sanitize::replace_tabs(&self.prompt).replace('\n', " "),
			width.saturating_sub(1),
		);
		let prompt_w = sanitize::visible_width(&prompt);
		let mut found = (0, prompt_w);
		for (i, (text, r, start)) in self.visual(width).iter().enumerate() {
			if *r == self.row && self.col >= *start && self.col <= start + text.chars().count() {
				let value: String = self.lines[*r][*start..self.col].iter().collect();
				let w = sanitize::visible_width(&value);
				found = (i, prompt_w + w);
			}
		}
		(width > 0).then_some((found.0, found.1.min(width.saturating_sub(1))))
	}
}

fn normalize_input(input: &str, limit: usize) -> Option<String> {
	if input.len() > limit {
		return None;
	}
	let mut out = String::new();
	let mut chars = input.chars().peekable();
	while let Some(ch) = chars.next() {
		let value = match ch {
			'\r' => {
				if chars.peek() == Some(&'\n') {
					chars.next();
				}
				'\n'
			},
			ch if ch.is_control() && ch != '\n' && ch != '\t' => continue,
			ch => ch,
		};
		let bytes = if value == '\t' {
			sanitize::TAB_WIDTH
		} else {
			value.len_utf8()
		};
		if bytes > limit.saturating_sub(out.len()) {
			return None;
		}
		if value == '\t' {
			out.extend(std::iter::repeat_n(' ', sanitize::TAB_WIDTH));
		} else {
			out.push(value);
		}
	}
	Some(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn paste_normalization_and_oversize_rejection_keep_the_draft_editable() {
		let mut editor = Editor::default();
		editor.apply(&Key::Paste("a\r\nb\tc\u{1b}".into()));
		assert_eq!(editor.text(), "a\nb   c");
		assert_eq!(editor.apply(&Key::Paste("x".repeat(MAX_EDITOR_BYTES))), EditorAction::Rejected);
		assert_eq!(editor.text(), "a\nb   c");
		editor.apply(&Key::Backspace);
		assert_eq!(editor.apply(&Key::Enter), EditorAction::Submit("a\nb   ".into()));
	}

	#[test]
	fn narrow_editor_keeps_wrapped_graphemes_and_cursor_inside_the_viewport() {
		let mut editor = Editor::new("› ");
		editor.set_text("界👩‍💻abc");
		for width in [1, 2, 5] {
			let rows = editor.render(width);
			assert!(rows.iter().all(|row| sanitize::visible_width(row) <= width));
			let (row, column) = editor.cursor(width).unwrap();
			assert!(row < rows.len() && column < width);
		}
	}
}
