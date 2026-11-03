//! Source snippets are inert mutation/parser inputs, never executable fixtures.
use std::{fs, path::Path};

use anyhow::Result;
use harvest_omp::edit_benchmark::{
	self as benchmark, ContentIdentity, GenerationCase, GenerationOptions, GenerationQuota,
	MutationKind, ShapeConstraint,
};
use harvest_runtime::{cancel::CancelToken, content_hash};
fn one_line() -> ShapeConstraint {
	ShapeConstraint {
		min_changed_lines: 1,
		max_changed_lines: 1,
		min_hunks:         1,
		max_hunks:         1,
	}
}
#[test]
fn exact_changed_lines_use_change_runs_not_added_plus_removed() -> Result<()> {
	let shape =
		benchmark::measure_shape("a\nx\nc\ny\ne\n", "a\nb\nc\nd\ne\n", &CancelToken::default())?;
	assert_eq!(
		(shape.changed_lines, shape.hunks, shape.inserted_lines, shape.removed_lines),
		(2, 2, 2, 2)
	);
	Ok(())
}
#[test]
fn quota_expansion_is_deterministic_and_preserves_order_and_constraints() -> Result<()> {
	let quotas = vec![
		GenerationQuota { mutation: "flip-boolean".into(), count: 2, shape: Some(one_line()) },
		GenerationQuota { mutation: "swap-comparison".into(), count: 1, shape: None },
	];
	let a = benchmark::plan_quotas(&quotas, 42, &CancelToken::default())?;
	let b = benchmark::plan_quotas(&quotas, 42, &CancelToken::default())?;
	assert_eq!(serde_json::to_vec(&a)?, serde_json::to_vec(&b)?);
	assert_eq!(
		a.cases
			.iter()
			.map(|c| c.task_id.as_str())
			.collect::<Vec<_>>(),
		vec!["case-001", "case-002", "case-003"]
	);
	assert_eq!(a.cases[2].mutation, "swap-comparison");
	assert!(a.cases[0].shape.is_some());
	assert_ne!(a.cases[0].seed, a.cases[1].seed);
	Ok(())
}
#[test]
fn calibration_apportions_real_samples_and_keeps_explicit_shapes() -> Result<()> {
	let bytes = br#"{"per_call":{"samples":10,"sizes":{"1":3,"2-5":3,"6-20":4}}}"#;
	let quotas = vec![
		GenerationQuota { mutation: "flip-boolean".into(), count: 1, shape: Some(one_line()) },
		GenerationQuota { mutation: "composite-multi-edit".into(), count: 3, shape: None },
	];
	let plan = benchmark::plan_calibrated_quotas(&quotas, 9, bytes, &CancelToken::default())?;
	let cal = plan.calibration.as_ref().unwrap();
	assert_eq!(cal.source_sha256, content_hash(bytes));
	assert_eq!(cal.apportioned_cases, 3);
	assert_eq!((cal.allocated["1"], cal.allocated["2-5"], cal.allocated["6-20"]), (1, 1, 1));
	assert_eq!(plan.cases[0].shape.as_ref().unwrap().max_hunks, 1);
	let invalid = br#"{"per_call":{"samples":2,"sizes":{"1":3}}}"#;
	assert!(
		benchmark::plan_calibrated_quotas(&quotas, 9, invalid, &CancelToken::default()).is_err()
	);
	Ok(())
}
fn source(root: &Path) -> Result<std::path::PathBuf> {
	let source = root.join("source");
	fs::create_dir(&source)?;
	fs::write(source.join("one.ts"), "export const enabled = true;\n")?;
	Ok(source)
}
#[test]
fn infeasible_shape_and_duplicate_content_do_not_publish_fixtures() -> Result<()> {
	let root = tempfile::tempdir()?;
	let src = source(root.path())?;
	let output = root.path().join("never-created");
	let original = "export const enabled = true;\n";
	let options = GenerationOptions {
		shape: Some(ShapeConstraint {
			min_changed_lines: 2,
			max_changed_lines: 5,
			min_hunks:         1,
			max_hunks:         1,
		}),
		..Default::default()
	};
	assert!(
		benchmark::generate_task_with_options(
			&src,
			&output,
			"bad-shape",
			MutationKind::FlipBoolean,
			7,
			&options,
			&CancelToken::default()
		)
		.is_err()
	);
	assert!(!output.exists());
	let options = GenerationOptions {
		excluded_content: vec![ContentIdentity {
			original_sha256: content_hash(original.as_bytes()),
			mutated_sha256:  content_hash(b"export const enabled = false;\n"),
		}],
		..Default::default()
	};
	assert!(
		benchmark::generate_task_with_options(
			&src,
			&output,
			"duplicate",
			MutationKind::FlipBoolean,
			7,
			&options,
			&CancelToken::default()
		)
		.is_err()
	);
	assert!(!output.exists());
	assert_eq!(fs::read(src.join("one.ts"))?, original.as_bytes());
	Ok(())
}
#[test]
fn constrained_batch_records_measurement_and_rejects_cross_path_duplicates() -> Result<()> {
	let root = tempfile::tempdir()?;
	let src = source(root.path())?;
	fs::copy(src.join("one.ts"), src.join("two.ts"))?;
	let cases = (0..3)
		.map(|i| GenerationCase {
			task_id:  format!("case-{i}"),
			mutation: "flip-boolean".into(),
			seed:     i,
			shape:    Some(one_line()),
		})
		.collect::<Vec<_>>();
	let output = root.path().join("fixtures");
	let result = benchmark::generate_batch(
		&src,
		&output,
		&cases,
		&GenerationOptions { deduplicate_content: true, ..Default::default() },
		&CancelToken::default(),
	)?;
	assert!(!result.complete);
	assert_eq!(result.successful, 1);
	assert_eq!(result.cases.iter().map(|c| c.state).collect::<Vec<_>>(), vec![
		"completed",
		"failed",
		"not_started"
	]);
	let measured = result.cases[0]
		.report
		.as_ref()
		.unwrap()
		.measured_shape
		.as_ref()
		.unwrap();
	assert_eq!((measured.changed_lines, measured.hunks), (1, 1));
	assert!(!output.join("case-1").exists());
	assert!(output.join("generation.json").is_file());
	Ok(())
}
#[test]
fn invalid_quota_history_and_cancel_fail_before_output() -> Result<()> {
	let quotas =
		vec![GenerationQuota { mutation: "not-a-kind".into(), count: 1, shape: None }];
	assert!(benchmark::plan_quotas(&quotas, 0, &CancelToken::default()).is_err());
	assert!(
		ContentIdentity { original_sha256: "bad".into(), mutated_sha256: "bad".into() }
			.validate()
			.is_err()
	);
	let mut cancel = CancelToken::default();
	cancel
		.emplace_abort_token()
		.abort(harvest_runtime::cancel::AbortReason::User);
	let quotas =
		vec![GenerationQuota { mutation: "flip-boolean".into(), count: 1, shape: None }];
	assert!(benchmark::plan_quotas(&quotas, 0, &cancel).is_err());
	Ok(())
}
