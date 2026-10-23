use std::{
	collections::{BTreeMap, BTreeSet},
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;

fn timeout() -> u32 {
	30_000
}
fn enabled() -> bool {
	true
}
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestIdFormat {
	#[default]
	Number,
	String,
}
#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Transport {
	Stdio {
		command: PathBuf,
		#[serde(default)]
		args:    Vec<String>,
		#[serde(default)]
		env:     BTreeMap<String, String>,
		cwd:     Option<PathBuf>,
	},
	Http {
		url:     String,
		#[serde(default)]
		headers: BTreeMap<String, String>,
	},
	Sse {
		url:     String,
		#[serde(default)]
		headers: BTreeMap<String, String>,
	},
}
#[derive(Clone, Deserialize)]
pub struct Configuration {
	pub name:              String,
	#[serde(default = "timeout")]
	pub timeout_ms:        u32,
	#[serde(default = "enabled")]
	pub enabled:           bool,
	#[serde(default, alias = "requestIdFormat")]
	pub request_id_format: RequestIdFormat,
	#[serde(flatten)]
	pub transport:         Transport,
}
impl std::fmt::Debug for Configuration {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("McpConfiguration")
			.field("name", &self.name)
			.field("timeout_ms", &self.timeout_ms)
			.field("enabled", &self.enabled)
			.finish_non_exhaustive()
	}
}
impl Configuration {
	pub fn validate(&self) -> Result<()> {
		ensure!(!self.name.is_empty() && self.name.len() <= 256, "invalid MCP server name");
		ensure!(
			(1..=600_000).contains(&self.timeout_ms),
			"native MCP timeout must be between 1 and 600000 milliseconds; unbounded timeout \
			 conversion is pending"
		);
		Ok(())
	}
}

/// Strict JSON import of explicitly selected legacy mcpServers files. External
/// source discovery, JSONC and OAuth expansion remain separate conversion work.
pub fn load_file(root: &Path, path: &Path) -> Result<Vec<Configuration>> {
	let bytes = if path.is_absolute() {
		let parent = path.parent().context("MCP configuration has no parent")?;
		let name = path
			.file_name()
			.context("MCP configuration has no filename")?;
		harvest_runtime::JailedFs::open(parent)?.read(Path::new(name), 1024 * 1024)?
	} else {
		harvest_runtime::JailedFs::open(root)?.read(path, 1024 * 1024)?
	};
	let document: Value = serde_json::from_slice(&bytes)
		.map_err(|_| anyhow::anyhow!("invalid MCP JSON configuration"))?;
	let servers = document
		.get("mcpServers")
		.and_then(Value::as_object)
		.context("MCP configuration requires mcpServers")?;
	ensure!(servers.len() <= 32, "too many MCP servers");
	fn names(value: Option<&Value>) -> Result<BTreeSet<String>> {
		let Some(value) = value else {
			return Ok(BTreeSet::new());
		};
		value
			.as_array()
			.context("invalid MCP enable/disable list")?
			.iter()
			.map(|name| {
				name
					.as_str()
					.map(str::to_owned)
					.context("invalid MCP server selector")
			})
			.collect()
	}
	let disabled = names(document.get("disabledServers"))?;
	let enabled = names(document.get("enabledServers"))?;
	let mut result = Vec::new();
	for (name, server) in servers {
		if disabled.contains(name) {
			continue;
		}
		let mut server = server
			.as_object()
			.context("invalid MCP server configuration")?
			.clone();
		if server.get("enabled") == Some(&Value::Bool(false)) && !enabled.contains(name) {
			continue;
		}
		ensure!(
			!server.contains_key("auth") && !server.contains_key("oauth"),
			"native MCP OAuth/credential discovery is not converted; explicit host headers are \
			 required"
		);
		ensure!(
			server
				.get("headerPolicy")
				.is_none_or(|value| value == "origin-locked"),
			"unsupported MCP header policy"
		);
		ensure!(
			server
				.get("envPolicy")
				.is_none_or(|value| value == "literal"),
			"unsupported MCP environment policy"
		);
		server.remove("headerPolicy");
		server.remove("envPolicy");
		server.remove("envLiteralKeys");
		server.insert("name".into(), Value::String(name.clone()));
		server.insert("enabled".into(), Value::Bool(true));
		server
			.entry("type")
			.or_insert(Value::String("stdio".into()));
		if let Some(timeout) = server.remove("timeout") {
			server.insert("timeout_ms".into(), timeout);
		}
		let configuration: Configuration = serde_json::from_value(Value::Object(server))
			.map_err(|_| anyhow::anyhow!("invalid native MCP server configuration"))?;
		configuration.validate()?;
		result.push(configuration);
	}
	Ok(result)
}
