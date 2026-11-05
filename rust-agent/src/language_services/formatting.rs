//! Per-file formatting: workspace EditorConfig, content indentation, fallback.
//! Reuses the shared glob normalizer and jailed reads; no persistent stale
//! cache.
use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use globset::GlobBuilder;
use harvest_runtime::JailedFs;
use serde_json::{Value, json};

const MAX_CONFIG_BYTES: usize = 256 * 1024;
const MAX_CHAIN_BYTES: usize = 1024 * 1024;
const MAX_CHAIN_DEPTH: usize = 64;
const MAX_SECTIONS: usize = 1024;

#[derive(Clone, Default)]
struct Hints {
	spaces:      Option<bool>,
	size:        Option<usize>,
	size_is_tab: bool,
	tab_width:   Option<usize>,
}
impl Hints {
	fn property(&mut self, key: &str, value: &str) {
		match key {
			"indent_style" if value == "space" => self.spaces = Some(true),
			"indent_style" if value == "tab" => self.spaces = Some(false),
			"indent_size" if value == "tab" => {
				self.size = None;
				self.size_is_tab = true;
			},
			"indent_size" => {
				if let Some(width) = positive_width(value) {
					self.size = Some(width);
					self.size_is_tab = false;
				}
			},
			"tab_width" => {
				if let Some(width) = positive_width(value) {
					self.tab_width = Some(width);
				}
			},
			_ => {},
		}
	}
}

fn positive_width(value: &str) -> Option<usize> {
	if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
		return None;
	}
	let width = value
		.bytes()
		.fold(0usize, |n, digit| (n.saturating_mul(10) + usize::from(digit - b'0')).min(16));
	(width != 0).then_some(width)
}

struct Section {
	pattern:    String,
	properties: BTreeMap<String, String>,
}
struct Configuration {
	root:     bool,
	sections: Vec<Section>,
}

fn parse(source: &str) -> Result<Configuration> {
	let mut config = Configuration { root: false, sections: Vec::new() };
	let mut current = None;
	for line in source.lines().map(str::trim) {
		if line.is_empty() || (line.starts_with('#') || line.starts_with(';')) {
			continue;
		}
		if let Some(pattern) = line
			.strip_prefix('[')
			.and_then(|line| line.strip_suffix(']'))
		{
			let pattern = pattern.trim();
			if pattern.is_empty() {
				current = None;
				continue;
			}
			ensure!(config.sections.len() < MAX_SECTIONS, "EditorConfig exceeds section limit");
			config
				.sections
				.push(Section { pattern: pattern.into(), properties: BTreeMap::new() });
			current = Some(config.sections.len() - 1);
			continue;
		}
		let Some((key, value)) = line.split_once('=') else {
			continue;
		};
		let key = key.trim().to_ascii_lowercase();
		let value = value.trim().to_ascii_lowercase();
		if let Some(index) = current {
			if matches!(key.as_str(), "indent_style" | "indent_size" | "tab_width") {
				config.sections[index].properties.insert(key, value);
			}
		} else if key == "root" {
			config.root = value == "true";
		}
	}
	Ok(config)
}

fn matches(pattern: &str, path: &Path) -> bool {
	let pattern = pattern.trim_start_matches('/');
	if pattern.is_empty() {
		return false;
	}
	let Some(path) = path.to_str() else {
		return false;
	};
	let path = path.replace('\\', "/");
	for recursive in [false, true] {
		if recursive && pattern.contains('/') {
			continue;
		}
		let normalized = pi_walker::glob::build_glob_pattern(pattern, recursive);
		if GlobBuilder::new(&normalized)
			.literal_separator(true)
			.build()
			.is_ok_and(|glob| glob.compile_matcher().is_match(&path))
		{
			return true;
		}
	}
	false
}

fn config_hints(fs: &JailedFs, path: &Path) -> Result<Hints> {
	let relative = fs.relative(path)?;
	let mut cursor = relative.parent().map(Path::to_owned);
	let mut chain: Vec<(PathBuf, Configuration)> = Vec::new();
	let mut visited = 0usize;
	let mut bytes_read = 0usize;
	while let Some(directory) = cursor {
		visited += 1;
		ensure!(visited <= MAX_CHAIN_DEPTH, "EditorConfig ancestor depth exceeds limit");
		if let Some(bytes) = fs.read_optional(&directory.join(".editorconfig"), MAX_CONFIG_BYTES)? {
			bytes_read = bytes_read
				.checked_add(bytes.len())
				.context("EditorConfig size overflow")?;
			ensure!(bytes_read <= MAX_CHAIN_BYTES, "EditorConfig chain exceeds byte limit");
			let config = parse(std::str::from_utf8(&bytes).context("EditorConfig is not UTF-8")?)?;
			let root = config.root;
			chain.push((directory.clone(), config));
			if root {
				break;
			}
		}
		cursor = directory.parent().map(Path::to_owned);
	}
	let mut hints = Hints::default();
	for (directory, config) in chain.into_iter().rev() {
		let relative = relative
			.strip_prefix(&directory)
			.context("invalid EditorConfig ancestor")?;
		for section in config.sections {
			if matches(&section.pattern, relative) {
				for (key, value) in section.properties {
					hints.property(&key, &value);
				}
			}
		}
	}
	Ok(hints)
}

fn detected(content: &str) -> (Option<bool>, Option<usize>) {
	let mut spaces = None;
	let mut unit = 0;
	for line in content.lines().filter(|line| !line.trim().is_empty()) {
		if line.starts_with('\t') {
			spaces.get_or_insert(false);
		} else if line.starts_with(' ') {
			spaces.get_or_insert(true);
			let mut count = line.bytes().take_while(|byte| *byte == b' ').count();
			if unit == 0 {
				unit = count;
			} else {
				while count != 0 {
					let remainder = unit % count;
					unit = count;
					count = remainder;
				}
			}
		}
	}
	(spaces, (spaces == Some(true) && unit > 0).then_some(unit))
}

/// Host-approved workspace scope only. Ancestor configurations outside that
/// capability require an explicit host formatting-options override.
pub(super) fn resolve(fs: &JailedFs, path: &Path, content: &str) -> Result<Value> {
	let hints = config_hints(fs, path)?;
	let (spaces, unit) = detected(content);
	let configured_spaces = hints.spaces.or_else(|| {
		if hints.size_is_tab {
			Some(false)
		} else {
			hints.size.map(|_| true)
		}
	});
	Ok(json!({
		 "tabSize": hints.size.or(hints.tab_width).or(unit).unwrap_or(2),
		 "insertSpaces": configured_spaces.or(spaces).unwrap_or(true),
		 "trimTrailingWhitespace": true, "insertFinalNewline": true, "trimFinalNewlines": true
	}))
}
