//! Transitional N-API transport for the ordinary Rust DOCX engine.
use std::collections::BTreeMap;

use harvest_media::docx::{Options, Prepared};
use napi::{JsString, bindgen_prelude::*};
use napi_derive::napi;

use crate::{js::into_string, task};

#[napi]
pub fn docx_prepare(
	bytes: Buffer,
	options: JsString,
	signal: Option<Unknown>,
) -> Result<task::Promise<String>> {
	let options = into_string(options)?;
	if bytes.len() > harvest_media::docx::INPUT_LIMIT || options.len() > 128 * 1024 {
		return Err(Error::from_reason("DOCX input/options exceed limit"));
	}
	let options: Options =
		serde_json::from_str(&options).map_err(|e| Error::from_reason(e.to_string()))?;
	let bytes = bytes.to_vec();
	Ok(task::blocking("docx_prepare", task::CancelToken::new(Some(30_000), signal), move |ct| {
		let ct = ct.into_core();
		let document = harvest_media::docx::prepare(&bytes, &options, &ct)
			.map_err(|e| Error::from_reason(e.to_string()))?;
		serde_json::to_string(&document).map_err(|e| Error::from_reason(e.to_string()))
	}))
}
#[napi]
pub fn docx_finish(
	prepared: JsString,
	attributes: JsString,
	signal: Option<Unknown>,
) -> Result<task::Promise<String>> {
	let prepared = into_string(prepared)?;
	let attributes = into_string(attributes)?;
	if prepared.len() > 128 * 1024 * 1024 || attributes.len() > 64 * 1024 * 1024 {
		return Err(Error::from_reason("DOCX callback payload exceeds limit"));
	}
	Ok(task::blocking("docx_finish", task::CancelToken::new(Some(30_000), signal), move |ct| {
		let ct = ct.into_core();
		let prepared: Prepared =
			serde_json::from_str(&prepared).map_err(|e| Error::from_reason(e.to_string()))?;
		let attributes: Option<Vec<BTreeMap<String, String>>> =
			serde_json::from_str(&attributes).map_err(|e| Error::from_reason(e.to_string()))?;
		let document = prepared
			.finish(attributes.as_deref(), &ct)
			.map_err(|e| Error::from_reason(e.to_string()))?;
		serde_json::to_string(&document).map_err(|e| Error::from_reason(e.to_string()))
	}))
}

#[napi]
pub fn docx_prepare_file(
	path: JsString,
	options: JsString,
	signal: Option<Unknown>,
) -> Result<task::Promise<String>> {
	let path = into_string(path)?;
	let options = into_string(options)?;
	if options.len() > 128 * 1024 {
		return Err(Error::from_reason("DOCX options exceed limit"));
	}
	let options: Options =
		serde_json::from_str(&options).map_err(|e| Error::from_reason(e.to_string()))?;
	Ok(task::blocking(
		"docx_prepare_file",
		task::CancelToken::new(Some(30_000), signal),
		move |ct| {
			let ct = ct.into_core();
			ct.heartbeat()
				.map_err(|e| Error::from_reason(e.to_string()))?;
			let file = std::fs::File::open(path).map_err(|e| Error::from_reason(e.to_string()))?;
			let bytes = harvest_runtime::read_bounded(file, harvest_media::docx::INPUT_LIMIT)
				.map_err(|e| Error::from_reason(e.to_string()))?;
			let document = harvest_media::docx::prepare(&bytes, &options, &ct)
				.map_err(|e| Error::from_reason(e.to_string()))?;
			serde_json::to_string(&document).map_err(|e| Error::from_reason(e.to_string()))
		},
	))
}
