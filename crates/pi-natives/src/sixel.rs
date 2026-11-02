//! SIXEL terminal-image encoding and bounded decoding.
//!
//! Thin N-API adapter over `harvest_media::sixel`. General-purpose image
//! processing stays in `Bun.Image`; the native boundary exists because SIXEL
//! has no equivalent decoder there.

use napi::bindgen_prelude::*;
use napi_derive::napi;

/// Decode one complete SIXEL control string into a PNG.
#[napi]
pub fn decode_sixel_to_png(bytes: Uint8Array) -> Result<Uint8Array> {
	harvest_media::sixel::decode_sixel_to_png(bytes.as_ref())
		.map(Into::into)
		.map_err(|error| Error::from_reason(error.to_string()))
}

/// Encode image bytes into a SIXEL escape sequence for terminal rendering.
///
/// The input image is decoded and resized to the requested pixel dimensions
/// before encoding.
///
/// # Errors
/// Returns an error if the dimensions exceed limits, or decoding, resizing, or
/// SIXEL encoding fails.
#[napi]
pub fn encode_sixel(
	bytes: Uint8Array,
	target_width_px: u32,
	target_height_px: u32,
) -> Result<String> {
	harvest_media::sixel::encode_sixel(bytes.as_ref(), target_width_px, target_height_px)
		.map_err(|error| Error::from_reason(error.to_string()))
}
