use std::{
	fs,
	path::{Path, PathBuf},
	process::Command,
};

use anyhow::Result;
use tempfile::{TempDir, tempdir};

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
	let output = Command::new("git")
		.current_dir(root)
		.args([
			"-c",
			"user.name=fixture",
			"-c",
			"user.email=fixture@example.com",
			"-c",
			"commit.gpgsign=false",
		])
		.args(args)
		.output()?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	Ok(output.stdout)
}

fn fixture() -> Result<(TempDir, PathBuf, PathBuf)> {
	let root = tempdir()?;
	let tap = root.path().join("tap");
	fs::create_dir(&tap)?;
	git(&tap, &["init", "--quiet", "--initial-branch=main"])?;
	fs::write(tap.join("README.md"), b"tap\n")?;
	git(&tap, &["add", "README.md"])?;
	git(&tap, &["commit", "--quiet", "-m", "Create the tap"])?;
	let formula = root.path().join("omp.rb");
	fs::write(&formula, b"first formula\n")?;
	Ok((root, tap, formula))
}

#[test]
fn native_cli_matches_new_unchanged_updated_baseline_without_language_runtimes() -> Result<()> {
	let (root, tap, formula) = fixture()?;
	let bin = root.path().join("bin");
	let outside_index = root.path().join("outside-index");
	fs::write(&outside_index, b"protected index\n")?;
	fs::create_dir(&bin)?;
	#[cfg(unix)]
	{
		let candidate = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
			.map(|directory| directory.join("git"))
			.find(|path| path.is_file())
			.expect("Git must be installed for tap staging");
		std::os::unix::fs::symlink(candidate.canonicalize()?, bin.join("git"))?;
	}
	let run = |tag: &str| -> Result<()> {
		let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
		command
			.env("GIT_DIR", root.path().join("wrong-repository"))
			.env("GIT_INDEX_FILE", &outside_index)
			.arg("tap-update")
			.arg("--formula")
			.arg(&formula)
			.arg("--tap")
			.arg(&tap)
			.arg("--tag")
			.arg(tag);
		#[cfg(unix)]
		command.env("PATH", &bin);
		let output = command.output()?;
		assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
		Ok(())
	};
	let before = git(&tap, &["rev-parse", "HEAD"])?;
	let config = fs::read(tap.join(".git/config"))?;
	run("v1.0.0")?;
	let first = git(&tap, &["rev-parse", "HEAD"])?;
	assert_ne!(first, before);
	assert_eq!(git(&tap, &["show", "HEAD:Formula/omp.rb"])?, b"first formula\n");
	assert_eq!(git(&tap, &["log", "-1", "--format=%s"])?, b"Update the Omp formula to v1.0.0\n");
	assert_eq!(
		git(&tap, &["log", "-1", "--format=%an <%ae>"])?,
		b"pukbot[bot] <320458784+pukbot[bot]@users.noreply.github.com>\n"
	);
	run("v1.0.0")?;
	assert_eq!(git(&tap, &["rev-parse", "HEAD"])?, first);
	fs::write(&formula, b"second formula\n")?;
	run("v1.0.1")?;
	assert_ne!(git(&tap, &["rev-parse", "HEAD"])?, first);
	assert_eq!(git(&tap, &["show", "HEAD:Formula/omp.rb"])?, b"second formula\n");
	assert_eq!(fs::read(tap.join(".git/config"))?, config);
	assert_eq!(fs::read(outside_index)?, b"protected index\n");
	assert!(git(&tap, &["diff", "--cached", "--name-only"])?.is_empty());
	Ok(())
}

#[test]
fn unsafe_inputs_preserve_head_target_and_unrelated_staged_work() -> Result<()> {
	let (_root, tap, formula) = fixture()?;
	xtask::homebrew_tap::update(&formula, &tap, "v1.0.0")?;
	let before = git(&tap, &["rev-parse", "HEAD"])?;
	let target = tap.join("Formula/omp.rb");
	let original = fs::read(&target)?;
	let metadata = harvest_runtime::JailedFs::open(&tap.join(".git"))?;
	let owner = metadata.lock(Path::new("harvest-tap.lock"))?;
	assert!(xtask::homebrew_tap::update(&formula, &tap, "v1.0.1").is_err());
	drop(owner);
	assert!(xtask::homebrew_tap::update(&formula, &tap, "bad\ntag").is_err());
	assert!(xtask::homebrew_tap::update(&formula, &tap.join("Formula"), "v1.0.1").is_err());
	fs::write(&formula, vec![b'x'; 1024 * 1024 + 1])?;
	assert!(xtask::homebrew_tap::update(&formula, &tap, "v1.0.1").is_err());
	fs::write(&formula, b"new\n")?;
	fs::write(&target, b"local user edits\n")?;
	assert!(xtask::homebrew_tap::update(&formula, &tap, "v1.0.1").is_err());
	assert_eq!(fs::read(&target)?, b"local user edits\n");
	fs::write(&target, &original)?;
	fs::write(tap.join("README.md"), b"staged user edits\n")?;
	git(&tap, &["add", "README.md"])?;
	let index = fs::read(tap.join(".git/index"))?;
	assert!(xtask::homebrew_tap::update(&formula, &tap, "v1.0.1").is_err());
	assert_eq!(fs::read(tap.join(".git/index"))?, index);
	assert_eq!(fs::read(&target)?, original);
	assert_eq!(git(&tap, &["rev-parse", "HEAD"])?, before);
	Ok(())
}

#[cfg(unix)]
#[test]
fn escaping_formula_directory_is_rejected_without_outside_write() -> Result<()> {
	let (root, tap, formula) = fixture()?;
	let outside = root.path().join("outside");
	fs::create_dir(&outside)?;
	fs::write(outside.join("omp.rb"), b"protected\n")?;
	std::os::unix::fs::symlink(&outside, tap.join("Formula"))?;
	let before = git(&tap, &["rev-parse", "HEAD"])?;
	assert!(xtask::homebrew_tap::update(&formula, &tap, "v1.0.0").is_err());
	assert_eq!(fs::read(outside.join("omp.rb"))?, b"protected\n");
	assert_eq!(git(&tap, &["rev-parse", "HEAD"])?, before);
	Ok(())
}

#[test]
fn linked_worktree_uses_its_own_metadata_and_preserves_the_primary_checkout() -> Result<()> {
	let (root, tap, formula) = fixture()?;
	let linked = root.path().join("linked");
	git(&tap, &["worktree", "add", "-b", "tap-working", linked.to_str().unwrap()])?;
	let primary_head = git(&tap, &["rev-parse", "HEAD"])?;
	xtask::homebrew_tap::update(&formula, &linked, "v1.0.0")?;
	assert_eq!(git(&linked, &["show", "HEAD:Formula/omp.rb"])?, b"first formula\n");
	assert_eq!(git(&tap, &["rev-parse", "HEAD"])?, primary_head);
	assert!(!tap.join("Formula/omp.rb").exists());
	Ok(())
}

#[test]
fn git_normalization_equivalence_is_a_successful_noop() -> Result<()> {
	let (_root, tap, formula) = fixture()?;
	git(&tap, &["config", "core.autocrlf", "true"])?;
	fs::write(&formula, b"first formula\r\n")?;
	xtask::homebrew_tap::update(&formula, &tap, "v1.0.0")?;
	let before = git(&tap, &["rev-parse", "HEAD"])?;
	// A normal checkout on Windows may use CRLF while the generated input is LF.
	fs::write(&formula, b"first formula\n")?;
	assert!(git(&tap, &["status", "--porcelain", "--", "Formula/omp.rb"])?.is_empty());
	assert!(!xtask::homebrew_tap::update(&formula, &tap, "v1.0.0")?);
	assert_eq!(git(&tap, &["rev-parse", "HEAD"])?, before);
	assert!(git(&tap, &["diff", "--cached", "--name-only"])?.is_empty());
	assert_eq!(fs::read(tap.join("Formula/omp.rb"))?, b"first formula\n");
	Ok(())
}
