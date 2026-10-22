# review:memory / migrate:memory — Retained memory backends and automatic learning

This card is a reviewed **review procedure**. The migration task is not ready;
its source contracts, leaf interfaces and exact consumer checks still need review.
The machine-owned graph and source ledger hold membership and current state.

## Prerequisites and goal

Review prerequisite: `control:playbook`. Migration prerequisites: `review:memory`
and `migrate:storage`.

Preserve the distinction between Markdown brain and SQLite-backed memory; keep model/vector identities and migrations explicit.

## Inputs and connections to inspect

- `crates/harvest-brain`
- `packages/mnemopi`
- `packages/harvest-memory`
- `packages/harvest-schema`
- `packages/coding-agent/src/memory-backend`

Inspect the unit's registered fixtures, actual import/export and worker registries,
public APIs, persisted formats, callers, generators, Cargo/Bazel graph, CI and
packaging. Search sanctioned helpers before proposing a new engine. Source ledger
rows assigned to `migrate:memory` are the ownership checklist; include test fixtures,
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

Required contract scenarios: Database upgrades; interrupted learning; scope isolation; vector identity; cancellation; recovery; query ordering.

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

## migrate:memory implementation contract

**Goal:** Port the memory subsystem to Rust according to the reviewed scope and existing engines.

**Prerequisites:** `review:memory` (done).

**Inputs:** Source paths and fixtures registered for the memory work unit.

**Destination:** Existing or new Rust crates as required by consumers.

**Implementation steps:**
1. Inventory contracts and consumers.
2. Port behavior in dependency-ready groups.
3. Switch consumers and remove superseded sources.
4. Update ledger rows and evidence.

**Verification:**
- affected crate fmt/clippy/tests
- `cargo xtask playbook check`

**Recovery:** Revert leaf commit; TS implementation remains authoritative until cutover.

**Completion evidence:** Rust tests, consumer switch-over, deleted sources, updated ledger rows.
