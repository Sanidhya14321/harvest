# Current migration handoff

Work on **`ts-to-rust`**. Find the actual checkpoint with `git log -1` and preserve
user/other-agent changes. The original baseline is `d811cb2`; never modify the
immutable source baseline. The imported other-agent history through `6eefffc`
is preserved. Historical continuation narratives are in
`archive/HANDOFF-2026-10-23-before-python-retirement.md`.

## Continuing rule

Read `PLAYBOOK.md`, `SCOPE.md` and the authoritative manifests. The user requests
complete subsystem/consumer batches, focused checks while editing and one final
integration pass per batch. Avoid isolated file/query ports and per-file addon
rebuilds. Record this preference across handoffs. Direct implementation is active;
the cancelled OpenCode approach is not a blocker.

## Current outcome

`application:python-eval-retirement` removes 25 reviewed kernel, runner, injected
Python prelude and private execution-test files under the approved retirement
policy. Python execution, kernel tools, setup and direct session calls reject
without interpreter probes, hooks, subprocesses or JS fallback. Settings/PI_PY
cannot restore a kernel; old runtime controls are hidden. Historical transcript
and display records remain readable, and ordinary shell behavior is retained.

Two copied-native/SDK retirement checks, 125 Rust contracts, all 49 legacy
workspace chunks, 74 focused legacy contracts and 38 JS/schema connection
contracts pass. Updated shortcut-hook/schema tests also pass. Strict Rust Clippy,
affected TS lint/format and typechecks are required before committing. Final
source-bound logs/receipts are under `target/migration/python-retirement-*`.
The runtime/session integration run failed 19 of 34 groups; obsolete Python-hook
expectations are corrected with passing targeted checks. All 40 remaining failing contracts reproduce on unchanged `2ff0193` using
the same failing chunk commands. The raw run remains failed; do not call this a
passing transition. Baseline resources were rebuilt from its sources, using the
unchanged verified addon. See the current runtime section in
`tasks/LEGACY-SESSION-CONTRACTS.md`.
See `tasks/PYTHON-EVAL-RETIREMENT.md` and `docs/python-repl.md`.

JavaScript evaluation still serves retained browser/computer workers. Move those
consumers to native CDP/desktop APIs before removing the shared JS engine.
`tool:eval` retirement remains pending, and TS rejection/type facades remain
pending application cutover. Do not confuse an approved retirement with behavior
ported to Rust.

Previous completed batches include DOCX shared conversion, native CDP/extension
relay retirement, native terminal clock repair, Rust tooling ports and RPC v2
transport/client/transcript support. Their cards and archived handoff hold exact
checks and limitations; whole application/provider/UI parity remains incomplete.

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
verification receipts belong under ignored `target/migration/`. Push verified
migration batches to `ts-to-rust`; publishing/merging remains separate.
