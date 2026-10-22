# review:terminal / migrate:terminal — Complete native interaction and dashboard replacements

This card is a reviewed **review procedure**. The migration task is not ready;
its source contracts, leaf interfaces and exact consumer checks still need review.
The machine-owned graph and source ledger hold membership and current state.

## Prerequisites and goal

Review prerequisite: `control:playbook`. Migration prerequisites: `review:terminal`
and `migrate:protocol`, `migrate:primitives`, `migrate:application`.

Extend existing harvest-tui/harvest-terminal; no new GUI toolkit. Native modes replace stats, replay, collaboration guest, benchmark and automation dashboards. Retain their user workflows and shared protocols.

## Inputs and connections to inspect

- `packages/tui`
- `packages/coding-agent/src/modes`
- `packages/coding-agent/src/tui`

Inspect the unit's registered fixtures, actual import/export and worker registries,
public APIs, persisted formats, callers, generators, Cargo/Bazel graph, CI and
packaging. Search sanctioned helpers before proposing a new engine. Source ledger
rows assigned to `migrate:terminal` are the ownership checklist; include test fixtures,
generated code, declarations, examples and embedded code.

## Review steps and required output

1. List each externally observable contract and all consumers. Distinguish current
   behavior, an explicitly approved retirement and a required Rust replacement.
2. Capture bounded success/failure fixtures and the exact baseline revision.
   Record credentials/platform requirements without claiming unavailable checks.
3. Split the unit into bounded implementation tasks in the same execution graph.
   Each new card specifies Rust modules/APIs, exact edits, consumer switch-over,
   error/cancellation/persistence behavior, checks, deletion rules and recovery.
4. Add missing prerequisite edges; reject cycles. Transfer each affected ledger
   row's `task_id` to its concrete leaf task. Preserve all historical rows.
5. Mark leaf instructions reviewed and make only dependency-ready tasks ready.
   Keep the parent planned until all its contracts have concrete owners.

Required contract scenarios: Inline scrollback; composer; Unicode; keyboard/mouse; images; resize; slow output; process interruption; terminal restoration; native client workflows.

## Review verification and implementation acceptance

Run `cargo xtask playbook check`; expect source/capability/task coverage and a
valid acyclic graph. Run `cargo xtask playbook next --json` to inspect selection.
A completed review proves concrete reviewed task cards and complete ownership;
it does not prove Rust implementation or legacy removal.

Every leaf must replace this review procedure with exact Cargo/Bun/installed
commands and expected outcomes. Preserve shared sources until the last consumer
moves. Complete migration only after behavioral and installed verification,
eligible deletions, dependency cleanup and source/capability gate updates.
Recovery is a scoped revert of the leaf change, preserving existing data and
unrelated work. Evidence belongs to the task and CI artifacts, not private caches.
