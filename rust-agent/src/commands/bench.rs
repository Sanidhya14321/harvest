//! CLI surfaces for `bench`, `if-bench`, `dry-balance` and `gallery`
//! (ports of `commands/{bench,if-bench,dry-balance,gallery}.ts`).
//!
//! Arguments are validated natively. The engines behind them (`cli/bench-cli.ts`,
//! `if-bench/`, `cli/dry-balance-cli.ts`, `cli/gallery-cli.ts`) depend on the
//! TypeScript multi-provider streaming client, OAuth credential balancer and
//! tool-renderer registry, none of which the Rust runtime has yet; each command
//! reports that gap instead of fabricating results.
use anyhow::{Result, bail};

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum Profile {
	Mix,
	Chat,
	Prefill,
	Generation,
}

#[derive(clap::Args, Debug)]
pub struct BenchArgs {
	/// Model selectors (provider/model or fuzzy id, e.g. opus)
	#[arg(required = true)]
	pub models:             Vec<String>,
	/// Requests per model (default: 9 for mix, 10 chat, 5 prefill/generation)
	#[arg(long)]
	pub runs:               Option<u32>,
	/// Max output tokens per request
	#[arg(long = "max-tokens")]
	pub max_tokens:         Option<u32>,
	/// Custom prompt text (requires --profile chat or generation)
	#[arg(long)]
	pub prompt:             Option<String>,
	/// Benchmark workload (default mix rotates all)
	#[arg(long, value_enum)]
	pub profile:            Option<Profile>,
	/// Synthetic input size for prefill challenges (default: 32768)
	#[arg(long = "prefill-bytes")]
	pub prefill_bytes:      Option<u32>,
	/// Service tier applied per model family (`none` omits it)
	#[arg(long = "service-tier")]
	pub service_tier:       Option<String>,
	/// Output JSON
	#[arg(long)]
	pub json:               bool,
	/// Execute runs with N parallel requests (default: 4)
	#[arg(long)]
	pub par:                Option<u32>,
	/// Run independent cold/warm prompt-cache pairs
	#[arg(long)]
	pub cache:              bool,
	/// Stable prompt prefix file for --cache
	#[arg(long = "cache-prefix-file")]
	pub cache_prefix_file:  Option<String>,
	/// Stable prefix byte budget for --cache (default: 8192)
	#[arg(long = "cache-prefix-bytes")]
	pub cache_prefix_bytes: Option<u32>,
	/// Cold/warm pairs per model for --cache (default: 1)
	#[arg(long = "cache-pairs")]
	pub cache_pairs:        Option<u32>,
	/// Concurrent cache pairs for --cache (default: 1)
	#[arg(long = "cache-concurrency")]
	pub cache_concurrency:  Option<u32>,
}

#[derive(clap::Args, Debug)]
pub struct IfBenchArgs {
	/// Model selectors (provider/model or fuzzy id, e.g. opus)
	#[arg(required = true)]
	pub models:     Vec<String>,
	/// Maximum turns per model; turn N issues N actions (default: 24)
	#[arg(long)]
	pub turns:      Option<u32>,
	/// Character-array length, even, 8-26 (default: 24)
	#[arg(long)]
	pub length:     Option<u32>,
	/// Max output tokens per turn (default: 32768)
	#[arg(long = "max-tokens")]
	pub max_tokens: Option<u32>,
	/// Longest accepted cat sound in nya{1,N} (default: 8)
	#[arg(long = "nya-max")]
	pub nya_max:    Option<u32>,
	/// Models benchmarked concurrently (default: 4)
	#[arg(long)]
	pub par:        Option<u32>,
	/// Output JSON
	#[arg(long)]
	pub json:       bool,
}

#[derive(clap::Args, Debug)]
pub struct DryBalanceArgs {
	/// Model selector; defaults to the configured default model
	pub model:        Option<String>,
	/// Model selector (same syntax as --model on harvest)
	#[arg(long = "model", id = "model_flag")]
	pub model_flag:   Option<String>,
	/// Number of random session ids to try
	#[arg(long, default_value_t = 100)]
	pub count:        u32,
	/// Maximum concurrent credential resolutions
	#[arg(long, default_value_t = 32)]
	pub concurrency:  u32,
	/// Output JSON
	#[arg(long)]
	pub json:         bool,
	/// Send one live benchmark request per OAuth account
	#[arg(long)]
	pub bench:        bool,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum Surface {
	Tool,
	Composer,
	Segment,
}

#[derive(clap::Args, Debug)]
pub struct GalleryArgs {
	/// Render only tool, composer, or segment surfaces (repeatable; default: all)
	#[arg(long, value_enum)]
	pub surface:    Vec<Surface>,
	/// Render a single tool by name
	#[arg(short = 't', long)]
	pub tool:       Option<String>,
	/// Render a single composer shape by name
	#[arg(long)]
	pub composer:   Option<String>,
	/// Render a single status-line segment by name
	#[arg(long)]
	pub segment:    Option<String>,
	/// Render only the given lifecycle state(s)
	#[arg(short = 's', long)]
	pub state:      Vec<String>,
	/// Render width in columns
	#[arg(short = 'w', long)]
	pub width:      Option<u16>,
	/// Render the expanded variant of each renderer
	#[arg(short = 'e', long)]
	pub expanded:   bool,
	/// Strip ANSI styling from the output
	#[arg(long)]
	pub plain:      bool,
	/// Capture the rendered output as PNG screenshot(s) via VHS (requires vhs)
	#[arg(long)]
	pub screenshot: bool,
	/// Screenshot output path (with --screenshot)
	#[arg(short = 'o', long)]
	pub out:        Option<String>,
	/// Screenshot font family (default: JetBrainsMono Nerd Font)
	#[arg(long)]
	pub font:       Option<String>,
	/// Screenshot font size in points (default: 18)
	#[arg(long = "font-size")]
	pub font_size:  Option<u16>,
}

fn unported(name: &str, needs: &str) -> Result<u8> {
	bail!("`harvest {name}` needs {needs}, which the Rust runtime does not provide yet; use the TypeScript CLI")
}

pub fn bench(args: BenchArgs) -> Result<u8> {
	if args.prompt.is_some() && !matches!(args.profile, Some(Profile::Chat | Profile::Generation)) {
		bail!("--prompt requires --profile chat or generation");
	}
	unported("bench", "the multi-provider streaming client")
}

pub fn if_bench(args: IfBenchArgs) -> Result<u8> {
	if let Some(length) = args.length
		&& (length % 2 != 0 || !(8..=26).contains(&length))
	{
		bail!("--length must be even, between 8 and 26");
	}
	unported("if-bench", "the multi-provider streaming client")
}

pub fn dry_balance(_args: DryBalanceArgs) -> Result<u8> {
	unported("dry-balance", "the OAuth credential balancer")
}

pub fn gallery(_args: GalleryArgs) -> Result<u8> {
	unported("gallery", "the TypeScript tool-renderer registry")
}
