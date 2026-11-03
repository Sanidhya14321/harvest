//! `harvest say` — synthesize text with the local TTS engine.
//!
//! Ported from `packages/coding-agent/src/commands/say.ts`. Argument handling,
//! settings resolution and input validation match the TypeScript command, but
//! synthesis itself is not ported: the TS engine runs Kokoro-82M through a
//! lazily installed `kokoro-js` + onnxruntime side runtime in a worker and
//! streams PCM to an audio player, which needs a native inference/audio stack
//! the Rust app does not have yet. The command reports that gap and exits 1.
use std::path::Path;

use anyhow::Result;
use harvest_runtime::JailedFs;

use super::Paint;

const VOICES: [&str; 12] = [
	"af_heart", "af_bella", "af_nicole", "af_aoede", "af_kore", "af_sarah", "am_michael", "am_fenrir",
	"am_puck", "bf_emma", "bm_george", "bm_fable",
];

#[derive(Debug, clap::Args)]
#[command(after_help = "Examples:\n  harvest say \"hello world\"\n  harvest say --file notes.md --voice \
                        bm_fable\n  harvest say \"hello world\" --out /tmp/hello.wav")]
pub struct SayArgs {
	/// Text to speak (or use --file)
	text:  Option<String>,
	/// Voice id
	#[arg(long, value_parser = clap::builder::PossibleValuesParser::new(VOICES))]
	voice: Option<String>,
	/// Local TTS model key
	#[arg(long)]
	model: Option<String>,
	/// Read the text to speak from this file
	#[arg(short, long)]
	file:  Option<String>,
	/// Write WAV to this path instead of playing
	#[arg(short, long)]
	out:   Option<String>,
}

pub fn execute(args: SayArgs) -> Result<u8> {
	let paint = Paint::stderr();
	if args.text.is_some() && args.file.is_some() {
		eprintln!("{}", paint.red("error: pass either text or --file, not both"));
		return Ok(1);
	}
	let cwd = std::env::current_dir()?;
	let settings = crate::settings::Settings::load(None, &cwd, &[])?;
	let model = args.model.or_else(|| settings.string("tts.localModel")).unwrap_or_else(|| "kokoro".into());

	let text = match &args.file {
		Some(file) => {
			let path = std::path::absolute(cwd.join(file))?;
			let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
				eprintln!("{}", paint.red(&format!("error: invalid file {file}")));
				return Ok(1);
			};
			let bytes = JailedFs::open(dir)?.read(Path::new(name), harvest_runtime::MAX_FILE_BYTES)?;
			String::from_utf8_lossy(&bytes).into_owned()
		},
		None => args.text.unwrap_or_default(),
	};
	if !text.chars().any(char::is_alphanumeric) {
		eprintln!("{}", paint.red("error: nothing speakable in the input"));
		return Ok(1);
	}
	eprintln!(
		"{}",
		paint.red(&format!(
			"error: could not synthesize with local TTS model \"{model}\": local TTS is not yet \
			 available in the native app. Use the TypeScript `harvest say` for now."
		))
	);
	Ok(1)
}
