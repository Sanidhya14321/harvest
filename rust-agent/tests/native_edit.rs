//! The native application must preserve file encoding and reject unsafe edits.
use std::path::Path;

use anyhow::Result;
use harvest_omp::{
	chat::{FunctionCall, ToolCall},
	tools::{Approval, invoke, invoke_with_approval, invoke_with_cancellation},
};
use harvest_runtime::{MAX_FILE_BYTES, cancel::CancelToken};
use serde_json::json;

struct Allow;
impl Approval for Allow {
	fn approve(&self, _: &ToolCall) -> Result<bool> {
		Ok(true)
	}
}

fn call(path: &str, old: &str, new: &str) -> ToolCall {
	ToolCall {
		id:       "edit-contract".into(),
		kind:     "function".into(),
		function: FunctionCall {
			name:      "edit".into(),
			arguments: json!({"path":path,"old":old,"new":new}).to_string(),
		},
	}
}

fn edit(root: &Path, path: &str, old: &str, new: &str) -> Result<String> {
	invoke_with_approval(root, &call(path, old, new), &Allow)
}

#[test]
fn normalized_multiline_edit_preserves_utf8_bom_and_crlf() -> Result<()> {
	let root = tempfile::tempdir()?;
	let path = root.path().join("encoded.txt");
	std::fs::write(&path, "\u{feff}alpha\r\nbeta\r\n")?;
	edit(root.path(), "encoded.txt", "alpha\nbeta", "gamma\ndelta")?;
	assert_eq!(std::fs::read(&path)?, "\u{feff}gamma\r\ndelta\r\n".as_bytes());
	Ok(())
}

#[test]
fn editing_notebook_source_preserves_other_cells_and_notebook_metadata() -> Result<()> {
	let root = tempfile::tempdir()?;
	let path = root.path().join("notes.ipynb");
	let before = json!({"nbformat":4,"nbformat_minor":5,"metadata":{"owner":"user"},"cells":[
		{"cell_type":"code","metadata":{},"source":["print(1)\n"],"outputs":[],"execution_count":null},
		{"cell_type":"markdown","metadata":{"tag":"keep"},"source":["Keep this cell\n"]}
	]});
	std::fs::write(&path, serde_json::to_vec(&before)?)?;
	edit(root.path(), "notes.ipynb", "print(1)", "print(2)")?;
	let after: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
	assert_eq!(after["cells"][0]["source"], json!(["print(2)\n"]));
	assert_eq!(after["cells"][1], before["cells"][1]);
	assert_eq!(after["metadata"], before["metadata"]);
	Ok(())
}

#[test]
fn ambiguous_and_generated_targets_remain_unchanged() -> Result<()> {
	let root = tempfile::tempdir()?;
	let path = root.path().join("file.txt");
	std::fs::write(&path, "one one")?;
	let error = edit(root.path(), "file.txt", "one", "two").unwrap_err();
	assert!(error.to_string().contains("2 occurrences"));
	assert_eq!(std::fs::read_to_string(&path)?, "one one");
	let generated = root.path().join("generated.ts");
	std::fs::write(&generated, "const one = 1;\n")?;
	let error = edit(root.path(), "generated.ts", "one", "two").unwrap_err();
	assert!(error.to_string().contains("auto-generated"));
	assert_eq!(std::fs::read_to_string(&generated)?, "const one = 1;\n");
	Ok(())
}

#[test]
fn invalid_utf8_and_oversized_files_are_rejected_without_data_loss() -> Result<()> {
	let root = tempfile::tempdir()?;
	let path = root.path().join("invalid.txt");
	std::fs::write(&path, [b'a', 0xff])?;
	assert!(
		edit(root.path(), "invalid.txt", "a", "b")
			.unwrap_err()
			.to_string()
			.contains("UTF-8")
	);
	assert_eq!(std::fs::read(&path)?, [b'a', 0xff]);
	let large = root.path().join("large.txt");
	let bytes = vec![b'a'; MAX_FILE_BYTES + 1];
	std::fs::write(&large, &bytes)?;
	assert!(edit(root.path(), "large.txt", "a", "b").is_err());
	assert_eq!(std::fs::read(&large)?, bytes);
	Ok(())
}

#[test]
fn denied_or_cancelled_edit_does_not_modify_the_target() -> Result<()> {
	let root = tempfile::tempdir()?;
	let path = root.path().join("file.txt");
	std::fs::write(&path, "before")?;
	let request = call("file.txt", "before", "after");
	assert!(invoke(root.path(), &request).is_err());
	let mut token = CancelToken::default();
	token
		.emplace_abort_token()
		.abort(harvest_runtime::cancel::AbortReason::User);
	assert!(invoke_with_cancellation(root.path(), &request, &Allow, &token).is_err());
	assert_eq!(std::fs::read_to_string(&path)?, "before");
	Ok(())
}

#[cfg(unix)]
#[test]
fn symlinked_targets_and_missing_leaves_under_escaping_parents_are_rejected() -> Result<()> {
	let root = tempfile::tempdir()?;
	let outside = tempfile::tempdir()?;
	let secret = outside.path().join("secret.txt");
	std::fs::write(&secret, "secret")?;
	std::os::unix::fs::symlink(outside.path(), root.path().join("escape"))?;
	assert!(edit(root.path(), "escape/secret.txt", "secret", "changed").is_err());
	assert!(edit(root.path(), "escape/missing.txt", "secret", "changed").is_err());
	assert_eq!(std::fs::read_to_string(&secret)?, "secret");
	assert!(!outside.path().join("missing.txt").exists());
	Ok(())
}
