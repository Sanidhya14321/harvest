//! Shared directory, process and terminal helpers for the ported
//! maintenance commands (`gc`, `ps`, `worktree`). Mirrors
//! `packages/utils/src/dirs.ts` for the subset these commands read.

use std::{
	env,
	io::IsTerminal,
	path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
pub enum Xdg {
	Data,
	State,
}

fn home() -> PathBuf {
	env::var_os("HOME")
		.or_else(|| env::var_os("USERPROFILE"))
		.map(PathBuf::from)
		.unwrap_or_else(|| PathBuf::from("."))
}

fn config_dir_name() -> String {
	["HARVEST_CONFIG_DIR", "PI_CONFIG_DIR"]
		.iter()
		.find_map(|key| env::var(key).ok().filter(|value| !value.is_empty()))
		.unwrap_or_else(|| ".harvest".to_owned())
}

/// `~/.harvest` (profile-independent base root).
pub fn base_config_root() -> PathBuf {
	home().join(config_dir_name())
}

fn xdg_root(category: Xdg) -> Option<PathBuf> {
	if !cfg!(any(target_os = "linux", target_os = "macos")) {
		return None;
	}
	let key = match category {
		Xdg::Data => "XDG_DATA_HOME",
		Xdg::State => "XDG_STATE_HOME",
	};
	let value = env::var_os(key).filter(|value| !value.is_empty())?;
	let root = PathBuf::from(value).join("harvest");
	root.exists().then_some(root)
}

pub fn root_subdir(subdir: impl AsRef<Path>, category: Xdg) -> PathBuf {
	xdg_root(category)
		.unwrap_or_else(base_config_root)
		.join(subdir)
}

/// `~` expansion that requires an absolute result (`resolveWorktreeBase`).
pub fn resolve_absolute_base(value: Option<&str>) -> Option<PathBuf> {
	let trimmed = value?.trim();
	if trimmed.is_empty() {
		return None;
	}
	let path = if trimmed == "~" {
		home()
	} else if let Some(rest) = trimmed
		.strip_prefix("~/")
		.or_else(|| trimmed.strip_prefix("~\\"))
	{
		home().join(rest)
	} else {
		PathBuf::from(trimmed)
	};
	path.is_absolute().then_some(path)
}

/// Agent-managed worktrees dir: env override, then `worktree.base`, then
/// `~/.harvest/wt`.
pub fn worktrees_dir(setting: Option<&str>) -> PathBuf {
	let from_env = env::var("HARVEST_WORKTREE_DIR")
		.ok()
		.filter(|value| !value.is_empty())
		.or_else(|| env::var("OMP_WORKTREE_DIR").ok());
	resolve_absolute_base(from_env.as_deref())
		.or_else(|| resolve_absolute_base(setting))
		.unwrap_or_else(|| root_subdir("wt", Xdg::Data))
}

pub fn daemon_runtime_root() -> PathBuf {
	root_subdir(Path::new("run").join("daemons"), Xdg::State)
}

pub fn global_daemon_runtime_root() -> PathBuf {
	base_config_root()
		.join("run")
		.join("daemons")
		.join("global")
}

pub fn daemon_runtime_dir(project_dir: &Path) -> PathBuf {
	let key = wyhash(project_dir.to_string_lossy().as_bytes(), 0);
	daemon_runtime_root().join(format!("{key:016x}"))
}

/// `kill(pid, 0)`; `Err(errno)` when the signal probe fails.
pub fn probe_pid(pid: i64) -> Result<(), i32> {
	let Ok(pid) = libc::pid_t::try_from(pid) else {
		return Err(libc::ESRCH);
	};
	if pid <= 0 {
		return Err(libc::EINVAL);
	}
	// SAFETY: signal 0 performs only an existence/permission check.
	if unsafe { libc::kill(pid, 0) } == 0 {
		Ok(())
	} else {
		Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
	}
}

pub fn color_enabled() -> bool {
	env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

pub struct Paint {
	pub enabled: bool,
}

impl Paint {
	pub fn new() -> Self {
		Self { enabled: color_enabled() }
	}

	fn wrap(&self, code: &str, text: &str) -> String {
		if self.enabled {
			format!("\x1b[{code}m{text}\x1b[0m")
		} else {
			text.to_owned()
		}
	}

	pub fn dim(&self, text: &str) -> String {
		self.wrap("2", text)
	}

	pub fn red(&self, text: &str) -> String {
		self.wrap("31", text)
	}

	pub fn green(&self, text: &str) -> String {
		self.wrap("32", text)
	}

	pub fn yellow(&self, text: &str) -> String {
		self.wrap("33", text)
	}

	pub fn bold(&self, text: &str) -> String {
		self.wrap("1", text)
	}
}

pub fn terminal_columns() -> Option<usize> {
	if !std::io::stdout().is_terminal() {
		return None;
	}
	// SAFETY: TIOCGWINSZ fills a plain winsize struct for stdout.
	unsafe {
		let mut size: libc::winsize = std::mem::zeroed();
		if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) == 0 && size.ws_col > 0 {
			return Some(usize::from(size.ws_col));
		}
	}
	Some(120)
}

/// `formatDuration` from `@harvest/pi-utils`.
pub fn format_duration(ms: f64) -> String {
	const SEC: f64 = 1_000.0;
	const MIN: f64 = 60.0 * SEC;
	const HOUR: f64 = 60.0 * MIN;
	const DAY: f64 = 24.0 * HOUR;
	if !ms.is_finite() || ms <= 0.0 {
		return "0ms".to_owned();
	}
	if ms < SEC {
		return format!("{}ms", ms.floor());
	}
	if ms < MIN {
		return format!("{:.1}s", ms / SEC);
	}
	let pair = |big: f64, small: f64, a: &str, b: &str| {
		let major = (ms / big).floor();
		let minor = ((ms % big) / small).floor();
		if minor > 0.0 {
			format!("{major}{a}{minor}{b}")
		} else {
			format!("{major}{a}")
		}
	};
	if ms < HOUR {
		return pair(MIN, SEC, "m", "s");
	}
	if ms < DAY {
		return pair(HOUR, MIN, "h", "m");
	}
	pair(DAY, HOUR, "d", "h")
}

pub fn now_ms() -> f64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0.0, |elapsed| elapsed.as_millis() as f64)
}

// Zig std.hash.Wyhash (what `Bun.hash.wyhash` uses).
const SECRET: [u64; 4] =
	[0xa076_1d64_78bd_642f, 0xe703_7ed1_a0b4_28db, 0x8ebc_6af0_9c88_c6e3, 0x5899_65cc_7537_4cc3];

fn mum(a: &mut u64, b: &mut u64) {
	let product = u128::from(*a).wrapping_mul(u128::from(*b));
	*a = product as u64;
	*b = (product >> 64) as u64;
}

fn mix(mut a: u64, mut b: u64) -> u64 {
	mum(&mut a, &mut b);
	a ^ b
}

fn read64(bytes: &[u8], at: usize) -> u64 {
	u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or([0; 8]))
}

fn read32(bytes: &[u8], at: usize) -> u64 {
	u64::from(u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or([0; 4])))
}

pub fn wyhash(input: &[u8], seed: u64) -> u64 {
	let init = seed ^ mix(seed ^ SECRET[0], SECRET[1]);
	let mut state = [init; 3];
	let len = input.len();
	let (a, b);
	if len <= 16 {
		if len >= 4 {
			let end = len - 4;
			let quarter = (len >> 3) << 2;
			a = (read32(input, 0) << 32) | read32(input, quarter);
			b = (read32(input, end) << 32) | read32(input, end - quarter);
		} else if len > 0 {
			a = (u64::from(input[0]) << 16)
				| (u64::from(input[len >> 1]) << 8)
				| u64::from(input[len - 1]);
			b = 0;
		} else {
			a = 0;
			b = 0;
		}
	} else {
		let mut i = 0;
		if len >= 48 {
			while i + 48 < len {
				for lane in 0..3 {
					let x = read64(input, i + 16 * lane);
					let y = read64(input, i + 16 * lane + 8);
					state[lane] = mix(x ^ SECRET[lane + 1], y ^ state[lane]);
				}
				i += 48;
			}
			state[0] ^= state[1] ^ state[2];
		}
		while i + 16 < len {
			state[0] = mix(read64(input, i) ^ SECRET[1], read64(input, i + 8) ^ state[0]);
			i += 16;
		}
		a = read64(input, len - 16);
		b = read64(input, len - 8);
	}
	let mut a = a ^ SECRET[1];
	let mut b = b ^ state[0];
	mum(&mut a, &mut b);
	mix(a ^ SECRET[0] ^ len as u64, b ^ SECRET[1])
}
