//! Configuration errors and SDK diagnostics must not expose credentials.
use std::{fs, process::Command};

use anyhow::Result;
use harvest_omp::config::{self, Environment, Overrides};

const KEY: &str = "credential-from-config-7b360";
const URL: &str = "https://url-user:url-password@localhost/private?token=query-secret";

#[test]
fn sdk_configuration_debug_omits_credentials_from_all_resolution_layers() -> Result<()> {
	let root = tempfile::tempdir()?;
	let flags = Overrides {
		model:     Some("fixture".into()),
		base_url:  Some(URL.into()),
		api_key:   Some(KEY.into()),
		max_steps: Some(2),
	};
	let environment = Environment {
		model:     Some("fixture".into()),
		base_url:  Some(URL.into()),
		api_key:   Some(KEY.into()),
		max_steps: Some(2),
	};
	let settings = config::resolve(root.path(), &flags, &environment)?;
	// The public Debug boundary is commonly consumed by embedders' loggers.
	for debug in [
		format!("{flags:?} {flags:#?}"),
		format!("{environment:?} {environment:#?}"),
		format!("{settings:?} {settings:#?}"),
	] {
		for sensitive in [KEY, URL, "url-user", "url-password", "query-secret"] {
			assert!(!debug.contains(sensitive), "credential leaked from SDK Debug");
		}
	}
	Ok(())
}

#[test]
fn invalid_toml_and_environment_errors_redact_underlying_source_and_error_chains() -> Result<()> {
	let root = tempfile::tempdir()?;
	fs::create_dir(root.path().join(".omp"))?;
	for text in [
		format!("api_key = \"{KEY}"),
		format!("base_url = \"{URL}"),
		format!("api_key = \"{KEY}\"\nmax_steps = \"{KEY}\"\n"),
		format!("base_url = [\"{URL}\"]\n"),
	] {
		fs::write(root.path().join(".omp/config.toml"), &text)?;
		let error =
			config::resolve(root.path(), &Overrides::default(), &Environment::default()).unwrap_err();
		let diagnostic = format!("{error:#} {error:?}");
		assert!(diagnostic.contains("failed to parse .omp/config.toml"));
		for sensitive in [KEY, URL, "url-password", "query-secret"] {
			assert!(!diagnostic.contains(sensitive), "configuration error leaked source bytes");
		}
		assert_eq!(fs::read_to_string(root.path().join(".omp/config.toml"))?, text);
	}
	let error =
		config::environment_from(|name| (name == "OMP_MAX_STEPS").then(|| KEY.into())).unwrap_err();
	let diagnostic = format!("{error:#} {error:?}");
	assert!(diagnostic.contains("OMP_MAX_STEPS"));
	assert!(!diagnostic.contains(KEY));
	Ok(())
}

#[test]
fn both_cli_aliases_fail_without_revealing_invalid_config_or_opening_a_session() -> Result<()> {
	for binary in [env!("CARGO_BIN_EXE_harvest"), env!("CARGO_BIN_EXE_omp")] {
		let root = tempfile::tempdir()?;
		fs::create_dir(root.path().join(".omp"))?;
		fs::write(root.path().join(".omp/config.toml"), format!("api_key = \"{KEY}"))?;
		let output = Command::new(binary)
			.args(["run", "--workspace"])
			.arg(root.path())
			.args(["--prompt", "fixture"])
			.env_remove("OMP_MAX_STEPS")
			.output()?;
		assert!(!output.status.success());
		let diagnostic = String::from_utf8_lossy(&output.stderr);
		assert!(diagnostic.contains("failed to parse .omp/config.toml"));
		assert!(!diagnostic.contains(KEY));
		assert!(!root.path().join(".omp/session.jsonl").exists());
		assert!(output.stdout.is_empty());
	}
	Ok(())
}
