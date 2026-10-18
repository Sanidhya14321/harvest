//! Native grep must use shared ignore policy and retain jailed bounded reads.
use std::path::Path;

use anyhow::Result;
use harvest_omp::{
	chat::{FunctionCall, ToolCall},
	tools::invoke,
};
use harvest_runtime::MAX_FILE_BYTES;
use serde_json::{Value, json};

fn search(root: &Path, arguments: Value) -> Result<String> {
	invoke(root, &ToolCall {
		id:       "search-contract".into(),
		kind:     "function".into(),
		function: FunctionCall { name: "grep".into(), arguments: arguments.to_string() },
	})
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

	let normal = search(root.path(), json!({"pattern":"needle"}))?;
	assert!(
		normal.contains("keep.log:1:")
			&& normal.contains(".visible:1:")
			&& normal.contains("note.txt:1:")
	);
	assert!(!normal.contains("drop.log:") && !normal.contains("ignore-only.txt:"));
	let expanded = search(root.path(), json!({"pattern":"needle","gitignore":false}))?;
	assert!(expanded.contains("drop.log:1:") && expanded.contains("ignore-only.txt:1:"));
	let nested = search(root.path(), json!({"pattern":"needle","path":"nested"}))?;
	assert!(nested.contains(&format!("{}:1:", Path::new("nested").join("keep.log").display())));
	assert!(!nested.contains("drop.log:"));

	let explicit = search(root.path(), json!({"pattern":"needle","path":"drop.log"}))?;
	assert!(explicit.starts_with("drop.log:1:"));
	Ok(())
}

#[test]
fn explicit_false_case_option_enables_case_insensitive_search() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("note"), "HELLO world\n")?;
	assert_eq!(search(root.path(), json!({"pattern":"hello"}))?, "");
	assert!(
		search(root.path(), json!({"pattern":"hello","case":false}))?.contains("note:1:HELLO world")
	);
	Ok(())
}

#[test]
fn binary_invalid_utf8_oversized_and_transient_files_do_not_enter_search_results() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("binary"), b"needle\0")?;
	std::fs::write(root.path().join("invalid"), b"needle\xff")?;
	std::fs::write(root.path().join("oversized"), vec![b'x'; MAX_FILE_BYTES + 1])?;
	for directory in [".git", ".omp", ".harvest", "node_modules", "target"] {
		std::fs::create_dir(root.path().join(directory))?;
		std::fs::write(root.path().join(directory).join("private"), "needle")?;
	}
	std::fs::write(root.path().join("text"), "needle")?;
	let result = search(root.path(), json!({"pattern":"needle|xxx","gitignore":false}))?;
	assert_eq!(result, "text:1:needle");
	Ok(())
}

#[test]
fn match_limit_is_reported_instead_of_silently_claiming_an_exhaustive_search() -> Result<()> {
	let root = tempfile::tempdir()?;
	std::fs::write(root.path().join("first"), "needle\n".repeat(80))?;
	std::fs::write(root.path().join("second"), "needle")?;
	let result = search(root.path(), json!({"pattern":"needle"}))?;
	assert_eq!(
		result
			.lines()
			.filter(|line| line.starts_with("first:"))
			.count(),
		50
	);
	assert!(!result.contains("second:"));
	assert!(result.ends_with("[search limit reached]"));
	Ok(())
}

#[test]
fn invalid_patterns_and_nonexistent_roots_surface_errors() -> Result<()> {
	let root = tempfile::tempdir()?;
	assert!(
		search(root.path(), json!({"pattern":"["}))
			.unwrap_err()
			.to_string()
			.contains("invalid regular expression")
	);
	assert!(search(root.path(), json!({"pattern":"needle","path":"missing"})).is_err());
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
	assert_eq!(search(root.path(), json!({"pattern":"outside-secret"}))?, "");
	Ok(())
}

#[test]
fn depth_limits_are_visible_and_explicit_file_search_can_reach_the_target() -> Result<()> {
	let root = tempfile::tempdir()?;
	let mut deep = root.path().to_path_buf();
	for _ in 0..34 {
		deep.push("d");
	}
	std::fs::create_dir_all(&deep)?;
	std::fs::write(deep.join("note"), "needle")?;
	let result = search(root.path(), json!({"pattern":"needle"}))?;
	assert_eq!(result, "[search limit reached]");
	let direct = deep.join("note");
	assert!(search(root.path(), json!({"pattern":"needle","path":direct}))?.ends_with(":1:needle"));
	Ok(())
}
