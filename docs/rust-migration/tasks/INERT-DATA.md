# control:inert-data — consistent completion for reviewed non-executable data

Prerequisite: `control:playbook`. This changes accounting, not application code
or any actual source row's completion state. No source deletion is authorized.

Discovery: strict source auditing already recognizes eight exact security-scanner
inputs as inert language data. But the source ledger's final gate and completed
migration tasks require every row to be physically removed. Thus an explicitly
allowed retained inert input can never complete, even after its Rust consumer
and behavioral/installed contracts are verified.

Contract/destination: share the existing exact inert-input allowlist in
`xtask/src/source_audit.rs`; do not expand it or exempt fixture directories.
Extend `migration.rs` final accounting and `playbook.rs` task accounting to accept
retained rows only when they are classified `fixture`, have all reviewed contract,
consumer, destination and evidence requirements, and both behavioral and installed
verification stages pass. Report these as `verified_inert`, separately from
`verified_removed`. Do not count them as Rust source replacements or infer that
their current TS consumers have migrated. Keep every current row unchanged.

Steps: extract the existing reviewed path predicate; reuse it in source auditing,
the ledger and task validator; implement separate final counts; test retained
allowed data, unfinished evidence and a mislabeled executable fixture; document
the state. No asynchronous process or new persisted format is introduced. Keep
existing schema/status values and immutable baseline unchanged.

```sh
cargo fmt -p xtask -- --check
cargo clippy --locked -p xtask --all-targets --all-features -- -D warnings
cargo xtask test -p xtask
cargo xtask migration
cargo xtask playbook check
```

Completion: those gate contracts pass, arbitrary fixtures cannot use the
exception, actual removed-source progress remains unchanged, and final
`--require-complete` still rejects the unfinished repository. Recovery: revert
the accounting change and this card; do not mark new fixture rows verified or
modify original inventory data to compensate.
