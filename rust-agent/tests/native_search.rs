//! `grep` keeps the shared ignore policy, the workspace jail and the legacy
//! output contract.
use std::path::Path;

use anyhow::Result;
use harvest_omp::{
	chat::{FunctionCall, ToolCall},
	tools::invoke,
};
use serde_json::{Value, json};

fn search(root: &Path, arguments: Value) -> Result<String> {
	invoke(root, &ToolCall {
		id:       "search-contract".into(),
		kind:     "function".into(),
		function: FunctionCall { name: "grep".into(), arguments: arguments.to_string() },
	})
}

/// File headers (`# name#TAG`) without their snapshot tags.
fn headers(output: &str) -> Vec<String> {
	output
		.lines()
		.filter(|line| line.starts_with('#'))
		.map(|line| match line.rsplit_once('#') {
			Some((head, tag)) if !tag.contains(' ') && !head.trim_matches('#').is_empty() => head,
			_ => line,
		})
		.map(str::to_owned)
		.collect()
}

#[test]
fn shared_ignore_rules_apply_by_default_and_explicit_false_or_file_paths_override_them()
-> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::create_dir(root.path().join(".git"))?;
	std::fs::write(root.path().join(".gitignore"), "*.log\n!keep.log\n")?;
	std::fs::write(root.path().join(".ignore"), "ignore-only.txt\n")?;
	for name in ["drop.log", "keep.log", "ignore-only.txt", ".visible", "note.txt"] {
		std::fs::write(root.path().join(name), "searchable needle")?;
	}
	std::fs::create_dir(root.path().join("nested"))?;
	for name in ["drop.log", "keep.log"] {
		std::fs::write(root.path().join("nested").join(name), "searchable needle")?;
	}

	let normal = headers(&search(root.path(), json!({"pattern":"needle"}))?);
	for name in ["# keep.log", "# .visible", "# note.txt"] {
		assert!(normal.iter().any(|header| header == name), "{normal:?}");
	}
	assert!(
		!normal
			.iter()
			.any(|header| header.contains("drop.log") || header.contains("ignore-only"))
	);
	let expanded = headers(&search(root.path(), json!({"pattern":"needle","gitignore":false}))?);
	assert!(expanded.iter().any(|header| header == "# drop.log"), "{expanded:?}");
	assert!(expanded.iter().any(|header| header == "# ignore-only.txt"), "{expanded:?}");
	let nested = search(root.path(), json!({"pattern":"needle","path":"nested"}))?;
	assert_eq!(headers(&nested), ["# nested/", "## keep.log"]);

	let explicit = search(root.path(), json!({"pattern":"needle","path":"drop.log"}))?;
	assert!(explicit.starts_with("[drop.log#") && explicit.ends_with("\n*1:searchable needle"));
	Ok(())
}

#[test]
fn explicit_false_case_option_enables_case_insensitive_search() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("note"), "HELLO world\n")?;
	assert_eq!(search(root.path(), json!({"pattern":"hello"}))?, "No matches found");
	assert!(
		search(root.path(), json!({"pattern":"hello","case":false}))?.ends_with("\n*1:HELLO world")
	);
	Ok(())
}

#[test]
fn binary_files_and_vcs_or_dependency_directories_stay_out_of_results() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("binary"), b"needle\0")?;
	for directory in [".git", "node_modules"] {
		std::fs::create_dir(root.path().join(directory))?;
		std::fs::write(root.path().join(directory).join("private"), "needle")?;
	}
	std::fs::write(root.path().join("text"), "needle")?;
	let result = search(root.path(), json!({"pattern":"needle","gitignore":false}))?;
	assert_eq!(headers(&result), ["# text"]);
	Ok(())
}

#[test]
fn unparseable_patterns_fall_back_to_literal_search_and_missing_roots_fail() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("note"), "a [ b\n")?;
	assert!(search(root.path(), json!({"pattern":"["}))?.contains("*1:a [ b"));
	let error = search(root.path(), json!({"pattern":"needle","path":"missing"})).unwrap_err();
	assert_eq!(error.to_string(), "Path not found: missing");
	Ok(())
}

#[cfg(unix)]
#[test]
fn explicit_symlink_operands_cannot_read_outside_the_workspace() -> Result<()> {
	let root = tempfile::tempdir()?;
	let outside = tempfile::tempdir()?;
	std::fs::write(outside.path().join("secret"), "outside-secret")?;
	std::os::unix::fs::symlink(outside.path(), root.path().join("escape"))?;
	assert!(
		search(root.path(), json!({"pattern":"outside-secret","path":"escape/secret"})).is_err()
	);
	assert_eq!(search(root.path(), json!({"pattern":"outside-secret"}))?, "No matches found");
	Ok(())
}

#[test]
fn deep_trees_are_searched_and_absolute_file_paths_resolve_inside_the_jail() -> Result<()> {
	let root = tempfile::tempdir()?;
	let mut deep = root.path().to_path_buf();
	for _ in 0..34 {
		deep.push("d");
	}
	std::fs::create_dir_all(&deep)?;
	std::fs::write(deep.join("note"), "needle")?;
	let result = search(root.path(), json!({"pattern":"needle"}))?;
	assert!(result.starts_with(&format!("# {}/", ["d"; 34].join("/"))), "{result}");
	let direct = deep.join("note");
	assert!(
		search(root.path(), json!({"pattern":"needle","path":direct}))?.ends_with("\n*1:needle")
	);
	Ok(())
}
