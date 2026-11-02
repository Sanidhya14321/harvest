//! SIXEL terminal-image encoding and bounded decoding.
//!
//! Encodes PNG/JPEG/WebP/GIF/BMP images for terminal display and decodes
//! captured SIXEL control strings back to bounded PNG attachments. Every
//! dimension is checked before the corresponding allocation happens.
use std::io::Cursor;

use anyhow::{Result, bail};
use icy_sixel::{EncodeOptions, SixelImage, sixel_encode};
use image::{DynamicImage, ImageFormat, ImageReader, Limits, RgbaImage, imageops::FilterType};

/// Largest decoded or encoded image, in pixels.
pub const MAX_DECODED_PIXELS: usize = 4 * 1024 * 1024;
/// Largest image edge, in pixels.
pub const MAX_IMAGE_EDGE: usize = 8192;
/// Largest accepted SIXEL payload.
pub const MAX_SIXEL_BYTES: usize = 20 * 1024 * 1024;
/// Largest PNG produced from a decoded SIXEL.
pub const MAX_PNG_BYTES: usize = 20 * 1024 * 1024;
/// Largest source image decoded for encoding, in pixels.
pub const MAX_SOURCE_PIXELS: u64 = 64 * 1024 * 1024;
/// Largest source image file accepted for encoding.
pub const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;

/// Decode one complete SIXEL control string into PNG bytes.
///
/// Raster declarations, repeats and row advances are scanned first so hostile
/// dimensions cannot make the decoder allocate its much larger internal
/// maximum.
pub fn decode_sixel_to_png(bytes: &[u8]) -> Result<Vec<u8>> {
	if bytes.len() > MAX_SIXEL_BYTES {
		bail!("SIXEL payload exceeds the 20 MiB limit");
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
		bail!("Decoded SIXEL PNG exceeds the 20 MiB limit");
	}
	Ok(png)
}

/// Encode image bytes into a SIXEL escape sequence, resized to exactly
/// `target_width_px` × `target_height_px`.
///
/// Target and source dimensions are validated before decoding or resizing.
pub fn encode_sixel(bytes: &[u8], target_width_px: u32, target_height_px: u32) -> Result<String> {
	if target_width_px == 0 || target_height_px == 0 {
		bail!("Target SIXEL dimensions must be greater than zero");
	}
	if guard_dimensions(target_width_px as usize, target_height_px as usize).is_err() {
		bail!("Target SIXEL dimensions exceed the encode limit");
	}

	let source = decode_image_from_bytes(bytes)?;
	let resized = if source.width() == target_width_px && source.height() == target_height_px {
		source
	} else {
		source.resize_exact(target_width_px, target_height_px, FilterType::Lanczos3)
	};
	let rgba = resized.to_rgba8();
	let options = EncodeOptions::default();
	sixel_encode(rgba.as_raw(), target_width_px as usize, target_height_px as usize, &options)
		.map_err(|err| anyhow::Error::msg(format!("Failed to encode SIXEL: {err}")))
}

fn decode_image_from_bytes(bytes: &[u8]) -> Result<DynamicImage> {
	if bytes.len() > MAX_SOURCE_BYTES {
		bail!("Source image exceeds the 64 MiB limit");
	}
	let reader = || {
		ImageReader::new(Cursor::new(bytes))
			.with_guessed_format()
			.map_err(|e| anyhow::Error::msg(format!("Failed to detect image format: {e}")))
	};
	// `Limits` is not strictly enforced by every decoder, so check the header
	// dimensions explicitly before any pixel allocation.
	let (width, height) = reader()?
		.into_dimensions()
		.map_err(|e| anyhow::Error::msg(format!("Failed to decode image: {e}")))?;
	if u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS {
		bail!("Source image dimensions exceed the decode limit");
	}

	let mut limits = Limits::default();
	limits.max_image_width = Some(width);
	limits.max_image_height = Some(height);
	limits.max_alloc = Some(MAX_SOURCE_PIXELS * 4 * 2);
	let mut reader = reader()?;
	reader.limits(limits);
	reader
		.decode()
		.map_err(|e| anyhow::Error::msg(format!("Failed to decode image: {e}")))
}

fn guard_dimensions(width: usize, height: usize) -> Result<()> {
	if width == 0
		|| height == 0
		|| width > MAX_IMAGE_EDGE
		|| height > MAX_IMAGE_EDGE
		|| width.saturating_mul(height) > MAX_DECODED_PIXELS
	{
		bail!("SIXEL image dimensions exceed the decode limit");
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

#[cfg(test)]
mod tests {
	use super::*;

	fn png(width: u32, height: u32) -> Vec<u8> {
		let image = RgbaImage::from_fn(width, height, |x, y| {
			image::Rgba([(x * 40) as u8, (y * 40) as u8, 128, 255])
		});
		let mut out = Vec::new();
		DynamicImage::ImageRgba8(image)
			.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
			.unwrap();
		out
	}

	#[test]
	fn encode_then_decode_roundtrips_dimensions() -> Result<()> {
		let sixel = encode_sixel(&png(4, 6), 8, 12)?;
		assert!(sixel.starts_with("\x1bP"));
		let decoded = image::load_from_memory(&decode_sixel_to_png(sixel.as_bytes())?)?;
		assert_eq!((decoded.width(), decoded.height()), (8, 12));
		Ok(())
	}

	#[test]
	fn oversized_encode_target_is_rejected_before_decoding() {
		// Garbage source bytes prove the target check runs before any decode.
		let err = encode_sixel(b"not an image", 9000, 10).unwrap_err();
		assert_eq!(err.to_string(), "Target SIXEL dimensions exceed the encode limit");
		let err = encode_sixel(b"not an image", 0, 10).unwrap_err();
		assert_eq!(err.to_string(), "Target SIXEL dimensions must be greater than zero");
	}

	#[test]
	fn hostile_raster_declaration_is_rejected() {
		let err = decode_sixel_to_png(b"\x1bPq\"1;1;100000;100000#0~\x1b\\").unwrap_err();
		assert_eq!(err.to_string(), "SIXEL image dimensions exceed the decode limit");
	}

	#[test]
	fn repeat_count_overflow_is_rejected() {
		let err = decode_sixel_to_png(b"\x1bPq!99999999~\x1b\\").unwrap_err();
		assert_eq!(err.to_string(), "SIXEL image dimensions exceed the decode limit");
	}

	#[test]
	fn missing_introducers_map_to_errors() {
		assert_eq!(
			decode_sixel_to_png(b"q~").unwrap_err().to_string(),
			"SIXEL data is missing a DCS introducer"
		);
		assert_eq!(
			decode_sixel_to_png(b"\x1bP~").unwrap_err().to_string(),
			"SIXEL data is missing its q introducer"
		);
	}

	#[test]
	fn undecodable_source_maps_to_decode_error() {
		let err = encode_sixel(b"definitely not an image", 4, 4).unwrap_err();
		assert!(err.to_string().starts_with("Failed to"), "{err}");
	}
}
