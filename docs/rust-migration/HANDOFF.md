# Current migration handoff

Work on **`ts-to-rust`**. Find the actual checkpoint with `git log -1` and preserve
user/other-agent changes. The original baseline is `d811cb2`; never modify the
immutable source baseline. The imported other-agent history through `6eefffc`
is preserved. Historical continuation narratives are in
`archive/HANDOFF-2026-10-23-before-python-retirement.md`.

## Continuing rule

**Conversion first, stabilization/tests later** is the latest explicit user
instruction. Read `PLAYBOOK.md` and the active shared task/card
`CONVERSION-PASS.md` first. It reserves interfaces and lists all features,
written/connected code, unfinished contracts and deferred verification. Avoid
per-file tests, repeated legacy suites and addon rebuilds. Commit coherent
in-progress conversion checkpoints without claiming verified feature completion.

## Active checkpoint

`application:conversion-first` is in_progress and has scheduling priority. Native
owned tool schemas, executable plugin protocol/host/Rust SDK, layered TOML
configuration/profiles, rust-v1 edit scoring, native MCP and language/debug services are written and connected across
native APIs/CLI/sync/async/RPC. The example plugin uses host-jailed reads. Startup
is explicitly host-enabled. The redacted config inspector starts no plugins.

MCP now uses a shared bounded process owner with plugins, and the existing
cancellable HTTP/SSE owner for streamable HTTP. Client initialization, bounded
pagination, tool/resource/prompt APIs, host approval, native config and CLI are
connected. See `NATIVE-MCP.md` for unfinished OAuth/discovery/legacy SSE,
notifications/reconnect and platform process cleanup. Legacy MCP sources remain.

The previous language-services continuation added bounded Content-Length framing, LSP initialization/
queries/document sync/UTF-16 text edits and DAP adapter/session state. Both share
the existing process/tool/config/approval owners. Native run/RPC host flags are
--enable-lsp/--enable-debug; direct SDK and terminal CLI connections are present.
See NATIVE-LSP-DAP.md for incomplete discovery/diagnostics/resource operations/
recovery/socket/reverse-request/platform behavior. Legacy consumers remain.

Compiler connection results are under
`target/migration/conversion-lsp-dap-connections*.log`. Behavioral tests, Clippy,
installation, performance and legacy cutover checks remain **deferred**. No new
capability is verified/done or legacy-removed. Continue broad code conversion via
the shared task/card, then consolidate stabilization. Source inventories and the
generated feature snapshot retain honest pending ownership.

Default application and consumers still use TS. Do not delete their sources or
replace the product with the experimental native subset. Preserve final Rust-only
acceptance gates. Whole-source conversion remains unfinished; the common card
contains the generated full feature snapshot. Previous validated Anthropic and
Python-retirement checkpoints are in the archived handoff and their cards.

## Current continuation

Mutation-producer continuation base `e29f5cd`: shared pi-ast now has code paths
for all 23 named mutation families. Regex changes use the existing Regex grammar;
composites combine non-overlapping edits from distinct families; identifier edits
use a conservative JavaScript lexical binding resolver. Full binding/grammar
eligibility remains partial. Native generation now reports bounded rejection
examples. SDK/CLI `pack` exports validated inert fixtures through the same portable
archive/path rules consumed by native unpack/list/validate/run. See
RUST-BENCHMARK.md and the common card for supported forms, limits and policies.

No TS source is deleted: mutations.ts/generate.ts still serve the legacy generator.
Next: complete reviewed binding/grammar eligibility, corpus/difficulty/dedup/
calibration and specialized prompts, then actual generator/metaharness cutover.
Behavioral/installed/platform checks remain deferred; compiler/format/ownership
checks do not establish parity. Verified capability/removal counters do not advance.

The user explicitly requests minimal interaction: continue across ready batches
within active execution, push coherent checkpoints and do not ask for another
`continue` after a commit. The autonomy rule is persisted in PLAYBOOK.md. An enabled hourly automation
("Continue Harvest Rust migration") now schedules coding resumptions for this
task. This supersedes the earlier statement that no scheduling option existed.
Creation is confirmed; actual automated coding/run results must be verified from
the service and remote commits. Do not create duplicate writers. Read the current
remote tip and save checkpoints for later resumptions without asking the user to
type continue. Pause scheduling on user cancellation or verified completion.
Progress percentages must distinguish verified capability parity, partial code
and verified source removal; never describe partial-code ratios as completion.

## Open blockers

- Normal application launch is still TypeScript. The Rust candidate is opt-in.
- 35 inherited deleted sources lack verified replacements/retirements. The source
  gate first rejects `crates/pi-natives/tools/bench-natives.ts`. See
  `tasks/CONCURRENT-REMOVALS.md`; preserve failed evidence and never weaken gates.
- Legacy session/runtime failures need bounded contract review. Existing singleton
  failures are in `tasks/LEGACY-SESSION-CONTRACTS.md`; current runtime results stay
  failed even when the retirement-specific checks pass.
- Whole-addon strict Clippy has 14 existing diagnostics; do not suppress them.
  Native addon tests/terminal failures were repaired separately.
- Platform CI, actual installed parity and release checks remain pending/failed.
  `tasks/PORTABILITY.md` and `tasks/ARTIFACT-SIZE.md` record known blockers; the
  measured release binary exceeds the unchanged 20 MiB budget.

## Resume and evidence

```sh
cargo xtask playbook status --json
cargo xtask playbook next --json
cargo xtask migration
cargo xtask capabilities
```

Use generated counts, not manual percentages. Native RPC transport is implemented
but its production commands/consumers and v1 display behavior remain partial.
Reuse existing Rust engines; preserve approval, cancellation, persistence and
shutdown contracts. Keep shared sources until their last consumer moves.

Use `cargo xtask test` (nextest plus doctests), not direct `cargo test`. Frozen
verification receipts belong under ignored `target/migration/`. Push coherent
in-progress conversion checkpoints to `ts-to-rust`; only stabilization evidence
can establish verified migration. Publishing/merging remains separate.
