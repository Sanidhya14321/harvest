//! Read-only browser rendering of the portable session display projection.
//!
//! This first slice does not execute tools, provider requests, or persisted
//! JavaScript. The original session codec remains the replay authority.
use base64::{Engine, engine::general_purpose::STANDARD};
use harvest_protocol::display::{DisplayBlock, DisplayContent, DisplayEntry};

#[cfg(target_arch = "wasm32")]
mod browser;

/// Embedded raster images are the only image sources accepted by replay.
/// Reject SVG/HTML, URLs, malformed base64, and oversized inline payloads.
pub fn image_source(mime: &str, data: &str) -> Option<String> {
	if !matches!(mime, "image/png" | "image/jpeg" | "image/gif" | "image/webp")
		|| data.len() > 8 * 1024 * 1024
		|| STANDARD.decode(data).is_err()
	{
		return None;
	}
	Some(format!("data:{mime};base64,{data}"))
}

/// Search the display allowlist, never the opaque persisted provider payload.
pub fn searchable_text(entry: &DisplayEntry) -> String {
	let mut text = String::new();
	match &entry.content {
		DisplayContent::Message { message } => {
			text.push_str(&message.role);
			if let Some(name) = &message.tool_name {
				text.push_str(name);
			}
			append_blocks(&mut text, &message.blocks);
			if let Some(execution) = &message.execution {
				text.push_str(&execution.code);
				text.push_str(&execution.output);
			}
		},
		DisplayContent::CustomMessage { custom_type, blocks } => {
			text.push_str(custom_type);
			append_blocks(&mut text, blocks);
		},
		DisplayContent::Compaction { summary, short_summary, .. } => {
			text.push_str(summary);
			if let Some(short) = short_summary {
				text.push_str(short);
			}
		},
		DisplayContent::BranchSummary { summary, .. } => text.push_str(summary),
		DisplayContent::ModelChange { model, .. } => text.push_str(model),
		DisplayContent::ThinkingLevelChange { thinking_level } => {
			if let Some(level) = thinking_level {
				text.push_str(level);
			}
		},
		DisplayContent::Hidden | DisplayContent::Unknown { .. } => {},
	}
	text.to_lowercase()
}

fn append_blocks(text: &mut String, blocks: &[DisplayBlock]) {
	for block in blocks {
		text.push('\n');
		match block {
			DisplayBlock::Text { text: body } | DisplayBlock::Thinking { thinking: body } => {
				text.push_str(body);
			},
			DisplayBlock::ToolCall { name, arguments, intent, .. } => {
				text.push_str(name);
				if let Some(arguments) = arguments {
					text.push_str(arguments.get());
				}
				if let Some(intent) = intent {
					text.push_str(intent);
				}
			},
			DisplayBlock::Image { .. }
			| DisplayBlock::RedactedThinking
			| DisplayBlock::Unknown { .. } => {},
		}
	}
}
