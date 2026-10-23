# Current Rust migration handoff

Use existing `ts-to-rust`; obtain the actual remote tip and preserve history,
user changes and other writers. Original immutable baseline: `d811cb2`. Imported
other-agent history through `6eefffc` remains preserved. Prior checkpoint detail:
[archived handoff](archive/HANDOFF-2026-10-24-before-context-refresh.md).

## Execution rule and ownership

Active task: `application:conversion-first`, in progress. Read root AGENTS.md,
PLAYBOOK.md, SCOPE.md, CONVERSION-PASS.md and authoritative ledgers. The user wants
connected Rust conversion batches first, consolidated behavioral/installed/platform
stabilization later. Avoid per-file tests, repeated addon builds and unsupported
completion claims. Commit/push coherent unverified checkpoints; keep advancing
without requesting another continue. Do not merge to main or publish releases.
An hourly continuation automation is configured; inspect actual run state and
avoid duplicate writers rather than assuming schedule creation proves execution.

## Current checkpoint

Provider values (`165827d`): configuration/values.rs and values/command.rs connect
live env/literal and explicitly authorized shell credential/header sources to the
shared HttpModel::from_configuration builder. Sync/async run, RPC, SDK and benchmark
use it for OpenAI-compatible and Messages requests. Cache/budget/refresh/redaction
contracts and unported registry/OAuth/retry work are in NATIVE-PROVIDER-VALUES.md.

Plugin-event continuation base `165827d`: native author/host admission, ToolRuntime,
sync/async session owners, CLI --events and RPC v1/v2 now forward bounded declarative
progress/notification/choice data with session/run/sequence/tool-call correlation.
These events never enter durable model history. Interactive choice replies, TUI
rendering and persistent UI/reconnect state remain unfinished. See
../rust-plugin-protocol.md and the common conversion card.

Raw-byte continuation base `34e1876`: shared process readers now retain actual
incoming byte counts (JSONL or Content-Length headers/body), and plugin exchange
admission uses them instead of re-encoding decoded frames. Existing typed receive/
SDK consumers keep their APIs and bounded queues. Whitespace/ignored fields count;
queue prefetch is separate from the consumed-frame quota. Boundary/cancellation/
framing and MCP/LSP/DAP/SDK runtime regressions are still pending.

Benchmark continuation base `91ee52d`: native run_tasks/CLI now have bounded
same-session verification/no-edit retries, cumulative per-attempt artifacts and
exact-match stopping after complete durable tool batches. Native view/SummaryView
reads completed rust-v1 summaries without providers. Normal session callers keep
existing behavior. Provider/attempt-timeout recovery, guided policies, interactive
views and metaharness consumer moves remain pending. See RUST-BENCHMARK.md and the
reviewed common card; runner.ts is partial/retained, no TS deletion.

Compiler, format and ownership checks only; behavioral/installed/platform evidence
is deferred. No TS source is deleted and no verified capability/removal counter
advances in these continuations. Generate counts from `cargo xtask playbook status
--json`; partial Rust code and raw language bytes are not engineering completion.
Ignored receipts/logs: target/migration/conversion-provider-values-*,
conversion-plugin-events-*, conversion-process-byte-count-* and
conversion-benchmark-retries-*. Prior source hashes/evidence remain historical.

## Next dependency-ready work

- Plugin raw-wire accounting code is connected; retain exact boundary, whitespace,
  ignored-field, queue-prefetch and framed-client checks for stabilization. Preserve
  one process and session event owner; do not infer runtime evidence from compilation.
- Plugin TUI/state/choice-response work needs explicit request/reply and reconnect
  contracts; do not invent executable callbacks or claim choices are interactive.
- Providers: full models schema/discovery/override/KDL consumers, stored credentials,
  OAuth refresh, account stickiness/usage backoff/rotation and automatic auth retry.
  Reuse current value resolver/invalidation and cancellable transport.
- Benchmark: shared AST has partial paths for 23 mutation names, paired preparation,
  exact hunks, rust-v1 scoring, native archive export/import and orchestration.
  Finish corpus/difficulty/dedup/calibration/prompts, provider/timeout retries and interactive native views
  and actual metaharness cutover. See RUST-BENCHMARK.md and the common card.
- Settings: ConfigFile<T> owns typed bounded decoding/cache/default/fallback/
  relocation and explicit JSON-to-YAML creation. Full schemas/discovery, implicit
  startup migration and conflict/quarantine/retry-aware persistent editing remain.

## Preserve pending consumers and blockers

Default product still launches TS. Never replace it with the partial candidate.
Keep shared TS until its final consumer moves. In particular JS eval workers still
serve browser/computer; their retirement stays blocked on native consumers. Native
shell completion/bootstrap generation is retired, but TS generation and owned
installer/test scripts remain pending; in-application completion is still absent.

35 inherited removals lack evidence; the source gate first rejects
crates/pi-natives/tools/bench-natives.ts. Additional recent removal evidence stays
pending. See tasks/CONCURRENT-REMOVALS.md. Legacy session/runtime failures reproduce
baseline 2ff0193; whole-addon Clippy has 14 existing diagnostics. Release binary
exceeds the unchanged 20 MiB budget; eight-platform installed/release evidence is
pending. Preserve failures and gates. Use cargo xtask test (nextest + doctests)
when stabilization starts. Ordinary build caches may be reclaimed; sources,
immutable baseline, evidence and native-engine/dependency caches are preserved.
