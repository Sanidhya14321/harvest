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

/// `PREVIEW_PENDING_NOTICE`.
pub(crate) const PREVIEW_PENDING_NOTICE: &str = "Staged as a proposal — files NOT modified yet. \
                                                 To apply: write a one-sentence reason to \
                                                 xd://resolve. To discard: write to xd://reject.";

#[derive(Debug, Deserialize)]
struct EditOp {
	pat: String,
	out: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EditArgs {
	ops:   Vec<EditOp>,
	paths: Vec<String>,
}

/// A staged `ast_edit` preview, re-run against the workspace on apply.
pub(crate) struct EditPlan {
	pub(crate) label:  String,
	rewrites:          HashMap<String, String>,
	search_path:       std::path::PathBuf,
	is_directory:      bool,
	glob:              Option<String>,
	targets:           Option<Vec<(std::path::PathBuf, Option<String>)>>,
	max_files:         u32,
	expected_total:    u32,
	expected_files:    u32,
	expected_per_file: HashMap<String, u32>,
}

struct EditRun {
	/// Changes keyed by display path, in encounter order.
	changes:       Vec<(String, engine::AstReplaceChange)>,
	/// Per-file counts in `fileChanges` order.
	file_counts:   Vec<(String, u32)>,
	total:         u32,
	files_touched: u32,
	limit_reached: bool,
	parse_errors:  Vec<String>,
	/// Absolute path for each display path.
	absolute:      HashMap<String, std::path::PathBuf>,
}

fn plural(label: &str, count: u32) -> String {
	if count == 1 {
		format!("{count} {label}")
	} else {
		format!("{count} {label}s")
	}
}

fn run_edit(ws: &Workspace, plan: &EditPlan) -> Result<EditRun> {
	let targets: Vec<(std::path::PathBuf, Option<String>)> = match &plan.targets {
		Some(targets) => targets.clone(),
		None => vec![(plan.search_path.clone(), plan.glob.clone())],
	};
	let mut run = EditRun {
		changes:       Vec::new(),
		file_counts:   Vec::new(),
		total:         0,
		files_touched: 0,
		limit_reached: false,
		parse_errors:  Vec::new(),
		absolute:      HashMap::new(),
	};
	for (base, glob) in &targets {
		let engine_root = ws.engine_path(base)?;
		let result = engine::ast_edit(
			engine::AstReplaceOptions {
				rewrites:            Some(plan.rewrites.clone()),
				lang:                None,
				path:                Some(engine_root.to_string_lossy().into_owned()),
				glob:                glob.clone(),
				selector:            None,
				strictness:          None,
				dry_run:             Some(true),
				max_replacements:    None,
				max_files:           Some(plan.max_files),
				fail_on_parse_error: Some(false),
			},
			CancelToken::default(),
		)
		.map_err(|error| anyhow!("{error}"))?;
		run.total += result.total_replacements;
		run.limit_reached |= result.limit_reached;
		run.parse_errors
			.extend(result.parse_errors.unwrap_or_default());
		let is_dir = engine_root.is_dir();
		let mut locate = |engine_path: &str| -> String {
			let absolute = if is_dir {
				base.join(engine_path)
			} else {
				base.clone()
			};
			let display = if plan.is_directory {
				ws.display(
					&plan
						.search_path
						.join(posix_relative(&plan.search_path, &absolute)),
				)
			} else {
				ws.display(&plan.search_path)
			};
			run.absolute.insert(display.clone(), absolute);
			display
		};
		for file in result.file_changes {
			let display = locate(&file.path);
			match run
				.file_counts
				.iter_mut()
				.find(|(path, _)| *path == display)
			{
				Some((_, count)) => *count += file.count,
				None => run.file_counts.push((display, file.count)),
			}
		}
		for change in result.changes {
			let display = locate(&change.path);
			run.changes.push((display, change));
		}
	}
	run.files_touched = run.file_counts.len() as u32;
	Ok(run)
}

fn render_edit(ws: &Workspace, plan: &EditPlan, run: &EditRun, store: &EditStore) -> Vec<String> {
	let mut files: Vec<String> = Vec::new();
	for (path, _) in &run.file_counts {
		if !files.contains(path) {
			files.push(path.clone());
		}
	}
	for (path, _) in &run.changes {
		if !files.contains(path) {
			files.push(path.clone());
		}
	}
	let mut tags: HashMap<String, String> = HashMap::new();
	for file in &files {
		if let Some(tag) = store.record_file(&ws.root.join(file), None) {
			tags.insert(file.clone(), tag);
		}
	}
	let first_line = |text: &str| -> String {
		text
			.split('\n')
			.next()
			.unwrap_or("")
			.chars()
			.take(120)
			.collect()
	};
	let render = |file: &str| -> Vec<String> {
		let hashed = tags.contains_key(file);
		let mut out = Vec::new();
		for (_, change) in run.changes.iter().filter(|(path, _)| path == file) {
			let reference = if hashed {
				format!("{}:", change.start_line)
			} else {
				format!("{}:{} ", change.start_line, change.start_column)
			};
			out.push(format!("-{reference}{}", first_line(&change.before)));
			out.push(format!("+{reference}{}", first_line(&change.after)));
		}
		out
	};
	let count_of = |file: &str| {
		run.file_counts
			.iter()
			.find(|(path, _)| path == file)
			.map_or(0, |(_, count)| *count)
	};
	let mut output = Vec::new();
	if plan.is_directory {
		output.extend(format_grouped_files(&files, &mut |file| {
			let hash = tags
				.get(file)
				.map_or_else(String::new, |tag| format!("#{tag}"));
			(render(file), format!("{hash} ({})", plural("replacement", count_of(file))))
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
	output
}

/// Stage an `ast_edit` preview; the plan is returned for the session's
/// pending-action queue when there is something to apply.
pub(crate) fn ast_edit(
	root: &Path,
	args: &EditArgs,
	store: &EditStore,
	cancellation: &CancelToken,
) -> Result<(String, Option<EditPlan>)> {
	cancellation.heartbeat()?;
	if args.ops.is_empty() {
		bail!("`ops` must include at least one op entry");
	}
	let mut rewrites = HashMap::new();
	for (index, op) in args.ops.iter().enumerate() {
		if op.pat.is_empty() {
			bail!("`ops[{index}].pat` must be a non-empty pattern");
		}
		if rewrites.insert(op.pat.clone(), op.out.clone()).is_some() {
			bail!("Duplicate rewrite pattern: {}", op.pat);
		}
	}
	if args.paths.is_empty() {
		bail!("`paths` must contain at least one path or glob");
	}
	let max_files = std::env::var("PI_MAX_AST_FILES")
		.ok()
		.and_then(|value| value.parse::<u32>().ok())
		.filter(|value| *value > 0)
		.unwrap_or(1000);
	let fs = JailedFs::open(root)?;
	let ws = Workspace { root: fs.root().to_path_buf(), fs };
	let entries = expand_entries(&ws, &args.paths)?;
	for entry in &entries {
		if crate::search::is_url_like(entry) {
			bail!("Internal URLs are not supported by this host: {entry}");
		}
	}
	let scope = resolve_scope(&ws, &entries, false)?;
	let mut plan = EditPlan {
		label: String::new(),
		rewrites,
		search_path: scope.search_path.clone(),
		is_directory: scope.is_directory,
		glob: scope.glob.clone(),
		targets: scope.targets.as_ref().map(|targets| {
			targets
				.iter()
				.map(|t| (t.base.clone(), t.glob.clone()))
				.collect()
		}),
		max_files,
		expected_total: 0,
		expected_files: 0,
		expected_per_file: HashMap::new(),
	};
	let run = run_edit(&ws, &plan)?;
	let parse_lines = parse_error_lines(&run.parse_errors);
	if run.total == 0 {
		let mut text = "No replacements made".to_owned();
		if !parse_lines.is_empty() {
			text.push('\n');
			text.push_str(&parse_lines.join("\n"));
		}
		return Ok((text, None));
	}
	let mut output = render_edit(&ws, &plan, &run, store);
	if run.limit_reached {
		output.push(String::new());
		output.push("Limit reached; narrow paths.".to_owned());
	}
	if !parse_lines.is_empty() {
		output.push(String::new());
		output.extend(parse_lines);
	}
	output.insert(0, String::new());
	output.insert(0, PREVIEW_PENDING_NOTICE.to_owned());
	plan.label = format!(
		"AST Edit: {} in {}",
		plural("replacement", run.total),
		plural("file", run.files_touched)
	);
	plan.expected_total = run.total;
	plan.expected_files = run.files_touched;
	plan.expected_per_file = run.file_counts.iter().cloned().collect();
	Ok((output.join("\n"), Some(plan)))
}

/// Apply a staged plan through the workspace capability: re-run the rewrite,
/// splice each file's replacements back to front, and report fresh tags.
pub(crate) fn apply_edit(root: &Path, plan: &EditPlan, store: &EditStore) -> Result<String> {
	let fs = JailedFs::open(root)?;
	let ws = Workspace { root: fs.root().to_path_buf(), fs };
	let run = run_edit(&ws, plan)?;
	let mut by_file: Vec<(String, Vec<&engine::AstReplaceChange>)> = Vec::new();
	for (path, change) in &run.changes {
		match by_file.iter_mut().find(|(existing, _)| existing == path) {
			Some((_, list)) => list.push(change),
			None => by_file.push((path.clone(), vec![change])),
		}
	}
	for (display, changes) in &by_file {
		let absolute = &run.absolute[display];
		let relative = ws.fs.relative(absolute)?;
		let bytes = ws.fs.read(&relative, harvest_runtime::MAX_FILE_BYTES)?;
		let mut text = String::from_utf8(bytes).map_err(|_| anyhow!("{display} is not UTF-8"))?;
		let mut ordered = changes.clone();
		ordered.sort_by_key(|change| std::cmp::Reverse(change.byte_start));
		let mut previous_start = usize::MAX;
		for change in ordered {
			let (start, end) = (change.byte_start as usize, change.byte_end as usize);
			if end > previous_start {
				bail!("Apply failed: overlapping replacements in {display}");
			}
			if text.get(start..end) != Some(change.before.as_str()) {
				bail!("Apply failed: {display} changed since the preview");
			}
			text.replace_range(start..end, &change.after);
			previous_start = start;
		}
		ws.fs.atomic_write(&relative, text.as_bytes())?;
	}
	let mut fresh = Vec::new();
	for (display, _) in &by_file {
		if let Some(tag) = store.record_file(&ws.root.join(display), None) {
			fresh.push(format_hashline_header(display, &tag));
		}
	}
	let stale = run.total != plan.expected_total
		|| run.files_touched != plan.expected_files
		|| run
			.file_counts
			.iter()
			.any(|(path, count)| plan.expected_per_file.get(path) != Some(count))
		|| plan.expected_per_file.len() != run.file_counts.len();
	let text = if stale {
		let expected = format!(
			"{} in {}",
			plural("replacement", plan.expected_total),
			plural("file", plan.expected_files)
		);
		let message = if run.total == 0 {
			format!(
				"Preview is stale / no longer matches; no replacements were applied. Preview expected \
				 {expected}."
			)
		} else if run.total < plan.expected_total {
			format!(
				"Preview is stale / no longer matches; only {} of {} replacements were applied in {} \
				 of {} files.",
				run.total, plan.expected_total, run.files_touched, plan.expected_files
			)
		} else {
			format!(
				"Preview is stale / no longer matches; applied {} replacements but preview expected \
				 {}.",
				run.total, plan.expected_total
			)
		};
		let message = if fresh.is_empty() {
			message
		} else {
			format!("{message}\n{}", fresh.join("\n"))
		};
		bail!("{message}");
	} else {
		format!(
			"Applied {} in {}.",
			plural("replacement", run.total),
			plural("file", run.files_touched)
		)
	};
	Ok(if fresh.is_empty() {
		text
	} else {
		format!("{text}\n{}", fresh.join("\n"))
	})
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

	#[test]
	fn edits_stage_a_preview_and_apply_through_the_jail() -> Result<()> {
		let root = tempfile::tempdir()?;
		fs::write(root.path().join("a.ts"), "foo(1);\nfoo(2);\n")?;
		let store = EditStore::new();
		let args: EditArgs =
			serde_json::from_str(r#"{"ops":[{"pat":"foo($A)","out":"bar($A)"}],"paths":["a.ts"]}"#)?;
		let (text, plan) = ast_edit(root.path(), &args, &store, &CancelToken::default())?;
		assert!(text.starts_with(PREVIEW_PENDING_NOTICE), "{text}");
		assert!(text.contains("\n-1:foo(1)\n+1:bar(1)"), "{text}");
		assert_eq!(fs::read_to_string(root.path().join("a.ts"))?, "foo(1);\nfoo(2);\n");
		let plan = plan.expect("plan");
		assert_eq!(plan.label, "AST Edit: 2 replacements in 1 file");
		let applied = apply_edit(root.path(), &plan, &store)?;
		assert!(applied.starts_with("Applied 2 replacements in 1 file.\n[a.ts#"), "{applied}");
		assert_eq!(fs::read_to_string(root.path().join("a.ts"))?, "bar(1);\nbar(2);\n");
		let stale = apply_edit(root.path(), &plan, &store)
			.unwrap_err()
			.to_string();
		assert!(
			stale.starts_with("Preview is stale / no longer matches; no replacements were applied."),
			"{stale}"
		);
		Ok(())
	}
}
