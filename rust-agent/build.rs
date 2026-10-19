//! Keep CLI identity tied to the inherited workspace version in both build
//! systems; registry-normalized manifests use Cargo's resolved package version.
use std::{env, fs, path::Path};

fn main() {
	let directory = env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory is required");
	let manifest = Path::new(&directory).join("Cargo.toml");
	println!("cargo:rerun-if-changed={}", manifest.display());
	let own: toml::Value =
		toml::from_str(&fs::read_to_string(&manifest).expect("read application manifest"))
			.expect("parse application manifest");
	let inherited = own
		.get("package")
		.and_then(|value| value.get("version"))
		.and_then(|value| value.get("workspace"))
		.and_then(toml::Value::as_bool)
		.unwrap_or(false);
	let version = if inherited {
		let root = Path::new(&directory).join("../Cargo.toml");
		println!("cargo:rerun-if-changed={}", root.display());
		let workspace: toml::Value =
			toml::from_str(&fs::read_to_string(root).expect("read workspace manifest"))
				.expect("parse workspace manifest");
		workspace
			.get("workspace")
			.and_then(|value| value.get("package"))
			.and_then(|value| value.get("version"))
			.and_then(toml::Value::as_str)
			.expect("workspace package version is required")
			.to_owned()
	} else {
		env::var("CARGO_PKG_VERSION").expect("resolved Cargo package version is required")
	};
	semver::Version::parse(&version).expect("CLI version must be valid semver");
	println!("cargo:rustc-env=HARVEST_VERSION={version}");
}
