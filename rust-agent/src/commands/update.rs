//! `harvest update` — port of `commands/update.ts` + `cli/update-cli.ts`.
//!
//! Replacing the running binary always requires explicit consent: `--yes`,
//! or an interactive confirmation on a terminal. Non-interactive runs without
//! `--yes` stop after reporting the available version.

use std::{
	io::{BufRead, IsTerminal, Read, Write},
	path::{Path, PathBuf},
	process::{Command, Stdio},
	time::Duration,
};

use anyhow::{Context, Result, bail};
use clap::Args as ClapArgs;
use harvest_runtime::{JailedFs, MAX_HTTP_BYTES, read_bounded};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
	Paint,
	plugin::{Action, Flags, run as run_plugin},
	plugin_store::{APP_NAME, which},
};

const PACKAGE: &str = "@harvest/pi-coding-agent";
const DEFAULT_REPO: &str = "Sanidhya14321/harvest";
const HOMEBREW_FORMULA: &str = "Sanidhya14321/tap/omp";
const RELEASE_BINARY_PREFIX: &str = "omp";
const NPM_REGISTRY: &str = "https://registry.npmjs.org/";
const GITHUB_API: &str = "https://api.github.com";
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MAX_RENAME_HOPS: usize = 3;
const MAX_BINARY_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, ClapArgs)]
#[command(
	about = "Check for and install updates",
	after_help = "Examples:\n  harvest update\n  harvest update --check\n  harvest update --canary\n  # If GitHub rate-limits release metadata, set GITHUB_TOKEN or GH_TOKEN\n  GITHUB_TOKEN=... harvest update"
)]
pub struct UpdateArgs {
	/// Force update
	#[arg(short = 'f', long)]
	pub force:   bool,
	/// Check for updates without installing
	#[arg(short = 'c', long)]
	pub check:   bool,
	/// Update installed plugins
	#[arg(short = 'l', long)]
	pub plugins: bool,
	/// Switch to the canary channel and update
	#[arg(long, conflicts_with = "stable")]
	pub canary:  bool,
	/// Switch back to the stable channel
	#[arg(long)]
	pub stable:  bool,
	/// Replace the installed binary without asking for confirmation
	#[arg(short = 'y', long)]
	pub yes:     bool,
}

struct Release {
	version: String,
	dist:    Option<&'static str>,
	package: String,
}

fn repo() -> String {
	std::env::var("HARVEST_REPO").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| DEFAULT_REPO.into())
}

fn agent(timeout: Duration) -> ureq::Agent {
	ureq::AgentBuilder::new().timeout(timeout).build()
}

fn get_json(request: ureq::Request) -> Result<(u16, Value)> {
	let response = match request.call() {
		Ok(r) => r,
		Err(ureq::Error::Status(_, r)) => r,
		Err(error) => return Err(error.into()),
	};
	let status = response.status();
	let bytes = read_bounded(response.into_reader(), MAX_HTTP_BYTES)?;
	Ok((status, serde_json::from_slice(&bytes).unwrap_or(Value::Null)))
}

fn fetch_manifest(pkg: &str, canary: bool) -> Result<Value> {
	let tag = if canary { "canary" } else { "latest" };
	let (status, data) = get_json(agent(METADATA_TIMEOUT).get(&format!("{NPM_REGISTRY}{pkg}/{tag}")))?;
	if status == 404 && canary {
		bail!("No canary release has been published for {pkg} yet. Try `{APP_NAME} update --stable`.");
	}
	if !(200..300).contains(&status) {
		bail!("Failed to fetch release info for {pkg}: HTTP {status}");
	}
	if !data.get("version").is_some_and(Value::is_string) {
		bail!("Malformed npm registry response for {pkg}: missing version");
	}
	Ok(data)
}

/// `getLatestRelease`, following `omp.rename` pointers.
fn latest_release(canary: bool) -> Result<Release> {
	let mut package = PACKAGE.to_owned();
	let mut visited = vec![package.clone()];
	let mut manifest = fetch_manifest(&package, canary)?;
	for _ in 0..MAX_RENAME_HOPS {
		let Some(next) = manifest
			.get("omp")
			.and_then(|o| o.get("rename"))
			.and_then(|r| r.as_str().or_else(|| r.get("pkg").and_then(Value::as_str)))
			.map(str::to_owned)
		else {
			break;
		};
		if visited.contains(&next) {
			break;
		}
		visited.push(next.clone());
		package = next;
		manifest = fetch_manifest(&package, canary)?;
	}
	let dist = manifest
		.get("omp")
		.and_then(|o| o.get("dist"))
		.map(|d| if d.as_str() == Some("npm") { "npm" } else { "binary" });
	Ok(Release { version: manifest["version"].as_str().unwrap_or_default().to_owned(), dist, package })
}

fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
	match (semver::Version::parse(a), semver::Version::parse(b)) {
		(Ok(a), Ok(b)) => a.cmp(&b),
		_ => a.cmp(b),
	}
}

fn release_binary_name() -> Result<String> {
	let os = match std::env::consts::OS {
		"linux" if cfg!(target_env = "musl") => "linux-musl",
		"linux" => "linux",
		"macos" => "darwin",
		"windows" => "windows",
		other => bail!("Unsupported platform: {other}"),
	};
	let arch = match std::env::consts::ARCH {
		"x86_64" => "x64",
		"aarch64" => "arm64",
		other => bail!("Unsupported architecture: {other}"),
	};
	let ext = if os == "windows" { ".exe" } else { "" };
	Ok(format!("{RELEASE_BINARY_PREFIX}-{os}-{arch}{ext}"))
}

struct Asset {
	url:    String,
	size:   u64,
	digest: String,
}

/// `resolveReleaseBinaryAsset`.
fn release_asset(version: &str, binary: &str, allow_prerelease: bool) -> Result<Asset> {
	let tag = format!("v{version}");
	let repo = repo();
	let token = std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN")).ok().filter(|t| !t.is_empty());
	let mut request = agent(METADATA_TIMEOUT)
		.get(&format!("{GITHUB_API}/repos/{repo}/releases/tags/{tag}"))
		.set("Accept", "application/vnd.github+json")
		.set("X-GitHub-Api-Version", "2022-11-28");
	if let Some(token) = &token {
		request = request.set("Authorization", &format!("Bearer {token}"));
	}
	let (status, release) = get_json(request)?;
	if (status == 403 && token.is_none()) || status == 429 {
		bail!("GitHub API rate limit exceeded while fetching release metadata; retry later or set GITHUB_TOKEN or GH_TOKEN");
	}
	if !(200..300).contains(&status) {
		bail!("Failed to fetch GitHub release metadata: HTTP {status}");
	}
	if release.get("tag_name").and_then(Value::as_str) != Some(tag.as_str()) {
		bail!("GitHub release tag mismatch: expected {tag}");
	}
	if release.get("draft") != Some(&Value::Bool(false)) {
		bail!("GitHub release {tag} is a draft, not a published release");
	}
	if release.get("prerelease") != Some(&Value::Bool(false)) && !allow_prerelease {
		bail!("GitHub release {tag} is a prerelease; only canary updates install prerelease assets");
	}
	let assets = release.get("assets").and_then(Value::as_array).with_context(|| format!("GitHub release {tag} has no asset list"))?;
	let matches: Vec<&Value> = assets.iter().filter(|a| a.get("name").and_then(Value::as_str) == Some(binary)).collect();
	if matches.len() != 1 {
		bail!("GitHub release {tag} has {} assets named {binary}", matches.len());
	}
	let asset = matches[0];
	if asset.get("state").and_then(Value::as_str) != Some("uploaded") {
		bail!("GitHub release asset {binary} is not fully uploaded");
	}
	let size = asset.get("size").and_then(Value::as_u64).filter(|s| *s > 0 && *s <= MAX_BINARY_BYTES)
		.with_context(|| format!("GitHub release asset {binary} has an invalid size"))?;
	let digest = asset
		.get("digest")
		.and_then(Value::as_str)
		.and_then(|d| d.strip_prefix("sha256:").or_else(|| d.strip_prefix("SHA256:")))
		.filter(|d| d.len() == 64 && d.chars().all(|c| c.is_ascii_hexdigit()))
		.with_context(|| format!("GitHub release asset {binary} has an unsupported digest"))?
		.to_ascii_lowercase();
	let url = format!("https://github.com/{repo}/releases/download/{tag}/{binary}");
	if asset.get("browser_download_url").and_then(Value::as_str) != Some(url.as_str()) {
		bail!("GitHub release asset {binary} has an unexpected download URL");
	}
	Ok(Asset { url, size, digest: format!("sha256:{digest}") })
}

struct Hashing<R> {
	inner: R,
	hash:  Sha256,
	size:  u64,
}

impl<R: Read> Read for Hashing<R> {
	fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
		let n = self.inner.read(buf)?;
		self.hash.update(&buf[..n]);
		self.size += n as u64;
		Ok(n)
	}
}

enum Method {
	Nix,
	Brew,
	Mise,
	Bun,
	Npm,
	Binary(PathBuf),
}

fn resolve_method() -> Result<Method> {
	let exe = std::env::current_exe().context("cannot resolve the running binary")?;
	let real = exe.canonicalize().unwrap_or_else(|_| exe.clone());
	let text = real.to_string_lossy().replace('\\', "/");
	if text.starts_with("/nix/store/") {
		return Ok(Method::Nix);
	}
	if text.contains("/Cellar/") || text.contains("/homebrew/") {
		return Ok(Method::Brew);
	}
	if text.contains("/mise/installs/") {
		return Ok(Method::Mise);
	}
	if text.contains("/node_modules/") {
		return Ok(if text.contains("/.bun/") { Method::Bun } else { Method::Npm });
	}
	Ok(Method::Binary(exe))
}

fn run_tool(program: &str, args: &[&str]) -> Result<()> {
	let path = which(program).with_context(|| format!("{program} not found in PATH"))?;
	let status = Command::new(path).args(args).stdin(Stdio::null()).status()?;
	if !status.success() {
		bail!("{program} {} failed with {status}", args.join(" "));
	}
	Ok(())
}

fn confirm(question: &str) -> Result<bool> {
	if !std::io::stdin().is_terminal() {
		return Ok(false);
	}
	print!("{question} [y/N] ");
	std::io::stdout().flush()?;
	let mut line = String::new();
	std::io::stdin().lock().read_line(&mut line)?;
	Ok(matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

fn reported_version(binary: &Path) -> Option<String> {
	let output = Command::new(binary).arg("--version").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
	let text = String::from_utf8_lossy(&output.stdout[..output.stdout.len().min(4096)]).into_owned();
	text.split(|c: char| c.is_whitespace() || c == '/')
		.map(|t| t.trim_start_matches('v'))
		.find(|t| semver::Version::parse(t).is_ok())
		.map(str::to_owned)
}

/// `updateViaBinaryAt` + `replaceBinaryForUpdate`.
fn update_binary(target: &Path, version: &str, allow_prerelease: bool) -> Result<()> {
	let p = Paint::stdout();
	let binary = release_binary_name()?;
	let asset = release_asset(version, &binary, allow_prerelease)?;
	let parent = target.parent().context("binary has no parent directory")?;
	let name = target.file_name().context("binary has no file name")?.to_string_lossy().into_owned();
	let fs = JailedFs::open(parent)?;
	let attempt = format!(
		"{}.{}",
		std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis(),
		std::process::id()
	);
	let temp_rel = PathBuf::from(format!("{name}.{attempt}.new"));
	let backup_rel = PathBuf::from(format!("{name}.{attempt}.bak"));
	println!("{}", p.dim(&format!("Downloading {binary}…")));
	let response = match agent(DOWNLOAD_TIMEOUT).get(&asset.url).call() {
		Ok(r) => r,
		Err(ureq::Error::Status(code, _)) => bail!("Download failed: HTTP {code}"),
		Err(error) => return Err(error.into()),
	};
	let mut reader = Hashing { inner: response.into_reader(), hash: Sha256::new(), size: 0 };
	let written = fs.atomic_write_from(&temp_rel, &mut reader, asset.size);
	let cleanup = |fs: &JailedFs| {
		let _ = fs.remove(&temp_rel);
	};
	if let Err(error) = written {
		cleanup(&fs);
		return Err(error.context(format!("Downloaded binary size mismatch: expected {} bytes", asset.size)));
	}
	let digest = format!("sha256:{:x}", reader.hash.finalize());
	if reader.size != asset.size {
		cleanup(&fs);
		bail!("Downloaded binary size mismatch: expected {} bytes, received {}", asset.size, reader.size);
	}
	if digest != asset.digest {
		cleanup(&fs);
		bail!("Downloaded binary digest mismatch: expected {}, received {digest}", asset.digest);
	}
	println!("{}", p.dim(&format!("Verified {digest}")));
	let temp = fs.resource_path(&temp_rel)?;
	let backup = fs.resource_path(&backup_rel)?;
	let target = fs.resource_path(Path::new(&name))?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755))?;
	}
	let _lock = fs.resource_lock(Path::new(&name))?;
	println!("{}", p.dim("Installing update..."));
	let backup_ready = match std::fs::rename(&target, &backup) {
		Ok(()) => true,
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
		Err(e) => {
			cleanup(&fs);
			return Err(e.into());
		},
	};
	let swapped = std::fs::rename(&temp, &target);
	let actual = swapped.as_ref().ok().and_then(|()| reported_version(&target));
	if swapped.is_err() || actual.as_deref() != Some(version) {
		if backup_ready {
			let _ = std::fs::remove_file(&target);
			std::fs::rename(&backup, &target)?;
		}
		cleanup(&fs);
		swapped?;
		bail!(
			"Installed binary reports {} instead of {version}; restored previous {APP_NAME} binary",
			actual.as_deref().unwrap_or("no version")
		);
	}
	if backup_ready {
		let _ = std::fs::remove_file(&backup);
	}
	println!("{}", p.green(&format!("✔ Updated to {version}")));
	println!("{}", p.dim(&format!("Restart {APP_NAME} to use the new version")));
	Ok(())
}

pub fn execute(args: UpdateArgs) -> Result<u8> {
	if args.plugins {
		return run_plugin(Action::Upgrade, &[], &Flags::default());
	}
	let p = Paint::stdout();
	let e = Paint::stderr();
	let current = env!("HARVEST_VERSION");
	println!("{}", p.dim(&format!("Current version: {current}")));
	let canary = args.canary;
	if canary {
		println!("{}", p.dim("Current channel: canary"));
	}
	let release = match latest_release(canary) {
		Ok(r) => r,
		Err(err) => {
			eprintln!("{}", e.red(&format!("Failed to check for updates: {err:#}")));
			return Ok(1);
		},
	};
	let newer = compare_versions(&release.version, current).is_gt();
	if !newer && !args.force && !canary {
		println!("{}", p.green("✔ Already up to date"));
		return Ok(0);
	}
	if canary && !newer {
		println!("{}", p.yellow(&format!("Switching to canary {} (downgrade from {current})", release.version)));
	} else if newer {
		println!("{}", p.cyan(&format!("New version available: {}", release.version)));
	} else {
		println!("{}", p.yellow(&format!("Forcing reinstall of {}", release.version)));
	}
	if release.package != PACKAGE {
		println!("{}", p.cyan(&format!("The npm package moved to {}; updating migrates this install.", release.package)));
	}
	if args.check {
		return Ok(0);
	}
	let result = (|| -> Result<u8> {
		let method = resolve_method()?;
		if canary && matches!(method, Method::Nix | Method::Brew | Method::Mise) {
			println!("{}", p.yellow("Canary updates are only supported for bun, npm, or binary installs."));
			return Ok(0);
		}
		let spec = format!("{}@{}", release.package, release.version);
		match method {
			Method::Nix => {
				println!("{}", p.yellow("This installation is managed by Nix and cannot update itself."));
				println!("{}", p.dim("Update the flake input or profile that provides omp, then rebuild."));
			},
			Method::Brew => {
				run_tool("brew", &["update"])?;
				run_tool("brew", &[if args.force { "reinstall" } else { "upgrade" }, HOMEBREW_FORMULA])?;
			},
			Method::Mise => run_tool("mise", &["upgrade", &format!("github:{}", repo())])?,
			Method::Bun if release.dist != Some("binary") => run_tool("bun", &["install", "-g", &spec])?,
			Method::Npm if release.dist != Some("binary") => run_tool("npm", &["install", "-g", &spec])?,
			Method::Bun | Method::Npm => bail!(
				"This release ships as a standalone binary; reinstall with: curl -fsSL https://omp.sh/install | sh -s -- --binary"
			),
			Method::Binary(path) => {
				if !args.yes && !confirm(&format!("Replace {} with {}?", path.display(), release.version))? {
					println!("{}", p.yellow("Update not applied. Re-run with --yes to replace the installed binary."));
					return Ok(1);
				}
				update_binary(&path, &release.version, canary)?;
			},
		}
		Ok(0)
	})();
	match result {
		Ok(code) => Ok(code),
		Err(err) => {
			eprintln!("{}", e.red(&format!("Update failed: {err:#}")));
			Ok(1)
		},
	}
}
