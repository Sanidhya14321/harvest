//! Explicit user skill invocation. A versioned user-content envelope preserves
//! the selected body for replay and the original request for memory learning.
use std::{path::Path, sync::LazyLock};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

const KIND: &str = "harvest_skill_invocation_v1";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Invocation {
	kind:            String,
	original_prompt: String,
	skill:           String,
	instructions:    String,
}

fn parse(text: &str) -> Option<(&str, String)> {
	let trimmed = text.trim_start();
	if let Some(rest) = trimmed.strip_prefix("/skill:") {
		let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
		let name = &rest[..end];
		return (!name.is_empty()).then(|| (name, rest[end..].trim().to_owned()));
	}
	if trimmed.starts_with(['/', '!']) {
		return None;
	}
	if trimmed.starts_with('$') {
		let tail = trimmed.strip_prefix("$$").unwrap_or(&trimmed[1..]);
		if tail.is_empty() || tail.starts_with([' ', '\t', '\n', '\r']) {
			return None;
		}
	}
	static TOKEN: LazyLock<regex::Regex> = LazyLock::new(|| {
		regex::Regex::new(r"(?:^|\s)/skill:([^\s/]+)(?:\s|$)").expect("skill invocation grammar")
	});
	let captures = TOKEN.captures(text)?;
	let name = captures.get(1)?;
	let token_start = name.start() - "/skill:".len();
	let before = text[..token_start].trim_end();
	let after = text[name.end()..].trim_start();
	let args = [before, after]
		.into_iter()
		.filter(|s| !s.is_empty())
		.collect::<Vec<_>>()
		.join(" ");
	Some((name.as_str(), args))
}

pub(crate) fn prepare(
	cwd: &Path,
	directory: Option<&Path>,
	settings: &crate::settings::Settings,
	prompt: &str,
) -> Result<Option<String>> {
	if !settings.bool("skills.enabled")
		|| !settings.bool("skills.enableSkillCommands")
		|| !prompt.contains("/skill:")
	{
		return Ok(None);
	}
	ensure!(
		prompt.len() <= harvest_runtime::MAX_EVENT_BYTES / 2,
		"skill input exceeds session frame budget"
	);
	let Some((name, args)) = parse(prompt) else {
		return Ok(None);
	};
	if name.is_empty()
		|| name.len() > 128
		|| !name
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
	{
		return Ok(None);
	}
	let selected = match super::catalog::read(cwd, directory, settings, &format!("skill://{name}")) {
		Ok(selected) => selected,
		Err(error)
			if error
				.downcast_ref::<super::catalog::SkillNotFound>()
				.is_some() =>
		{
			return Ok(None);
		},
		Err(error) => return Err(error),
	};
	let details = selected
		.details
		.context("skill invocation metadata unavailable")?;
	let path = details["path"]
		.as_str()
		.context("skill invocation path unavailable")?;
	let base = Path::new(path)
		.parent()
		.context("skill invocation directory unavailable")?;
	let mut templates = handlebars::Handlebars::new();
	templates.register_escape_fn(handlebars::no_escape);
	let instructions = templates.render_template(
		include_str!("../prompts/skill-invocation-native.md"),
		&serde_json::json!({"name":name,"body":selected.content,"baseDir":base,"userArgs":args}),
	)?;
	let value = Invocation {
		kind: KIND.into(),
		original_prompt: prompt.into(),
		skill: name.into(),
		instructions,
	};
	let mut writer = harvest_runtime::BoundedWriter::new(harvest_runtime::MAX_EVENT_BYTES / 2);
	serde_json::to_writer(&mut writer, &value)
		.context("skill invocation exceeds session frame budget")?;
	Ok(Some(String::from_utf8(writer.into_inner())?))
}

/// Only recognize the exact native versioned envelope. Ordinary user JSON and
/// malformed/unknown versions remain ordinary conversation text.
pub(crate) fn original_prompt(text: &str) -> Option<String> {
	if text.len() > harvest_runtime::MAX_EVENT_BYTES / 2 || !text.starts_with('{') {
		return None;
	}
	let value: Invocation = serde_json::from_str(text).ok()?;
	(value.kind == KIND).then_some(value.original_prompt)
}
