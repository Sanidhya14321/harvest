//! The `ast_grep` tool: structural search over the shared `pi_ast::engine`
//! with the legacy scope resolution, paging and grouped output.
use std::{collections::HashMap, path::Path};

use anyhow::{Result, anyhow, bail};
use harvest_runtime::{JailedFs, cancel::CancelToken};
use pi_ast::engine::{self, AstFindMatch, AstFindOptions};
use pi_edit::{
	EditStore, modes::hashline::format::format_hashline_header, path_policy::canonical_key,
};
use serde::Deserialize;

use crate::search::{
	Workspace, expand_entries, format_grouped_files, posix_relative, resolve_scope, to_path_list,
};

const DEFAULT_LIMIT: u32 = 50;
/// `PARSE_ERRORS_LIMIT`.
const PARSE_ERRORS_LIMIT: usize = 20;

#[derive(Debug, Deserialize)]
pub(crate) struct Args {
	pat:  String,
	path: Option<String>,
	skip: Option<f64>,
}

struct Found {
	matches:            Vec<AstFindMatch>,
	total_matches:      u32,
	files_with_matches: u32,
	files_searched:     u32,
	limit_reached:      bool,
	parse_errors:       Vec<String>,
}

fn find(
	pattern: &str,
	path: &Path,
	glob: Option<&str>,
	offset: u32,
	limit: Option<u32>,
) -> Result<Found> {
	let result = engine::ast_grep(
		AstFindOptions {
			patterns: Some(vec![pattern.to_owned()]),
			lang: None,
			path: Some(path.to_string_lossy().into_owned()),
			glob: glob.map(str::to_owned),
			selector: None,
			strictness: None,
			limit,
			offset: Some(offset),
			include_meta: Some(true),
			context: None,
		},
		CancelToken::default(),
	)
	.map_err(|error| anyhow!("{error}"))?;
	Ok(Found {
		matches:            result.matches,
		total_matches:      result.total_matches,
		files_with_matches: result.files_with_matches,
		files_searched:     result.files_searched,
		limit_reached:      result.limit_reached,
		parse_errors:       result.parse_errors.unwrap_or_default(),
	})
}

fn match_order(a: &AstFindMatch, b: &AstFindMatch) -> std::cmp::Ordering {
	a.path
		.cmp(&b.path)
		.then(a.start_line.cmp(&b.start_line))
		.then(a.start_column.cmp(&b.start_column))
		.then(a.end_line.cmp(&b.end_line))
		.then(a.end_column.cmp(&b.end_column))
		.then(a.byte_start.cmp(&b.byte_start))
		.then(a.byte_end.cmp(&b.byte_end))
}

/// `dedupeParseErrors` + `capParseErrors` + `formatParseErrors`.
fn parse_error_lines(errors: &[String]) -> Vec<String> {
	let mut unique: Vec<&String> = Vec::new();
	for error in errors {
		if !unique.contains(&error) {
			unique.push(error);
		}
	}
	if unique.is_empty() {
		return Vec::new();
	}
	let shown = &unique[..unique.len().min(PARSE_ERRORS_LIMIT)];
	let header = if unique.len() > shown.len() {
		format!("Parse issues ({} / {}):", shown.len(), unique.len())
	} else {
		"Parse issues:".to_owned()
	};
	std::iter::once(header)
		.chain(shown.iter().map(|error| format!("- {error}")))
		.collect()
}

/// `^.+: (.+: parse error \(syntax tree contains error nodes\))$` → group 1.
fn normalize_parse_error(error: &str) -> String {
	const SUFFIX: &str = ": parse error (syntax tree contains error nodes)";
	if let Some(head) = error.strip_suffix(SUFFIX)
		&& let Some((_, file)) = head.split_once(": ")
		&& !file.is_empty()
	{
		return format!("{file}{SUFFIX}");
	}
	error.to_owned()
}

pub(crate) fn ast_grep(
	root: &Path,
	args: &Args,
	store: &EditStore,
	cancellation: &CancelToken,
) -> Result<String> {
	cancellation.heartbeat()?;
	let pattern = args.pat.trim();
	if pattern.is_empty() {
		bail!("`pat` must be a non-empty pattern");
	}
	let skip = match args.skip {
		None => 0,
		Some(skip) if skip.is_finite() && skip.floor() >= 0.0 => skip.floor() as u32,
		Some(_) => bail!("skip must be a non-negative number"),
	};
	let fs = JailedFs::open(root)?;
	let ws = Workspace { root: fs.root().to_path_buf(), fs };
	let scoped = to_path_list(args.path.as_deref());
	let raw = if scoped.is_empty() {
		vec![".".to_owned()]
	} else {
		scoped
	};
	let entries = expand_entries(&ws, &raw)?;
	for entry in &entries {
		if crate::search::is_url_like(entry) {
			bail!("Internal URLs are not supported by this host: {entry}");
		}
	}
	let scope = resolve_scope(&ws, &entries, false)?;
	let found = match &scope.targets {
		None => {
			find(pattern, &ws.engine_path(&scope.search_path)?, scope.glob.as_deref(), skip, None)?
		},
		Some(targets) => {
			let capacity = skip + DEFAULT_LIMIT + 1;
			let mut merged = Found {
				matches:            Vec::new(),
				total_matches:      0,
				files_with_matches: 0,
				files_searched:     0,
				limit_reached:      false,
				parse_errors:       Vec::new(),
			};
			for target in targets {
				cancellation.heartbeat()?;
				let base = ws.engine_path(&target.base)?;
				let result = find(pattern, &base, target.glob.as_deref(), 0, Some(capacity))?;
				merged.total_matches += result.total_matches;
				merged.files_with_matches += result.files_with_matches;
				merged.files_searched += result.files_searched;
				merged.limit_reached |= result.limit_reached;
				merged.parse_errors.extend(result.parse_errors);
				let is_dir = base.is_dir();
				for mut m in result.matches {
					let absolute = if is_dir {
						target.base.join(&m.path)
					} else {
						target.base.clone()
					};
					m.path = posix_relative(&scope.search_path, &absolute);
					merged.matches.push(m);
				}
			}
			merged.matches.sort_by(match_order);
			merged.matches.truncate(capacity as usize);
			let visible: Vec<AstFindMatch> = merged.matches.drain(..).skip(skip as usize).collect();
			merged.limit_reached |= visible.len() > DEFAULT_LIMIT as usize;
			merged.matches = visible.into_iter().take(DEFAULT_LIMIT as usize).collect();
			merged
		},
	};
	cancellation.heartbeat()?;
	let parse_errors: Vec<String> = found
		.parse_errors
		.iter()
		.map(|e| normalize_parse_error(e))
		.collect();
	let parse_lines = parse_error_lines(&parse_errors);
	if found.matches.is_empty() {
		let mut text = if parse_lines.is_empty() {
			"No matches found".to_owned()
		} else {
			"No matches found. Parse issues mean the query may be mis-scoped; narrow `path` before \
			 concluding absence."
				.to_owned()
		};
		if !parse_lines.is_empty() {
			text.push('\n');
			text.push_str(&parse_lines.join("\n"));
		}
		return Ok(text);
	}

	let format_path = |match_path: &str| -> String {
		if scope.is_directory {
			ws.display(&scope.search_path.join(match_path.trim_start_matches('/')))
		} else {
			ws.display(&scope.search_path)
		}
	};
	let mut files: Vec<String> = Vec::new();
	let mut by_file: HashMap<String, Vec<AstFindMatch>> = HashMap::new();
	for m in found.matches {
		let display = format_path(&m.path);
		by_file
			.entry(display.clone())
			.or_insert_with(|| {
				files.push(display.clone());
				Vec::new()
			})
			.push(m);
	}
	let mut tags: HashMap<String, String> = HashMap::new();
	for file in &files {
		if let Some(tag) = store.record_file(&ws.root.join(file), None) {
			tags.insert(file.clone(), tag);
		}
	}
	let render = |file: &str| -> Vec<String> {
		let hashed = tags.contains_key(file);
		let separator = if hashed { ':' } else { '|' };
		let mut out = Vec::new();
		for m in by_file.get(file).map_or(&[][..], Vec::as_slice) {
			for (index, line) in m.text.split('\n').enumerate() {
				let marker = if index == 0 { '*' } else { ' ' };
				out.push(format!("{marker}{}{separator}{line}", m.start_line as usize + index));
			}
			if let Some(meta) = m.meta_variables.as_ref().filter(|meta| !meta.is_empty()) {
				let mut pairs: Vec<(&String, &String)> = meta.iter().collect();
				pairs.sort_by(|a, b| crate::tree::locale_compare(a.0, b.0));
				let serialized: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect();
				out.push(format!("  meta: {}", serialized.join(", ")));
			}
		}
		if let Some(tag) = tags.get(file) {
			let seen = pi_edit::store::seen_lines_from_body(&out.join("\n"));
			store.record_seen_lines(&canonical_key(&ws.root.join(file)), tag, &seen);
		}
		out
	};
	let mut output: Vec<String> = Vec::new();
	if scope.is_directory {
		output.extend(format_grouped_files(&files, &mut |file| {
			let suffix = tags
				.get(file)
				.map_or_else(String::new, |tag| format!("#{tag}"));
			(render(file), suffix)
		}));
	} else {
		for file in &files {
			let body = render(file);
			if body.is_empty() {
				continue;
			}
			if !output.is_empty() {
				output.push(String::new());
			}
			if let Some(tag) = tags.get(file) {
				output.push(format_hashline_header(file, tag));
			}
			output.extend(body);
		}
	}
	if found.limit_reached {
		output.push(String::new());
		output.push("Result limit reached; narrow path or increase limit.".to_owned());
	}
	if !parse_lines.is_empty() {
		output.push(String::new());
		output.extend(parse_lines);
	}
	Ok(output.join("\n"))
}

#[cfg(test)]
mod tests {
	use std::fs;

	use super::*;

	fn run(root: &Path, args: &str) -> Result<String> {
		ast_grep(root, &serde_json::from_str(args)?, &EditStore::new(), &CancelToken::default())
	}

	#[test]
	fn directory_matches_group_with_meta_bindings_and_tags() -> Result<()> {
		let root = tempfile::tempdir()?;
		fs::create_dir_all(root.path().join("src"))?;
		fs::write(root.path().join("src/a.ts"), "const x = 1;\nconsole.log(x, 2);\n")?;
		let out = run(root.path(), r#"{"pat":"console.log($A, $B)","path":"src"}"#)?;
		let lines: Vec<&str> = out.lines().collect();
		assert_eq!(lines[0], "# src/");
		assert!(lines[1].starts_with("## a.ts#"), "{out}");
		assert_eq!(&lines[2..], ["*2:console.log(x, 2)", "  meta: A=x, B=2"]);
		Ok(())
	}

	#[test]
	fn empty_patterns_bad_skip_and_misses_report_the_legacy_text() -> Result<()> {
		let root = tempfile::tempdir()?;
		fs::write(root.path().join("a.ts"), "let y = 2;\n")?;
		assert_eq!(run(root.path(), r#"{"pat":"foo($A)"}"#)?, "No matches found");
		assert!(
			run(root.path(), r#"{"pat":"  "}"#)
				.unwrap_err()
				.to_string()
				.contains("non-empty pattern")
		);
		assert!(
			run(root.path(), r#"{"pat":"x","skip":-2}"#)
				.unwrap_err()
				.to_string()
				.contains("non-negative")
		);
		Ok(())
	}
}
