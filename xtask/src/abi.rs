//! Fail closed on GNU symbol requirements above the shipping portability floor.
use std::path::Path;

#[cfg(any(target_os = "linux", test))]
use anyhow::Context;
use anyhow::{Result, bail};
use harvest_runtime::JailedFs;
#[cfg(any(target_os = "linux", test))]
use regex::Regex;
use semver::Version;

pub fn verify_glibc(fs: &JailedFs, path: &Path, ceiling: &str) -> Result<String> {
	let ceiling = parse_version(ceiling)?;
	#[cfg(target_os = "linux")]
	{
		let input = fs.open_read(path, 256 * 1024 * 1024)?;
		let mut command = std::process::Command::new("readelf");
		// The child receives the already authorized inode; it never reopens an
		// ambient workspace path after a jail check.
		command
			.args(["--version-info", "--wide", "/proc/self/fd/0"])
			.stdin(input);
		let output = crate::process::capture(command, 1024 * 1024)
			.context("GNU ABI inspection requires working Linux binutils/readelf")?;
		let required = required_glibc(std::str::from_utf8(&output)?, &ceiling)?;
		Ok(serde_json::to_string(
			&serde_json::json!({"maxRequired":required.map(|version| version.to_string()),"ceiling":ceiling.to_string()}),
		)?)
	}
	#[cfg(not(target_os = "linux"))]
	{
		let _ = (fs, path, ceiling);
		bail!("GNU ABI inspection requires a Linux host")
	}
}

fn parse_version(version: &str) -> Result<Version> {
	let parts = version
		.split('.')
		.map(str::parse::<u64>)
		.collect::<Result<Vec<_>, _>>()?;
	match parts.as_slice() {
		[major, minor] => Ok(Version::new(*major, *minor, 0)),
		[major, minor, patch] => Ok(Version::new(*major, *minor, *patch)),
		_ => bail!("GNU ABI versions must contain two or three integer components"),
	}
}

#[cfg(any(target_os = "linux", test))]
fn required_glibc(output: &str, ceiling: &Version) -> Result<Option<Version>> {
	let pattern = Regex::new(r"\bGLIBC_([A-Za-z0-9_.]+)")?;
	let mut highest = None;
	for capture in pattern.captures_iter(output) {
		let version = parse_version(&capture[1]).context("unsupported GNU ABI requirement")?;
		if version > *ceiling {
			bail!("binary requires GLIBC_{version}, above portability ceiling {ceiling}")
		}
		highest = highest.max(Some(version));
	}
	Ok(highest)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn gnu_versions_compare_numerically_and_do_not_confuse_cxx_requirements() -> Result<()> {
		assert_eq!(
			required_glibc(
				"GLIBC_2.9 GLIBCXX_3.4.30 GLIBC_2.2.5 GLIBC_2.17",
				&parse_version("2.17")?
			)?,
			Some(Version::new(2, 17, 0))
		);
		assert_eq!(required_glibc("No version information found", &parse_version("2.17")?)?, None);
		for invalid in
			["GLIBC_2.39", "GLIBC_PRIVATE", "GLIBC_ABI_DT_RELR", "GLIBC_999999999999999999999999"]
		{
			assert!(required_glibc(invalid, &parse_version("2.17")?).is_err(), "accepted {invalid}");
		}
		Ok(())
	}
}
