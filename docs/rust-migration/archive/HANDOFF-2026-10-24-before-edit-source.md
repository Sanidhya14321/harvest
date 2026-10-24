# Current Rust migration handoff

Continue existing ts-to-rust from its actual remote tip, preserving user/other
agent changes and history. Immutable baseline d811cb2; imported history through
6eefffc is preserved. Previous detail: archive/HANDOFF-2026-10-24-before-difficulty.md
and the earlier context-refresh archive.

## Rules and ownership

Read AGENTS.md, PLAYBOOK.md, SCOPE.md, CONVERSION-PASS.md and authoritative source
and capability manifests. Active conversion coordinator: application:conversion-first; reviewed active
leaf ownership is recorded in CONVERSION-PASS.md. One writer, no overlapping run. Convert connected subsystems first; compiler/format/ownership checks
at coherent checkpoints. Consolidated behavioral/installed/platform stabilization
comes later. Keep code-written and verified states separate. Commit/push on this
branch without routine approval or another continue request. No main merge or
publication. An hourly continuation is configured; inspect actual writers/run
state, never create overlapping runs or infer work from scheduling alone.

## Current checkpoint

Edit-source continuation base fd657c6: application:edit-file-source now reviewed/
in progress. Shared Session owns an injected Send+Sync FileSource with matching
policy, preserves default Session::new/FileCache, and exposes one fresh all-path
stage used by existing apply and native exact replacement. Native source/writer
share the same jailed capability; approval and bounded staged metadata remain.
Read updated EDIT-FILE-SOURCE.md; four contracts written/compiled, not executed.
Aggregate pi-edit/application compiler/format/ownership only; addon, behavior,
installed/platform/removal verification pending. No TS deletion or verified count
advance. Receipt/log prefix target/migration/conversion-edit-source-*.

application:edit-write-contract separately owns multi-target publication/failure
review. Shared apply stays sequential and public stage commits no persisted
snapshot/clipboard. Hashline still needs read provenance, mode/approval and writer
connections; do not expose hashline/guidance or claim a transaction yet.

Calibration/quota continuation base c99df19 (owner application:conversion-first):
existing per-call shape measurements now feed explicit deterministic quota plans,
raw exact-hunk constraints and path-independent target-content SHA pair history
before the shared seeded AST reservoirs. Native CLI plan/generate/generate-batch,
SDK and package generate:plan/generate:native connect through the existing snapshot/
fixture writer/archives/run. Batch dedup carry is opt-in; constraints never relax.
Versioned calibration source/weights/allocation and measured/rejected candidates
are journaled. See RUST-BENCHMARK.md and common card for bounds/commands/recovery.
Six new contracts written/compiled, not executed; aggregate conversion checks only.
No TS deletion/parity/installed/source-verification count advances. Legacy producer
specialized prompts/formatter/grammar/metaharness consumers remain retained.
Receipt/log prefix target/migration/conversion-calibration-*.

New bounded prerequisite application:edit-file-source precedes hashline. Shared
Session owns concrete ambient FileCache and applies sequential writes, without
whole-call rollback. Review injectable jailed/cancellable source and actual host
multi-file write contract before ready; never shortcut native jail or seen-lines.

Metaharness narrative trace continuation base dd34b68: reviewed leaf
application:metaharness-trace-report writes/connects the whole native map/reduce
report pipeline, normalized HTTP/offline and selected persisted-session branch
inputs, deterministic tool/error log, bounded concurrent model calls/retries,
usage/diagnostics and atomic jailed output. Native CLI/SDK, package trace:report
and primary README move together. Reuse shared providers/config/live credentials,
HTTP/cancellation/session projection; static prompt assets use existing builds.
Six scripted contracts are written, execution deferred to consolidation. Compiler/
format/playbook evidence only; no installed/parity or source-removal advance.

The last trace-report.ts/explicit legacy-auth alias remains for stored/OAuth users.
application:legacy-provider-credentials is a bounded planned/review-pending
prerequisite, not permission to invent a leaf-specific auth store. Read both new
cards for exact contracts/next actions. Zero TS deletions in this continuation;
cleanup-discovery deletion from the prior checkpoint remains preserved. Ignored
receipt/log prefix conversion-trace-*.

Difficulty/region continuation base 518458c: corpus CLI/SDK inspection and existing
native AST generator now share bounded lexical difficulty metrics, level/minimum
filters, native preferred/fallback reservoirs and source-hash-bound region
exclusions covering every selected edit. Report/provenance expose policy versions,
counts, score/fallback and regions. Explicit CLI region input is read-only; no
history writer or multi-case scheduler. Prior APIs keep no-policy defaults.
See RUST-BENCHMARK.md and the common reviewed card for native versus historical
semantics, bounds and exact remaining consumers. generate.ts remains retained.
Ignored receipt/log prefix target/migration/conversion-difficulty-*.

Multi-case continuation base 8c19c66: generate-batch CLI/SDK now executes explicit
bounded plans against one immutable corpus snapshot. It carries successful regions,
borrows the existing fixture writer lock/publisher, charges actual output bytes
and preserves plan/case journals with completed/failed/cancelled/not_started states.
No resume or shape-quota/calibration policy is claimed. See the same benchmark
and common card; ignored receipts target/migration/conversion-generation-batch-*.

Whole cleanup discovery continuation base 8b30730: tooling:cleanup-discovery owns
the complete six-report scanner, shared AST facts/reference index/barrel trace,
clone grouping and native CLI/SDK. Both cleanup command prompts and package alias
now call Rust; scripts/cleanup-scan.ts (492 lines/17,304 bytes) and its stale
source-index record are removed. Five focused contracts and copied Linux CLI
checks pass; complete legacy/default row sets agree, with deterministic ordering
and explicit diagnostics. See tasks/CLEANUP-DISCOVERY.md. Implementation complete,
source-bound/consolidated/platform verification still pending; task stays active.
Ignored receipts/oracles: conversion-cleanup-scan-* and cleanup-oracle/.

Existing owners remain: configuration/value and command cache; shared provider
builder; plugin protocol/process/event forwarding; MCP/LSP/DAP; source corpus,
AST mutations/hunks/fixture archives; native session/run/scoring/report owners.
Benchmark retries include no-edit, selected typed pre-mutation HTTP/IO auth-cache
invalidation and optional child submit timeouts. Every recovery requires complete
tool results and stays inside the original run deadline. All remain unverified
foundations, not completed retained workflows. Earlier exact contracts/receipts
are in NATIVE-PROVIDER-VALUES.md, RUST-BENCHMARK.md, common card and archives.

Earlier foundations have compiler/format/ownership evidence only; cleanup has the
focused evidence above. Actual cleanup TS deletion is pending source verification;
no verified capability/source-removal count advances from it alone.
Generate honest counts from cargo xtask playbook status --json. Default product
still launches TS; do not cut it over to this partial candidate.

## Next dependency-ready connections

- Benchmark: consolidate written quota/calibration/target-content policy; finish
  specialized prompts, full syntax eligibility, retained metaharness
  run/report/queue workflows and native interactive views; move actual TS consumers.
- application:hashline-host-bridge remains planned/review pending. Native read
  provenance/session EditStore/mode/approval contracts must connect before legacy
  hashline guidance. See tasks/HASHLINE-HOST-BRIDGE.md.
- First-provider-event/idle deadlines need the shared transport notification
  contract; local Started events do not prove provider activity. Broader provider
  account/OAuth/models schema/discovery/overrides/KDL/auth-retry work remains.
- Settings: full schemas/discovery, implicit migration, conflict/quarantine and
  retry-aware persistence/UI. Reuse existing typed config/value owners.
- Plugin native TUI/choice responses/reconnect persistence need explicit contracts;
  do not turn declarative events into executable callbacks.

## Preserve consumers and blockers

Keep shared TS until last consumers move. JS eval workers still serve browser/
computer. Native shell bootstrap/completion generation is retired, but TS generator,
owned installers/test scripts and in-app completion remain pending.
35 inherited removals lack evidence; source gate first rejects
crates/pi-natives/tools/bench-natives.ts. Recent removal evidence is also pending;
see tasks/CONCURRENT-REMOVALS.md. Legacy session/runtime failures reproduce baseline
2ff0193; whole-addon Clippy has 14 existing diagnostics. Release binary exceeds the
unchanged 20 MiB budget; eight-platform installed/release evidence is pending.
Preserve failures/gates and historical evidence. Use cargo xtask test, not direct
cargo test, during consolidation. Reclaim only ordinary caches if needed.
