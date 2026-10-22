//! Host CPU/addon detection for native builds.
use std::{
	env,
	fs,
	process::Command,
};

#[derive(serde::Serialize)]
struct HostAddon {
	filename:     String,
	x64_variant:  Option<&'static str>,
	avx2:         bool,
}

pub fn detect_avx2() -> bool {
	if env::consts::ARCH != "x86_64" {
		return false;
	}
	match env::consts::OS {
		"linux" => fs::read_to_string("/proc/cpuinfo")
			.map(|info| info.lines().any(|line| line.split_whitespace().any(|token| token.eq_ignore_ascii_case("avx2"))))
			.unwrap_or(false),
		"macos" => {
			if let Ok(output) = Command::new("sysctl").args(["-n", "machdep.cpu.leaf7_features"]).output()
				&& output.status.success()
				&& String::from_utf8_lossy(&output.stdout).to_uppercase().contains("AVX2")
			{
				return true;
			}
			Command::new("sysctl")
				.args(["-n", "machdep.cpu.features"])
				.output()
				.ok()
				.filter(|output| output.status.success())
				.map(|output| String::from_utf8_lossy(&output.stdout).to_uppercase().contains("AVX2"))
				.unwrap_or(false)
		},
		"windows" => {
			#[cfg(target_os = "windows")]
			{
				#[link(name = "kernel32")]
				unsafe extern "system" {
					fn IsProcessorFeaturePresent(feature: u32) -> i32;
				}
				// SAFETY: calling a documented Win32 function.
				unsafe { IsProcessorFeaturePresent(40) != 0 }
			}
			#[cfg(not(target_os = "windows"))]
			{
				false
			}
		},
		_ => false,
	}
}

pub fn execute() -> anyhow::Result<()> {
	let avx2 = detect_avx2();
	let x64_variant = if env::consts::ARCH == "x86_64" {
		Some(if avx2 { "modern" } else { "baseline" })
	} else {
		None
	};
	let suffix = x64_variant.map(|v| format!("-{v}")).unwrap_or_default();
	let platform = match env::consts::OS {
		"linux" => "linux",
		"macos" => "darwin",
		"windows" => "win32",
		other => other,
	};
	let filename = format!("pi_natives.{platform}-{}{suffix}.node", env::consts::ARCH);
	let addon = HostAddon {
		filename,
		x64_variant,
		avx2,
	};
	println!("{}", serde_json::to_string(&addon)?);
	Ok(())
}
