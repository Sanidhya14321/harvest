//! Focused-editor adapters; protocol parsing stays in the shared key engine.

#[cfg(not(target_arch = "wasm32"))]
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::keys::{self, InputMode};

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
	/// Normalize an already-framed terminal event using the legacy editor's
	/// shared parser. Protocol replies and release events never insert text.
	#[must_use]
	pub fn from_bytes(bytes: &[u8], mode: InputMode) -> Option<Self> {
		if let Ok(value) = std::str::from_utf8(bytes)
			&& let Some(text) = keys::extract_printable_text(value)
		{
			let mut chars = text.chars();
			let first = chars.next()?;
			return Some(if chars.next().is_none() {
				Self::Char(first)
			} else {
				Self::Paste(text.into_owned())
			});
		}
		let key = keys::parse_input_key(bytes, mode)?;
		Some(match key.as_ref() {
			"enter" => Self::Enter,
			"shift+enter" | "alt+enter" | "ctrl+enter" => Self::ShiftEnter,
			"backspace" => Self::Backspace,
			"ctrl+backspace" | "alt+backspace" => Self::Ctrl('w'),
			"delete" => Self::Delete,
			"left" => Self::Left,
			"right" => Self::Right,
			"up" => Self::Up,
			"down" => Self::Down,
			"home" => Self::Home,
			"end" => Self::End,
			"pageUp" => Self::PageUp,
			"pageDown" => Self::PageDown,
			"tab" => Self::Tab,
			"shift+tab" => Self::BackTab,
			"escape" => Self::Escape,
			"ctrl+left" | "alt+left" => Self::WordLeft,
			"ctrl+right" | "alt+right" => Self::WordRight,
			"space" => Self::Char(' '),
			other => {
				let (modifier, letter) = other.split_once('+')?;
				let mut chars = letter.chars();
				let character = chars.next()?;
				if chars.next().is_some() {
					return None;
				}
				match modifier {
					"ctrl" => Self::Ctrl(character),
					"alt" => Self::Alt(character),
					_ => return None,
				}
			},
		})
	}

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
			KeyCode::Enter if shift || alt || ctrl => Self::ShiftEnter,
			KeyCode::Enter => Self::Enter,
			KeyCode::Backspace if ctrl || alt => Self::Ctrl('w'),
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
