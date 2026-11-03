//! `harvest share <session>` — upload a saved session as an encrypted link.
//!
//! Ported from `packages/coding-agent/src/commands/share.ts` and
//! `src/export/share.ts`. The session snapshot is redacted through the secret
//! obfuscator (`share.redactSecrets` && `secrets.enabled`), gzipped, sealed
//! with a fresh AES-256-GCM key (`[12B IV][ciphertext+tag]`) and pushed to the
//! share server or a secret GitHub gist. The key travels only in the URL
//! fragment.
use std::{
	io::Write,
	path::{Path, PathBuf},
	process::Command,
};

use aes_gcm::{
	Aes256Gcm, KeyInit,
	aead::{Aead, AeadCore, OsRng},
};
use anyhow::{Context, Result, bail};
use base64::Engine;
use harvest_protocol::session::SESSION_LIMIT;
use harvest_runtime::{JailedFs, read_bounded};
use serde_json::{Map, Value, json};

use super::secrets::SecretObfuscator;

const DEFAULT_SHARE_URL: &str = "https://my.omp.sh/s";
/// Hard cap for blobs accepted by the share server (mirrors relay shareMaxBytes).
const SERVER_MAX_SEALED_BYTES: usize = 1_000_000;
/// Gist raw fetches cap at 10 MB; keep base64 (x4/3) comfortably under it.
const GIST_MAX_SEALED_BYTES: usize = 5_000_000;
const GIST_FILENAME: &str = "session.ompshare.txt";
const TEXT_CAPS: [usize; 4] = [32_768, 8_192, 2_048, 512];
const BLANK_IMAGE_DATA_URL: &str =
	"data:image/gif;base64,R0lGODlhAQABAAAAACH5BAEKAAEALAAAAAABAAEAAAICTAEAOw==";
const IMAGE_OMITTED_TEXT: &str = "[image omitted from share]";
const TITLE_CHANGE_ENTRY_TYPE: &str = "title_change";

#[derive(Debug, clap::Args)]
pub struct ShareArgs {
	/// Session id (prefix) or path to a session .jsonl
	session: String,
	/// Upload to a secret GitHub gist instead of the share server
	#[arg(long)]
	gist:    bool,
}

pub fn execute(args: ShareArgs) -> Result<u8> {
	let cwd = std::env::current_dir()?;
	let path = match super::render::resolve_target(Some(&args.session), &cwd) {
		Ok(path) => path,
		Err(err) => {
			eprintln!("{err}");
			return Ok(1);
		},
	};
	let (header, entries) = load_session(&path)?;
	let session_cwd = header
		.as_ref()
		.and_then(|h| h.get("cwd"))
		.and_then(Value::as_str)
		.map_or_else(|| cwd.clone(), PathBuf::from);
	// Settings resolve against the session's own project so its policy governs.
	let settings = crate::settings::Settings::load(None, &session_cwd, &[])?;
	let obfuscator = if settings.bool("share.redactSecrets") && settings.bool("secrets.enabled") {
		Some(SecretObfuscator::build(&session_cwd, &super::agent_dir()?))
	} else {
		None
	};

	let leaf_id = entries.iter().rev().find_map(|e| e.get("id").cloned()).unwrap_or(Value::Null);
	let mut header = header;
	if let Some(Value::Object(h)) = header.as_mut() {
		h.remove("previousSessionFiles");
	}
	let mut data = json!({ "header": header, "entries": entries, "leafId": leaf_id });
	if let Some(o) = obfuscator.as_ref().filter(|o| o.has_secrets()) {
		redact_session_data(o, &mut data);
	}

	let base = normalize_server_url(settings.string("share.serverUrl").as_deref());
	let store_gist = args.gist || settings.string("share.store").as_deref() == Some("gist");
	let key = Aes256Gcm::generate_key(OsRng);
	let cipher = Aes256Gcm::new(&key);
	let key_text = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key);

	let mut lines = Vec::new();
	let mut pre_fit = None;
	if store_gist {
		let fit = seal_to_fit(&cipher, &data, GIST_MAX_SEALED_BYTES)?;
		if let Some((id, url)) = try_create_gist(&fit.0) {
			lines.push(format!("Share URL: {base}/{id}#{key_text}"));
			lines.push(format!("Gist: {url}"));
			if fit.1 {
				lines.push("Note: large content was trimmed to fit the share size limit.".into());
			}
			println!("{}", lines.join("\n"));
			return Ok(0);
		}
		pre_fit = Some(fit);
	}
	let (sealed, truncated) = match pre_fit {
		Some(fit) if fit.0.len() <= SERVER_MAX_SEALED_BYTES => fit,
		_ => seal_to_fit(&cipher, &data, SERVER_MAX_SEALED_BYTES)?,
	};
	let id = upload_to_server(&sealed, &base)?;
	lines.push(format!("Share URL: {base}/{id}#{key_text}"));
	if truncated {
		lines.push("Note: large content was trimmed to fit the share size limit.".into());
	}
	println!("{}", lines.join("\n"));
	Ok(0)
}

fn load_session(path: &Path) -> Result<(Option<Value>, Vec<Value>)> {
	let parent = path.parent().context("session path has no parent")?;
	let name = path.file_name().context("session path has no file name")?;
	let bytes = JailedFs::open(parent)?.read(Path::new(name), SESSION_LIMIT)?;
	let text = String::from_utf8_lossy(&bytes);
	let mut header = None;
	let mut entries = Vec::new();
	for line in text.lines().filter(|l| !l.trim().is_empty()) {
		let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
		if value.get("type").and_then(Value::as_str) == Some("session") && header.is_none() {
			header = Some(value);
		} else {
			entries.push(value);
		}
	}
	Ok((header, entries))
}

fn normalize_server_url(url: Option<&str>) -> String {
	let base = url.unwrap_or(DEFAULT_SHARE_URL).trim().trim_end_matches('/');
	if base.is_empty() { DEFAULT_SHARE_URL.into() } else { base.into() }
}

fn seal(cipher: &Aes256Gcm, data: &Value) -> Result<Vec<u8>> {
	let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
	gz.write_all(&serde_json::to_vec(data)?)?;
	let compressed = gz.finish()?;
	let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
	let ciphertext = cipher
		.encrypt(&nonce, compressed.as_slice())
		.map_err(|_| anyhow::anyhow!("share encryption failed"))?;
	let mut out = nonce.to_vec();
	out.extend_from_slice(&ciphertext);
	Ok(out)
}

/// Seal `data`, trimming content until the sealed blob fits `max`.
fn seal_to_fit(cipher: &Aes256Gcm, data: &Value, max: usize) -> Result<(Vec<u8>, bool)> {
	let mut sealed = seal(cipher, data)?;
	if sealed.len() <= max {
		return Ok((sealed, false));
	}
	let mut working = data.clone();
	strip_image_payloads(&mut working);
	sealed = seal(cipher, &working)?;
	if sealed.len() <= max {
		return Ok((sealed, true));
	}
	for cap in TEXT_CAPS {
		cap_long_strings(&mut working, cap);
		sealed = seal(cipher, &working)?;
		if sealed.len() <= max {
			return Ok((sealed, true));
		}
	}
	while let Some(Value::Array(entries)) = working.get_mut("entries")
		&& entries.len() > 4
	{
		let drop = entries.len() - entries.len().div_ceil(2);
		entries.drain(..drop);
		sealed = seal(cipher, &working)?;
		if sealed.len() <= max {
			return Ok((sealed, true));
		}
	}
	bail!(
		"Session too large to share: {} bytes sealed exceeds the {max} byte limit",
		sealed.len()
	)
}

fn strip_image_payloads(value: &mut Value) {
	match value {
		Value::Array(items) => {
			for item in items {
				let is_image = item.get("type").and_then(Value::as_str) == Some("image")
					&& item.get("data").and_then(Value::as_str).is_some_and(|d| d.len() > 1024);
				if is_image {
					*item = json!({ "type": "text", "text": IMAGE_OMITTED_TEXT });
				} else {
					strip_image_payloads(item);
				}
			}
		},
		Value::Object(map) => {
			for v in map.values_mut() {
				if let Value::String(s) = v {
					if s.len() > 1024 && s.starts_with("data:") {
						*v = Value::String(BLANK_IMAGE_DATA_URL.into());
					}
				} else {
					strip_image_payloads(v);
				}
			}
		},
		_ => {},
	}
}

fn cap_long_strings(value: &mut Value, cap: usize) {
	let cap_one = |s: &mut String| {
		if s.chars().count() > cap {
			let cut: String = s.chars().take(cap).collect();
			*s = format!("{cut}\n…[truncated for share]");
		}
	};
	match value {
		Value::Array(items) => items.iter_mut().for_each(|v| match v {
			Value::String(s) => cap_one(s),
			other => cap_long_strings(other, cap),
		}),
		Value::Object(map) => map.values_mut().for_each(|v| match v {
			Value::String(s) => cap_one(s),
			other => cap_long_strings(other, cap),
		}),
		_ => {},
	}
}

/// Create a secret gist holding base64 of the sealed blob; None when `gh` is unusable.
fn try_create_gist(sealed: &[u8]) -> Option<(String, String)> {
	let ok = Command::new("gh").args(["auth", "status"]).output().ok()?.status.success();
	if !ok {
		return None;
	}
	let dir = tempfile::Builder::new().prefix("omp-share-").tempdir().ok()?;
	let fs = JailedFs::open(dir.path()).ok()?;
	let encoded = base64::engine::general_purpose::STANDARD.encode(sealed);
	fs.atomic_write(Path::new(GIST_FILENAME), encoded.as_bytes()).ok()?;
	let out = Command::new("gh")
		.args(["gist", "create", "--public=false"])
		.arg(dir.path().join(GIST_FILENAME))
		.output()
		.ok()?;
	if !out.status.success() {
		return None;
	}
	let text = String::from_utf8_lossy(&out.stdout);
	let url = text.trim().lines().last()?.trim().to_owned();
	let id = url.rsplit('/').next()?.to_owned();
	let hex = (20..=64).contains(&id.len()) && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
	hex.then_some((id, url))
}

fn upload_to_server(sealed: &[u8], base: &str) -> Result<String> {
	let response = match ureq::post(base)
		.set("Content-Type", "application/octet-stream")
		.send_bytes(sealed)
	{
		Ok(r) => r,
		Err(ureq::Error::Status(code, r)) => {
			let body = read_bounded(r.into_reader(), 64 * 1024).unwrap_or_default();
			let detail: String = String::from_utf8_lossy(&body).trim().chars().take(200).collect();
			let detail = if detail.is_empty() { String::new() } else { format!(" ({detail})") };
			bail!("Share upload to {base} failed: HTTP {code}{detail}");
		},
		Err(err) => bail!("Share upload to {base} failed: {err}"),
	};
	let body = read_bounded(response.into_reader(), 64 * 1024)?;
	let id = serde_json::from_slice::<Value>(&body)
		.ok()
		.and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_owned))
		.unwrap_or_default();
	let valid = (10..=64).contains(&id.len())
		&& id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
	if !valid {
		bail!("Share upload to {base} failed: server returned no usable id");
	}
	Ok(id)
}

// ── Redaction (typed walk, ported from redactSessionDataForShare) ──────────

fn red_str(o: &SecretObfuscator, map: &mut Map<String, Value>, key: &str) {
	if let Some(Value::String(s)) = map.get_mut(key) {
		*s = o.obfuscate(s);
	}
}

fn red_content(o: &SecretObfuscator, value: Option<&mut Value>) {
	match value {
		Some(Value::String(s)) => *s = o.obfuscate(s),
		Some(Value::Array(blocks)) => {
			for block in blocks {
				if let Some(b) = block.as_object_mut()
					&& b.get("type").and_then(Value::as_str) == Some("text")
				{
					red_str(o, b, "text");
				}
			}
		},
		_ => {},
	}
}

fn red_json_strings(o: &SecretObfuscator, value: &mut Value) {
	match value {
		Value::String(s) => *s = o.obfuscate(s),
		Value::Array(items) => items.iter_mut().for_each(|v| red_json_strings(o, v)),
		Value::Object(map) => map.values_mut().for_each(|v| red_json_strings(o, v)),
		_ => {},
	}
}

fn red_meta(o: &SecretObfuscator, map: &mut Map<String, Value>) {
	let Some(Value::Object(meta)) = map.get_mut("meta") else { return };
	if let Some(Value::Object(source)) = meta.get_mut("source") {
		red_str(o, source, "value");
	}
	if let Some(Value::Object(diag)) = meta.get_mut("diagnostics") {
		red_str(o, diag, "summary");
		if let Some(Value::Array(messages)) = diag.get_mut("messages") {
			messages.iter_mut().for_each(|m| red_json_strings(o, m));
		}
	}
}

fn redact_header(o: &SecretObfuscator, header: Option<&mut Value>) {
	if let Some(Value::Object(h)) = header {
		red_str(o, h, "title");
		red_str(o, h, "cwd");
		if let Some(Value::Array(files)) = h.get_mut("previousSessionFiles") {
			files.iter_mut().for_each(|f| red_json_strings(o, f));
		}
	}
}

fn redact_session_data(o: &SecretObfuscator, data: &mut Value) {
	redact_header(o, data.get_mut("header"));
	if let Some(Value::Array(entries)) = data.get_mut("entries") {
		for entry in entries {
			redact_entry(o, entry);
		}
	}
}

fn redact_entry(o: &SecretObfuscator, entry: &mut Value) {
	let Some(e) = entry.as_object_mut() else { return };
	let kind = e.get("type").and_then(Value::as_str).unwrap_or_default().to_owned();
	match kind.as_str() {
		"message" => {
			if let Some(message) = e.get_mut("message") {
				redact_message(o, message);
			}
		},
		"compaction" => {
			red_str(o, e, "summary");
			red_str(o, e, "shortSummary");
			e.remove("details");
			e.remove("preserveData");
		},
		"branch_summary" => {
			red_str(o, e, "summary");
			e.remove("details");
		},
		"custom_message" => {
			red_content(o, e.get_mut("content"));
			e.remove("details");
		},
		"custom" | "mode_change" => {
			e.remove("data");
		},
		"session_init" => {
			red_str(o, e, "systemPrompt");
			red_str(o, e, "task");
			e.remove("outputSchema");
		},
		"label" => red_str(o, e, "label"),
		TITLE_CHANGE_ENTRY_TYPE => {
			red_str(o, e, "title");
			red_str(o, e, "previousTitle");
			red_str(o, e, "trigger");
		},
		_ => {},
	}
}

fn redact_message(o: &SecretObfuscator, message: &mut Value) {
	let Some(m) = message.as_object_mut() else { return };
	let role = m.get("role").and_then(Value::as_str).unwrap_or_default().to_owned();
	match role.as_str() {
		"user" | "developer" => {
			m.remove("providerPayload");
			red_content(o, m.get_mut("content"));
		},
		"custom" | "hookMessage" | "toolResult" => {
			m.remove("details");
			red_content(o, m.get_mut("content"));
		},
		"assistant" => {
			m.remove("providerPayload");
			red_str(o, m, "errorMessage");
			if let Some(Value::Array(blocks)) = m.get_mut("content") {
				blocks.retain(|b| {
					!matches!(
						b.get("type").and_then(Value::as_str),
						Some("redactedThinking" | "anthropicServerTool")
					)
				});
				for block in blocks {
					let Some(b) = block.as_object_mut() else { continue };
					match b.get("type").and_then(Value::as_str) {
						Some("text") => red_str(o, b, "text"),
						Some("thinking") => red_str(o, b, "thinking"),
						Some("toolCall") => {
							if let Some(args) = b.get_mut("arguments") {
								red_json_strings(o, args);
							}
							red_str(o, b, "intent");
							red_str(o, b, "rawBlock");
						},
						_ => {},
					}
				}
			}
		},
		"bashExecution" => {
			red_str(o, m, "command");
			red_str(o, m, "output");
			red_meta(o, m);
		},
		"pythonExecution" => {
			red_str(o, m, "code");
			red_str(o, m, "output");
			red_meta(o, m);
		},
		"branchSummary" => red_str(o, m, "summary"),
		"compactionSummary" => {
			m.remove("providerPayload");
			red_str(o, m, "summary");
			red_str(o, m, "shortSummary");
			red_content(o, m.get_mut("blocks"));
		},
		"fileMention" => {
			if let Some(Value::Array(files)) = m.get_mut("files") {
				for file in files.iter_mut().filter_map(Value::as_object_mut) {
					red_str(o, file, "path");
					red_str(o, file, "content");
				}
			}
		},
		_ => {},
	}
}
