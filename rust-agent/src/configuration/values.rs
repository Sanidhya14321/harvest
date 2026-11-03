//! Live provider values. Keep secrets out of debug/errors and share one bounded
//! command cache across all request clones in an explicitly selected workspace.
use std::{
	collections::BTreeMap,
	fmt,
	path::{Path, PathBuf},
	sync::{Mutex, TryLockError},
	time::{Duration, Instant},
};

use anyhow::{Result, bail, ensure};
use harvest_runtime::{JailedFs, cancel::CancelToken, content_hash};

mod command;
const MAX_VALUE: usize = 16 * 1024;
const MAX_CACHE: usize = 128;
const MAX_CACHE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Default)]
pub struct ProviderValues {
	/// Only configuration-file values use env-name / !command interpretation.
	/// CLI/environment API keys remain literal credentials in Settings.
	pub api_key: Option<String>,
	pub headers: BTreeMap<String, String>,
}
impl fmt::Debug for ProviderValues {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("ProviderValues")
			.field("key_source_configured", &self.api_key.is_some())
			.field("header_count", &self.headers.len())
			.finish()
	}
}
impl ProviderValues {
	pub fn validate(&self) -> Result<()> {
		ensure!(self.headers.len() <= 128, "too many provider headers");
		let mut total = self.api_key.as_ref().map_or(0, String::len);
		if let Some(key) = &self.api_key {
			ensure!(key.len() <= MAX_VALUE, "provider key source exceeds limit");
		}
		let mut names = std::collections::BTreeSet::new();
		for (name, value) in &self.headers {
			validate_name(name)?;
			ensure!(
				names.insert(name.to_ascii_lowercase()),
				"duplicate case-insensitive provider header"
			);
			ensure!(value.len() <= MAX_VALUE, "provider header source exceeds limit");
			total += name.len() + value.len();
		}
		ensure!(total <= 64 * 1024, "provider value sources exceed total limit");
		Ok(())
	}

	pub fn is_empty(&self) -> bool {
		self.api_key.is_none() && self.headers.is_empty()
	}
}
fn validate_name(name: &str) -> Result<()> {
	ensure!(
		!name.is_empty()
			&& name.len() <= 128
			&& name
				.bytes()
				.all(|c| c.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&c)),
		"invalid provider header name"
	);
	ensure!(
		!matches!(
			name.to_ascii_lowercase().as_str(),
			"host" | "content-length" | "transfer-encoding" | "connection" | "content-type" | "accept"
		),
		"provider header conflicts with transport ownership"
	);
	Ok(())
}
fn validate_value(value: &str) -> Result<()> {
	ensure!(
		value.len() <= MAX_VALUE && !value.bytes().any(|b| b < 0x20 && b != b'\t' || b == 0x7f),
		"invalid or oversized provider header value"
	);
	Ok(())
}
fn exact_environment(source: &str) -> Option<String> {
	if source.is_empty() || source.contains(['\0', '=']) {
		return None;
	}
	#[cfg(windows)]
	{
		// Windows getters fold case; enumeration preserves the real key spelling.
		std::env::vars_os()
			.find(|(name, _)| name.as_os_str() == std::ffi::OsStr::new(source))
			.and_then(|(_, value)| value.into_string().ok())
			.filter(|v| !v.is_empty())
	}
	#[cfg(not(windows))]
	{
		std::env::var(source).ok().filter(|v| !v.is_empty())
	}
}
struct Cached {
	value:    Option<String>,
	retry_at: Option<Instant>,
}
/// A cache entry never contains the command text; identity is a source hash.
/// Success persists until explicit invalidation or bounded eviction. Failures
/// back off for 30 seconds; cancellation is not negative-cached.
pub struct ValueResolver {
	root:           PathBuf,
	allow_commands: bool,
	cache:          Mutex<BTreeMap<String, Cached>>,
}
impl fmt::Debug for ValueResolver {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("ValueResolver")
			.field("commands_enabled", &self.allow_commands)
			.finish_non_exhaustive()
	}
}
impl ValueResolver {
	pub fn new(root: &Path, allow_commands: bool) -> Result<Self> {
		let root = JailedFs::open(root)?.root().to_owned();
		Ok(Self { root, allow_commands, cache: Mutex::new(BTreeMap::new()) })
	}

	fn lock(
		&self,
		cancellation: &CancelToken,
	) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, Cached>>> {
		loop {
			cancellation.heartbeat()?;
			match self.cache.try_lock() {
				Ok(cache) => return Ok(cache),
				Err(TryLockError::Poisoned(_)) => bail!("provider value cache is unavailable"),
				Err(TryLockError::WouldBlock) => std::thread::park_timeout(Duration::from_millis(5)),
			}
		}
	}

	pub fn invalidate(&self, value: &str, cancellation: &CancelToken) -> Result<()> {
		if let Some(command) = value.strip_prefix('!') {
			self
				.lock(cancellation)?
				.remove(&content_hash(command.trim().as_bytes()));
		}
		Ok(())
	}

	pub fn resolve(
		&self,
		source: &str,
		refresh: bool,
		cancellation: &CancelToken,
	) -> Result<Option<String>> {
		cancellation.heartbeat()?;
		ensure!(source.len() <= MAX_VALUE, "provider value source exceeds limit");
		let Some(command) = source.strip_prefix('!') else {
			// Invalid OS environment keys are literal values, never panicking
			// environment lookups. Empty variables retain literal fallback.
			let environment = exact_environment(source);
			let value = environment.unwrap_or_else(|| source.to_owned());
			validate_value(&value)?;
			return Ok(Some(value));
		};
		ensure!(
			self.allow_commands,
			"credential commands require explicit host opt-in (--allow-credential-commands)"
		);
		let command = command.trim();
		ensure!(!command.is_empty(), "credential command is empty");
		let identity = content_hash(command.as_bytes());
		let mut cache = self.lock(cancellation)?;
		if refresh {
			cache.remove(&identity);
		}
		if let Some(previous) = cache.get(&identity) {
			if previous.retry_at.is_none_or(|at| Instant::now() < at) {
				return Ok(previous.value.clone());
			}
		}
		cache.remove(&identity);
		let value = command::execute(&self.root, command, cancellation)?;
		cancellation.heartbeat()?;
		let value = value.filter(|v| !v.is_empty() && validate_value(v).is_ok());
		let cost = identity.len() + value.as_ref().map_or(0, String::len);
		while cache.len() >= MAX_CACHE
			|| cache
				.iter()
				.map(|(k, v)| k.len() + v.value.as_ref().map_or(0, String::len))
				.sum::<usize>()
				+ cost > MAX_CACHE_BYTES
		{
			let Some(key) = cache.keys().next().cloned() else {
				break;
			};
			cache.remove(&key);
		}
		cache.insert(identity, Cached {
			retry_at: value
				.is_none()
				.then(|| Instant::now() + Duration::from_secs(30)),
			value:    value.clone(),
		});
		Ok(value)
	}

	/// Flattened sources are folded once, in order. Failed/empty values do not
	/// erase an earlier source. Header identity is case-insensitive on the wire.
	pub fn headers(
		&self,
		sources: &[&BTreeMap<String, String>],
		cancellation: &CancelToken,
	) -> Result<BTreeMap<String, String>> {
		ensure!(
			sources.len() <= 32 && sources.iter().map(|s| s.len()).sum::<usize>() <= 1024,
			"provider header source budget exceeded"
		);
		for source in sources {
			for name in source.keys() {
				validate_name(name)?;
			}
		}
		let mut output: BTreeMap<String, String> = BTreeMap::new();
		let mut total = 0;
		for source in sources {
			for (name, value) in *source {
				if let Some(value) = self
					.resolve(value, false, cancellation)?
					.filter(|v| !v.is_empty())
				{
					let name = name.to_ascii_lowercase();
					let old = output.get(&name);
					let next_total =
						total - old.map_or(0, |v| name.len() + v.len()) + name.len() + value.len();
					ensure!(
						next_total <= 64 * 1024 && (old.is_some() || output.len() < 128),
						"resolved provider headers exceed limit"
					);
					total = next_total;
					output.insert(name, value);
				}
			}
		}
		ensure!(
			output.len() <= 128
				&& output.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>() <= 64 * 1024,
			"resolved provider headers exceed limit"
		);
		Ok(output)
	}
}

pub(crate) fn check_headers(headers: &BTreeMap<String, String>) -> Result<()> {
	ensure!(
		headers.len() <= 128
			&& headers
				.iter()
				.map(|(k, v)| k.len() + v.len())
				.sum::<usize>()
				<= 64 * 1024,
		"resolved provider headers exceed limit"
	);
	for (name, value) in headers {
		validate_name(name)?;
		validate_value(value)?;
	}
	Ok(())
}
