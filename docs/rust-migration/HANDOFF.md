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
configuration/profiles, rust-v1 edit scoring and native MCP are written and connected across
native APIs/CLI/sync/async/RPC. The example plugin uses host-jailed reads. Startup
is explicitly host-enabled. The redacted config inspector starts no plugins.

MCP now uses a shared bounded process owner with plugins, and the existing
cancellable HTTP/SSE owner for streamable HTTP. Client initialization, bounded
pagination, tool/resource/prompt APIs, host approval, native config and CLI are
connected. See `NATIVE-MCP.md` for unfinished OAuth/discovery/legacy SSE,
notifications/reconnect and platform process cleanup. Legacy MCP sources remain.

An aggregate all-target/all-feature compiler connection check passes, recorded in
`target/migration/conversion-mcp-connections*.log`. Behavioral
tests, Clippy, installation and legacy cutover checks are **deferred** at the
user's instruction; no new capability is verified/done or legacy-removed. The
common card identifies remaining plugin lifecycle/UI/marketplace, full legacy
settings/schema and benchmark generation/orchestration contracts. Keep converting
independent remaining areas, then run consolidated stabilization passes.

Default application and consumers still use TS. Do not delete their sources or
replace the product with the experimental native subset. Preserve final Rust-only
acceptance gates. Whole-source conversion remains unfinished; the common card
contains the generated full feature snapshot. Previous validated Anthropic and
Python-retirement checkpoints are in the archived handoff and their cards.

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
