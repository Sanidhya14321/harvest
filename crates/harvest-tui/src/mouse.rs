//! SGR mouse reports shared by the terminal frontend and host bindings.
//!
//! Coordinates are zero based. Decimal fields retain the legacy frontend's
//! IEEE-754 conversion, including its low-32-bit button flag interpretation.

/// A complete decoded SGR report, suitable for frame-local hit testing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SgrMouseEvent {
	pub button:     f64,
	pub col:        f64,
	pub row:        f64,
	pub release:    bool,
	/// -1 is vertical up, 1 is down; horizontal wheel events have no direction.
	pub wheel:      Option<i8>,
	pub motion:     bool,
	pub left_click: bool,
}

fn decimal(field: &str) -> Option<f64> {
	if field.is_empty() || !field.bytes().all(|byte| byte.is_ascii_digit()) {
		return None;
	}
	field.parse().ok()
}

/// Decode exactly one report. No whitespace, signed fields, concatenated
/// reports, or trailing line terminators are accepted.
///
/// ```
/// let event = harvest_tui::mouse::parse_sgr_mouse("\x1b[<65;4;8M").unwrap();
/// assert_eq!((event.col, event.row, event.wheel), (3.0, 7.0, Some(1)));
/// assert!(!event.left_click);
/// ```
pub fn parse_sgr_mouse(data: &str) -> Option<SgrMouseEvent> {
	let body = data.strip_prefix("\x1b[<")?;
	let (&suffix, _) = body.as_bytes().split_last()?;
	let release = match suffix {
		b'M' => false,
		b'm' => true,
		_ => return None,
	};
	let mut fields = body[..body.len() - 1].split(';');
	let button = decimal(fields.next()?)?;
	let col = decimal(fields.next()?)? - 1.0;
	let row = decimal(fields.next()?)? - 1.0;
	if fields.next().is_some() {
		return None;
	}
	// ECMAScript ToUint32: decimal values are nonnegative, and infinity
	// becomes zero. A direct saturating float cast would misroute big buttons.
	let flags = if button.is_finite() {
		(button % 4_294_967_296.0) as u32
	} else {
		0
	};
	let wheel = if flags & 64 != 0 && flags & 2 == 0 {
		Some(if flags & 1 == 0 { -1 } else { 1 })
	} else {
		None
	};
	let motion = flags & 32 != 0 && wheel.is_none();
	let left_click = !release && wheel.is_none() && !motion && flags & 3 == 0;
	Some(SgrMouseEvent { button, col, row, release, wheel, motion, left_click })
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn clicks_releases_and_drags_route_differently() {
		let click = parse_sgr_mouse("\x1b[<0;5;9M").unwrap();
		assert_eq!((click.col, click.row), (4.0, 8.0));
		assert!(click.left_click);
		let release = parse_sgr_mouse("\x1b[<0;5;9m").unwrap();
		assert!(release.release);
		assert!(!release.left_click);
		let drag = parse_sgr_mouse("\x1b[<32;5;9M").unwrap();
		assert!(drag.motion);
		assert!(!drag.left_click);
	}

	#[test]
	fn horizontal_scroll_never_becomes_vertical_scroll() {
		for (button, wheel) in
			[(64, Some(-1)), (65, Some(1)), (66, None), (67, None), (69, Some(1)), (70, None)]
		{
			let event = parse_sgr_mouse(&format!("\x1b[<{button};1;1M")).unwrap();
			assert_eq!(event.wheel, wheel);
			assert!(!event.left_click);
			assert!(!event.motion);
		}
	}

	#[test]
	fn invalid_reports_do_not_become_clicks() {
		for report in [
			"",
			"a",
			"\x1b[A",
			"\x1b[<;1;1M",
			"\x1b[<-1;1;1M",
			"\x1b[<0;1;1;1M",
			"\x1b[<0;1;1M\n",
			"\x1b[<0;1;1M\r",
			"\x1b[<0;１;1M",
			"\x1b[<0;1;1M\x1b[<0;1;1M",
			"\x1b[<0;1;1é",
		] {
			assert_eq!(parse_sgr_mouse(report), None, "{report:?}");
		}
	}

	#[test]
	fn numeric_boundaries_preserve_legacy_hit_testing_and_flags() {
		let event = parse_sgr_mouse("\x1b[<4294967361;0000;9007199254740993M").unwrap();
		assert_eq!(event.wheel, Some(1));
		assert_eq!(event.col, -1.0);
		assert_eq!(event.row, 9_007_199_254_740_991.0);
		let event = parse_sgr_mouse(&format!("\x1b[<{};1;1M", "9".repeat(400))).unwrap();
		assert_eq!(event.button, f64::INFINITY);
		assert!(event.left_click);
	}
}
