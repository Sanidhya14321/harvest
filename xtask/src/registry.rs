//! Verify real package archives through a read-only loopback sparse registry.
//! No publishing endpoint, workspace patch, or skipped package verification is
//! used.
use std::{
	collections::{BTreeMap, BTreeSet, VecDeque},
	env,
	io::{Read, Write},
	net::{TcpListener, TcpStream},
	path::{Path, PathBuf},
	process::{Command, Stdio},
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, Ordering},
	},
	thread,
	time::Duration,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use harvest_runtime::JailedFs;
use reqwest::blocking::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tempfile::TempDir;

const MAX_METADATA: usize = 32 * 1024 * 1024;
const MAX_INDEX: usize = 16 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 128 * 1024 * 1024;
const MAX_PACKAGES: usize = 4096;
const MAX_LOG: usize = 64 * 1024;
const MAX_REQUESTS: usize = 65_536;

#[derive(Parser)]
#[command(name = "cargo xtask registry")]
struct Arguments {
	#[command(subcommand)]
	command: RegistryCommand,
}

#[derive(Subcommand)]
enum RegistryCommand {
	/// Sequentially package a dependency closure, then build a detached registry
	/// consumer.
	Verify {
		#[arg(long)]
		root:           Option<PathBuf>,
		#[arg(short = 'p', long, default_value = "harvest-omp")]
		package:        String,
		#[arg(long, default_value = "target/registry-stage")]
		output:         PathBuf,
		/// Optional registry-only consumer project; runs its executable after
		/// packaging.
		#[arg(long)]
		consumer:       Option<PathBuf>,
		#[arg(long)]
		all_features:   bool,
		#[arg(long)]
		allow_dirty:    bool,
		/// Forbid upstream requests; local registry HTTP remains enabled.
		#[arg(long)]
		offline:        bool,
		/// Reuse downloaded upstream data; Cargo still verifies package
		/// checksums.
		#[arg(long)]
		upstream_cache: Option<PathBuf>,
	},
}

#[derive(Clone, Deserialize)]
struct Dependency {
	name:                  String,
	req:                   String,
	path:                  Option<PathBuf>,
	kind:                  Option<String>,
	target:                Option<String>,
	registry:              Option<String>,
	rename:                Option<String>,
	features:              Vec<String>,
	optional:              bool,
	uses_default_features: bool,
}

#[derive(Clone, Deserialize)]
struct Package {
	name:          String,
	version:       String,
	manifest_path: PathBuf,
	dependencies:  Vec<Dependency>,
	features:      BTreeMap<String, Vec<String>>,
	publish:       Option<Vec<String>>,
	links:         Option<String>,
	rust_version:  Option<String>,
}

#[derive(Deserialize)]
struct Metadata {
	packages: Vec<Package>,
}

#[derive(Serialize)]
struct ArchiveReceipt {
	name:    String,
	version: String,
	path:    String,
	sha256:  String,
}

#[derive(Serialize)]
struct Receipt {
	schema_version:           u32,
	verification:             &'static str,
	package:                  String,
	all_features:             bool,
	allow_dirty:              bool,
	upstream_allowed:         bool,
	toolchain:                String,
	lockfile_sha256:          String,
	archives:                 Vec<ArchiveReceipt>,
	consumer:                 &'static str,
	consumer_stdout:          String,
	consumer_lockfile_sha256: String,
	upstream_cache:           Option<String>,
}

fn drain(mut input: impl Read, limit: usize, tail: bool) -> std::io::Result<(Vec<u8>, bool)> {
	let mut bytes = VecDeque::new();
	let mut truncated = false;
	let mut buffer = [0_u8; 8192];
	loop {
		let count = input.read(&mut buffer)?;
		if count == 0 {
			return Ok((bytes.into_iter().collect(), truncated));
		}
		if tail {
			let remove = bytes
				.len()
				.saturating_add(count)
				.saturating_sub(limit)
				.min(bytes.len());
			bytes.drain(..remove);
			bytes.extend(&buffer[count.saturating_sub(limit)..count]);
			continue;
		}
		let retained = count.min(limit.saturating_sub(bytes.len()));
		bytes.extend(&buffer[..retained]);
		truncated |= retained != count;
	}
}

fn run(mut command: Command, stdout_limit: usize) -> Result<Vec<u8>> {
	let mut child = command
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.context("could not start registry verification command")?;
	let stdout = child.stdout.take().context("missing command stdout")?;
	let stderr = child.stderr.take().context("missing command stderr")?;
	let out = thread::spawn(move || drain(stdout, stdout_limit, false));
	let err = thread::spawn(move || drain(stderr, MAX_LOG, true));
	let status = child.wait()?;
	let (stdout, truncated) = out
		.join()
		.map_err(|_| anyhow::anyhow!("stdout reader panicked"))??;
	let (stderr, _) = err
		.join()
		.map_err(|_| anyhow::anyhow!("stderr reader panicked"))??;
	if !status.success() {
		bail!(
			"registry verification command failed ({status}): {}",
			String::from_utf8_lossy(&stderr)
		);
	}
	if truncated {
		bail!("registry verification stdout exceeds {stdout_limit} bytes");
	}
	Ok(stdout)
}

fn cargo(root: &Path) -> Command {
	let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
	command.current_dir(root).env("CARGO_TERM_COLOR", "never");
	command
}

fn valid_name(name: &str) -> bool {
	!name.is_empty()
		&& name.len() <= 128
		&& name
			.bytes()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
}

fn index_path(name: &str) -> Result<String> {
	if !valid_name(name) {
		bail!("invalid registry package name");
	}
	let name = name.to_ascii_lowercase();
	Ok(match name.len() {
		1 => format!("1/{name}"),
		2 => format!("2/{name}"),
		3 => format!("3/{}/{name}", &name[..1]),
		_ => format!("{}/{}/{name}", &name[..2], &name[2..4]),
	})
}

fn index_record(package: &Package, checksum: &str) -> Result<Vec<u8>> {
	let mut dependencies = Vec::new();
	for dependency in &package.dependencies {
		if dependency
			.registry
			.as_deref()
			.is_some_and(|registry| registry != "https://github.com/rust-lang/crates.io-index")
		{
			bail!("non-crates.io dependency requires an explicit staging implementation");
		}
		dependencies.push(json!({
			"name": dependency.rename.as_ref().unwrap_or(&dependency.name),
			"package": dependency.rename.as_ref().map(|_| &dependency.name),
			"req": dependency.req, "features": dependency.features,
			"optional": dependency.optional, "default_features": dependency.uses_default_features,
			"target": dependency.target, "kind": dependency.kind.as_deref().unwrap_or("normal"),
			"registry": null
		}));
	}
	let mut record = serde_json::to_vec(&json!({
		"name": package.name, "vers": package.version, "deps": dependencies,
		"cksum": checksum, "features": {}, "features2": package.features,
		"yanked": false, "links": package.links, "rust_version": package.rust_version, "v": 2
	}))?;
	record.push(b'\n');
	Ok(record)
}

fn local_dependencies(
	package: &Package,
	packages: &BTreeMap<PathBuf, Package>,
	fs: &JailedFs,
) -> Result<Vec<PathBuf>> {
	package
		.dependencies
		.iter()
		.filter_map(|dependency| dependency.path.as_ref().map(|path| (dependency, path)))
		.map(|(dependency, path)| {
			let manifest = fs.resource_path(&path.join("Cargo.toml"))?;
			let target = packages
				.get(&manifest)
				.context("path dependency is not a workspace member")?;
			if dependency.req == "*" {
				bail!("{} lacks a publishable requirement for {}", package.name, target.name);
			}
			if !semver::VersionReq::parse(&dependency.req)?.matches(&Version::parse(&target.version)?)
			{
				bail!("path dependency version does not satisfy its registry requirement");
			}
			Ok(manifest)
		})
		.collect()
}

fn closure(
	fs: &JailedFs,
	metadata: Metadata,
	selected: &str,
) -> Result<(Vec<Package>, BTreeSet<String>)> {
	let mut packages = BTreeMap::new();
	let mut first_party = BTreeSet::new();
	for mut package in metadata.packages {
		if !valid_name(&package.name) {
			bail!("invalid workspace package name");
		}
		Version::parse(&package.version)?;
		package.manifest_path = fs.resource_path(&package.manifest_path)?;
		first_party.insert(package.name.clone());
		packages.insert(package.manifest_path.clone(), package);
	}
	let selected = packages
		.iter()
		.find(|(_, p)| p.name == selected)
		.map(|(path, _)| path.clone())
		.context("selected package is not a workspace member")?;
	let mut remaining = BTreeSet::new();
	let mut pending = vec![selected];
	while let Some(path) = pending.pop() {
		if !remaining.insert(path.clone()) {
			continue;
		}
		if remaining.len() > MAX_PACKAGES {
			bail!("registry closure exceeds package limit");
		}
		let package = &packages[&path];
		if package.publish.as_ref().is_some_and(|registries| {
			!registries.is_empty() && registries != &["crates-io".to_owned()]
		}) || package.publish.as_ref().is_some_and(Vec::is_empty)
		{
			bail!("{} is not publishable to crates.io", package.name);
		}
		pending.extend(local_dependencies(package, &packages, fs)?);
	}
	let mut completed = BTreeSet::new();
	let mut ordered = Vec::new();
	while !remaining.is_empty() {
		let mut ready = None;
		for path in &remaining {
			if local_dependencies(&packages[path], &packages, fs)?
				.iter()
				.all(|dependency| completed.contains(dependency))
			{
				ready = Some(path.clone());
				break;
			}
		}
		let path =
			ready.context("cyclic workspace package dependencies cannot be staged sequentially")?;
		remaining.remove(&path);
		completed.insert(path.clone());
		ordered.push(packages[&path].clone());
	}
	Ok((ordered, first_party))
}

struct Mirror {
	fs:          Arc<JailedFs>,
	first_party: BTreeSet<String>,
	client:      Client,
	offline:     bool,
	url:         String,
	cache:       Mutex<BTreeMap<String, (PathBuf, u64)>>,
	seed:        Option<Arc<JailedFs>>,
}

enum Body {
	Bytes(Vec<u8>),
	File(PathBuf, u64),
}

impl Mirror {
	fn upstream(&self, path: &str, url: &str, limit: u64) -> Result<Option<Body>> {
		let cached = self
			.cache
			.lock()
			.map_err(|_| anyhow::anyhow!("registry cache poisoned"))?
			.get(path)
			.cloned();
		if let Some((path, bytes)) = cached {
			return Ok(Some(Body::File(path, bytes)));
		}
		let file = PathBuf::from("upstream").join(path);
		if let Some(seed) = &self.seed
			&& seed.exists(Path::new(path))?
		{
			let mut size = 0;
			self.fs.atomic_write_with(&file, |output| {
				size = seed.copy_file_to(Path::new(path), output, limit)?;
				Ok(())
			})?;
			let mut cache = self
				.cache
				.lock()
				.map_err(|_| anyhow::anyhow!("registry cache poisoned"))?;
			if cache.len() >= MAX_PACKAGES * 4 {
				bail!("registry cache exceeds package limit");
			}
			cache.insert(path.to_owned(), (file.clone(), size));
			return Ok(Some(Body::File(file, size)));
		}
		if self.offline {
			return Ok(None);
		}
		let response = self.client.get(url).send()?;
		if response.status() == reqwest::StatusCode::NOT_FOUND {
			return Ok(None);
		}
		let response = response.error_for_status()?;
		let mut size = 0;
		self.fs.atomic_write_with(&file, |output| {
			size = std::io::copy(&mut response.take(limit + 1), output)?;
			if size > limit {
				bail!("registry upstream response exceeds limit");
			}
			Ok(())
		})?;
		let mut cache = self
			.cache
			.lock()
			.map_err(|_| anyhow::anyhow!("registry cache poisoned"))?;
		if cache.len() >= MAX_PACKAGES * 4 {
			bail!("registry cache exceeds package limit");
		}
		cache.insert(path.to_owned(), (file.clone(), size));
		Ok(Some(Body::File(file, size)))
	}

	fn body(&self, path: &str) -> Result<Option<Body>> {
		if path == "/index/config.json" {
			return Ok(Some(Body::Bytes(serde_json::to_vec(
				&json!({"dl":format!("{}/api/v1/crates/{{crate}}/{{version}}/download", self.url)}),
			)?)));
		}
		if let Some(index) = path.strip_prefix("/index/") {
			let name = index.rsplit('/').next().context("missing index name")?;
			if index_path(name)? != index {
				bail!("invalid index path");
			}
			let staged = PathBuf::from("index").join(index);
			if self.first_party.contains(name) {
				return Ok(self.fs.read_optional(&staged, MAX_INDEX)?.map(Body::Bytes));
			}
			return self.upstream(
				&format!("index/{index}"),
				&format!("https://index.crates.io/{index}"),
				MAX_INDEX as u64,
			);
		}
		let parts: Vec<_> = path.split('/').collect();
		if parts.len() == 7 && parts[..4] == ["", "api", "v1", "crates"] && parts[6] == "download" {
			let (name, version) = (parts[4], parts[5]);
			if !valid_name(name) {
				bail!("invalid download package");
			}
			Version::parse(version)?;
			let archive = PathBuf::from("archives").join(format!("{name}-{version}.crate"));
			if self.first_party.contains(name) {
				let size = self
					.fs
					.entries(Path::new("archives"), MAX_PACKAGES)?
					.iter()
					.find(|entry| entry.name == format!("{name}-{version}.crate") && entry.is_file)
					.map(|entry| entry.size);
				return Ok(size.map(|size| Body::File(archive, size)));
			}
			return self.upstream(
				&format!("archives/{name}-{version}.crate"),
				&format!("https://static.crates.io/crates/{name}/{name}-{version}.crate"),
				MAX_ARCHIVE,
			);
		}
		Ok(None)
	}

	fn serve(&self, mut socket: TcpStream) -> Result<()> {
		socket.set_read_timeout(Some(Duration::from_secs(5)))?;
		socket.set_write_timeout(Some(Duration::from_secs(30)))?;
		let mut request = Vec::new();
		let mut buffer = [0_u8; 1024];
		while !request.windows(4).any(|window| window == b"\r\n\r\n") {
			let count = socket.read(&mut buffer)?;
			if count == 0 {
				return Ok(());
			}
			request.extend_from_slice(&buffer[..count]);
			if request.len() > 16 * 1024 {
				bail!("registry request headers exceed limit");
			}
		}
		let request = std::str::from_utf8(&request)?;
		let mut words = request
			.lines()
			.next()
			.context("empty registry request")?
			.split_whitespace();
		let method = words.next().context("missing request method")?;
		let path = words.next().context("missing request path")?;
		let response = if method == "GET" && !path.contains(['?', '#', '%', '\\']) {
			self.body(path)
		} else {
			Ok(None)
		};
		let (status, body) = match response {
			Ok(Some(body)) => ("200 OK", body),
			Ok(None) => ("404 Not Found", Body::Bytes(b"not staged\n".to_vec())),
			Err(_) => ("502 Bad Gateway", Body::Bytes(b"registry staging request failed\n".to_vec())),
		};
		let length = match &body {
			Body::Bytes(bytes) => bytes.len() as u64,
			Body::File(_, length) => *length,
		};
		write!(socket, "HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n")?;
		match body {
			Body::Bytes(bytes) => socket.write_all(&bytes)?,
			Body::File(path, _) => {
				self.fs.copy_file_to(&path, &mut socket, MAX_ARCHIVE)?;
			},
		}
		Ok(())
	}
}

struct Server {
	mirror: Arc<Mirror>,
	stop:   Arc<AtomicBool>,
	worker: Option<thread::JoinHandle<()>>,
}

impl Server {
	fn start(
		fs: Arc<JailedFs>,
		first_party: BTreeSet<String>,
		offline: bool,
		seed: Option<Arc<JailedFs>>,
	) -> Result<Self> {
		let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
		listener.set_nonblocking(true)?;
		let mirror = Arc::new(Mirror {
			fs,
			first_party,
			offline,
			url: format!("http://{}", listener.local_addr()?),
			client: Client::builder()
				.https_only(true)
				.connect_timeout(Duration::from_secs(10))
				.timeout(Duration::from_secs(60))
				.redirect(reqwest::redirect::Policy::limited(3))
				.build()?,
			cache: Mutex::new(BTreeMap::new()),
			seed,
		});
		let stop = Arc::new(AtomicBool::new(false));
		let active = Arc::clone(&mirror);
		let stopped = Arc::clone(&stop);
		let worker = thread::spawn(move || {
			let mut requests = 0;
			while !stopped.load(Ordering::Acquire) && requests < MAX_REQUESTS {
				match listener.accept() {
					Ok((socket, _)) => {
						requests += 1;
						let _ = active.serve(socket);
					},
					Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
						thread::sleep(Duration::from_millis(10));
					},
					Err(_) => break,
				}
			}
		});
		Ok(Self { mirror, stop, worker: Some(worker) })
	}
}

impl Drop for Server {
	fn drop(&mut self) {
		self.stop.store(true, Ordering::Release);
		if let Some(worker) = self.worker.take() {
			let _ = worker.join();
		}
	}
}

fn staged_cargo(root: &Path, home: &Path, server: &Server, toolchain: &str) -> Command {
	let mut command = cargo(root);
	command
		.env("CARGO_HOME", home)
		.env("RUSTUP_TOOLCHAIN", toolchain)
		.env("CARGO_NET_OFFLINE", "false")
		.env("CARGO_NET_RETRY", "1")
		.env("NO_PROXY", "127.0.0.1,localhost")
		.env_remove("CARGO_HTTP_PROXY")
		.args(["--config", "source.crates-io.replace-with=\"harvest-stage\"", "--config"])
		.arg(format!("source.harvest-stage.registry=\"sparse+{}/index/\"", server.mirror.url));
	command
}

fn reject_consumer_overrides(document: &toml_edit::DocumentMut) -> Result<()> {
	fn walk(item: &toml_edit::Item) -> bool {
		let table_contains_override = item.as_table_like().is_some_and(|table| {
			table.iter().any(|(key, item)| {
				matches!(key, "path" | "workspace" | "patch" | "replace") || walk(item)
			})
		});
		table_contains_override
			|| item.as_array_of_tables().is_some_and(|tables| {
				tables.iter().any(|table| {
					table.iter().any(|(key, item)| {
						matches!(key, "path" | "workspace" | "patch" | "replace") || walk(item)
					})
				})
			})
	}
	if document
		.iter()
		.any(|(key, item)| matches!(key, "workspace" | "patch" | "replace") || walk(item))
	{
		bail!("detached registry consumer cannot contain workspace, patch or path overrides");
	}
	Ok(())
}

fn require_staged_consumer(lockfile: &[u8], subject: &ArchiveReceipt) -> Result<()> {
	let document = std::str::from_utf8(lockfile)?.parse::<toml_edit::DocumentMut>()?;
	let packages = document
		.get("package")
		.and_then(toml_edit::Item::as_array_of_tables)
		.context("consumer lockfile has no packages")?;
	if !packages.iter().any(|package| {
		package.get("name").and_then(toml_edit::Item::as_str) == Some(subject.name.as_str())
			&& package.get("version").and_then(toml_edit::Item::as_str)
				== Some(subject.version.as_str())
			&& package.get("source").and_then(toml_edit::Item::as_str)
				== Some("registry+https://github.com/rust-lang/crates.io-index")
			&& package.get("checksum").and_then(toml_edit::Item::as_str)
				== Some(subject.sha256.as_str())
	}) {
		bail!(
			"detached consumer did not resolve staged package {} {} with its archive checksum",
			subject.name,
			subject.version
		);
	}
	Ok(())
}

fn copy_consumer(
	source: &JailedFs,
	from: &Path,
	output: &JailedFs,
	into: &Path,
	depth: usize,
	budget: &mut (usize, usize),
) -> Result<()> {
	if depth > 32 {
		bail!("consumer source nesting exceeds limit");
	}
	for entry in source.entries(from, 4096)? {
		budget.0 = budget
			.0
			.checked_sub(1)
			.context("consumer file count exceeds limit")?;
		if matches!(entry.name.as_str(), ".cargo" | "target" | ".git") {
			bail!("consumer must be a clean source-only project without Cargo configuration");
		}
		let input = from.join(&entry.name);
		let destination = into.join(&entry.name);
		if entry.is_symlink {
			bail!("consumer source cannot contain symlinks");
		}
		if entry.is_directory {
			copy_consumer(source, &input, output, &destination, depth + 1, budget)?;
		} else if entry.is_file {
			let bytes = source.read(&input, MAX_INDEX)?;
			budget.1 = budget
				.1
				.checked_sub(bytes.len())
				.context("consumer source bytes exceed limit")?;
			output.atomic_write(&destination, &bytes)?;
		} else {
			bail!("consumer source must contain regular files");
		}
	}
	Ok(())
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let arguments = Arguments::parse_from(std::iter::once("registry".into()).chain(arguments));
	let RegistryCommand::Verify {
		root,
		package,
		output,
		consumer,
		all_features,
		allow_dirty,
		offline,
		upstream_cache,
	} = arguments.command;
	let fs = JailedFs::open(&root.unwrap_or(env::current_dir()?))?;
	if let Some(consumer) = &consumer {
		let manifest = fs.relative(consumer)?.join("Cargo.toml");
		reject_consumer_overrides(&String::from_utf8(fs.read(&manifest, MAX_INDEX)?)?.parse()?)?;
	}
	let seed = upstream_cache
		.as_ref()
		.map(|path| Ok::<_, anyhow::Error>(Arc::new(JailedFs::open(&fs.resource_path(path)?)?)))
		.transpose()?;
	let mut active_toolchain = Command::new("rustup");
	active_toolchain
		.current_dir(fs.root())
		.args(["show", "active-toolchain"]);
	let active_toolchain = String::from_utf8(run(active_toolchain, MAX_LOG)?)?;
	let toolchain = active_toolchain
		.split_whitespace()
		.next()
		.context("missing active toolchain")?
		.to_owned();
	let lockfile_sha256 = fs.hash_file(Path::new("Cargo.lock"), MAX_METADATA as u64)?;
	let mut metadata = cargo(fs.root());
	metadata.args(["metadata", "--no-deps", "--format-version", "1", "--locked"]);
	if offline {
		metadata.arg("--offline");
	}
	let (packages, first_party) =
		closure(&fs, serde_json::from_slice(&run(metadata, MAX_METADATA)?)?, &package)?;
	let consumer_features: Vec<String> = if all_features {
		packages
			.iter()
			.find(|member| member.name == package)
			.context("missing selected package")?
			.features
			.keys()
			.cloned()
			.collect()
	} else {
		Vec::new()
	};
	let output = fs.resource_path(&output)?;
	let staged = Arc::new(JailedFs::open_or_create(&output)?);
	if !staged.entries(Path::new(""), 4096)?.is_empty() {
		bail!(
			"registry staging output must be empty; preserve previous evidence and select a new \
			 directory"
		);
	}
	let _ownership = staged.lock(Path::new(".registry.lock"))?;
	let home = TempDir::new()?;
	let server = Server::start(Arc::clone(&staged), first_party, offline, seed)?;
	let target = staged.resource_path(Path::new("cargo-target"))?;
	let mut archives = Vec::new();
	for member in packages {
		println!("Verifying registry package {} {}", member.name, member.version);
		let mut command = staged_cargo(fs.root(), home.path(), &server, &toolchain);
		command
			.args(["package", "--locked", "--manifest-path"])
			.arg(&member.manifest_path)
			.arg("--target-dir")
			.arg(&target);
		if all_features && member.name == package {
			command.arg("--all-features");
		}
		if allow_dirty {
			command.arg("--allow-dirty");
		}
		run(command, MAX_LOG)?;
		let filename = format!("{}-{}.crate", member.name, member.version);
		let produced = target.join("package").join(&filename);
		let destination = PathBuf::from("archives").join(&filename);
		staged.atomic_write_with(&destination, |file| {
			fs.copy_file_to(&produced, file, MAX_ARCHIVE)?;
			Ok(())
		})?;
		let checksum = staged.hash_file(&destination, MAX_ARCHIVE)?;
		staged.atomic_write(
			&PathBuf::from("index").join(index_path(&member.name)?),
			&index_record(&member, &checksum)?,
		)?;
		archives.push(ArchiveReceipt {
			name:    member.name,
			version: member.version,
			path:    destination.to_string_lossy().into_owned(),
			sha256:  checksum,
		});
	}
	let subject = archives
		.iter()
		.find(|archive| archive.name == package)
		.context("missing selected archive")?;
	let detached = TempDir::new()?;
	let detached_fs = JailedFs::open(detached.path())?;
	if let Some(consumer) = &consumer {
		copy_consumer(
			&fs,
			&fs.relative(consumer)?,
			&detached_fs,
			Path::new(""),
			0,
			&mut (MAX_PACKAGES, MAX_METADATA),
		)?;
		reject_consumer_overrides(
			&String::from_utf8(detached_fs.read(Path::new("Cargo.toml"), MAX_INDEX)?)?.parse()?,
		)?;
	} else {
		detached_fs.atomic_write(
			Path::new("Cargo.toml"),
			format!(
				"[package]\nname = \"harvest-registry-consumer\"\nversion = \"0.0.0\"\nedition = \
				 \"2024\"\n[dependencies.subject]\npackage = {:?}\nversion = {:?}\nfeatures = {:?}\n",
				subject.name,
				format!("={}", subject.version),
				consumer_features
			)
			.as_bytes(),
		)?;
		detached_fs
			.atomic_write(Path::new("src/main.rs"), b"extern crate subject;\nfn main() {}\n")?;
	}
	let mut generate = staged_cargo(detached.path(), home.path(), &server, &toolchain);
	generate.arg("generate-lockfile");
	run(generate, MAX_LOG)?;
	let mut verify = staged_cargo(detached.path(), home.path(), &server, &toolchain);
	verify
		.args([if consumer.is_some() { "run" } else { "build" }, "--locked", "--target-dir"])
		.arg(staged.resource_path(Path::new("consumer-target"))?);
	let consumer_stdout = String::from_utf8(run(verify, MAX_LOG)?)?;
	let consumer_lockfile = detached_fs.read(Path::new("Cargo.lock"), MAX_INDEX)?;
	require_staged_consumer(&consumer_lockfile, subject)?;
	let consumer_lockfile_sha256 = harvest_runtime::content_hash(&consumer_lockfile);
	if fs.hash_file(Path::new("Cargo.lock"), MAX_METADATA as u64)? != lockfile_sha256 {
		bail!("workspace lockfile changed during registry verification");
	}
	let receipt = Receipt {
		schema_version: 1,
		verification: "real cargo package verification plus detached registry consumer; no runtime \
		               parity claim",
		package,
		all_features,
		allow_dirty,
		upstream_allowed: !offline,
		toolchain,
		lockfile_sha256,
		archives,
		consumer: if consumer.is_some() {
			"provided registry-only project built and ran"
		} else {
			"generated registry-only library consumer built"
		},
		consumer_stdout,
		consumer_lockfile_sha256,
		upstream_cache: upstream_cache
			.as_ref()
			.map(|path| path.to_string_lossy().into_owned()),
	};
	staged.atomic_write(Path::new("consumer.Cargo.lock"), &consumer_lockfile)?;
	staged.atomic_write(Path::new("receipt.json"), &serde_json::to_vec_pretty(&receipt)?)?;
	println!("Registry closure verified; receipt: {}", output.join("receipt.json").display());
	Ok(())
}
