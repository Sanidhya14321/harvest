# review:catalog-providers / migrate:catalog-providers — KDL catalog, credentials and provider families

This card is a reviewed **review procedure**. The migration task is not ready;
its source contracts, leaf interfaces and exact consumer checks still need review.
The machine-owned graph and source ledger hold membership and current state.

## Prerequisites and goal

Review prerequisite: `control:playbook`. Migration prerequisites: `review:catalog-providers`
and `migrate:protocol`.

Port configuration and KDL compilation before broad provider calls. Split a leaf card per provider family and credential lifecycle. Reuse the existing cancellable transport.

## Inputs and connections to inspect

- `packages/catalog`
- `packages/ai`
- `rust-agent/src/provider.rs`
- `rust-agent/src/transport.rs`

Inspect the unit's registered fixtures, actual import/export and worker registries,
public APIs, persisted formats, callers, generators, Cargo/Bazel graph, CI and
packaging. Search sanctioned helpers before proposing a new engine. Source ledger
rows assigned to `migrate:catalog-providers` are the ownership checklist; include test fixtures,
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

Required contract scenarios: Streaming tool blocks; usage unknown versus zero; retries; OAuth refresh; status/error redaction; cancellation before headers and during body.

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

## migrate:catalog-providers implementation contract

**Goal:** Port catalog generation and KDL policy resolution and provider behavior to Rust while preserving KDL as policy source.

**Prerequisites:** `review:catalog-providers` (done).

**Inputs:** `packages/catalog`, `packages/ai`, `rust-agent` provider/transport.

**Destination:** Rust catalog/provider crates and engines.

**Implementation steps:**
1. Inventory provider contracts and fixtures.
2. Port catalog generation and KDL resolution.
3. Port providers individually with streaming/tool-call/retry/cancellation.
4. Switch consumers and remove TS sources.

**Verification:**
- affected crate fmt/clippy/tests
- `cargo xtask playbook check`

**Recovery:** Revert leaf commit; TS catalog remains authoritative.

**Completion evidence:** Rust tests, consumer switch-over, deleted TS sources, updated ledger rows.
