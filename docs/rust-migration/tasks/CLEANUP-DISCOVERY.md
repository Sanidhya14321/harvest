# Cleanup discovery: whole-feature Rust cutover

ID `tooling:cleanup-discovery`; reviewed, in progress. Prerequisites:
`control:playbook`, `review:tooling`. Base `8b30730`. Same active migration agent
owns this batch and the shared CLI/lib registrations; no parallel writer.
Retained capability `surface:cleanup-discovery` was missing from the inventory
and is now registered explicitly. Source: `scripts/cleanup-scan.ts` (492 lines,
17,304 bytes). Original source baseline is unchanged.

## Contract and destinations

Read-only discovery for the cleanup command, with all six reports: oversized
objects, normalized clone clusters/ranges, junk drawers, dead-export candidates
and public-surface tiers, deep imports, defensive-check density. Preserve defaults
coding-agent/ai/catalog/utils, explicit comma-separated packages/all, top and JSON.
Results are heuristic candidates: protected public exports are not deletion
permission. Never execute source, invoke a model/runtime, edit files or infer
semantic dead code. Corpus includes TS tests/scripts for reference classification.

Sole Rust implementation: `rust-agent/src/cleanup_scan/{mod,clones,render}.rs`;
structural facts use `crates/pi-ast/src/cleanup.rs` and existing cached parser;
shared source whitespace pattern in source_text.rs and existing pi-edit trimming.
The prior benchmark difficulty helper now uses those same utilities rather than
a second whitespace implementation. Native CLI `harvest cleanup-scan` / `omp
cleanup-scan` and ordinary Rust scan/Options/Report API share the implementation.

All consumers: both `.harvest/{commands,prompts}/cleanup.md` now invoke the native
Cargo command; package.json cleanup:scan provides the same developer entrypoint.
The stale old-script cache record is removed from .harvest/code-index.json; no
other generated index content is rewritten and no Rust index entries are invented.
No first-party source imports or tests consumed the standalone script. The catalog
baseline/ledger retains its historical row. Existing ts-morph dependencies remain
for other tools, including inline-functions; they are not orphaned by this move.

## Written behavior and removal

Traversal/indexing, AST declarations and class methods, relative re-export/barrel
closure, lexical reference/test-only index, clone union/find interval grouping,
all ranking/filtering/reporting paths and text/JSON CLI are ported together.
Reports retain the six legacy field names and add version, scope/corpus counts
and diagnostics. Native deterministic path tie ordering replaces filesystem/glob/
locale tie order. Floating JSON numbers may include .0. Text labels are JSON
escaped to prevent terminal control injection. Parser recovery and unavailable
exports metadata are explicit diagnostics; legacy silently used those fallbacks.
No parser recovery result proves semantic dead code.

Native bounds: 20,000 TS files, 100,000 visited entries, depth 64, path <=4096
UTF-8 bytes and total path bytes <=4 MiB; 4 MiB/file, 128 MiB corpus; 500,000
identifier vocabulary and four million distinct reference pairs; one million
clone windows; 100,000 dead-export candidates, 16 MiB serialized command output;
top 1..1000. Source is inert UTF-8 text (replacement decoding is diagnosed).
Symlinks/generated/vendor/node_modules trees are not followed. Package selection
must resolve to known child directories. Cancellation reaches indexing, parsing
walks and clone grouping. Output errors/budget failures fail explicitly, not by
silently returning incomplete rankings. No report file is written automatically.

Legacy deletion is staged only after the complete consumer move. One TS source
is removed; no library/shared source is deleted. Recovery: revert the coherent
feature commit, restoring script and both prompt entrypoints together.

## Evidence and remaining gates

Five focused Rust integration contracts cover all six reports, declaration/
ambient/default/alias/escaped-specifier/class facts, reference/public protections,
normalization/ranges, invalid scope/top/cancellation, symlink/generated exclusions,
and copied native CLI execution with language runtimes absent from PATH. Run:

```text
cargo xtask test -p harvest-omp --test cleanup_scan
cargo fmt -p harvest-omp -p pi-ast -- --check
cargo check --locked -p harvest-omp -p pi-ast --all-targets --all-features
cargo xtask playbook check
```

The old source at base 8b30730 was executed only as a migration oracle using the
existing temporary Bun development tool. Comparisons cover a controlled fixture,
real utils scope, and the default four-package scope (1,798 scanned files). At
--top=1000 all six row sets match with order/numeric representation normalized:
158 god objects, 757 clone clusters, 243 junk drawers, 4,678 dead exports,
56 deep imports, 938 density entries. Default top-20 differences are equal-score
boundary ordering; native ordering is explicit and deterministic. Default scan
also reports 21 parser-recovery diagnostics. No legacy runtime enters production.
A copied Linux x86_64 candidate separately passed JSON/text/invalid top/unsafe
package checks with PATH=/nonexistent.

Ignored receipts/oracles/logs: target/migration/cleanup-oracle and
conversion-cleanup-scan-*. Full source hashes and final checks bind the commit;
archive historical oracle evidence rather than relabeling it after future edits.

Code/consumer cutover is complete, but this task remains in progress for the
source-removal gate, consolidated cross-consumer strict Clippy/tests and promised
platform/artifact verification. The inherited global source gate still rejects
35 earlier removals first; do not weaken it or infer verified removal from Git D.
Keep capability behavioral/candidate/legacy/promotion stages unverified/pending
until their actual validators accept source-bound evidence. A local copied CLI
check is not all-platform installation or complete migration acceptance.
