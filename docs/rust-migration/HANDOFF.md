# Current Rust migration handoff

Continue existing ts-to-rust from its actual remote tip, preserving user/other
agent changes and history. Immutable baseline d811cb2; imported history through
6eefffc is preserved. Previous detail: archive/HANDOFF-2026-10-24-before-difficulty.md
and the earlier context-refresh archive.

## Rules and ownership

Read AGENTS.md, PLAYBOOK.md, SCOPE.md, CONVERSION-PASS.md and authoritative source
and capability manifests. Active sole owner/task: application:conversion-first,
in progress. Convert connected subsystems first; compiler/format/ownership checks
at coherent checkpoints. Consolidated behavioral/installed/platform stabilization
comes later. Keep code-written and verified states separate. Commit/push on this
branch without routine approval or another continue request. No main merge or
publication. An hourly continuation is configured; inspect actual writers/run
state, never create overlapping runs or infer work from scheduling alone.

## Current checkpoint

Difficulty/region continuation base 518458c: corpus CLI/SDK inspection and existing
native AST generator now share bounded lexical difficulty metrics, level/minimum
filters, native preferred/fallback reservoirs and source-hash-bound region
exclusions covering every selected edit. Report/provenance expose policy versions,
counts, score/fallback and regions. Explicit CLI region input is read-only; no
history writer or multi-case scheduler. Prior APIs keep no-policy defaults.
See RUST-BENCHMARK.md and the common reviewed card for native versus historical
semantics, bounds and exact remaining consumers. generate.ts remains retained.
Ignored receipt/log prefix target/migration/conversion-difficulty-*.

Existing owners remain: configuration/value and command cache; shared provider
builder; plugin protocol/process/event forwarding; MCP/LSP/DAP; source corpus,
AST mutations/hunks/fixture archives; native session/run/scoring/report owners.
Benchmark retries include no-edit, selected typed pre-mutation HTTP/IO auth-cache
invalidation and optional child submit timeouts. Every recovery requires complete
tool results and stays inside the original run deadline. All remain unverified
foundations, not completed retained workflows. Earlier exact contracts/receipts
are in NATIVE-PROVIDER-VALUES.md, RUST-BENCHMARK.md, common card and archives.

Compiler, format and ownership only; no TS deletion or verified count advance.
Generate honest counts from cargo xtask playbook status --json. Default product
still launches TS; do not cut it over to this partial candidate.

## Next dependency-ready connections

- Benchmark: multi-case generation, edit-shape quotas/calibration and whole-content
  deduplication, specialized prompts, full syntax eligibility, retained metaharness
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
