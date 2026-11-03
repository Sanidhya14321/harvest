use std::process::Command;

use harvest_omp::{
	chat::{FunctionCall, ToolCall},
	tools::{self, DenyMutations},
};

#[test]
fn retired_python_requests_fail_before_approval_or_execution() {
	let root = tempfile::tempdir().unwrap();
	for (name, args) in [
		("python", "{}"),
		("eval", r#"{"language":"py","code":"never execute"}"#),
		("eval", r#"{"language":"python"}"#),
	] {
		let call = ToolCall {
			id:       "retired".into(),
			kind:     "function".into(),
			function: FunctionCall { name: name.into(), arguments: args.into() },
		};
		let error = tools::invoke_with_approval(root.path(), &call, &DenyMutations).unwrap_err();
		assert!(error.to_string().contains("Python evaluation was retired"));
		assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
	}
}

#[test]
fn copied_native_cli_reports_retired_setup_without_a_language_runtime() {
	let installed = tempfile::tempdir().unwrap();
	let candidate = installed
		.path()
		.join(format!("harvest{}", std::env::consts::EXE_SUFFIX));
	std::fs::copy(env!("CARGO_BIN_EXE_omp"), &candidate).unwrap();
	let output = Command::new(&candidate)
		.args(["setup", "python", "--check", "--json"])
		.env("PATH", installed.path())
		.current_dir(installed.path())
		.output()
		.unwrap();
	assert_eq!(output.status.code(), Some(1));
	let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
	assert_eq!(result["available"], false);
	assert_eq!(result["retired"], true);
	assert!(result["reason"].as_str().unwrap().contains("Rust SDK"));
}
