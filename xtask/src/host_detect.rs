//! Native host identity consumed by the transitional addon builders.
use std::env;

use anyhow::{Result, bail};

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HostAddon {
	platform:    &'static str,
	arch:        &'static str,
	filename:    String,
	x64_variant: Option<&'static str>,
	avx2:        bool,
}

pub fn detect_avx2() -> bool {
	#[cfg(target_arch = "x86_64")]
	{
		std::arch::is_x86_feature_detected!("avx2")
	}
	#[cfg(not(target_arch = "x86_64"))]
	{
		false
	}
}

fn resolve(os: &str, arch: &str, avx2: bool) -> Result<HostAddon> {
	let platform = match os {
		"linux" => "linux",
		"macos" => "darwin",
		"windows" => "win32",
		_ => bail!("unsupported native addon operating system: {os}"),
	};
	let arch = match arch {
		"x86_64" => "x64",
		"aarch64" => "arm64",
		_ => bail!("unsupported native addon architecture: {arch}"),
	};
	let avx2 = arch == "x64" && avx2;
	let x64_variant = (arch == "x64").then_some(if avx2 { "modern" } else { "baseline" });
	let suffix = x64_variant.map(|v| format!("-{v}")).unwrap_or_default();
	let filename = format!("pi_natives.{platform}-{arch}{suffix}.node");
	Ok(HostAddon { platform, arch, filename, x64_variant, avx2 })
}

pub fn execute() -> Result<()> {
	let addon = resolve(env::consts::OS, env::consts::ARCH, detect_avx2())?;
	println!("{}", serde_json::to_string(&addon)?);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::resolve;

	#[test]
	fn native_builders_receive_node_platform_arch_variant_and_exact_addon_filename() {
		for (os, platform) in [("linux", "linux"), ("macos", "darwin"), ("windows", "win32")] {
			for (rust_arch, node_arch, avx2, variant) in [
				("x86_64", "x64", false, Some("baseline")),
				("x86_64", "x64", true, Some("modern")),
				("aarch64", "arm64", true, None),
			] {
				let output = serde_json::to_value(resolve(os, rust_arch, avx2).unwrap()).unwrap();
				let suffix = variant.map(|v| format!("-{v}")).unwrap_or_default();
				assert_eq!(
					output,
					serde_json::json!({
						 "platform": platform, "arch": node_arch,
						 "filename": format!("pi_natives.{platform}-{node_arch}{suffix}.node"),
						 "x64Variant": variant, "avx2": node_arch == "x64" && avx2,
					})
				);
			}
		}
	}

	#[test]
	fn unsupported_hosts_fail_instead_of_emitting_an_unloadable_addon_name() {
		assert!(resolve("freebsd", "x86_64", true).is_err());
		assert!(resolve("linux", "riscv64", false).is_err());
	}
}
