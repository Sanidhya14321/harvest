//! Key normalization (port of packages/tui `keys.ts` onto crossterm events).

#[cfg(not(target_arch = "wasm32"))]
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
	Char(char),
	Ctrl(char),
	Alt(char),
	Enter,
	ShiftEnter,
	Backspace,
	Delete,
	Left,
	Right,
	Up,
	Down,
	Home,
	End,
	PageUp,
	PageDown,
	Tab,
	BackTab,
	Escape,
	WordLeft,
	WordRight,
	Paste(String),
	Unknown,
}

impl Key {
	/// Normalize one crossterm key event; release events map to `None`.
	#[must_use]
	#[cfg(not(target_arch = "wasm32"))]
	pub fn from_event(event: &KeyEvent) -> Option<Self> {
		if event.kind == KeyEventKind::Release {
			return None;
		}
		let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
		let alt = event.modifiers.contains(KeyModifiers::ALT);
		let shift = event.modifiers.contains(KeyModifiers::SHIFT);
		Some(match event.code {
			KeyCode::Char(c) if ctrl => Self::Ctrl(c.to_ascii_lowercase()),
			KeyCode::Char(c) if alt => Self::Alt(c),
			KeyCode::Char(c) => Self::Char(c),
			KeyCode::Enter if shift || alt => Self::ShiftEnter,
			KeyCode::Enter => Self::Enter,
			KeyCode::Backspace => Self::Backspace,
			KeyCode::Delete => Self::Delete,
			KeyCode::Left if ctrl || alt => Self::WordLeft,
			KeyCode::Right if ctrl || alt => Self::WordRight,
			KeyCode::Left => Self::Left,
			KeyCode::Right => Self::Right,
			KeyCode::Up => Self::Up,
			KeyCode::Down => Self::Down,
			KeyCode::Home => Self::Home,
			KeyCode::End => Self::End,
			KeyCode::PageUp => Self::PageUp,
			KeyCode::PageDown => Self::PageDown,
			KeyCode::Tab => Self::Tab,
			KeyCode::BackTab => Self::BackTab,
			KeyCode::Esc => Self::Escape,
			_ => Self::Unknown,
		})
	}
}
