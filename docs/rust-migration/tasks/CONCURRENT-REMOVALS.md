# control:concurrent-removals — reconcile inherited deletion claims

Prerequisite: `control:playbook`. Imported checkpoint: `6eefffc` (28 commits after
`043709e`). Preserve that history and the common MIGRATION_CHECKLIST.md.

The inherited ledger used invalid `implementation: verified` values and marked
removed sources as verified despite missing contract/consumer/installed evidence.
The source-removal gate correctly rejects these claims. These rows are now pending
review, retaining their actual removed state; no prior deletion is silently undone.
This is a blocker, not a verified retirement or a Rust replacement.

The protocol engine remains in progress: ported Rust types are useful, but
`packages/wire` still serves live TypeScript/browser consumers. Moving its rows to
another task does not complete protocol switch-over.

## Inputs and exact next actions

For each source below, inspect its original bytes with
`git show 043709e:PATH`, its deletion commit and all remaining runtime/build/docs
consumers. Start with `crates/pi-natives/tools/bench-natives.ts`. Create a bounded
implementation/retirement card before expanding the patch. Determine its actual
input/output/failure contract, current owner, Rust destination and executable
verification. Port retained behavior or document an explicitly approved removal;
restore a required source from Git if its working replacement is absent. Do not
reinterpret 'no import references' as proof that a standalone CLI was unused.
Only advance individual rows after behavior and installed-consumer evidence exists.
Do not change the immutable baseline or weaken `migration` to accept missing proof.

- `crates/pi-natives/tools/bench-natives.ts`
- `crates/pi-natives/tools/check-glm-rankmerge.py`
- `crates/pi-natives/tools/gen-kimi-fixtures.py`
- `crates/pi-natives/tools/gen-openai-fixtures.py`
- `crates/pi-natives/tools/pack-deepseek.ts`
- `crates/pi-natives/tools/pack-glm.ts`
- `crates/pi-natives/tools/pack-kimi.ts`
- `crates/pi-natives/tools/pack-openai.ts`
- `scratch/benchmark_logging_event_loop.ts`
- `scratch/compare_criteria_formats.ts`
- `scratch/isolated_step1_benchmark.py`
- `scratch/live_http_benchmark.py`
- `scratch/test_live_scout.py`
- `scratch/threshold_sweep.py`
- `scripts/analyze_small_edits.py`
- `scripts/bash_corpus_stats.jl`
- `scripts/ci-update-brew-formula.test.ts`
- `scripts/ci-update-brew-formula.ts`
- `scripts/eval-bench-runs.ts`
- `scripts/fix-test-imports.ts`
- `scripts/gen-nix-bun.test.ts`
- `scripts/gen-nix-bun.ts`
- `scripts/merge-pr.test.ts`
- `scripts/merge-pr.ts`
- `scripts/rate-edit-tool.py`
- `scripts/rewrite-system-prompt.ts`
- `scripts/session-stats/analyze_search_relevance.py`
- `scripts/session-stats/harmony_backtest.py`
- `scripts/session-stats/optimize_read_config.py`
- `scripts/session-stats/plot_read_summarizer.py`
- `scripts/session-stats/plot_tools.py`
- `scripts/session-stats/read_optimizer.py`
- `scripts/setup-npm-trust.ts`
- `scripts/test-loader.mjs`
- `scripts/tool-prompt-usage.ts`
- `scripts/tool_io.py`
- `scripts/trace-loader.ts`

## Verification and completion

Run `cargo xtask migration` (currently fails: source removed without verified
replacement or retirement), `cargo xtask playbook check`, and the new bounded
card's package/consumer/installed checks. Keep exact failed commands/logs under
`target/migration/`. Completion requires every listed row to pass its own removal
contract and the shared migration gate; compiling new Rust types does not suffice.
Recovery restores the individual source and consumer together from Git, preserving
user data and unrelated migration work. The new Rust host-detection replacement
has its own direct builder verification and is handled separately from this list.

Delegated bounded prerequisite: the two Nix generation rows are now owned by
`tooling:gen-nix` / GEN-NIX.md for OpenCode implementation and supervised review.
They remain unverified until that task's behavioral and installed checks pass.
