//! Bounded host adapter for optional rectangular background fills.
use harvest_tui::deccara::{self, MAX_PLAN_LINES, MAX_PLAN_UNITS};
use napi::{Error, JsString, Result, bindgen_prelude::Array};
use napi_derive::napi;

use crate::{js, terminal::cells};

#[napi(object)]
pub struct NativeBgFillAnalysis {
	pub cut:      u32,
	pub left_col: u32,
	pub bg:       String,
}

#[napi(object)]
pub struct NativeDeccaraCuts {
	/// UTF-16 cutoffs; -1 means preserve the original row. Empty means no
	/// changes.
	pub cuts:     Vec<i32>,
	pub sequence: String,
}

#[napi]
pub fn analyze_deccara_line(line: JsString, width: f64) -> Result<Option<NativeBgFillAnalysis>> {
	let width = cells(width)?;
	let line = js::utf16_bounded(line, MAX_PLAN_UNITS)?;
	Ok(deccara::analyze_line(&line, width).map(|analysis| NativeBgFillAnalysis {
		// Both offsets are bounded by the validated frame/input geometry.
		cut:      analysis.cut as u32,
		left_col: analysis.left_col as u32,
		bg:       analysis.bg,
	}))
}

#[napi]
pub fn encode_deccara_rectangle(
	top: f64,
	left: f64,
	bottom: f64,
	right: f64,
	sgr: JsString,
) -> Result<String> {
	let (top, left, bottom, right) = (cells(top)?, cells(left)?, cells(bottom)?, cells(right)?);
	let params = js::utf16_bounded(sgr, harvest_tui::MAX_CSI_BYTES)?;
	let params = String::from_utf16(&params)
		.map_err(|_| Error::from_reason("rectangle attributes must be valid SGR parameters"))?;
	deccara::encode_rectangle(top, left, bottom, right, &params)
		.ok_or_else(|| Error::from_reason("rectangle coordinates or SGR parameters are invalid"))
}

#[napi]
pub fn plan_deccara_cuts(
	#[napi(ts_arg_type = "string[]")] lines: Array,
	width: f64,
	first_screen_row: Option<f64>,
) -> Result<NativeDeccaraCuts> {
	let width = cells(width)?;
	let first = cells(first_screen_row.unwrap_or(0.0))?;
	if lines.len() as usize > MAX_PLAN_LINES {
		return Err(Error::from_reason("terminal fill frame exceeds its row limit"));
	}
	let mut owned = Vec::with_capacity(lines.len() as usize);
	let mut remaining = MAX_PLAN_UNITS;
	for index in 0..lines.len() {
		let line = lines
			.get::<JsString>(index)?
			.ok_or_else(|| Error::from_reason("terminal row changed while planning fills"))?;
		let units = js::utf16_bounded(line, remaining)?;
		remaining -= units.len();
		owned.push(units.to_vec());
	}
	let plan = deccara::plan_fills(&owned, width, first);
	Ok(NativeDeccaraCuts {
		// MAX_PLAN_UNITS is below i32::MAX, so every cutoff fits exactly.
		cuts:     plan
			.cuts
			.into_iter()
			.map(|cut| cut.map_or(-1, |value| value as i32))
			.collect(),
		sequence: plan.sequence,
	})
}
