//! Experimental plain terminal replay from the shared selected-branch model.
use std::{
	io::{self, Write},
	path::Path,
};

use anyhow::{Result, bail};
use harvest_protocol::{
	display::project_branch,
	session::{SESSION_LIMIT, SessionDocument},
};
use harvest_runtime::JailedFs;
use harvest_terminal::{layout::PlainTextLayout, replay::render_projection};

pub fn execute(workspace: &Path, path: &Path, leaf: &str, columns: u16) -> Result<()> {
	if columns == 0 {
		bail!("replay columns must be positive");
	}
	let fs = JailedFs::open(workspace)?;
	let document = SessionDocument::decode(&fs.read(path, SESSION_LIMIT)?)?;
	let projection = project_branch(&document, leaf)?;
	let mut layout = PlainTextLayout::default();
	let rows = render_projection(&projection, usize::from(columns), &mut layout)?;
	let stdout = io::stdout();
	let mut output = stdout.lock();
	for row in rows {
		// Standalone command output is intentional; no model/protocol work runs here.
		writeln!(output, "{row}")?;
	}
	Ok(())
}
