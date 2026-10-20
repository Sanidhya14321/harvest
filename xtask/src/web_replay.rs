//! Assemble a standalone Rust/WASM viewer from the selected session branch.
//!
//! JavaScript here is generated bindings and initialization only. Rendering,
//! search and display-state decisions are implemented by harvest-web.
use std::{
	env,
	path::{Path, PathBuf},
	process::Command,
};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use clap::Parser;
use harvest_protocol::{
	display::{ReplayProjection, project_branch},
	session::{SESSION_LIMIT, SessionDocument},
};
use harvest_runtime::JailedFs;

#[derive(Parser)]
#[command(name = "cargo xtask web-replay")]
struct Arguments {
	#[arg(long)]
	session:       PathBuf,
	#[arg(long)]
	leaf:          String,
	#[arg(long)]
	out:           PathBuf,
	/// Exercise the generated file in headless Chrome, including Rust search.
	#[arg(long)]
	check_browser: bool,
}

fn run(command: &mut Command) -> Result<()> {
	let status = command
		.status()
		.context("could not start WASM build tool")?;
	if !status.success() {
		bail!("WASM build tool failed with {status}");
	}
	Ok(())
}

/// Embed an escaped display snapshot, generated no-module bindings and WASM.
/// No persisted message is interpreted as HTML, JavaScript or a fetch URL.
pub fn render(projection: &ReplayProjection, bindings: &str, wasm: &[u8]) -> Result<String> {
	if !wasm.starts_with(b"\0asm\x01\0\0\0") {
		bail!("replay artifact is not a WebAssembly module");
	}
	if bindings.len() > 8 * 1024 * 1024 || wasm.len() > 32 * 1024 * 1024 {
		bail!("replay assets exceed size limits");
	}
	if bindings.to_ascii_lowercase().contains("</script") {
		bail!("generated bindings contain an unsafe script terminator");
	}
	let data = serde_json::to_string(projection)?
		.replace('<', "\\u003c")
		.replace('>', "\\u003e")
		.replace('&', "\\u0026")
		.replace('\u{2028}', "\\u2028")
		.replace('\u{2029}', "\\u2029");
	let template = include_str!("../templates/replay.html");
	// Split the trusted template before inserting data: snapshot text and
	// generated binding literals can never become replacement instructions.
	let bootstrap = format!(
		"{bindings}\nwasm_bindgen({{module_or_path:Uint8Array.from(atob('{}'),c=>c.\
		 charCodeAt(0))}}).catch(()=>{{const \
		 root=document.getElementById('harvest-replay');root.setAttribute('role','alert');root.\
		 textContent='The replay viewer could not start.';}});",
		STANDARD.encode(wasm)
	);
	let (head, rest) = template
		.split_once("<!--HARVEST_DATA-->")
		.context("missing replay data slot")?;
	let (middle, tail) = rest
		.split_once("<!--HARVEST_BOOTSTRAP-->")
		.context("missing replay bootstrap slot")?;
	Ok(format!("{head}{data}{middle}{bootstrap}{tail}"))
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let args = Arguments::parse_from(std::iter::once("web-replay".into()).chain(arguments));
	let root = JailedFs::open(&env::current_dir()?)?;
	let out = root.relative(&args.out)?;
	if root.resource_path(&out)? == root.resource_path(&args.session)? {
		bail!("replay output cannot overwrite the session input");
	}
	let document = SessionDocument::decode(&root.read(&args.session, SESSION_LIMIT)?)?;
	let projection = project_branch(&document, &args.leaf)?;
	let target =
		Path::new("target/web-replay/build/wasm32-unknown-unknown/release/harvest_web.wasm");
	run(
		Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
			.current_dir(root.root())
			.args([
				"build",
				"--locked",
				"--release",
				"--target-dir",
				"target/web-replay/build",
				"--target",
				"wasm32-unknown-unknown",
				"-p",
				"harvest-web",
			]),
	)?;
	let package = Path::new("target/web-replay/bindings");
	let package_root = root.resource_path(package)?;
	run(
		Command::new(env::var_os("HARVEST_WASM_BINDGEN").unwrap_or_else(|| "wasm-bindgen".into()))
			.current_dir(root.root())
			.arg(root.resource_path(target)?)
			.args([
				"--target",
				"no-modules",
				"--no-typescript",
				"--out-name",
				"harvest_replay",
				"--out-dir",
			])
			.arg(package_root),
	)?;
	let bindings =
		String::from_utf8(root.read(&package.join("harvest_replay.js"), 8 * 1024 * 1024)?)?;
	let wasm = root.read(&package.join("harvest_replay_bg.wasm"), 32 * 1024 * 1024)?;
	root.atomic_write(&out, render(&projection, &bindings, &wasm)?.as_bytes())?;
	if args.check_browser {
		crate::browser_check::verify(&root, &out, &projection)?;
	}
	println!("Read-only Rust/WASM replay: {}", out.display());
	Ok(())
}
