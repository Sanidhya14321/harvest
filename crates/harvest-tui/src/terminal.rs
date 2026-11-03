//! Terminal lifecycle (port of packages/tui `ProcessTerminal`): raw mode,
//! bracketed paste, alternate screen and guaranteed restoration.

use std::{
	io::{self, IsTerminal, Write},
	sync::{
		Once,
		atomic::{AtomicBool, Ordering},
	},
};

use crossterm::{
	cursor,
	event::{DisableBracketedPaste, EnableBracketedPaste},
	execute, terminal,
};

/// RAII guard: entering enables raw mode; dropping restores the terminal.
pub struct TerminalGuard {
	_private: (),
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static PANIC_HOOK: Once = Once::new();

impl TerminalGuard {
	pub fn enter() -> io::Result<Self> {
		Self::check()?;
		if ACTIVE.swap(true, Ordering::AcqRel) {
			return Err(io::Error::other("interactive terminal is already owned"));
		}
		// Construct before any fallible terminal changes, so partial entry also
		// restores raw mode and the alternate screen.
		let guard = Self { _private: () };
		terminal::enable_raw_mode()?;
		PANIC_HOOK.call_once(|| {
			let previous = std::panic::take_hook();
			std::panic::set_hook(Box::new(move |info| {
				restore();
				previous(info);
			}));
		});
		execute!(io::stdout(), terminal::EnterAlternateScreen, EnableBracketedPaste, cursor::Hide)?;
		Ok(guard)
	}

	/// Reject redirected input/output before creating an interactive session.
	pub fn check() -> io::Result<()> {
		if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
			return Err(io::Error::other(
				"interactive chat requires a terminal; use run or rpc for redirected I/O",
			));
		}
		Ok(())
	}

	/// Current (columns, rows), defaulting to 80x24.
	#[must_use]
	pub fn size() -> (u16, u16) {
		terminal::size().unwrap_or((80, 24))
	}

	/// A caught callback panic may already have restored the terminal. Hosts
	/// must stop drawing in that case instead of repainting the normal screen.
	#[must_use]
	pub fn is_active(&self) -> bool {
		ACTIVE.load(Ordering::Acquire)
	}
}

fn restore() {
	if !ACTIVE.swap(false, Ordering::AcqRel) {
		return;
	}
	let mut out = io::stdout();
	let _ = execute!(out, DisableBracketedPaste, cursor::Show, terminal::LeaveAlternateScreen);
	let _ = terminal::disable_raw_mode();
	let _ = out.flush();
}

impl Drop for TerminalGuard {
	fn drop(&mut self) {
		restore();
	}
}
