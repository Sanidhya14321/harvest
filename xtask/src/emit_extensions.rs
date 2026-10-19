//! Repair relative ESM specifiers in retained package output without Bun/Node.
use std::{
	path::{Path, PathBuf},
	sync::LazyLock,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES};
use regex::Regex;

const MAX_ENTRIES: usize = 50_000;
const MAX_DEPTH: usize = 128;
const MAX_PLAN_BYTES: usize = 128 * 1024 * 1024;

// Match the same emitted import/export forms as the legacy post-emit pass.
// Explicit ASCII word boundaries reproduce JavaScript's non-Unicode \b.
static SPECIFIERS: LazyLock<Regex> = LazyLock::new(|| {
	// ECMAScript whitespace includes BOM but excludes U+0085 (NEL).
	let space =
		r"[\t-\r \x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}]";
	Regex::new(&format!(
		r#"(?P<prefix>(?-u:\b)(?:from|import|module)(?-u:\b){space}*(?:\({space}*)?)(?:"(?P<double>\.[^"']*)"|'(?P<single>\.[^"']*)')"#
	)).expect("static emitted-specifier expression")
});

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
	Types,
	Js,
}
impl Kind {
	const fn suffix(self) -> &'static str {
		match self {
			Self::Types => ".d.ts",
			Self::Js => ".js",
		}
	}
}

#[derive(Parser)]
#[command(name = "cargo xtask emit-extensions")]
struct Arguments {
	#[arg(long)]
	directory: PathBuf,
	#[arg(long, value_enum, default_value = "types")]
	kind:      Kind,
	#[arg(long)]
	check:     bool,
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let Some(args) = crate::cli::arguments::<Arguments>("emit-extensions", arguments)? else {
		return Ok(());
	};
	let (files, specifiers) = repair(&args.directory, args.kind, args.check)?;
	println!(
		"emit-extensions: rewrote {specifiers} specifiers across {files} files in {}",
		args.directory.display()
	);
	Ok(())
}

fn resolve(fs: &JailedFs, directory: &Path, spec: &str, kind: Kind) -> Result<Option<String>> {
	if [".js", ".json", ".mjs", ".cjs"]
		.iter()
		.any(|suffix| spec.ends_with(suffix))
	{
		return Ok(None);
	}
	if let Some(stem) = spec.strip_suffix(".d.ts") {
		return Ok(Some(format!("{stem}.js")));
	}
	let base = directory.join(spec);
	let sibling = directory.join(format!("{spec}{}", kind.suffix()));
	if fs.exists(&sibling)? {
		return Ok(Some(format!("{spec}.js")));
	}
	if fs.exists(&base.join(format!("index{}", kind.suffix())))? {
		return Ok(Some(format!("{}/index.js", spec.strip_suffix('/').unwrap_or(spec))));
	}
	Ok(None)
}

fn rewrite(fs: &JailedFs, path: &Path, kind: Kind) -> Result<(String, usize)> {
	let bytes = fs.read(path, MAX_FILE_BYTES)?;
	let source = std::str::from_utf8(&bytes).context("emitted source must be UTF-8")?;
	let mut output = String::new();
	let mut end = 0;
	let mut count = 0;
	for captures in SPECIFIERS.captures_iter(source) {
		let matched = captures.get(0).context("missing specifier match")?;
		let (spec, quote) = if let Some(spec) = captures.name("double") {
			(spec.as_str(), '"')
		} else {
			(
				captures
					.name("single")
					.context("missing quoted specifier")?
					.as_str(),
				'\'',
			)
		};
		let Some(replacement) =
			resolve(fs, path.parent().unwrap_or_else(|| Path::new("")), spec, kind)?
		else {
			continue;
		};
		if replacement == spec {
			continue;
		}
		output.push_str(&source[end..matched.start()]);
		output.push_str(&captures["prefix"]);
		output.push(quote);
		output.push_str(&replacement);
		output.push(quote);
		end = matched.end();
		count += 1;
		if output.len() > MAX_FILE_BYTES {
			bail!("rewritten emitted file exceeds its byte limit");
		}
	}
	output.push_str(&source[end..]);
	if output.len() > MAX_FILE_BYTES {
		bail!("rewritten emitted file exceeds its byte limit");
	}
	Ok((output, count))
}

fn repair(root: &Path, kind: Kind, check: bool) -> Result<(usize, usize)> {
	let fs = JailedFs::open(root)?;
	let mut pending = vec![(PathBuf::new(), 0)];
	let mut remaining = MAX_ENTRIES;
	let mut plan = Vec::new();
	let mut total_bytes = 0;
	let mut specifiers = 0;
	while let Some((directory, depth)) = pending.pop() {
		if depth > MAX_DEPTH {
			bail!("emitted tree exceeds its directory depth limit");
		}
		let entries = fs.entries(&directory, remaining)?;
		remaining -= entries.len();
		for entry in entries {
			if entry.is_symlink {
				continue;
			}
			let path = directory.join(entry.name);
			if entry.is_directory {
				pending.push((path, depth + 1));
			} else if entry.is_file && path.to_string_lossy().ends_with(kind.suffix()) {
				let (output, count) = rewrite(&fs, &path, kind)?;
				if count != 0 {
					total_bytes += output.len();
					if total_bytes > MAX_PLAN_BYTES {
						bail!("emitted rewrite plan exceeds its byte limit");
					}
					specifiers += count;
					plan.push((path, output));
				}
			}
		}
	}
	if check && !plan.is_empty() {
		bail!("{} emitted files need explicit ESM extensions", plan.len());
	}
	for (path, output) in &plan {
		fs.atomic_write(path, output.as_bytes())?;
	}
	Ok((plan.len(), specifiers))
}

#[cfg(test)]
mod tests {
	use super::*;
	fn put(root: &Path, path: &str, body: &str) -> Result<()> {
		let path = root.join(path);
		std::fs::create_dir_all(path.parent().unwrap())?;
		std::fs::write(path, body)?;
		Ok(())
	}
	#[test]
	fn declarations_resolve_siblings_barrels_and_dynamic_imports() -> Result<()> {
		let root = tempfile::tempdir()?;
		put(root.path(), "sdk.d.ts", "export {};\n")?;
		put(root.path(), "nested/index.d.ts", "export {};\n")?;
		put(
			root.path(),
			"entry.d.ts",
			"export * from \"./sdk\";\nimport './nested';\ntype T=import( './sdk').T;\ndeclare \
			 module \"./sdk\" {}\nexport * from \"./missing\";\nimport 'pkg';\nimport \
			 './data.json';\nimport './ready.js';\n",
		)?;
		assert_eq!(repair(root.path(), Kind::Types, false)?, (1, 4));
		assert_eq!(
			std::fs::read_to_string(root.path().join("entry.d.ts"))?,
			"export * from \"./sdk.js\";\nimport './nested/index.js';\ntype T=import( \
			 './sdk.js').T;\ndeclare module \"./sdk.js\" {}\nexport * from \"./missing\";\nimport \
			 'pkg';\nimport './data.json';\nimport './ready.js';\n"
		);
		assert_eq!(repair(root.path(), Kind::Types, false)?, (0, 0));
		Ok(())
	}
	#[test]
	fn runtime_output_uses_runtime_siblings_and_preserves_quotes() -> Result<()> {
		let root = tempfile::tempdir()?;
		put(root.path(), "x.js", "")?;
		put(root.path(), "types.d.ts", "")?;
		put(root.path(), "dir/index.js", "")?;
		put(
			root.path(),
			"entry.js",
			"import './x'; import \"./dir/\"; import './types'; import './named.d.ts'; import \
			 './x.mjs';",
		)?;
		assert_eq!(repair(root.path(), Kind::Js, false)?, (1, 3));
		assert_eq!(
			std::fs::read_to_string(root.path().join("entry.js"))?,
			"import './x.js'; import \"./dir/index.js\"; import './types'; import './named.js'; \
			 import './x.mjs';"
		);
		Ok(())
	}
	#[test]
	fn check_mode_reports_drift_without_writing() -> Result<()> {
		let root = tempfile::tempdir()?;
		put(root.path(), "x.js", "")?;
		let original = "import './x';";
		put(root.path(), "entry.js", original)?;
		assert!(repair(root.path(), Kind::Js, true).is_err());
		assert_eq!(std::fs::read_to_string(root.path().join("entry.js"))?, original);
		repair(root.path(), Kind::Js, false)?;
		assert_eq!(repair(root.path(), Kind::Js, true)?, (0, 0));
		Ok(())
	}
	#[test]
	fn escaping_specifier_prevents_all_planned_mutations() -> Result<()> {
		let root = tempfile::tempdir()?;
		put(root.path(), "x.js", "")?;
		put(root.path(), "a.js", "import './x';")?;
		put(root.path(), "z.js", "import '../outside';")?;
		assert!(repair(root.path(), Kind::Js, false).is_err());
		assert_eq!(std::fs::read_to_string(root.path().join("a.js"))?, "import './x';");
		Ok(())
	}
	#[cfg(unix)]
	#[test]
	fn symlinked_emitted_files_cannot_modify_external_data() -> Result<()> {
		let root = tempfile::tempdir()?;
		let outside = tempfile::tempdir()?;
		put(outside.path(), "victim.js", "import './x';")?;
		std::os::unix::fs::symlink(outside.path().join("victim.js"), root.path().join("linked.js"))?;
		assert_eq!(repair(root.path(), Kind::Js, false)?, (0, 0));
		assert_eq!(std::fs::read_to_string(outside.path().join("victim.js"))?, "import './x';");
		Ok(())
	}
	#[test]
	fn javascript_whitespace_rules_and_invalid_utf8_do_not_rewrite_wrong_files() -> Result<()> {
		let root = tempfile::tempdir()?;
		put(root.path(), "x.js", "")?;
		put(root.path(), "entry.js", "import\u{feff}'./x'; import\u{85}'./x';")?;
		assert_eq!(repair(root.path(), Kind::Js, false)?, (1, 1));
		assert_eq!(
			std::fs::read_to_string(root.path().join("entry.js"))?,
			"import\u{feff}'./x.js'; import\u{85}'./x';"
		);
		put(root.path(), "a.js", "import './x';")?;
		std::fs::write(root.path().join("z.js"), [0xff])?;
		assert!(repair(root.path(), Kind::Js, false).is_err());
		assert_eq!(std::fs::read_to_string(root.path().join("a.js"))?, "import './x';");
		Ok(())
	}
}
