use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use harvest_runtime::cancel::CancelToken;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::process_transport::{Framing, Process};

fn timeout() -> u32 {
	30_000
}
fn object() -> Value {
	json!({})
}
#[derive(Clone, Deserialize)]
pub struct ProcessConfiguration {
	pub name:       String,
	pub command:    PathBuf,
	#[serde(default)]
	pub args:       Vec<String>,
	#[serde(default)]
	pub env:        BTreeMap<String, String>,
	pub cwd:        Option<PathBuf>,
	#[serde(default = "timeout")]
	pub timeout_ms: u32,
}
impl std::fmt::Debug for ProcessConfiguration {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ServiceConfiguration")
			.field("name", &self.name)
			.field("timeout_ms", &self.timeout_ms)
			.finish_non_exhaustive()
	}
}
impl ProcessConfiguration {
	pub fn validate(&self) -> Result<()> {
		ensure!(
			!self.name.is_empty() && self.name.len() <= 256 && !self.command.as_os_str().is_empty(),
			"invalid native service identity"
		);
		ensure!((1..=600_000).contains(&self.timeout_ms), "invalid native service timeout");
		Ok(())
	}

	pub(crate) fn launch(&self, root: &Path, cancellation: &CancelToken) -> Result<Process<Value>> {
		self.validate()?;
		let cwd = self
			.cwd
			.as_ref()
			.map(|path| {
				if path.is_absolute() {
					path.clone()
				} else {
					root.join(path)
				}
			})
			.unwrap_or_else(|| root.to_path_buf())
			.canonicalize()
			.context("cannot resolve native service directory")?;
		let command = if self.command.is_relative() && self.command.components().count() > 1 {
			root.join(&self.command)
		} else {
			self.command.clone()
		};
		Process::launch_framed(
			&cwd,
			&command,
			&self.args,
			&self.env,
			Framing::ContentLength,
			cancellation,
		)
	}
}
#[derive(Clone, Deserialize)]
pub struct LspConfiguration {
	#[serde(flatten)]
	pub process:                ProcessConfiguration,
	#[serde(default, alias = "languageId")]
	pub language_id:            String,
	#[serde(default)]
	pub extensions:             Vec<String>,
	#[serde(default = "object", alias = "initOptions", alias = "initializationOptions")]
	pub initialization_options: Value,
	#[serde(default = "object")]
	pub settings:               Value,
	#[serde(default)]
	pub is_linter:              bool,
	/// Discovery is a new executable trust boundary, unlike an explicit host
	/// process configuration. Approval is checked immediately before launch.
	#[serde(default)]
	pub startup_approval:       bool,
}
impl LspConfiguration {
	pub fn validate(&self) -> Result<()> {
		self.process.validate()?;
		ensure!(
			self.language_id.len() <= 128
				&& self.settings.is_object()
				&& self.extensions.len() <= 128
				&& self
					.extensions
					.iter()
					.all(|e| !e.is_empty() && e.len() <= 256),
			"invalid native LSP configuration"
		);
		Ok(())
	}

	pub fn supports_path(&self, path: &Path) -> bool {
		let extension = path.extension().and_then(|v| v.to_str()).unwrap_or("");
		let file = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
		self.extensions.is_empty()
			|| self.extensions.iter().any(|candidate| {
				let candidate = candidate.trim_start_matches('.');
				candidate.eq_ignore_ascii_case(extension)
					|| candidate.eq_ignore_ascii_case(file.trim_start_matches('.'))
			})
	}

	pub fn document_language(&self, path: &Path) -> String {
		if !self.language_id.is_empty() {
			return self.language_id.clone();
		}
		let extension = path
			.extension()
			.and_then(|v| v.to_str())
			.unwrap_or("")
			.to_ascii_lowercase();
		match extension.as_str() {
			"tsx" => "typescriptreact".into(),
			"jsx" => "javascriptreact".into(),
			"tf" => "terraform".into(),
			_ => pi_ast::language::SupportLang::from_path(path)
				.map(|language| {
					match language {
						pi_ast::language::SupportLang::Bash => "shellscript",
						_ => language.canonical_name(),
					}
					.to_owned()
				})
				.unwrap_or_else(|| "plaintext".into()),
		}
	}
}
#[derive(Clone, Deserialize)]
pub struct DapConfiguration {
	#[serde(flatten)]
	pub process:         ProcessConfiguration,
	#[serde(default = "object")]
	pub launch_defaults: Value,
	#[serde(default = "object")]
	pub attach_defaults: Value,
}
impl DapConfiguration {
	pub fn validate(&self) -> Result<()> {
		self.process.validate()?;
		ensure!(
			self.launch_defaults.is_object() && self.attach_defaults.is_object(),
			"invalid native debugger defaults"
		);
		Ok(())
	}
}

impl std::fmt::Debug for LspConfiguration {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("LspConfiguration")
			.field("process", &self.process)
			.finish_non_exhaustive()
	}
}
impl std::fmt::Debug for DapConfiguration {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("DapConfiguration")
			.field("process", &self.process)
			.finish_non_exhaustive()
	}
}
