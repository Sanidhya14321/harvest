//! Project a selected session branch into model context without altering its
//! persisted history. Opaque compaction payloads require their own adapters.
use anyhow::{Context, Result, bail};
use handlebars::{Handlebars, no_escape};
use serde_json::json;

use crate::{
	chat::{self, ChatMessage},
	durable_session::replay,
	session_format::{SessionObject, decoded_string},
};

const COMPACTION: &str = include_str!("prompts/compaction-summary-context.md");
const HANDOFF: &str = include_str!("prompts/handoff-summary-context.md");
const BRANCH: &str = include_str!("prompts/branch-summary-context.md");

pub(crate) fn rebuild(path: &[&SessionObject]) -> Result<Vec<ChatMessage>> {
	let kinds = path
		.iter()
		.map(|entry| decoded_string(entry, "type"))
		.collect::<Result<Vec<_>>>()?;
	let compaction = kinds.iter().rposition(|kind| kind == "compaction");
	let reset = kinds.iter().rposition(|kind| kind == "reset_boundary");
	let mut messages = Vec::new();
	if let Some(reset) = reset.filter(|reset| compaction.is_none_or(|index| *reset > index)) {
		append(&path[reset + 1..], &mut messages)?;
	} else if let Some(index) = compaction {
		let entry = path[index];
		let summary = decoded_string(entry, "summary")?;
		let method = entry
			.get("method")
			.map(|raw| serde_json::from_str::<String>(raw.get()))
			.transpose()
			.context("invalid compaction method")?;
		if !matches!(method.as_deref(), None | Some("soft" | "handoff" | "shake")) {
			bail!("compaction method needs a replay adapter before it can be resumed");
		}
		if let Some(raw) = entry.get("preserveData") {
			let preserved: SessionObject =
				serde_json::from_str(raw.get()).context("invalid compaction preserveData")?;
			if !preserved.is_empty() {
				bail!("preserved compaction data needs a replay adapter before it can be resumed");
			}
		}
		if entry.contains_key("providerReplayThroughEntryId") || entry.contains_key("providerPayload")
		{
			bail!("provider compaction replay is not implemented");
		}
		let first_kept = decoded_string(entry, "firstKeptEntryId")?;
		// The compaction's own ID denotes an empty kept tail (imported logs).
		// Missing, sibling and forward references must never silently drop turns.
		let start = path[..=index]
			.iter()
			.position(|row| decoded_string(row, "id").is_ok_and(|id| id == first_kept))
			.context("compaction kept entry is not on the selected preceding branch")?;
		let template = if method.as_deref() == Some("handoff") {
			HANDOFF
		} else {
			COMPACTION
		};
		messages.push(summary_message(template, &summary)?);
		append(&path[start..index], &mut messages)?;
		append(&path[index + 1..], &mut messages)?;
	} else {
		append(path, &mut messages)?;
	}
	Ok(messages)
}

fn append(path: &[&SessionObject], messages: &mut Vec<ChatMessage>) -> Result<()> {
	for entry in path {
		let kind = decoded_string(entry, "type")?;
		match kind.as_str() {
			"message" => messages.push(replay(
				entry
					.get("message")
					.context("missing session message")?
					.as_ref(),
			)?),
			"branch_summary" => {
				let summary = decoded_string(entry, "summary")?;
				if !summary.is_empty() {
					messages.push(summary_message(BRANCH, &summary)?);
				}
			},
			// Previous rewrites inside the kept range are superseded by the latest
			// summary. Metadata has no model-message payload.
			"compaction"
			| "reset_boundary"
			| "model_change"
			| "thinking_level_change"
			| "service_tier_change"
			| "model_usage"
			| "title_change" => {},
			_ => bail!("session entry {kind} needs a replay adapter before it can be resumed"),
		}
	}
	Ok(())
}

fn summary_message(template: &str, summary: &str) -> Result<ChatMessage> {
	let mut renderer = Handlebars::new();
	renderer.register_escape_fn(no_escape);
	// Never compile the summary itself: braces and HTML are literal memory.
	// Preserve summary whitespace; general legacy prompt formatting is pending.
	let content = renderer.render_template(template.trim_end(), &json!({"summary":summary}))?;
	Ok(chat::user(content))
}
