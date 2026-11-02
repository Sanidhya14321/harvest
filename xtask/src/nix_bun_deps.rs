//! Regenerate `nix/bun.nix` from `bun.lock` with the pinned bun2nix.
use std::{
	env,
	path::{Path, PathBuf},
	process::Command,
	sync::LazyLock,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use harvest_runtime::{JailedFs, MAX_FILE_BYTES};
use regex::Regex;

/// Pinned npm package matching flake.lock's bun2nix revision (`0f2a1f…`).
pub const BUN2NIX_NPM_SPEC: &str = "bun2nix@2.1.2";
const LOCK: &str = "bun.lock";
const OUTPUT: &str = "nix/bun.nix";
/// Bound for bun.lock and nix/bun.nix (both currently well under 1 MiB).
const LOCK_LIMIT: usize = MAX_FILE_BYTES;
static STAMP: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r#"(?m)^(\s*"lockfileVersion":\s*)(\d+)(,)"#).expect("valid regex"));
const GENERATOR_ARGS: [&str; 6] = ["-l", LOCK, "-c", "../", "-o", OUTPUT];

#[derive(Parser)]
#[command(name = "cargo xtask nix-bun-deps")]
/// Regenerate nix/bun.nix with the pinned bun2nix.
struct Arguments {
	#[arg(long, default_value = ".")]
	workspace:       PathBuf,
	/// Print which generator would run (bun2nix, nix or bunx) and exit.
	#[arg(long)]
	print_generator: bool,
}

/// How the pinned bun2nix is invoked.
#[derive(Debug, PartialEq, Eq)]
pub enum Generator {
	/// bun2nix from the active development shell.
	Bun2nix(PathBuf),
	/// Enter the Nix development shell, which provides bun2nix.
	Nix(PathBuf),
	/// The pinned portable npm package via bunx.
	Bunx,
}

impl Generator {
	/// Stable kind name printed for release logs.
	pub const fn kind(&self) -> &'static str {
		match self {
			Self::Bun2nix(_) => "bun2nix",
			Self::Nix(_) => "nix",
			Self::Bunx => "bunx",
		}
	}

	fn command(&self) -> Command {
		match self {
			Self::Bun2nix(executable) => {
				let mut command = Command::new(executable);
				command.args(GENERATOR_ARGS);
				command
			},
			Self::Nix(executable) => {
				let mut command = Command::new(executable);
				command
					.args(["--extra-experimental-features", "nix-command flakes"])
					.args(["--accept-flake-config", "develop", "--command", "bun2nix"])
					.args(GENERATOR_ARGS);
				command
			},
			Self::Bunx => {
				let mut command = Command::new("bunx");
				command.arg(BUN2NIX_NPM_SPEC).args(GENERATOR_ARGS);
				command
			},
		}
	}
}

/// Prefer bun2nix, then nix, then the pinned bunx package.
pub fn resolve(find: impl Fn(&str) -> Option<PathBuf>) -> Generator {
	find("bun2nix")
		.map(Generator::Bun2nix)
		.or_else(|| find("nix").map(Generator::Nix))
		.unwrap_or(Generator::Bunx)
}

fn find_on_path(name: &str) -> Option<PathBuf> {
	env::split_paths(&env::var_os("PATH")?)
		.map(|dir| dir.join(name))
		.find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
	use std::os::unix::fs::PermissionsExt;
	path
		.metadata()
		.is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
	path.with_extension("exe").is_file() || path.is_file()
}

/// Restamp a lockfileVersion 2 `bun.lock` to 1 so the pinned bun2nix parses it.
///
/// v1 and v2 share byte-identical content (v2 only added parse-time
/// strictness); v3+ changes content (scoped overrides) and is rejected.
pub fn normalize_lockfile_version(contents: &str) -> Result<String> {
	let captures = STAMP
		.captures(contents)
		.context("bun.lock is missing a lockfileVersion stamp")?;
	let version: u64 = captures[2].parse()?;
	if version <= 1 {
		return Ok(contents.to_owned());
	}
	if version > 2 {
		bail!(
			"bun.lock is lockfileVersion {version}, which changes content (scoped overrides) and \
			 cannot be downgraded for bun2nix"
		);
	}
	let digits = captures.get(2).expect("group");
	Ok(format!("{}1{}", &contents[..digits.start()], &contents[digits.end()..]))
}

/// Canonicalize generated Nix output to exactly one trailing LF.
pub fn normalize_generated_nix(contents: &str) -> String {
	format!("{}\n", contents.trim_end_matches(['\r', '\n']))
}

fn rewrite(fs: &JailedFs, path: &str, transform: impl Fn(&str) -> Result<String>) -> Result<()> {
	let bytes = fs.read(Path::new(path), LOCK_LIMIT)?;
	let contents = std::str::from_utf8(&bytes).with_context(|| format!("{path} is not UTF-8"))?;
	let normalized = transform(contents)?;
	if normalized != contents {
		fs.atomic_write(Path::new(path), normalized.as_bytes())?;
	}
	Ok(())
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let Some(args) = crate::cli::arguments::<Arguments>("nix-bun-deps", arguments)? else {
		return Ok(());
	};
	let generator = resolve(find_on_path);
	if args.print_generator {
		println!("{}", generator.kind());
		return Ok(());
	}
	let fs = JailedFs::open(&args.workspace)?;
	rewrite(&fs, LOCK, normalize_lockfile_version)?;
	let status = generator
		.command()
		.current_dir(&args.workspace)
		.status()
		.with_context(|| format!("could not start the {} generator", generator.kind()))?;
	if !status.success() {
		bail!("{} generator failed with {status}", generator.kind());
	}
	rewrite(&fs, OUTPUT, |contents| Ok(normalize_generated_nix(contents)))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn generator_preference_is_bun2nix_then_nix_then_bunx() {
		let all = |name: &str| Some(PathBuf::from(format!("/nix/store/{name}")));
		assert_eq!(resolve(all), Generator::Bun2nix("/nix/store/bun2nix".into()));
		let nix_only = |name: &str| (name == "nix").then(|| PathBuf::from("/usr/bin/nix"));
		assert_eq!(resolve(nix_only), Generator::Nix("/usr/bin/nix".into()));
		assert_eq!(resolve(|_| None), Generator::Bunx);
	}

	#[test]
	fn lockfile_v2_is_restamped_and_v1_is_untouched() -> Result<()> {
		let v2 = "{\n  \"lockfileVersion\": 2,\n  \"configVersion\": 1,\n  \"workspaces\": {},\n}\n";
		assert_eq!(
			normalize_lockfile_version(v2)?,
			"{\n  \"lockfileVersion\": 1,\n  \"configVersion\": 1,\n  \"workspaces\": {},\n}\n"
		);
		let v1 = "{\n  \"lockfileVersion\": 1,\n  \"workspaces\": {},\n}\n";
		assert_eq!(normalize_lockfile_version(v1)?, v1);
		Ok(())
	}

	#[test]
	fn lockfile_v3_and_unstamped_locks_are_rejected() {
		let v3 = normalize_lockfile_version("{\n  \"lockfileVersion\": 3,\n}\n").unwrap_err();
		assert!(v3.to_string().contains("lockfileVersion 3"), "{v3}");
		let missing = normalize_lockfile_version("{}\n").unwrap_err();
		assert!(missing.to_string().contains("missing a lockfileVersion"), "{missing}");
	}

	#[test]
	fn generated_nix_ends_with_exactly_one_lf() {
		assert_eq!(normalize_generated_nix("}"), "}\n");
		assert_eq!(normalize_generated_nix("}\n"), "}\n");
		assert_eq!(normalize_generated_nix("}\r\n\r\n"), "}\n");
	}
}
