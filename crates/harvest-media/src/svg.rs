//! Bounded SVG/SVGZ previews with file-backed image resolution disabled.
use std::sync::{Arc, LazyLock};

use anyhow::Result;
use resvg::{tiny_skia, usvg};

const MAX_RENDER_PIXELS: u64 = 16 * 1024 * 1024;

static FONT_DB: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
	let mut database = usvg::fontdb::Database::new();
	database.load_system_fonts();
	Arc::new(database)
});

/// Rasterize SVG/SVGZ bytes within a nonzero pixel budget, without upscaling.
/// System fonts and embedded data images are supported; external image paths
/// and URLs are never resolved by the document.
pub fn rasterize_svg(input: &[u8], max_width_px: u32, max_height_px: u32) -> Result<Vec<u8>> {
	if max_width_px == 0 || max_height_px == 0 {
		return Err(anyhow::Error::msg("SVG render limits must be greater than zero"));
	}
	if u64::from(max_width_px) * u64::from(max_height_px) > MAX_RENDER_PIXELS {
		return Err(anyhow::Error::msg(format!(
			"SVG render limits exceed the {MAX_RENDER_PIXELS}-pixel safety cap"
		)));
	}

	let mut options = usvg::Options { fontdb: Arc::clone(&FONT_DB), ..usvg::Options::default() };
	// Repository-controlled SVGs must not read arbitrary host files through an
	// <image href="…"> reference. Embedded data URLs retain the default resolver.
	options.image_href_resolver.resolve_string = Box::new(|_, _| None);
	let tree = usvg::Tree::from_data(input, &options)
		.map_err(|error| anyhow::Error::msg(format!("Failed to parse SVG: {error}")))?;
	let source = tree.size();
	let scale = (max_width_px as f32 / source.width())
		.min(max_height_px as f32 / source.height())
		.min(1.0);
	let width = (source.width() * scale).ceil().max(1.0) as u32;
	let height = (source.height() * scale).ceil().max(1.0) as u32;
	let mut pixmap = tiny_skia::Pixmap::new(width, height)
		.ok_or_else(|| anyhow::Error::msg("Failed to allocate SVG render surface"))?;
	resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
	pixmap
		.encode_png()
		.map_err(|error| anyhow::Error::msg(format!("Failed to encode SVG preview: {error}")))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rasterizes_svg_at_intrinsic_size() {
		let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="7"><rect width="12" height="7" fill="red"/></svg>"#;
		let png = rasterize_svg(svg, 100, 100).expect("SVG should rasterize");
		let image = image::load_from_memory(&png).expect("result should be PNG");
		assert_eq!((image.width(), image.height()), (12, 7));
	}

	#[test]
	fn rejects_unbounded_render_surface() {
		let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#;
		let error = rasterize_svg(svg, 8192, 8192).expect_err("oversized canvas should fail");
		assert!(error.to_string().contains("safety cap"));
	}
	#[test]
	fn downscaling_preserves_aspect_ratio_and_visible_pixels() -> Result<()> {
		let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="12"><rect width="24" height="12" fill="blue"/></svg>"#;
		let png = rasterize_svg(svg, 6, 6)?;
		let image = image::load_from_memory(&png)?.to_rgba8();
		assert_eq!(image.dimensions(), (6, 3));
		assert_eq!(image.get_pixel(2, 1).0, [0, 0, 255, 255]);
		Ok(())
	}

	#[test]
	fn external_images_are_blocked_while_embedded_images_render() -> Result<()> {
		use base64::{Engine, engine::general_purpose::STANDARD};
		let root = tempfile::tempdir()?;
		let path = root.path().join("private.png");
		let pixel = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 255, 0, 255]));
		pixel.save(&path)?;
		let encoded = STANDARD.encode(std::fs::read(&path)?);
		for external in [
			path.to_string_lossy().into_owned(),
			format!("file://{}", path.display()),
			"https://127.0.0.1:1/private.png".to_owned(),
		] {
			let svg = format!(
				r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><image width="1" height="1" href="{external}"/></svg>"#
			);
			let png = rasterize_svg(svg.as_bytes(), 1, 1)?;
			let image = image::load_from_memory(&png)?.to_rgba8();
			assert_eq!(image.get_pixel(0, 0).0, [0, 0, 0, 0]);
		}
		let svg = format!(
			r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><image width="1" height="1" href="data:image/png;base64,{encoded}"/></svg>"#
		);
		let png = rasterize_svg(svg.as_bytes(), 1, 1)?;
		assert_eq!(image::load_from_memory(&png)?.to_rgba8().get_pixel(0, 0).0, [0, 255, 0, 255]);
		Ok(())
	}
}
