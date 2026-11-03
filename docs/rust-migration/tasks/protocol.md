# review:protocol / migrate:protocol — Shared messages and lossless session data

This card is a reviewed **review procedure**. The migration task is not ready;
its source contracts, leaf interfaces and exact consumer checks still need review.
The machine-owned graph and source ledger hold membership and current state.

## Prerequisites and goal

Review prerequisite: `control:playbook`. Migration prerequisites: `review:protocol`
and none beyond shared control prerequisites.

Keep the existing lossless codec and display projection. Port remaining wire types without changing persisted opaque payloads, branch ordering or protocol blocks.

## Inputs and connections to inspect

- `crates/harvest-protocol`
- `packages/wire`

Inspect the unit's registered fixtures, actual import/export and worker registries,
public APIs, persisted formats, callers, generators, Cargo/Bazel graph, CI and
packaging. Search sanctioned helpers before proposing a new engine. Source ledger
rows assigned to `migrate:protocol` are the ownership checklist; include test fixtures,
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

Required contract scenarios: Malformed records; missing parents; unsupported versions; size limits; display redaction; roundtrip preservation.

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

## migrate:protocol implementation contract

**Goal:** Replace the TypeScript `packages/wire` wire-type surface with Rust-owned
types in `crates/harvest-protocol` (or a dedicated wire crate) while preserving
the collab protocol constants and JSON shapes consumed by host, guest and relay
clients.

**Prerequisites:** `review:protocol` (done).

**Inputs:** `packages/wire/src/index.ts`, `packages/wire/test/constants.test.ts`,
`crates/harvest-protocol/src/*`, collab protocol consumers in `packages/coding-agent`
and `packages/collab-web`.

**Destination:** `crates/harvest-protocol` (Rust structs/serde) plus generated or
hand-written bindings only where an external TS consumer remains during transition.

**Connections:** collab host/guest/relay, browser clients, SDK exports, tests.

**Implementation steps:**
1. Enumerate all exported wire types/constants and map each to a Rust serde type.
2. Add Rust tests for constant values and round-trip JSON.
3. Switch consumers to Rust-owned definitions or generated bindings.
4. Remove `packages/wire` source after last consumer moves and update ledger rows.

**Verification:**
- `cargo fmt -p harvest-protocol -- --check`
- `cargo clippy --locked -p harvest-protocol --all-targets --all-features -- -D warnings`
- `cargo xtask test -p harvest-protocol`
- `cargo xtask playbook check`

**Recovery:** Revert the leaf commit; TS wire package remains the source of truth
until migration completes.

**Completion evidence:** Rust tests, consumer switch-over, deleted TS source, updated
ledger rows.
