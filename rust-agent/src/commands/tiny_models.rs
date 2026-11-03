//! `harvest tiny-models [download|list] [model|all]` (port of `cli/tiny-models-cli.ts`).
//!
//! `list` and model resolution are native. `download` needs the local inference
//! worker (transformers.js ONNX / mlx-lm) that the TypeScript runtime hosts; the
//! Rust app has no native tiny-inference backend yet, so it reports that gap.
use std::io::Write;

use anyhow::{Result, bail};

use super::Paint;

pub struct TinyModelSpec {
	pub key:          &'static str,
	pub repo:         &'static str,
	pub dtype:        &'static str,
	pub mlx_repo:     &'static str,
	pub label:        &'static str,
	pub description:  &'static str,
	pub context_note: &'static str,
}

pub const DEFAULT_KEY: &str = "lfm2.5-230m";

pub const TINY_LOCAL_MODELS: &[TinyModelSpec] = &[
	TinyModelSpec {
		key:          "lfm2.5-230m",
		repo:         "LiquidAI/LFM2.5-230M-ONNX",
		dtype:        "q4",
		mlx_repo:     "LiquidAI/LFM2.5-230M-MLX-4bit",
		label:        "LFM2.5 230M",
		description:  "Recommended local model; fastest LFM2.5 option, about 214 MB cached.",
		context_note: "Best balance among the current compact title models.",
	},
	TinyModelSpec {
		key:          "lfm2.5-350m",
		repo:         "onnx-community/LFM2.5-350M-ONNX",
		dtype:        "q4",
		mlx_repo:     "LiquidAI/LFM2.5-350M-MLX-4bit",
		label:        "LFM2.5 350M",
		description:  "Larger LFM2.5 option, about 292 MB cached; tends toward terse titles.",
		context_note: "Use when compact labels are preferred over descriptive titles.",
	},
	TinyModelSpec {
		key:          "falcon-h1-90m",
		repo:         "onnx-community/Falcon-H1-Tiny-90M-Instruct-ONNX",
		dtype:        "q4",
		mlx_repo:     "mlx-community/Falcon-H1-Tiny-90M-Instruct-4bit",
		label:        "Falcon H1 Tiny 90M",
		description:  "Smallest option, about 147 MB cached; lower fidelity on complex prompts.",
		context_note: "Use on constrained machines where download size matters most.",
	},
];

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
	Download,
	List,
}

#[derive(clap::Args, Debug)]
pub struct TinyModelsArgs {
	/// Action to perform
	#[arg(value_enum)]
	pub action: Option<Action>,
	/// Model key, or all
	pub model:  Option<String>,
	/// Output JSON
	#[arg(long)]
	pub json:   bool,
}

pub fn resolve_models(model: Option<&str>) -> Result<Vec<&'static str>> {
	match model {
		None => Ok(vec![DEFAULT_KEY]),
		Some("all") => Ok(TINY_LOCAL_MODELS.iter().map(|s| s.key).collect()),
		Some(key) => match TINY_LOCAL_MODELS.iter().find(|s| s.key == key) {
			Some(spec) => Ok(vec![spec.key]),
			None => {
				let values: Vec<_> = TINY_LOCAL_MODELS.iter().map(|s| s.key).collect();
				bail!("Unknown tiny local model: {key}. Expected one of: {}, all", values.join(", "))
			},
		},
	}
}

fn list(json: bool) -> Result<u8> {
	let mut out = std::io::stdout().lock();
	if json {
		let models: Vec<_> = TINY_LOCAL_MODELS
			.iter()
			.map(|s| {
				serde_json::json!({
					"key": s.key, "repo": s.repo, "dtype": s.dtype, "mlxRepo": s.mlx_repo,
					"label": s.label, "description": s.description, "contextNote": s.context_note,
				})
			})
			.collect();
		writeln!(out, "{}", serde_json::json!({ "models": models }))?;
		return Ok(0);
	}
	let paint = Paint::stdout();
	writeln!(out, "{}", paint.bold("Tiny local models"))?;
	for spec in TINY_LOCAL_MODELS {
		let mark = if spec.key == DEFAULT_KEY { paint.cyan(" default") } else { String::new() };
		writeln!(out, "{}{mark}", paint.cyan(spec.key))?;
		writeln!(out, "  {} — {}", spec.label, spec.description)?;
	}
	Ok(0)
}

pub fn execute(args: TinyModelsArgs) -> Result<u8> {
	match args.action.unwrap_or(Action::Download) {
		Action::List => list(args.json),
		Action::Download => {
			let models = resolve_models(args.model.as_deref())?;
			bail!(
				"tiny-models download ({}) requires the local tiny-inference worker (ONNX/MLX), which \
				 the Rust runtime does not provide yet; use the TypeScript `harvest tiny-models download`",
				models.join(", ")
			)
		},
	}
}
