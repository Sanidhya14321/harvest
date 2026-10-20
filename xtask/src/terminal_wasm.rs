//! Build one portable terminal payload and generate self-contained bindings.
//! Embedded bytes survive npm packaging and Bun's single-file compilation.
use std::{
	env,
	path::{Path, PathBuf},
	process::Command,
};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use harvest_runtime::JailedFs;

pub fn execute(arguments: Vec<String>) -> Result<()> {
	if arguments.iter().any(|argument| argument != "--check") {
		bail!("usage: cargo xtask terminal-wasm [--check]");
	}
	let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.parent()
		.context("workspace root")?
		.to_path_buf();
	let filesystem = JailedFs::open(&root)?;
	let target = filesystem.resource_path(Path::new("target/terminal-wasm"))?;
	let status = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
		.current_dir(&root)
		.args([
			"build",
			"--locked",
			"-p",
			"harvest-terminal-wasm",
			"--target",
			"wasm32-unknown-unknown",
			"--profile",
			"ci",
			"--target-dir",
		])
		.arg(&target)
		// Host-specific native build flags must never leak into the WASM payload.
		.env_remove("RUSTFLAGS")
		.env_remove("CARGO_ENCODED_RUSTFLAGS")
		.status()
		.context("could not start terminal WASM build")?;
	if !status.success() {
		bail!("terminal WASM build failed with {status}");
	}
	let bytes = filesystem.read(
		&target.join("wasm32-unknown-unknown/ci/harvest_terminal_wasm.wasm"),
		64 * 1024 * 1024,
	)?;
	if !bytes.starts_with(b"\0asm\x01\0\0\0") {
		bail!("terminal payload is not a WebAssembly module");
	}
	let bindings = include_str!("../templates/terminal-wasm.js.in")
		.replace("__WASM_BASE64__", &STANDARD.encode(&bytes));
	for (path, contents) in [
		("packages/tui/wasm/terminal.js", bindings.as_str()),
		("packages/natives/native/terminal.js", bindings.as_str()),
		("packages/natives/native/terminal.d.ts", include_str!("../templates/terminal-wasm.d.ts.in")),
		("packages/tui/wasm/terminal.d.ts", include_str!("../templates/terminal-wasm.d.ts.in")),
		("packages/tui/src/fuzzy.js", include_str!("../templates/fuzzy.js.in")),
		("packages/tui/src/fuzzy.d.ts", include_str!("../templates/fuzzy.d.ts.in")),
	] {
		let path = root.join(path);
		if arguments.iter().any(|argument| argument == "--check") {
			if filesystem
				.read(&path, 128 * 1024 * 1024)
				.with_context(|| format!("missing generated binding {}", path.display()))?
				!= contents.as_bytes()
			{
				bail!("generated binding {} is stale; run cargo xtask terminal-wasm", path.display());
			}
		} else {
			filesystem.atomic_write(&path, contents.as_bytes())?;
		}
	}
	println!("Portable Rust terminal payload: {} bytes", bytes.len());
	Ok(())
}
