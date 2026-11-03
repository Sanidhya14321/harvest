//! Ported standalone CLI commands (`commands/*.ts` + `cli/*-cli.ts`).
pub mod agents;
pub mod compress;
pub mod gc;
pub mod grievances;
pub mod images;
pub(crate) mod paths;
pub mod ps;
pub mod render;
pub mod ssh;
pub mod usage;
pub mod worktree;

use std::{io::IsTerminal, path::PathBuf};

/// Minimal `chalk` replacement: styles apply only on a color-capable stream.
pub(crate) struct Paint {
	enabled: bool,
}

impl Paint {
	pub(crate) fn stdout() -> Self {
		Self::with(std::io::stdout().is_terminal())
	}

	pub(crate) fn stderr() -> Self {
		Self::with(std::io::stderr().is_terminal())
	}

	fn with(tty: bool) -> Self {
		let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
		let force = std::env::var_os("FORCE_COLOR").is_some_and(|v| !v.is_empty() && v != "0");
		Self { enabled: force || (tty && !no_color) }
	}

	fn wrap(&self, code: &str, text: &str) -> String {
		if self.enabled {
			format!("\x1b[{code}m{text}\x1b[0m")
		} else {
			text.to_owned()
		}
	}

	pub(crate) fn red(&self, t: &str) -> String {
		self.wrap("31", t)
	}

	pub(crate) fn green(&self, t: &str) -> String {
		self.wrap("32", t)
	}

	pub(crate) fn yellow(&self, t: &str) -> String {
		self.wrap("33", t)
	}

	pub(crate) fn cyan(&self, t: &str) -> String {
		self.wrap("36", t)
	}

	pub(crate) fn dim(&self, t: &str) -> String {
		self.wrap("2", t)
	}

	pub(crate) fn bold(&self, t: &str) -> String {
		self.wrap("1", t)
	}

	pub(crate) fn bold_cyan(&self, t: &str) -> String {
		self.wrap("1;36", t)
	}
}

pub(crate) fn agent_dir() -> anyhow::Result<PathBuf> {
	crate::config::agent_dir_from(None, |key| std::env::var_os(key))
		.ok_or_else(|| anyhow::anyhow!("cannot resolve the agent directory (HOME is unset)"))
}

/// `~/.harvest` — the parent of the agent dir (`dirs.rootSubdir`).
pub(crate) fn root_dir() -> anyhow::Result<PathBuf> {
	let agent = agent_dir()?;
	Ok(agent
		.parent()
		.map_or_else(|| agent.clone(), std::path::Path::to_path_buf))
}

pub(crate) const CONFIG_DIR_NAME: &str = ".harvest";

/// `formatDuration` from `@harvest/pi-utils`.
pub(crate) fn format_duration(ms: f64) -> String {
	const SEC: f64 = 1000.0;
	const MIN: f64 = 60.0 * SEC;
	const HOUR: f64 = 60.0 * MIN;
	const DAY: f64 = 24.0 * HOUR;
	if !ms.is_finite() || ms <= 0.0 {
		return "0ms".into();
	}
	if ms < SEC {
		return format!("{}ms", ms.floor());
	}
	if ms < MIN {
		return format!("{:.1}s", ms / SEC);
	}
	let pair = |a: f64, b: f64, ua: &str, ub: &str| {
		if b > 0.0 {
			format!("{a}{ua}{b}{ub}")
		} else {
			format!("{a}{ua}")
		}
	};
	if ms < HOUR {
		return pair((ms / MIN).floor(), ((ms % MIN) / SEC).floor(), "m", "s");
	}
	if ms < DAY {
		return pair((ms / HOUR).floor(), ((ms % HOUR) / MIN).floor(), "h", "m");
	}
	pair((ms / DAY).floor(), ((ms % DAY) / HOUR).floor(), "d", "h")
}

/// `formatNumber` from `@harvest/pi-utils`.
pub(crate) fn format_number(n: f64) -> String {
	fn trim1(n: f64) -> String {
		let s = format!("{n:.1}");
		s.strip_suffix(".0")
			.map_or_else(|| s.clone(), str::to_owned)
	}
	if n < 1_000.0 {
		return n.to_string();
	}
	if n < 10_000.0 {
		return format!("{}K", trim1(n / 1e3));
	}
	if n < 1e6 {
		return format!("{}K", (n / 1e3).round());
	}
	if n < 1e7 {
		return format!("{}M", trim1(n / 1e6));
	}
	if n < 1e9 {
		return format!("{}M", (n / 1e6).round());
	}
	if n < 1e10 {
		return format!("{}B", trim1(n / 1e9));
	}
	format!("{}B", (n / 1e9).round())
}
pub mod acp;
pub mod auth_broker;
pub mod auth_gateway;
pub mod bench;
pub mod cleanse;
pub mod commit;
pub mod complete;
pub mod git;
pub mod install;
pub mod join;
pub mod launch;
pub mod plugin;
mod plugin_store;
pub(crate) mod rules;
pub mod say;
mod secrets;
pub mod setup;
pub mod share;
pub mod shell;
pub mod stats;
pub mod tiny_models;
pub mod token;
pub mod ttsr;
pub mod update;
