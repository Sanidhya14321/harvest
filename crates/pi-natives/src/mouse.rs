//! SGR mouse input bridge; the shared Rust engine owns decoding policy.
use napi::{JsString, Result};
use napi_derive::napi;

use crate::js;

#[napi(object)]
pub struct NativeSgrMouseEvent {
	pub button:     f64,
	pub col:        f64,
	pub row:        f64,
	pub release:    bool,
	pub wheel:      Option<i32>,
	pub motion:     bool,
	pub left_click: bool,
}

#[napi]
pub fn parse_sgr_mouse(data: JsString) -> Result<Option<NativeSgrMouseEvent>> {
	let data = js::utf8(data)?;
	Ok(harvest_tui::mouse::parse_sgr_mouse(&data).map(|event| NativeSgrMouseEvent {
		button:     event.button,
		col:        event.col,
		row:        event.row,
		release:    event.release,
		wheel:      event.wheel.map(i32::from),
		motion:     event.motion,
		left_click: event.left_click,
	}))
}
