//! Read-only semantic rendering for portable session display projections.
//!
//! Every visible string passes through the host's shared text-layout engine.
//! This text fallback deliberately does not execute tools, emit raw image
//! payloads, or forward provider replay metadata to the terminal.
use harvest_protocol::display::{
	DisplayBlock, DisplayContent, DisplayEntry, DisplayExecution, ExecutionLanguage,
	ReplayProjection,
};

use crate::frame::TextLayout;

#[derive(Debug, Clone, Copy)]
pub struct ReplayLimits {
	pub text_bytes:         usize,
	pub layout_chunk_bytes: usize,
	pub rows:               usize,
	pub output_bytes:       usize,
}

impl Default for ReplayLimits {
	fn default() -> Self {
		Self {
			text_bytes:         32 * 1024 * 1024,
			layout_chunk_bytes: 64 * 1024,
			rows:               65_535,
			output_bytes:       16 * 1024 * 1024,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayError {
	TextLimit,
	LayoutChunkLimit,
	RowLimit,
	OutputLimit,
	InvalidLayoutRow,
}

impl std::fmt::Display for ReplayError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str(match self {
			Self::TextLimit => "terminal replay exceeds its displayed-text limit",
			Self::LayoutChunkLimit => "terminal replay contains an oversized text layout chunk",
			Self::RowLimit => "terminal replay exceeds its row limit",
			Self::OutputLimit => "terminal replay exceeds its output-byte limit",
			Self::InvalidLayoutRow => "terminal replay layout returned an embedded newline",
		})
	}
}

impl std::error::Error for ReplayError {}

/// Render one selected causal branch without changing session retirement.
/// The application can offer these rows as one complete history replay.
pub fn render_projection(
	projection: &ReplayProjection,
	columns: usize,
	layout: &mut impl TextLayout,
) -> Result<Vec<String>, ReplayError> {
	render_projection_bounded(projection, columns, layout, ReplayLimits::default())
}

/// Reject overflow without returning an incomplete transcript.
///
/// Input is preflighted before layout expansion or heading allocation.
/// Layout runs one bounded logical line at a time, rather than on a complete
/// large session message. A trusted host adapter must also bound its own
/// per-call expansion; the input chunk limit bounds its maximum input.
pub fn render_projection_bounded(
	projection: &ReplayProjection,
	columns: usize,
	layout: &mut impl TextLayout,
	limits: ReplayLimits,
) -> Result<Vec<String>, ReplayError> {
	preflight(projection, limits)?;
	let mut rows = BoundedRows { rows: Vec::new(), bytes: 0, limits, error: None };
	for entry in projection.entries.iter().filter(|entry| entry.is_visible()) {
		let before = rows.rows.len();
		if before > 0 {
			rows.push(String::new());
		}
		render_entry(entry, columns.max(1), layout, &mut rows);
		if rows.error.is_some() {
			break;
		}
		if rows.rows.len() == before + usize::from(before > 0) {
			if rows.rows.len() > before {
				rows.bytes -= 2;
			}
			rows.rows.truncate(before);
		}
	}
	match rows.error {
		Some(error) => Err(error),
		None => Ok(rows.rows),
	}
}

fn render_entry(
	entry: &DisplayEntry,
	columns: usize,
	layout: &mut impl TextLayout,
	rows: &mut BoundedRows,
) {
	match &entry.content {
		DisplayContent::Message { message } => {
			let heading = match (&message.tool_name, &message.tool_call_id) {
				(Some(name), Some(id)) => format!("Tool result: {name} [{id}]"),
				(None, Some(id)) => format!("Tool result [{id}]"),
				_ => message.role.clone(),
			};
			text(&heading, columns, layout, rows);
			for block in &message.blocks {
				render_block(block, columns, layout, rows);
			}
			if let Some(execution) = &message.execution {
				render_execution(execution, columns, layout, rows);
			}
			if let Some(error) = &message.error_message {
				text(error, columns, layout, rows);
			}
			if message.is_error {
				text("Tool reported an error", columns, layout, rows);
			}
		},
		DisplayContent::CustomMessage { custom_type, blocks } => {
			text(custom_type, columns, layout, rows);
			for block in blocks {
				render_block(block, columns, layout, rows);
			}
		},
		DisplayContent::Compaction {
			summary, short_summary, tokens_before, tokens_after, ..
		} => {
			text("Context compaction", columns, layout, rows);
			if let Some(short) = short_summary {
				text(short, columns, layout, rows);
			}
			text(summary, columns, layout, rows);
			if let Some((before, after)) = tokens_before.zip(*tokens_after) {
				text(&format!("{before} → {after} tokens"), columns, layout, rows);
			}
		},
		DisplayContent::BranchSummary { summary, .. } => {
			text("Branch summary", columns, layout, rows);
			text(summary, columns, layout, rows);
		},
		DisplayContent::ModelChange { model, role } => {
			text(&format!("Model changed: {model}"), columns, layout, rows);
			if let Some(role) = role {
				text(role, columns, layout, rows);
			}
		},
		DisplayContent::ThinkingLevelChange { thinking_level } => {
			text(
				&format!("Thinking level: {}", thinking_level.as_deref().unwrap_or("default")),
				columns,
				layout,
				rows,
			);
		},
		DisplayContent::Hidden | DisplayContent::Unknown { .. } => {},
	}
}

fn render_block(
	block: &DisplayBlock,
	columns: usize,
	layout: &mut impl TextLayout,
	rows: &mut BoundedRows,
) {
	match block {
		DisplayBlock::Text { text: body } => text(body, columns, layout, rows),
		DisplayBlock::Thinking { thinking } => {
			text("Reasoning:", columns, layout, rows);
			text(thinking, columns, layout, rows);
		},
		DisplayBlock::Image { mime_type, .. } => {
			text(&format!("[Image: {mime_type}]"), columns, layout, rows);
		},
		DisplayBlock::ToolCall { id, name, arguments, intent } => {
			text(&format!("Tool call: {name} [{id}]"), columns, layout, rows);
			if let Some(intent) = intent {
				text(intent, columns, layout, rows);
			}
			if let Some(arguments) = arguments {
				text(arguments.get(), columns, layout, rows);
			}
		},
		DisplayBlock::RedactedThinking => text("[Reasoning redacted]", columns, layout, rows),
		DisplayBlock::Unknown { .. } => {},
	}
}

fn render_execution(
	execution: &DisplayExecution,
	columns: usize,
	layout: &mut impl TextLayout,
	rows: &mut BoundedRows,
) {
	let language = match execution.language {
		ExecutionLanguage::Shell => "Shell",
		ExecutionLanguage::Python => "Python",
		ExecutionLanguage::Javascript => "Historical JavaScript",
	};
	text(&format!("{language} execution"), columns, layout, rows);
	text(&execution.code, columns, layout, rows);
	text(&execution.output, columns, layout, rows);
	if let Some(code) = execution.exit_code {
		text(&format!("Exit code: {code}"), columns, layout, rows);
	}
	if execution.cancelled {
		text("Cancelled", columns, layout, rows);
	}
	if execution.truncated {
		text("Output truncated", columns, layout, rows);
	}
}

fn text(value: &str, columns: usize, layout: &mut impl TextLayout, rows: &mut BoundedRows) {
	for line in value.split('\n') {
		if rows.error.is_some() {
			return;
		}
		if line.len() > rows.limits.layout_chunk_bytes {
			rows.error = Some(ReplayError::LayoutChunkLimit);
			return;
		}
		for row in layout.wrap_text(line, columns) {
			rows.push(row);
			if rows.error.is_some() {
				return;
			}
		}
	}
}

struct BoundedRows {
	rows:   Vec<String>,
	bytes:  usize,
	limits: ReplayLimits,
	error:  Option<ReplayError>,
}

impl BoundedRows {
	fn push(&mut self, row: String) {
		if self.error.is_some() {
			return;
		}
		let bytes = self.bytes.saturating_add(row.len()).saturating_add(2);
		self.error = if row.contains(['\r', '\n']) {
			Some(ReplayError::InvalidLayoutRow)
		} else if self.rows.len() >= self.limits.rows {
			Some(ReplayError::RowLimit)
		} else if bytes > self.limits.output_bytes {
			Some(ReplayError::OutputLimit)
		} else {
			None
		};
		if self.error.is_none() {
			self.bytes = bytes;
			self.rows.push(row);
		}
	}
}

struct TextBudget {
	bytes:  usize,
	limits: ReplayLimits,
}

impl TextBudget {
	fn text(&mut self, text: &str) -> Result<(), ReplayError> {
		self.bytes = self.bytes.saturating_add(text.len());
		if self.bytes > self.limits.text_bytes {
			return Err(ReplayError::TextLimit);
		}
		if text
			.split('\n')
			.any(|line| line.len() > self.limits.layout_chunk_bytes)
		{
			return Err(ReplayError::LayoutChunkLimit);
		}
		Ok(())
	}

	fn blocks(&mut self, blocks: &[DisplayBlock]) -> Result<(), ReplayError> {
		for block in blocks {
			match block {
				DisplayBlock::Text { text } | DisplayBlock::Thinking { thinking: text } => {
					self.text(text)?;
				},
				DisplayBlock::Image { mime_type, .. } => self.text(mime_type)?,
				DisplayBlock::ToolCall { id, name, arguments, intent } => {
					self.text(id)?;
					self.text(name)?;
					if let Some(arguments) = arguments {
						self.text(arguments.get())?;
					}
					if let Some(intent) = intent {
						self.text(intent)?;
					}
				},
				DisplayBlock::RedactedThinking | DisplayBlock::Unknown { .. } => {},
			}
		}
		Ok(())
	}
}

fn preflight(projection: &ReplayProjection, limits: ReplayLimits) -> Result<(), ReplayError> {
	let mut budget = TextBudget { bytes: 0, limits };
	for entry in projection.entries.iter().filter(|entry| entry.is_visible()) {
		match &entry.content {
			DisplayContent::Message { message } => {
				budget.text(&message.role)?;
				if let Some(name) = &message.tool_name {
					budget.text(name)?;
				}
				if let Some(id) = &message.tool_call_id {
					budget.text(id)?;
				}
				if let Some(error) = &message.error_message {
					budget.text(error)?;
				}
				budget.blocks(&message.blocks)?;
				if let Some(execution) = &message.execution {
					budget.text(&execution.code)?;
					budget.text(&execution.output)?;
				}
			},
			DisplayContent::CustomMessage { custom_type, blocks } => {
				budget.text(custom_type)?;
				budget.blocks(blocks)?;
			},
			DisplayContent::Compaction { summary, short_summary, .. } => {
				budget.text(summary)?;
				if let Some(short) = short_summary {
					budget.text(short)?;
				}
			},
			DisplayContent::BranchSummary { summary, .. } => budget.text(summary)?,
			DisplayContent::ModelChange { model, role } => {
				budget.text(model)?;
				if let Some(role) = role {
					budget.text(role)?;
				}
			},
			DisplayContent::ThinkingLevelChange { thinking_level } => {
				if let Some(level) = thinking_level {
					budget.text(level)?;
				}
			},
			DisplayContent::Hidden | DisplayContent::Unknown { .. } => {},
		}
	}
	Ok(())
}
