# rust-v1 edit benchmark conversion checkpoint

Native task loading, patch application and scoring are written; tests, model-run
integration and installed verification are pending. The legacy metaharness runner
still uses its historical scoring. No existing score is converted or relabeled.

`harvest edit-benchmark list --fixtures DIRECTORY` loads unpacked tasks with
`prompt.md`, `input/`, `expected/` and optional `metadata.json`. Traversal is
sorted, bounded and rejects symlinks. Owned benchmark inputs remain inert data;
the preserved fixture tarball and historical result file are unchanged.

`harvest edit-benchmark compare --expected DIRECTORY --actual DIRECTORY` checks
file sets and exact content, normalizing CRLF to LF only. Every other whitespace,
blank line and final-newline distinction remains significant.

`harvest edit-benchmark apply --expected DIRECTORY --actual ISOLATED_WORKSPACE
--manifest FILE.json` applies ordered replacements using the existing jailed
atomic edit engine. This command intentionally mutates the supplied actual
workspace. Stop at the first failed edit and report remaining edits as skipped.
The result separates patch application, output matching, elapsed microseconds and
optional usage. Compare-only results do not claim patch-application evidence.

The manifest has `scoring_version: "rust-v1"`, an `edits` array of
`{path, old, new}`, and optional `usage`. Usage is kept losslessly; only a report
with valid `input_tokens` and `output_tokens` makes measured usage available.
Unknown/partial/overflowing usage stays unavailable. The scorer never adds pricing
or combines versions. Result failures return nonzero CLI status.

Pending: Rust mutation/fixture generation, archive materialization, edit-shape
analysis, native model-run orchestration/usage integration and the metaharness
consumer cutover. Babel/Prettier historical scores retain their old meanings.
No TS benchmark source is removed before those remaining consumers move.
