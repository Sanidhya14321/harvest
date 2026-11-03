# review:primitives / migrate:primitives — Directly reusable native Rust engines

This card is a reviewed **review procedure**. The migration task is not ready;
its source contracts, leaf interfaces and exact consumer checks still need review.
The machine-owned graph and source ledger hold membership and current state.

## Prerequisites and goal

Review prerequisite: `control:playbook`. Migration prerequisites: `review:primitives`
and none beyond shared control prerequisites.

Extract ordinary engines from host bindings. Preserve sanctioned shell/edit/walker/VCS/AST/text/media behavior; keep only necessary transitional adapters.

## Inputs and connections to inspect

- `crates/harvest-terminal`
- `crates/harvest-terminal-wasm`
- `crates/pi-natives`
- `crates/pi-ast`
- `crates/pi-diff`
- `crates/pi-edit`
- `crates/pi-iso`
- `crates/pi-vcs`
- `crates/pi-walker`
- `crates/pi-shell`
- `crates/pi-builtins`
- `crates/vendor`
- `packages/natives`
- `crates/harvest-tui`
- `crates/harvest-tokenizer`
- `crates/harvest-media`

Inspect the unit's registered fixtures, actual import/export and worker registries,
public APIs, persisted formats, callers, generators, Cargo/Bazel graph, CI and
packaging. Search sanctioned helpers before proposing a new engine. Source ledger
rows assigned to `migrate:primitives` are the ownership checklist; include test fixtures,
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

Required contract scenarios: Unicode and malformed input; bounds; cancellation; stream chunking; native/addon equivalence; installed direct Rust calls.

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

## migrate:primitives implementation contract

**Goal:** Keep ordinary Rust engines as the source of truth for terminal/text/edit/AST/VCS/walker/shell behavior and remove transitional adapters only after consumers move.

**Prerequisites:** `review:primitives` (done).

**Inputs:** Native engine crates listed above, `packages/natives`, and their tests.

**Destination:** Existing Rust crates; no new engine without a named consumer.

**Implementation steps:**
1. Inventory exported native functions and Rust APIs.
2. Identify remaining TS/N-API adapters and their consumers.
3. Switch consumers to Rust APIs or generated bindings.
4. Delete adapters and unused dependencies after last consumer moves.

**Verification:**
- `cargo fmt --check` for affected crates
- `cargo clippy --locked -p <crate> --all-targets --all-features -- -D warnings`
- `cargo xtask test -p <crate>`
- `cargo xtask playbook check`

**Recovery:** Revert the leaf commit; adapters remain until migration completes.

**Completion evidence:** Rust tests, consumer switch-over, deleted adapters, updated ledger rows.
