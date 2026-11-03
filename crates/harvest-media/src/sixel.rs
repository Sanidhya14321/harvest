//! SIXEL terminal-image encoding and bounded decoding.
//!
//! Encodes PNG/JPEG/WebP/GIF images for terminal display and decodes captured
//! SIXEL control strings back to bounded PNG attachments.
//!
//! Shared by native Rust consumers and the temporary N-API host adapter.

use std::io::Cursor;

use anyhow::Result;
use icy_sixel::{EncodeOptions, SixelImage, sixel_encode};
use image::{
	DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, RgbaImage, imageops::FilterType,
};

const MAX_IMAGE_INPUT_BYTES: usize = 20 * 1024 * 1024;
const MAX_SOURCE_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_SOURCE_EDGE: u32 = 16384;
const MAX_SOURCE_ALLOC: u64 = 128 * 1024 * 1024;
const MAX_ENCODE_WORK_BYTES: u64 = 128 * 1024 * 1024;

const MAX_DECODED_PIXELS: usize = 4 * 1024 * 1024;
const MAX_IMAGE_EDGE: usize = 8192;
const MAX_SIXEL_BYTES: usize = 20 * 1024 * 1024;
const MAX_PNG_BYTES: usize = 20 * 1024 * 1024;

/// Decode one complete SIXEL control string into a PNG.
///
/// The decoder is deliberately bounded before handing the stream to
/// `icy_sixel`: raster declarations, repeats, and row advances are scanned
/// first so hostile dimensions cannot make the dependency allocate its much
/// larger internal maximum.
pub fn decode_sixel_to_png(bytes: &[u8]) -> Result<Vec<u8>> {
	if bytes.len() > MAX_SIXEL_BYTES {
		return Err(anyhow::Error::msg("SIXEL payload exceeds the 20 MiB limit"));
	}
	preflight_sixel_dimensions(bytes)?;

	let decoded = SixelImage::decode(bytes)
		.map_err(|err| anyhow::Error::msg(format!("Failed to decode SIXEL: {err}")))?;
	guard_dimensions(decoded.width, decoded.height)?;

	let width =
		u32::try_from(decoded.width).map_err(|_| anyhow::Error::msg("SIXEL width is too large"))?;
	let height =
		u32::try_from(decoded.height).map_err(|_| anyhow::Error::msg("SIXEL height is too large"))?;
	let rgba = RgbaImage::from_raw(width, height, decoded.pixels)
		.ok_or_else(|| anyhow::Error::msg("Decoded SIXEL pixel buffer has invalid dimensions"))?;
	let mut png = Vec::with_capacity(rgba.len().min(MAX_PNG_BYTES));
	DynamicImage::ImageRgba8(rgba)
		.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
		.map_err(|err| anyhow::Error::msg(format!("Failed to encode decoded SIXEL as PNG: {err}")))?;
	if png.len() > MAX_PNG_BYTES {
		return Err(anyhow::Error::msg("Decoded SIXEL PNG exceeds the 20 MiB limit"));
	}
	Ok(png)
}

fn guard_dimensions(width: usize, height: usize) -> Result<()> {
	if width == 0
		|| height == 0
		|| width > MAX_IMAGE_EDGE
		|| height > MAX_IMAGE_EDGE
		|| width.saturating_mul(height) > MAX_DECODED_PIXELS
	{
		return Err(anyhow::Error::msg("SIXEL image dimensions exceed the decode limit"));
	}
	Ok(())
}

fn preflight_sixel_dimensions(bytes: &[u8]) -> Result<()> {
	let payload_start = bytes
		.windows(2)
		.position(|window| window == b"\x1bP")
		.map(|index| index + 2)
		.or_else(|| {
			bytes
				.iter()
				.position(|byte| *byte == 0x90)
				.map(|index| index + 1)
		})
		.ok_or_else(|| anyhow::Error::msg("SIXEL data is missing a DCS introducer"))?;
	let q = bytes[payload_start..]
		.iter()
		.position(|byte| *byte == b'q')
		.map(|index| payload_start + index + 1)
		.ok_or_else(|| anyhow::Error::msg("SIXEL data is missing its q introducer"))?;

	let mut x = 0usize;
	let mut y = 0usize;
	let mut max_width = 0usize;
	let mut max_height = 0usize;
	let mut index = q;
	while index < bytes.len() {
		let byte = bytes[index];
		if byte == 0x9c || (byte == 0x1b && bytes.get(index + 1) == Some(&b'\\')) {
			break;
		}
		match byte {
			b'?'..=b'~' => {
				x = x.saturating_add(1);
				max_width = max_width.max(x);
				max_height = max_height.max(y.saturating_add(6));
			},
			b'!' => {
				let (count, next) = decimal(bytes, index + 1);
				if let Some(sixel) = bytes.get(next)
					&& (b'?'..=b'~').contains(sixel)
				{
					x = x.saturating_add(count);
					max_width = max_width.max(x);
					max_height = max_height.max(y.saturating_add(6));
					index = next;
				}
			},
			b'-' => {
				y = y.saturating_add(6);
				x = 0;
			},
			b'$' => x = 0,
			b'"' => {
				let mut values = [0usize; 4];
				let mut next = index + 1;
				for value in &mut values {
					let (parsed, end) = decimal(bytes, next);
					*value = parsed;
					next = end;
					if bytes.get(next) != Some(&b';') {
						break;
					}
					next += 1;
				}
				if values[2] > 0 || values[3] > 0 {
					guard_dimensions(values[2].max(1), values[3].max(1))?;
					max_width = max_width.max(values[2]);
					max_height = max_height.max(values[3]);
				}
				index = next.saturating_sub(1);
			},
			_ => {},
		}
		guard_dimensions(max_width.max(1), max_height.max(1))?;
		index += 1;
	}
	guard_dimensions(max_width.max(1), max_height.max(1))
}

fn decimal(bytes: &[u8], mut index: usize) -> (usize, usize) {
	let mut value = 0usize;
	while let Some(&digit @ b'0'..=b'9') = bytes.get(index) {
		value = value
			.saturating_mul(10)
			.saturating_add((digit - b'0') as usize);
		index += 1;
	}
	(value, index)
}

/// Encode image bytes into a SIXEL escape sequence for terminal rendering.
///
/// The input image is decoded and resized to the requested pixel dimensions
/// before encoding.
///
/// # Errors
/// Returns an error if decoding, resizing, or SIXEL encoding fails.
pub fn encode_sixel(bytes: &[u8], target_width_px: u32, target_height_px: u32) -> Result<String> {
	if target_width_px == 0 || target_height_px == 0 {
		return Err(anyhow::Error::msg("Target SIXEL dimensions must be greater than zero"));
	}

	if target_width_px as usize > MAX_IMAGE_EDGE
		|| target_height_px as usize > MAX_IMAGE_EDGE
		|| u64::from(target_width_px) * u64::from(target_height_px) > MAX_DECODED_PIXELS as u64
	{
		return Err(anyhow::Error::msg("Target SIXEL dimensions exceed the encode allocation limit"));
	}
	// Bound dimensions before arithmetic. The pinned encoder emits at most
	// one byte per column/color/band, five bytes per color row, and a bounded
	// header/palette. String growth can reserve twice this finite budget.
	let bands = u64::from(target_height_px).div_ceil(6);
	let worst_payload = bands * (u64::from(target_width_px) + 5) * 256 + bands + 8192;
	if worst_payload > MAX_ENCODE_WORK_BYTES {
		return Err(anyhow::Error::msg("Target SIXEL dimensions exceed the encode allocation limit"));
	}
	if bytes.len() > MAX_IMAGE_INPUT_BYTES {
		return Err(anyhow::Error::msg("SIXEL source image exceeds the 20 MiB limit"));
	}
	let source = decode_image_from_bytes(bytes)?;
	let resized = if source.width() == target_width_px && source.height() == target_height_px {
		source
	} else {
		source.resize_exact(target_width_px, target_height_px, FilterType::Lanczos3)
	};
	let rgba = resized.to_rgba8();
	let options = EncodeOptions::default();
	let encoded =
		sixel_encode(rgba.as_raw(), target_width_px as usize, target_height_px as usize, &options)
			.map_err(|err| anyhow::Error::msg(format!("Failed to encode SIXEL: {err}")))?;
	if encoded.len() > MAX_SIXEL_BYTES {
		return Err(anyhow::Error::msg("Encoded SIXEL payload exceeds the 20 MiB limit"));
	}
	Ok(encoded)
}

fn decode_image_from_bytes(bytes: &[u8]) -> Result<DynamicImage> {
	let mut reader = ImageReader::new(Cursor::new(bytes))
		.with_guessed_format()
		.map_err(|error| anyhow::Error::msg(format!("Failed to detect image format: {error}")))?;
	let mut limits = Limits::default();
	limits.max_image_width = Some(MAX_SOURCE_EDGE);
	limits.max_image_height = Some(MAX_SOURCE_EDGE);
	limits.max_alloc = Some(MAX_SOURCE_ALLOC);
	reader.limits(limits);
	let decoder = reader
		.into_decoder()
		.map_err(|error| anyhow::Error::msg(format!("Failed to decode image: {error}")))?;
	let (width, height) = decoder.dimensions();
	if width == 0
		|| height == 0
		|| u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS
		|| decoder.total_bytes() > MAX_SOURCE_ALLOC
	{
		return Err(anyhow::Error::msg(
			"SIXEL source image dimensions exceed the decode allocation limit",
		));
	}
	DynamicImage::from_decoder(decoder)
		.map_err(|error| anyhow::Error::msg(format!("Failed to decode image: {error}")))
}

#[cfg(test)]
mod tests {
	use base64::{Engine, engine::general_purpose::STANDARD};
	use serde_json::Value;

	use super::*;

	#[test]
	fn shared_engine_preserves_captured_native_frames_pngs_and_error_contracts() -> Result<()> {
		let oracle: Value =
			serde_json::from_str(include_str!("../tests/fixtures/sixel-oracle.json"))?;
		for case in oracle["encode"].as_array().expect("encode fixture rows") {
			let input = STANDARD.decode(case["input"].as_str().expect("base64 input"))?;
			let result = encode_sixel(
				&input,
				case["width"].as_u64().expect("width") as u32,
				case["height"].as_u64().expect("height") as u32,
			);
			if let Some(error) = case["error"].as_str() {
				assert_eq!(result.unwrap_err().to_string(), error);
			} else {
				assert_eq!(result?, case["output"].as_str().expect("SIXEL output"));
			}
		}
		for case in oracle["decode"].as_array().expect("decode fixture rows") {
			let input = STANDARD.decode(case["input"].as_str().expect("base64 input"))?;
			let result = decode_sixel_to_png(&input);
			if let Some(error) = case["error"].as_str() {
				assert_eq!(result.unwrap_err().to_string(), error);
			} else {
				assert_eq!(result?, STANDARD.decode(case["output"].as_str().expect("PNG output"))?);
			}
		}
		Ok(())
	}

	#[test]
	fn oversized_targets_are_rejected_before_source_decode_or_dimension_arithmetic() {
		// Ordinary full-HD display targets still reach source decoding.
		assert!(
			encode_sixel(b"not an image", 1920, 1080)
				.unwrap_err()
				.to_string()
				.starts_with("Failed to decode image:")
		);
		for (width, height) in [(u32::MAX, u32::MAX), (8192, 8192), (8192, 512)] {
			let error = encode_sixel(b"not an image", width, height).unwrap_err();
			assert_eq!(
				error.to_string(),
				"Target SIXEL dimensions exceed the encode allocation limit"
			);
		}
	}

	#[test]
	fn oversized_compressed_source_is_rejected_before_format_parsing() {
		let input = vec![0; MAX_IMAGE_INPUT_BYTES + 1];
		assert_eq!(
			encode_sixel(&input, 1, 1).unwrap_err().to_string(),
			"SIXEL source image exceeds the 20 MiB limit"
		);
	}

	#[test]
	fn source_dimensions_are_checked_before_allocating_a_decoded_bitmap() -> Result<()> {
		let mut bitmap = Vec::new();
		DynamicImage::ImageRgb8(image::RgbImage::new(1, 1))
			.write_to(&mut Cursor::new(&mut bitmap), ImageFormat::Bmp)?;
		bitmap[18..22].copy_from_slice(&6000_u32.to_le_bytes());
		bitmap[22..26].copy_from_slice(&6000_u32.to_le_bytes());
		assert!(
			encode_sixel(&bitmap, 1, 1)
				.unwrap_err()
				.to_string()
				.contains("allocation limit")
		);
		Ok(())
	}

	#[test]
	fn encoded_transparent_pixels_remain_transparent_after_decoding() -> Result<()> {
		let mut pixels = image::RgbaImage::new(2, 6);
		pixels.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
		let mut png = Vec::new();
		DynamicImage::ImageRgba8(pixels).write_to(&mut Cursor::new(&mut png), ImageFormat::Png)?;
		let encoded = encode_sixel(&png, 2, 6)?;
		let decoded = image::load_from_memory(&decode_sixel_to_png(encoded.as_bytes())?)?.to_rgba8();
		assert_eq!(decoded.dimensions(), (2, 6));
		assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
		assert_eq!(decoded.get_pixel(1, 0).0[3], 0);
		Ok(())
	}
}
