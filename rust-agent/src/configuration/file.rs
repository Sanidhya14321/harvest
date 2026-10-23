//! Typed configuration ownership. Ordinary reads never migrate or rewrite data.
//! Cache invalidation is explicit; callers do not silently replace an invalid
//! primary file with a valid fallback or stale defaults.
use std::{
	fmt,
	path::{Path, PathBuf},
	sync::Arc,
	time::SystemTime,
};

use harvest_runtime::{BoundedWriter, JailedFs, cancel::CancelToken};
use serde::{Serialize, de::DeserializeOwned};

const MAX_CONFIG_BYTES: usize = 256 * 1024;
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorStage {
	Open,
	Read,
	Decode,
	Validation,
	Lock,
	Serialize,
	Publish,
	Cancelled,
}
/// Contains sanitized diagnostics only. Never attach input/parser/validator
/// error chains, which may contain credentials or arbitrary source excerpts.
#[derive(Clone, Debug)]
pub struct ConfigError {
	pub stage: ErrorStage,
	message:   String,
}
impl ConfigError {
	fn new(stage: ErrorStage, message: &str) -> Self {
		Self { stage, message: message.into() }
	}
}
impl fmt::Display for ConfigError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(&self.message)
	}
}
impl std::error::Error for ConfigError {}
pub type LoadResult<T> = std::result::Result<Option<Arc<T>>, ConfigError>;
type Validation<T> = Box<dyn Fn(&T) -> anyhow::Result<()> + Send + Sync>;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MigrationOutcome {
	NotApplicable,
	DestinationPresent,
	FallbackPresent,
	SourceMissing,
	Created { bytes: usize, sha256: String },
}
/// A persistent SDK owner may reuse cached success, missing and failure results
/// until invalidate/relocate/validation changes. No cached value is printed by
/// Debug; values are shared immutably through Arc rather than copied per load.
pub struct ConfigFile<T> {
	filesystem:  JailedFs,
	primary:     PathBuf,
	fallback:    Option<PathBuf>,
	json_source: Option<PathBuf>,
	cache:       Option<LoadResult<T>>,
	validations: Vec<Validation<T>>,
}
impl<T> fmt::Debug for ConfigFile<T> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("ConfigFile")
			.field("cached", &self.cache.is_some())
			.finish_non_exhaustive()
	}
}
impl<T: DeserializeOwned> ConfigFile<T> {
	pub fn open(root: &Path, path: &Path) -> std::result::Result<Self, ConfigError> {
		let filesystem = JailedFs::open(root)
			.map_err(|_| ConfigError::new(ErrorStage::Open, "cannot open configuration root"))?;
		let primary = filesystem.relative(path).map_err(|_| {
			ConfigError::new(ErrorStage::Open, "configuration path is outside its root")
		})?;
		if primary.file_name().is_none() {
			return Err(ConfigError::new(ErrorStage::Open, "configuration path requires a filename"));
		}
		let mut result = Self {
			filesystem,
			primary,
			fallback: None,
			json_source: None,
			cache: None,
			validations: Vec::new(),
		};
		result.configure_paths();
		Ok(result)
	}

	fn configure_paths(&mut self) {
		self.fallback = (self.primary.extension().is_some_and(|s| s == "yml"))
			.then(|| self.primary.with_extension("yaml"));
		self.json_source = self
			.primary
			.extension()
			.filter(|s| *s == "yaml" || *s == "yml")
			.map(|_| self.primary.with_extension("json"));
	}

	pub fn invalidate(&mut self) {
		self.cache = None;
	}

	/// Keep the same filesystem capability and validators. Relocation never
	/// authorizes another root or silently performs a data migration.
	pub fn relocate(&mut self, path: &Path) -> std::result::Result<(), ConfigError> {
		let primary = self.filesystem.relative(path).map_err(|_| {
			ConfigError::new(ErrorStage::Open, "configuration path is outside its root")
		})?;
		if primary.file_name().is_none() {
			return Err(ConfigError::new(ErrorStage::Open, "configuration path requires a filename"));
		}
		if primary != self.primary {
			self.primary = primary;
			self.configure_paths();
			self.invalidate();
		}
		Ok(())
	}

	pub fn add_validation(
		&mut self,
		validation: impl Fn(&T) -> anyhow::Result<()> + Send + Sync + 'static,
	) -> std::result::Result<(), ConfigError> {
		if self.validations.len() >= 16 {
			return Err(ConfigError::new(ErrorStage::Validation, "too many configuration validators"));
		}
		self.validations.push(Box::new(validation));
		self.invalidate();
		Ok(())
	}

	fn validate(&self, value: &T) -> std::result::Result<(), ConfigError> {
		for validation in &self.validations {
			validation(value).map_err(|_| {
				ConfigError::new(ErrorStage::Validation, "configuration auxiliary validation failed")
			})?;
		}
		Ok(())
	}

	fn exists(&self, path: &Path) -> std::result::Result<bool, ConfigError> {
		self
			.filesystem
			.exists(path)
			.map_err(|_| ConfigError::new(ErrorStage::Read, "cannot inspect configuration path"))
	}

	/// Resolve the current path independently of cached contents. Invalid or
	/// unreadable primary contents are errors, never reasons to try fallback.
	pub fn path(&self) -> std::result::Result<PathBuf, ConfigError> {
		if !self.exists(&self.primary)?
			&& let Some(fallback) = &self.fallback
			&& self.exists(fallback)?
		{
			return Ok(fallback.clone());
		}
		Ok(self.primary.clone())
	}

	pub fn modified(&self) -> std::result::Result<Option<SystemTime>, ConfigError> {
		let path = self.path()?;
		if !self.exists(&path)? {
			return Ok(None);
		}
		let file = self
			.filesystem
			.open_read(&path, MAX_CONFIG_BYTES as u64)
			.map_err(|_| ConfigError::new(ErrorStage::Read, "cannot open configuration metadata"))?;
		file
			.metadata()
			.and_then(|m| m.modified())
			.map(Some)
			.map_err(|_| {
				ConfigError::new(ErrorStage::Read, "cannot inspect configuration modification time")
			})
	}

	fn uncached(&self) -> LoadResult<T> {
		let path = self.path()?;
		let Some(bytes) = self
			.filesystem
			.read_optional(&path, MAX_CONFIG_BYTES)
			.map_err(|_| {
				ConfigError::new(ErrorStage::Read, "cannot read configuration within byte limit")
			})?
		else {
			return Ok(None);
		};
		let source = std::str::from_utf8(&bytes)
			.map_err(|_| ConfigError::new(ErrorStage::Decode, "configuration is not UTF-8"))?;
		let label = if path == Path::new(".omp/config.toml") {
			".omp/config.toml"
		} else {
			"native configuration"
		};
		let value: T = super::document::parse(source, &path, label)
			.map_err(|error| ConfigError::new(ErrorStage::Decode, &error.to_string()))?;
		self.validate(&value)?;
		Ok(Some(Arc::new(value)))
	}

	pub fn try_load(&mut self) -> LoadResult<T> {
		if self.cache.is_none() {
			self.cache = Some(self.uncached());
		}
		self
			.cache
			.as_ref()
			.expect("load result initialized")
			.clone()
	}

	pub fn create_default(&self) -> std::result::Result<T, ConfigError>
	where
		T: Default,
	{
		let value = T::default();
		self.validate(&value)?;
		Ok(value)
	}

	/// Explicit legacy fallback semantics: missing AND failed loads use a
	/// validated default. Use try_load when failures must remain observable.
	pub fn load_or_default(&mut self) -> std::result::Result<Arc<T>, ConfigError>
	where
		T: Default,
	{
		match self.try_load() {
			Ok(Some(value)) => Ok(value),
			_ => self.create_default().map(Arc::new),
		}
	}

	/// Explicit, idempotent format migration. Preserve unknown JSON fields and
	/// the original source; never overwrite either existing YAML spelling.
	/// Callers retain responsibility for their full settings-schema migration.
	pub fn migrate_json(
		&mut self,
		cancellation: &CancelToken,
	) -> std::result::Result<MigrationOutcome, ConfigError> {
		self.invalidate();
		let Some(json_source) = &self.json_source else {
			return Ok(MigrationOutcome::NotApplicable);
		};
		let alternative =
			self
				.primary
				.with_extension(if self.primary.extension().is_some_and(|e| e == "yml") {
					"yaml"
				} else {
					"yml"
				});
		let heartbeat = || {
			cancellation.heartbeat().map_err(|_| {
				ConfigError::new(ErrorStage::Cancelled, "configuration migration cancelled")
			})
		};
		heartbeat()?;
		if self.exists(&self.primary)? {
			return Ok(MigrationOutcome::DestinationPresent);
		}
		if self.exists(&alternative)? {
			return Ok(MigrationOutcome::FallbackPresent);
		}
		if !self.exists(json_source)? {
			return Ok(MigrationOutcome::SourceMissing);
		}
		let _owner = self.filesystem.resource_lock(json_source).map_err(|_| {
			ConfigError::new(ErrorStage::Lock, "cannot acquire configuration migration ownership")
		})?;
		heartbeat()?;
		if self.exists(&self.primary)? {
			return Ok(MigrationOutcome::DestinationPresent);
		}
		if self.exists(&alternative)? {
			return Ok(MigrationOutcome::FallbackPresent);
		}
		let Some(bytes) = self
			.filesystem
			.read_optional(json_source, MAX_CONFIG_BYTES)
			.map_err(|_| {
				ConfigError::new(ErrorStage::Read, "cannot read configuration migration source")
			})?
		else {
			return Ok(MigrationOutcome::SourceMissing);
		};
		let source = std::str::from_utf8(&bytes)
			.map_err(|_| ConfigError::new(ErrorStage::Decode, "configuration is not UTF-8"))?;
		let raw: serde_json::Value =
			super::document::parse(source, json_source, "configuration migration source")
				.map_err(|error| ConfigError::new(ErrorStage::Decode, &error.to_string()))?;
		let value = T::deserialize(raw.clone()).map_err(|_| {
			ConfigError::new(ErrorStage::Decode, "invalid configuration migration schema")
		})?;
		self.validate(&value)?;
		heartbeat()?;
		let mut output = BoundedWriter::new(MAX_CONFIG_BYTES);
		serde_yaml_ng::to_writer(&mut output, &raw).map_err(|_| {
			ConfigError::new(ErrorStage::Serialize, "cannot serialize configuration within byte limit")
		})?;
		let output = output.into_inner();
		let yaml = std::str::from_utf8(&output).map_err(|_| {
			ConfigError::new(ErrorStage::Serialize, "configuration serializer returned invalid UTF-8")
		})?;
		let roundtrip: serde_json::Value = super::document::parse(
			yaml,
			&self.primary,
			"configuration migration result",
		)
		.map_err(|_| {
			ConfigError::new(ErrorStage::Serialize, "configuration migration cannot preserve data")
		})?;
		if roundtrip != raw {
			return Err(ConfigError::new(
				ErrorStage::Serialize,
				"configuration migration changed data",
			));
		}
		heartbeat()?;
		let source_hash = harvest_runtime::content_hash(&bytes);
		if self
			.filesystem
			.hash_file(json_source, MAX_CONFIG_BYTES as u64)
			.map_err(|_| {
				ConfigError::new(ErrorStage::Read, "cannot recheck configuration migration source")
			})? != source_hash
		{
			return Err(ConfigError::new(
				ErrorStage::Publish,
				"configuration migration source changed; retry after writers stop",
			));
		}
		heartbeat()?;
		self
			.filesystem
			.atomic_create(&self.primary, &output)
			.map_err(|_| {
				ConfigError::new(
					ErrorStage::Publish,
					"cannot publish configuration migration without replacing data",
				)
			})?;
		Ok(MigrationOutcome::Created {
			bytes:  output.len(),
			sha256: harvest_runtime::content_hash(&output),
		})
	}
}
