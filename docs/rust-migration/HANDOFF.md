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

Compiler, format and ownership checks only; behavioral/installed/platform evidence
is deferred. No TS source is deleted and no verified capability/removal counter
advances in these continuations. Generate counts from `cargo xtask playbook status
--json`; partial Rust code and raw language bytes are not engineering completion.
Ignored receipts/logs: target/migration/conversion-provider-values-* and
conversion-plugin-events-*. Prior source hashes/evidence remain historical.

## Next dependency-ready work

- Plugin raw-wire accounting: existing exchange budget re-encodes decoded frames;
  inspect shared process_transport reader/Process receive ownership and count actual
  incoming bytes, including whitespace/ignored fields. Current event-body bounds
  do not certify a raw-wire quota. Preserve one process and session event owner.
- Plugin TUI/state/choice-response work needs explicit request/reply and reconnect
  contracts; do not invent executable callbacks or claim choices are interactive.
- Providers: full models schema/discovery/override/KDL consumers, stored credentials,
  OAuth refresh, account stickiness/usage backoff/rotation and automatic auth retry.
  Reuse current value resolver/invalidation and cancellable transport.
- Benchmark: shared AST has partial paths for 23 mutation names, paired preparation,
  exact hunks, rust-v1 scoring, native archive export/import and orchestration.
  Finish corpus/difficulty/dedup/calibration/prompts, retries/early-stop/native views
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
