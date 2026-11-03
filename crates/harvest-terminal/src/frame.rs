//! Product-owned history transactions and a replaceable terminal viewport.
//!
//! This writer never decides which application rows are final. The provider
//! offers one immutable semantic transaction until its ID is acknowledged.
//! A host sink must accept the complete packet into its ordered output queue
//! or reject it without enqueueing bytes; `std::io::Write` alone does not make
//! that guarantee. Acceptance is not a promise that the terminal has drained.
use std::{fmt, io, num::NonZeroU64, sync::Arc};

const FRAME_LIMIT: usize = 32 * 1024 * 1024;
const ROW_LIMIT: usize = 1024 * 1024;
const LINE_END: &str = "\x1b[0m\x1b]8;;\x07";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportSize {
	pub columns: usize,
	pub rows:    usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryKind {
	Append,
	Replay,
}

/// Owned row snapshots cannot change behind an offered transaction.
#[derive(Debug, Clone)]
pub struct HistoryBatch {
	id:   NonZeroU64,
	rows: Arc<[String]>,
	kind: HistoryKind,
}

impl HistoryBatch {
	pub fn new(id: NonZeroU64, rows: Vec<String>, kind: HistoryKind) -> Self {
		Self { id, rows: rows.into(), kind }
	}

	pub const fn id(&self) -> NonZeroU64 {
		self.id
	}

	pub fn rows(&self) -> &[String] {
		&self.rows
	}

	pub const fn kind(&self) -> HistoryKind {
		self.kind
	}
}

#[derive(Debug, Clone, Default)]
pub struct FramePlan {
	pub history:  Option<HistoryBatch>,
	pub viewport: Vec<String>,
}

/// Application-level Composer/TranscriptStore retain semantic ownership.
pub trait FrameProvider {
	fn render_frame(&mut self, size: ViewportSize) -> FramePlan;
	fn acknowledge_history(&mut self, id: NonZeroU64);
	/// Must defer replay behind a currently offered transaction. Replay gets a
	/// fresh monotonic ID, without rewinding the application's retirement state.
	fn begin_history_replay(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputAcceptance {
	Accepted,
	Backpressure,
}

pub trait FrameSink {
	/// Errors and backpressure must enqueue none of `packet`.
	fn write_frame(&mut self, packet: &[u8]) -> io::Result<OutputAcceptance>;
}

/// Hosts route these operations through their central Unicode/ANSI engine.
///
/// Never pass raw provider/tool text through unchanged. Returned rows may
/// contain trusted styling, but no cursor movement or embedded CR/LF.
pub trait TextLayout {
	fn fit_row(&mut self, row: &str, columns: usize) -> String;
	fn wrap_text(&mut self, text: &str, columns: usize) -> Vec<String>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ResizeMode {
	Rebuild,
	Append,
	#[default]
	Preserve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
	/// The offered history remains eligible and no state/ACK has changed.
	Deferred,
	Accepted {
		wrote:        bool,
		history_ack:  Option<NonZeroU64>,
		request_next: bool,
	},
}

#[derive(Debug)]
pub enum FrameError {
	InvalidSize,
	FrameTooLarge,
	InvalidLayoutRow,
	Output(io::Error),
}

impl fmt::Display for FrameError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::InvalidSize => {
				formatter.write_str("terminal dimensions must be between 1 and 65535")
			},
			Self::FrameTooLarge => formatter.write_str("terminal frame exceeds its output limit"),
			Self::InvalidLayoutRow => {
				formatter.write_str("terminal layout returned an embedded newline")
			},
			Self::Output(error) => write!(formatter, "terminal output rejected the frame: {error}"),
		}
	}
}

impl std::error::Error for FrameError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Output(error) => Some(error),
			_ => None,
		}
	}
}

/// Serializes one accepted history/viewport transaction into one sink call.
///
/// This first slice uses a hidden parked cursor; host lifecycle, probes,
/// overlays, graphics, and resize-anchor recovery remain separate work.
#[derive(Debug, Default)]
pub struct FrameWriter {
	accepted_history: u64,
	previous_size:    Option<ViewportSize>,
	viewport_top:     usize,
	window:           Vec<String>,
	destructive:      bool,
	resize_prepared:  Option<ViewportSize>,
	synchronized:     bool,
}

impl FrameWriter {
	pub fn new(synchronized: bool) -> Self {
		Self { synchronized, ..Self::default() }
	}

	pub const fn accepted_history_id(&self) -> u64 {
		self.accepted_history
	}

	/// Only explicit user/session resets and rebuild resize mode erase history.
	pub fn reset_display(&mut self, provider: &mut impl FrameProvider) {
		if !self.destructive {
			provider.begin_history_replay();
		}
		self.destructive = true;
	}

	/// Called once resize has settled and the host has recovered its anchor.
	/// Height-only append resizes do not create an identical ledger copy.
	pub fn settle_resize(
		&mut self,
		size: ViewportSize,
		mode: ResizeMode,
		provider: &mut impl FrameProvider,
	) {
		let Some(previous) = self.previous_size else {
			return;
		};
		if previous == size || self.resize_prepared == Some(size) || mode == ResizeMode::Preserve {
			return;
		}
		self.resize_prepared = Some(size);
		match mode {
			ResizeMode::Rebuild => self.reset_display(provider),
			ResizeMode::Append if previous.columns != size.columns => provider.begin_history_replay(),
			ResizeMode::Append | ResizeMode::Preserve => {},
		}
	}

	pub fn render(
		&mut self,
		size: ViewportSize,
		provider: &mut impl FrameProvider,
		sink: &mut impl FrameSink,
		layout: &mut impl TextLayout,
	) -> Result<FrameOutcome, FrameError> {
		if size.columns == 0 || size.rows == 0 || size.columns > 65_535 || size.rows > 65_535 {
			return Err(FrameError::InvalidSize);
		}
		let plan = provider.render_frame(size);
		let offered_id = plan.history.as_ref().map(HistoryBatch::id);
		let history = plan
			.history
			.filter(|batch| batch.id.get() > self.accepted_history);
		// A reset requested during an outstanding append cannot clear the
		// screen before the provider has produced its complete replay.
		let destructive = self.destructive
			&& history
				.as_ref()
				.is_none_or(|batch| batch.kind == HistoryKind::Replay);
		check_rows(history.as_ref().map_or(&[], HistoryBatch::rows))?;
		check_rows(&plan.viewport)?;
		let mut viewport = plan.viewport;
		if viewport.len() > size.rows {
			viewport.drain(..viewport.len() - size.rows);
		}
		let mut history_rows = history.as_ref().map_or(&[][..], HistoryBatch::rows);
		let mut replay_rows = 0;
		if history
			.as_ref()
			.is_some_and(|batch| batch.kind == HistoryKind::Replay)
		{
			let padding = size.rows - viewport.len();
			viewport.splice(..0, std::iter::repeat_n(String::new(), padding));
			let blanks = viewport
				.iter()
				.take_while(|row| row.trim().is_empty())
				.count();
			replay_rows = history_rows.len().min(blanks);
			let split = history_rows.len() - replay_rows;
			viewport.splice(..replay_rows, history_rows[split..].iter().cloned());
			history_rows = &history_rows[..split];
		}
		let prepared = prepare_rows(&viewport, size.columns, layout)?;
		let prepared_history = prepare_rows(history_rows, size.columns, layout)?;
		let start_top = if destructive {
			0
		} else {
			self.viewport_top.min(size.rows - 1)
		};
		let new_top = start_top
			.saturating_add(prepared_history.len())
			.min(size.rows - prepared.len());
		let stable = self.previous_size == Some(size);
		let diffable = stable
			&& prepared_history.is_empty()
			&& start_top == new_top
			&& !destructive
			&& !self.window.is_empty();
		let changed = self.window != prepared;
		let needs_write = history.is_some() || destructive || !stable || changed;
		if needs_write {
			let packet = self.serialize(Paint {
				size,
				history: &prepared_history,
				viewport: &prepared,
				start_top,
				new_top,
				replay_rows,
				diffable,
				destructive,
			})?;
			match sink
				.write_frame(packet.as_bytes())
				.map_err(FrameError::Output)?
			{
				OutputAcceptance::Backpressure => return Ok(FrameOutcome::Deferred),
				OutputAcceptance::Accepted => {},
			}
			// Commit output bookkeeping before invoking application callbacks.
			self.viewport_top = new_top + replay_rows;
			self.window = prepared.into_iter().skip(replay_rows).collect();
			self.previous_size = Some(size);
			if destructive {
				self.destructive = false;
			}
			self.resize_prepared = None;
			if let Some(history) = &history {
				self.accepted_history = history.id.get();
			}
		}
		if let Some(id) = offered_id {
			provider.acknowledge_history(id);
		}
		Ok(FrameOutcome::Accepted {
			wrote:        needs_write,
			history_ack:  offered_id,
			request_next: history.is_some_and(|batch| batch.kind == HistoryKind::Append),
		})
	}

	fn serialize(&self, paint: Paint<'_>) -> Result<String, FrameError> {
		let Paint { size, history, viewport, start_top, new_top, replay_rows, diffable, destructive } =
			paint;
		let mut packet = Packet::default();
		packet.push("\x1b[?25l")?;
		if self.synchronized {
			packet.push("\x1b[?2026h")?;
		}
		packet.push("\x1b[?7l")?;
		if destructive {
			// ED2 before ED3 also removes the screen that tmux archives on ED2.
			packet.push("\x1b[H\x1b[2J\x1b[3J")?;
		}
		if diffable {
			for (index, row) in viewport.iter().enumerate() {
				if self.window.get(index) != Some(row) {
					packet.position(new_top + index)?;
					packet.row(row)?;
				}
			}
		} else {
			let pushed = start_top
				.saturating_add(history.len())
				.saturating_add(viewport.len())
				.saturating_sub(size.rows);
			if pushed > self.viewport_top && !self.window.is_empty() {
				erase_live_rows(&mut packet, self.viewport_top, size.rows)?;
			}
			packet.position(start_top)?;
			for (index, row) in history.iter().chain(viewport).enumerate() {
				if index > 0 {
					packet.push("\r\n")?;
				}
				packet.row(row)?;
			}
		}
		if new_top + viewport.len() < size.rows && (!diffable || self.window.len() > viewport.len()) {
			packet.position(new_top + viewport.len())?;
			packet.push("\x1b[J")?;
		}
		packet.position((new_top + replay_rows).min(size.rows - 1))?;
		packet.push("\x1b[?7h")?;
		if self.synchronized {
			packet.push("\x1b[?2026l")?;
		}
		Ok(packet.0)
	}
}

struct Paint<'a> {
	size:        ViewportSize,
	history:     &'a [String],
	viewport:    &'a [String],
	start_top:   usize,
	new_top:     usize,
	replay_rows: usize,
	diffable:    bool,
	destructive: bool,
}

fn check_rows(rows: &[String]) -> Result<(), FrameError> {
	let mut bytes = 0_usize;
	for row in rows {
		bytes = bytes
			.saturating_add(row.len())
			.saturating_add(LINE_END.len() + 8);
		if row.len() > ROW_LIMIT || bytes > FRAME_LIMIT {
			return Err(FrameError::FrameTooLarge);
		}
	}
	Ok(())
}

fn prepare_rows(
	rows: &[String],
	columns: usize,
	layout: &mut impl TextLayout,
) -> Result<Vec<String>, FrameError> {
	let mut bytes = 0_usize;
	let mut prepared = Vec::with_capacity(rows.len().min(65_535));
	for row in rows {
		if row.len() > ROW_LIMIT {
			return Err(FrameError::FrameTooLarge);
		}
		let row = layout.fit_row(row, columns);
		if row.contains(['\r', '\n']) {
			return Err(FrameError::InvalidLayoutRow);
		}
		bytes = bytes.saturating_add(row.len());
		if bytes > FRAME_LIMIT || row.len() > ROW_LIMIT {
			return Err(FrameError::FrameTooLarge);
		}
		prepared.push(row);
	}
	Ok(prepared)
}

fn erase_live_rows(packet: &mut Packet, row: usize, height: usize) -> Result<(), FrameError> {
	let top = row.min(height - 1);
	packet.position(top)?;
	if top > 0 {
		return packet.push("\x1b[J");
	}
	packet.push("\x1b[2K")?;
	if height > 1 {
		packet.position(1)?;
		packet.push("\x1b[J")?;
		packet.position(0)?;
	}
	Ok(())
}

#[derive(Default)]
struct Packet(String);

impl Packet {
	fn push(&mut self, text: &str) -> Result<(), FrameError> {
		if self.0.len().saturating_add(text.len()) > FRAME_LIMIT {
			return Err(FrameError::FrameTooLarge);
		}
		self.0.push_str(text);
		Ok(())
	}

	fn position(&mut self, row: usize) -> Result<(), FrameError> {
		self.push(&format!("\x1b[{};1H", row + 1))
	}

	fn row(&mut self, row: &str) -> Result<(), FrameError> {
		self.push("\x1b[2K")?;
		self.push(row)?;
		self.push(LINE_END)
	}
}
