//! Differential renderer (port of packages/tui `TUI.doRender`): repaints only
//! changed rows of the viewport inside synchronized output.

use std::{
	fmt::Write as _,
	io::{self, Write},
};

use crate::sanitize::truncate_to_width;

const SYNC_BEGIN: &str = "\x1b[?2026h";
const SYNC_END: &str = "\x1b[?2026l";
const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct Renderer {
	previous: Vec<String>,
	width:    usize,
	height:   usize,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn styled_overflow_is_clipped_and_unchanged_rows_are_not_repainted() {
		let mut renderer = Renderer::new();
		let rows = vec!["\x1b[31mabcdef\x1b[0m".to_owned()];
		let mut output = Vec::new();
		renderer
			.draw(&mut output, &rows, 3, 1, Some((99, 99)))
			.unwrap();
		let first = String::from_utf8(output).unwrap();
		assert!(first.contains("\x1b[31mab\x1b[0m…"));
		assert!(first.contains("\x1b[1;3H\x1b[?25h"));
		let mut second = Vec::new();
		renderer.draw(&mut second, &rows, 3, 1, None).unwrap();
		assert!(!String::from_utf8(second).unwrap().contains("\x1b[2K"));
	}
}

impl Renderer {
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Force a full repaint next frame.
	pub fn invalidate(&mut self) {
		self.previous.clear();
		self.width = 0;
	}

	/// Paint `lines` into a `height`-row viewport; `cursor` is (row, col).
	pub fn draw(
		&mut self,
		out: &mut impl Write,
		lines: &[String],
		width: usize,
		height: usize,
		cursor: Option<(usize, usize)>,
	) -> io::Result<()> {
		let width = width.min(u16::MAX as usize);
		let height = height.min(u16::MAX as usize);
		let full = width != self.width || height != self.height;
		let mut frame = String::from(SYNC_BEGIN);
		let mut next = Vec::with_capacity(height);
		frame.push_str("\x1b[?25l");
		if full {
			frame.push_str("\x1b[H\x1b[2J");
			self.previous.clear();
		}
		for row in 0..height {
			let source = lines.get(row).map_or("", String::as_str);
			if source.len() > MAX_FRAME_BYTES {
				self.invalidate();
				return Err(io::Error::other("terminal row exceeds byte limit"));
			}
			let line = truncate_to_width(source, width);
			let unchanged = !full && self.previous.get(row) == Some(&line);
			next.push(line);
			if unchanged {
				continue;
			}
			let line = &next[row];
			if line.len() > MAX_FRAME_BYTES.saturating_sub(frame.len().saturating_add(64)) {
				self.invalidate();
				return Err(io::Error::other("terminal frame exceeds byte limit"));
			}
			let _ = write!(frame, "\x1b[{};1H\x1b[2K", row + 1);
			// Styled lines are produced from sanitized text by components;
			// clip styled overflow through the same ANSI-aware width engine.
			frame.push_str(line);
			frame.push_str("\x1b[0m");
		}
		if let Some((r, c)) = cursor.filter(|_| width > 0 && height > 0) {
			let _ = write!(frame, "\x1b[{};{}H\x1b[?25h", r.min(height - 1) + 1, c.min(width - 1) + 1);
		}
		frame.push_str(SYNC_END);
		if let Err(error) = out.write_all(frame.as_bytes()).and_then(|()| out.flush()) {
			self.invalidate();
			return Err(error);
		}
		self.previous = next;
		self.width = width;
		self.height = height;
		Ok(())
	}
}
