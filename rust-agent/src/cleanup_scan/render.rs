use std::io::Write;

use anyhow::Result;

use super::Report;
fn label(value: &str) -> String {
	serde_json::to_string(value).expect("string encoding")
}
impl Report {
	pub fn write_text(&self, output: &mut impl Write) -> Result<()> {
		writeln!(
			output,
			"# cleanup-scan — packages: {} ({} files)",
			self
				.packages
				.iter()
				.map(|name| label(name))
				.collect::<Vec<_>>()
				.join(", "),
			self.scanned_files
		)?;
		writeln!(output, "\n## God-object candidates (LOC ≥ 800, ranked by size × structure)")?;
		for row in &self.god_objects {
			writeln!(
				output,
				"- {} — {} loc, {} exports, {} top-level stmts",
				label(&row.file),
				row.loc,
				row.exports,
				row.top_level_statements
			)?;
			for class in &row.classes {
				writeln!(output, "    {}:{} methods", label(&class.name), class.methods)?;
			}
		}
		writeln!(output, "\n## Clone clusters (normalized, ≥2 regions; candidates only)")?;
		for row in &self.clones {
			writeln!(
				output,
				"- ~{} dup lines across {} regions:",
				row.duplicated_sig_lines,
				row.regions.len()
			)?;
			for region in row.regions.iter().take(6) {
				writeln!(output, "    {}", label(region))?;
			}
			if row.regions.len() > 6 {
				writeln!(output, "    … {} more", row.regions.len() - 6)?;
			}
		}
		writeln!(output, "\n## Junk drawers (domain-less names; judge contents)")?;
		for row in &self.junk_drawers {
			writeln!(output, "- {} — {} loc, {} exports", label(&row.file), row.loc, row.exports)?;
		}
		writeln!(
			output,
			"\n## Dead-export candidates (zero non-test refs; prove with LSP + barrel trace)"
		)?;
		writeln!(output, "   barrel-public = explicit entry point: PROTECTED, do not delete.")?;
		writeln!(output, "   wildcard-only = confirm LSP references before any deletion.")?;
		for row in &self.dead_exports {
			writeln!(
				output,
				"- [{}]{} {} → {}",
				row.tier,
				if row.test_only {
					"[test-only-refs]"
				} else {
					""
				},
				label(&row.file),
				label(&row.name)
			)?;
		}
		writeln!(output, "\n## Deep relative imports (../../..)")?;
		for row in &self.deep_imports {
			writeln!(output, "- {} — {} imports", label(&row.file), row.count)?;
		}
		writeln!(output, "\n## Defensive-check density (per 100 loc)")?;
		for row in &self.check_density {
			let checks = &row.checks;
			writeln!(
				output,
				"- {} — {}/100loc (as:{} ?.:{} ??:{} typeof:{} isArr:{}, {} loc)",
				label(&row.file),
				row.per100,
				checks.as_casts,
				checks.opt_chain,
				checks.coalesce,
				checks.typeof_narrow,
				checks.is_array,
				row.loc
			)?;
		}
		if !self.diagnostics.is_empty() {
			writeln!(output, "\n## Analysis diagnostics")?;
			for diagnostic in &self.diagnostics {
				writeln!(output, "- {}", label(diagnostic))?;
			}
		}
		Ok(())
	}
}
