//! Target selectors belong to nextest; Cargo forbids combining them with --doc.
use anyhow::{Context, Result, bail};

/// Keep package, feature and build options for doctests while removing explicit
/// unit/integration/binary target selectors from a focused nextest invocation.
pub fn doctest_selection(selection: &[String]) -> Result<Vec<String>> {
	let mut output = Vec::new();
	let mut index = 0;
	while let Some(argument) = selection.get(index) {
		if argument == "--" {
			output.extend_from_slice(&selection[index..]);
			break;
		}
		let (option, attached) = argument
			.split_once('=')
			.map_or((argument.as_str(), None), |(option, value)| (option, Some(value)));
		if matches!(option, "--test" | "--bin" | "--bench" | "--example") {
			let value = if let Some(value) = attached {
				value
			} else {
				index += 1;
				selection
					.get(index)
					.with_context(|| format!("{option} needs a target name"))?
			};
			if value.is_empty() || value.starts_with('-') {
				bail!("{option} needs a target name");
			}
		} else if !matches!(
			option,
			"--lib" | "--bins" | "--tests" | "--benches" | "--examples" | "--all-targets"
		) {
			output.push(argument.clone());
		}
		index += 1;
	}
	Ok(output)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn args(values: &[&str]) -> Vec<String> {
		values.iter().map(|value| (*value).to_owned()).collect()
	}

	#[test]
	fn focused_integration_runs_keep_package_and_features_without_mixing_doc_and_test_targets()
	-> Result<()> {
		assert_eq!(
			doctest_selection(&args(&[
				"-p",
				"harvest-omp",
				"--test",
				"session_recovery",
				"--features",
				"full",
				"--release"
			]))?,
			args(&["-p", "harvest-omp", "--features", "full", "--release"])
		);
		Ok(())
	}

	#[test]
	fn joined_target_names_and_plural_switches_do_not_make_doctest_arguments_invalid() -> Result<()>
	{
		assert_eq!(
			doctest_selection(&args(&[
				"--package=xtask",
				"--bin=xtask",
				"--lib",
				"--tests",
				"--bins",
				"--benches",
				"--examples",
				"--all-targets",
				"--locked"
			]))?,
			args(&["--package=xtask", "--locked"])
		);
		Ok(())
	}

	#[test]
	fn incomplete_target_selectors_are_rejected_before_any_test_process_starts() {
		assert!(doctest_selection(&args(&["--test"])).is_err());
		assert!(doctest_selection(&args(&["--bin="])).is_err());
		assert!(doctest_selection(&args(&["--example", "--features", "full"])).is_err());
	}

	#[test]
	fn harness_test_mode_after_the_separator_is_not_consumed_as_a_cargo_target_selector()
	-> Result<()> {
		assert_eq!(doctest_selection(&args(&["--tests", "--", "--test"]))?, args(&["--", "--test"]));
		Ok(())
	}
}
