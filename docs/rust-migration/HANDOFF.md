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

Native Anthropic Messages text/tool support now connects the Rust SDK and
sync/async CLI plus RPC through `HttpModel::anthropic_messages`. Shared SSE
framing and cancellable transport retain bounds; origin checks prevent API-key
redirect leaks. Complete tool-result exchanges and lossless split usage flow
through existing session persistence. Explicit model/host flags avoid inventing
catalog policy. See `tasks/ANTHROPIC-TEXT-TOOLS.md` and source-bound
`target/migration/anthropic-*` results. Whole streaming-provider capability remains
partial/unverified: thinking, multimodal, OAuth/catalog policy, retries and live
credentialed checks remain pending. Default application remains TS; no legacy
provider source is removable at this stage. The package integration pass passed
136 checks; the subsequent affected-provider run passed six checks, including
the added cross-origin redirect probe (137 distinct native checks covered).
Strict package Clippy, formatting and playbook checks pass. The generated
verification plan still requests full verification; that transition was not
executed or certified for this native-only batch. The inherited removal gate
remains failed as recorded below.

Previous `application:python-eval-retirement` removed 25 approved kernel/prelude/
private-test sources. Its 125 Rust contracts and all 49 legacy workspace groups
passed. The runtime/session integration run failed 19/34 groups; all 40 remaining
contracts reproduced on unchanged `2ff0193`. Preserve the failed receipt rather
than rerunning unchanged legacy failures for native-only application additions.
See `tasks/PYTHON-EVAL-RETIREMENT.md`, `tasks/LEGACY-SESSION-CONTRACTS.md` and
`target/migration/python-retirement-*`.

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
