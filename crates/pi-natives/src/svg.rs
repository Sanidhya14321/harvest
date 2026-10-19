//! Safe SVG rasterization for terminal image previews.
//!
//! SVGs are parsed without loading file-backed image resources, capped to a
//! caller-provided canvas, and encoded as PNG for terminal graphics protocols.

use napi::bindgen_prelude::Uint8Array;
use napi_derive::napi;

use crate::task;

/// Rasterize SVG/SVGZ bytes into a bounded PNG without resolving local files.
///
/// Conversion runs on the native blocking pool so parsing and rendering do not
/// stall the JavaScript event loop.
///
/// # Errors
/// Returns an error for invalid SVG data, zero/oversized limits, allocation
/// failure, or PNG encoding failure.
#[napi(js_name = "rasterizeSvg")]
pub fn rasterize_svg(
	input: Uint8Array,
	max_width_px: u32,
	max_height_px: u32,
) -> task::Promise<Uint8Array> {
	let input = input.to_vec();
	task::blocking("svg.rasterize", (), move |_| {
		harvest_media::svg::rasterize_svg(&input, max_width_px, max_height_px)
			.map(Uint8Array::from)
			.map_err(|error| napi::Error::from_reason(error.to_string()))
	})
}
