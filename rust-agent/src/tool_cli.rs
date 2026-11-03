//! `harvest read|grep|search|models`: run one built-in tool from the command
//! line and print what the model would see (legacy `commands/{read,grep,
//! web-search,models}.ts`).
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::chat::{FunctionCall, ToolCall};

#[derive(Debug, clap::Args)]
pub struct ReadArgs {
	/// Path or URI to read; append `:sel` for line ranges or raw mode.
	pub path:      String,
	#[arg(long, default_value = ".")]
	pub workspace: PathBuf,
}

#[derive(Debug, clap::Args)]
pub struct GrepArgs {
	/// Regex pattern to search for.
	pub pattern:      String,
	/// Directory or file to search.
	pub path:         Option<String>,
	/// Include files excluded by .gitignore.
	#[arg(long)]
	pub no_gitignore: bool,
	/// Case-sensitive matching.
	#[arg(long)]
	pub case:         bool,
	#[arg(long, default_value = ".")]
	pub workspace:    PathBuf,
}

#[derive(Debug, clap::Args)]
pub struct SearchArgs {
	/// Search query text.
	#[arg(required = true)]
	pub query:   Vec<String>,
	/// Recency filter.
	#[arg(long, value_parser = ["day", "week", "month", "year"])]
	pub recency: Option<String>,
	/// Max results to return.
	#[arg(short, long)]
	pub limit:   Option<u32>,
}

#[derive(Debug, clap::Args)]
pub struct ModelsArgs {
	/// Case-insensitive substring filter over `provider/model`.
	pub filter: Option<String>,
	/// Emit JSON rows instead of a table.
	#[arg(long)]
	pub json:   bool,
}

fn invoke(workspace: &Path, name: &str, arguments: &Value) -> Result<()> {
	let root = workspace
		.canonicalize()
		.with_context(|| format!("failed to resolve {}", workspace.display()))?;
	let state = crate::core_tools::SessionState::with_settings(crate::settings::Settings::load(
		crate::config::agent_dir_from(None, |key| std::env::var_os(key)).as_deref(),
		&root,
		&[],
	)?);
	let call = ToolCall {
		id:       "cli".into(),
		kind:     "function".into(),
		function: FunctionCall { name: name.into(), arguments: arguments.to_string() },
	};
	let output = crate::core_tools::invoke_in_session(
		&root,
		&call,
		&crate::tools::DenyMutations,
		&harvest_runtime::cancel::CancelToken::default(),
		&pi_edit::EditStore::default(),
		None,
		&state,
	)?;
	println!("{output}");
	Ok(())
}

pub fn read(args: &ReadArgs) -> Result<()> {
	invoke(&args.workspace, "read", &json!({ "path": args.path }))
}

pub fn grep(args: &GrepArgs) -> Result<()> {
	let mut call = json!({ "pattern": args.pattern, "gitignore": !args.no_gitignore });
	if let Some(path) = &args.path {
		call["path"] = path.clone().into();
	}
	if args.case {
		call["case"] = true.into();
	}
	invoke(&args.workspace, "grep", &call)
}

pub fn search(args: &SearchArgs) -> Result<()> {
	let mut call = json!({ "query": args.query.join(" ") });
	if let Some(recency) = &args.recency {
		call["recency"] = recency.clone().into();
	}
	if let Some(limit) = args.limit {
		call["limit"] = limit.into();
	}
	invoke(Path::new("."), "web_search", &call)
}

pub fn models(args: &ModelsArgs) -> Result<()> {
	let filter = args.filter.as_deref().map(str::to_ascii_lowercase);
	let rows = crate::catalog::models().filter(|(provider, id, _)| {
		filter.as_deref().is_none_or(|needle| {
			format!("{provider}/{id}")
				.to_ascii_lowercase()
				.contains(needle)
		})
	});
	let mut out = String::new();
	for (provider, id, model) in rows {
		if args.json {
			out.push_str(
				&json!({
					"model": format!("{provider}/{id}"),
					"baseUrl": model.base_url,
					"contextWindow": model.context_window,
					"maxTokens": model.max_tokens,
				})
				.to_string(),
			);
		} else {
			use std::fmt::Write;
			let context = model
				.context_window
				.map_or_else(|| "-".to_owned(), |tokens| format!("{}k", tokens / 1000));
			let _ = write!(out, "{:<56} {context:>6}", format!("{provider}/{id}"));
		}
		out.push('\n');
	}
	print!("{out}");
	Ok(())
}
