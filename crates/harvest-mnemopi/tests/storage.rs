//! Persisted/session contracts; execution belongs to consolidated verification.

use harvest_mnemopi::{
	Mnemopi, RecallOptions, RememberOptions,
	beam::{BeamConfig, BeamOptions},
};

fn options(session: &str) -> BeamOptions {
	BeamOptions {
		session_id: Some(session.into()),
		config: Some(BeamConfig { working_memory_ttl_hours: 24, ..Default::default() }),
		..Default::default()
	}
}

#[test]
fn reopened_bank_preserves_rows_and_other_sessions_cannot_get_or_delete_private_memories() {
	let directory = tempfile::tempdir().unwrap();
	let path = directory.path().join("mnemopi.db");
	let mut first = options("first");
	first.db_path = Some(path.clone());
	let memory = Mnemopi::open(first).unwrap();
	let id = memory
		.remember("Storage contract uses the cobalt database", &RememberOptions::default())
		.unwrap();
	drop(memory);
	let mut second = options("second");
	second.db_path = Some(path.clone());
	let memory = Mnemopi::open(second).unwrap();
	assert!(memory.get(&id).unwrap().is_none());
	assert!(!memory.forget(&id).unwrap());
	assert!(
		memory
			.recall("cobalt database", 5, &RecallOptions::default())
			.unwrap()
			.is_empty()
	);
	drop(memory);
	let mut first = options("first");
	first.db_path = Some(path);
	let memory = Mnemopi::open(first).unwrap();
	assert_eq!(
		memory.get(&id).unwrap().unwrap()["content"],
		"Storage contract uses the cobalt database"
	);
}

#[test]
fn duplicate_retention_raises_importance_and_content_edit_replaces_full_text_index() {
	let memory = Mnemopi::open(options("session")).unwrap();
	let id = memory
		.remember("The current launch codename is jasper", &RememberOptions::default())
		.unwrap();
	let duplicate = memory
		.remember("The current launch codename is jasper", &RememberOptions {
			importance: Some(0.9),
			..Default::default()
		})
		.unwrap();
	assert_eq!(duplicate, id);
	assert_eq!(memory.get(&id).unwrap().unwrap()["importance"], 0.9);
	assert!(
		memory
			.update(&id, Some("The release codename is vermilion"), None)
			.unwrap()
	);
	assert!(
		memory
			.recall("jasper", 5, &RecallOptions::default())
			.unwrap()
			.is_empty()
	);
	let results = memory
		.recall("vermilion", 5, &RecallOptions::default())
		.unwrap();
	assert_eq!(results.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec![id.as_str()]);
}

#[test]
fn sleep_preview_is_read_only_and_failed_episode_publication_rolls_back_claims() {
	let memory = Mnemopi::open(options("session")).unwrap();
	let id = memory
		.remember("User prefers cobalt terminal colors", &RememberOptions::default())
		.unwrap();
	memory
		.beam()
		.db
		.execute("UPDATE working_memory SET timestamp='2020-01-01T00:00:00.000Z' WHERE id=?", [&id])
		.unwrap();
	let preview = memory.sleep(true).unwrap();
	assert_eq!(preview["items_consolidated"], 1);
	let claim: Option<String> = memory
		.beam()
		.db
		.query_row("SELECT consolidated_at FROM working_memory WHERE id=?", [&id], |r| r.get(0))
		.unwrap();
	assert!(claim.is_none());
	memory
		.beam()
		.db
		.execute_batch(
			"CREATE TRIGGER reject_episode BEFORE INSERT ON episodic_memory BEGIN SELECT \
			 RAISE(ABORT,'disk fixture'); END;",
		)
		.unwrap();
	assert!(
		memory
			.sleep(false)
			.unwrap_err()
			.to_string()
			.contains("disk fixture")
	);
	let claim: Option<String> = memory
		.beam()
		.db
		.query_row("SELECT consolidated_at FROM working_memory WHERE id=?", [&id], |r| r.get(0))
		.unwrap();
	assert!(claim.is_none());
	memory
		.beam()
		.db
		.execute_batch("DROP TRIGGER reject_episode")
		.unwrap();
	let result = memory.sleep(false).unwrap();
	assert_eq!(result["items_consolidated"], 1);
	let summary_of: String = memory
		.beam()
		.db
		.query_row("SELECT summary_of FROM episodic_memory", [], |r| r.get(0))
		.unwrap();
	assert_eq!(summary_of, id);
	assert_eq!(memory.sleep(false).unwrap()["status"], "no_op");
}

#[test]
fn oversized_memory_and_batch_preflight_leave_no_partial_rows() {
	let memory = Mnemopi::open(options("session")).unwrap();
	let large = "x".repeat(harvest_mnemopi::limits::MAX_CONTENT_BYTES + 1);
	assert!(
		memory
			.remember(&large, &RememberOptions::default())
			.unwrap_err()
			.to_string()
			.contains("exceeds")
	);
	let items = [
		harvest_mnemopi::RememberBatchItem {
			content: "first must not commit".into(),
			..Default::default()
		},
		harvest_mnemopi::RememberBatchItem {
			content: "invalid importance".into(),
			importance: Some(2.0),
			..Default::default()
		},
	];
	assert!(
		memory
			.remember_batch(&items, &Default::default())
			.unwrap_err()
			.to_string()
			.contains("importance")
	);
	assert_eq!(memory.stats().unwrap().total_memories, 0);
}

#[cfg(unix)]
#[test]
fn database_symlink_outside_host_approved_parent_is_rejected_without_changing_target() {
	let root = tempfile::tempdir().unwrap();
	let outside = tempfile::tempdir().unwrap();
	let target = outside.path().join("outside.db");
	std::fs::write(&target, b"preserve outside bytes").unwrap();
	let link = root.path().join("mnemopi.db");
	std::os::unix::fs::symlink(&target, &link).unwrap();
	let mut selected = options("session");
	selected.db_path = Some(link);
	assert!(Mnemopi::open(selected).is_err());
	assert_eq!(std::fs::read(target).unwrap(), b"preserve outside bytes");
}
