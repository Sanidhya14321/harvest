//! Native command/path completion. Filesystem discovery stays bounded and
//! workspace-jailed; editor token publication revalidates the captured draft.

use std::{path::Path, sync::Arc};

use anyhow::{Result, ensure};
use harvest_runtime::JailedFs;
use harvest_tui::{
	editor::CompletionContext,
	sanitize::{replace_tabs, shorten_path, truncate_to_width},
};

use super::history::History;

const MAX_DIRECTORY_ENTRIES: usize = 4096;
const MAX_ITEMS: usize = 100;
const MAX_VALUE_BYTES: usize = 16 * 1024;

pub(super) const COMMANDS: &[(&str, &str)] = &[
	("/help", "Native chat commands and editor keys"),
	("/history", "Find a prompt in native persistent history"),
	("/skills", "Choose an enabled skill for explicit invocation"),
	("/clear", "Clear this terminal's transcript display"),
	("/resume", "Resume follow-ups parked after interruption"),
	("/exit", "Close the native chat session"),
	("/quit", "Close the native chat session"),
];

pub(super) struct Item {
	pub label:       String,
	pub value:       String,
	pub description: String,
	pub directory:   bool,
}

pub(super) struct Suggestions {
	pub context:            CompletionContext,
	pub replace_characters: usize,
	pub items:              Vec<Item>,
	pub history:            bool,
}

pub(super) struct Menu {
	pub suggestions: Suggestions,
	pub selected:    usize,
}

impl Menu {
	pub fn select(&mut self, offset: isize) {
		if self.suggestions.items.is_empty() {
			return;
		}
		let size = self.suggestions.items.len();
		self.selected = if offset < 0 {
			self.selected.saturating_sub(offset.unsigned_abs())
		} else {
			self.selected.saturating_add(offset as usize).min(size - 1)
		};
	}

	pub fn render(&self, width: usize, max_rows: usize) -> Vec<String> {
		let rows = max_rows.min(9);
		if rows == 0 {
			return Vec::new();
		}
		let count = rows.saturating_sub(1);
		let start = self.selected.saturating_sub(count.saturating_sub(1));
		let mut lines = Vec::new();
		for (index, item) in self
			.suggestions
			.items
			.iter()
			.enumerate()
			.skip(start)
			.take(count)
		{
			let marker = if index == self.selected { "› " } else { "  " };
			let label = format!("{marker}{}  {}", item.label, item.description);
			lines.push(truncate_to_width(
				&shorten_path(&replace_tabs(&label).replace('\n', " ")),
				width,
			));
		}
		lines.push(truncate_to_width("↑/↓ choose · Tab/Enter insert · Esc cancel", width));
		lines
	}
}

pub(super) fn paths(
	fs: Arc<JailedFs>,
	context: CompletionContext,
	directory: Option<std::path::PathBuf>,
) -> Result<Suggestions> {
	let before: String = context.line.chars().take(context.column).collect();
	let trimmed = before.trim_start();
	if trimmed.starts_with('/') && !trimmed.chars().any(char::is_whitespace) {
		let mut items: Vec<_> = COMMANDS
			.iter()
			.filter(|(name, _)| name.starts_with(trimmed))
			.map(|(name, description)| Item {
				label:       (*name).into(),
				value:       format!("{name} "),
				description: (*description).into(),
				directory:   false,
			})
			.collect();
		// Discovery is on the existing bounded completion worker; publication
		// still checks the captured draft and cursor before offering candidates.
		if "/skill:".starts_with(trimmed) || trimmed.starts_with("/skill:") {
			let directory = crate::config::agent_dir_from(directory, |key| std::env::var_os(key));
			let settings = crate::settings::Settings::load(directory.as_deref(), fs.root(), &[])?;
			for (name, description) in
				crate::managed::catalog::commands(fs.root(), directory.as_deref(), &settings, trimmed)?
			{
				if name.starts_with(trimmed) && items.len() < MAX_ITEMS {
					items.push(Item {
						label: name.clone(),
						value: format!("{name} "),
						description,
						directory: false,
					});
				}
			}
		}
		if !items.is_empty() || trimmed.starts_with("/skill:") {
			return Ok(Suggestions {
				context,
				replace_characters: trimmed.chars().count(),
				items,
				history: false,
			});
		}
	}
	let token = token_prefix(&before);
	ensure!(token.len() <= MAX_VALUE_BYTES, "completion token exceeds its byte limit");
	let replace_characters = token.chars().count();
	let (at, token) = token
		.strip_prefix('@')
		.map_or((false, token), |value| (true, value));
	let quoted = token.starts_with('"');
	let closing_quote_exists = context.line.chars().nth(context.column) == Some('"');
	let raw = if quoted {
		unquote(token)
	} else {
		token.to_owned()
	};
	#[cfg(windows)]
	let raw = raw.replace('\\', "/");
	let split = raw.rfind('/').map_or(0, |index| index + 1);
	let (parent, prefix) = raw.split_at(split);
	let directory = if parent.is_empty() {
		Path::new(".")
	} else {
		Path::new(parent)
	};
	let relative = fs.relative(directory)?;
	let mut items = Vec::new();
	for entry in fs.entries(&relative, MAX_DIRECTORY_ENTRIES)? {
		if entry.is_symlink
			|| !(entry.is_file || entry.is_directory)
			|| !entry.name.starts_with(prefix)
		{
			continue;
		}
		if entry.name.chars().any(char::is_control) {
			continue;
		}
		let mut path = format!("{parent}{}", entry.name);
		if entry.is_directory {
			path.push('/');
		}
		if path.len() > MAX_VALUE_BYTES {
			continue;
		}
		let needs_quotes = quoted
			|| path
				.chars()
				.any(|character| character.is_whitespace() || character == '"');
		let mut value = if needs_quotes {
			format!(
				"{}\"{}{}",
				if at { "@" } else { "" },
				path.replace('\\', "\\\\").replace('"', "\\\""),
				if entry.is_directory || closing_quote_exists {
					""
				} else {
					"\""
				}
			)
		} else {
			format!("{}{path}", if at { "@" } else { "" })
		};
		if at && !entry.is_directory {
			value.push(' ');
		}
		items.push(Item {
			label: path,
			value,
			description: if entry.is_directory {
				"directory"
			} else {
				"file"
			}
			.into(),
			directory: entry.is_directory,
		});
	}
	items.sort_by(|left, right| {
		right
			.directory
			.cmp(&left.directory)
			.then_with(|| left.label.cmp(&right.label))
	});
	items.truncate(MAX_ITEMS);
	Ok(Suggestions { context, replace_characters, items, history: false })
}

pub(super) fn history(
	store: Arc<History>,
	context: CompletionContext,
	query: String,
) -> Result<Suggestions> {
	let items = store
		.search(&query, MAX_ITEMS)?
		.into_iter()
		.map(|entry| Item {
			label:       entry.prompt.lines().next().unwrap_or_default().to_owned(),
			value:       entry.prompt,
			description: entry.cwd,
			directory:   false,
		})
		.collect();
	Ok(Suggestions { context, replace_characters: 0, items, history: true })
}

fn token_prefix(before: &str) -> &str {
	let mut start = 0;
	let mut quoted = false;
	let mut escape = false;
	for (index, character) in before.char_indices() {
		if escape {
			escape = false;
			continue;
		}
		if quoted && character == '\\' {
			escape = true;
			continue;
		}
		if character == '"' {
			quoted = !quoted;
			continue;
		}
		if !quoted && (character.is_whitespace() || matches!(character, '=' | '\'')) {
			start = index + character.len_utf8();
		}
	}
	&before[start..]
}

fn unquote(token: &str) -> String {
	let token = token.strip_prefix('"').unwrap_or(token);
	let token = token.strip_suffix('"').unwrap_or(token);
	let mut output = String::new();
	let mut characters = token.chars();
	while let Some(character) = characters.next() {
		if character == '\\' {
			if let Some(next) = characters.next() {
				output.push(next);
			} else {
				output.push(character);
			}
		} else {
			output.push(character);
		}
	}
	output
}
