# Harvest Rust migration playbook

Work on `ts-to-rust`. Preserve its existing history and user changes. Commit one
verified behavior slice at a time; do not publish, release or merge to `main` as
part of migration work. [SCOPE.md](SCOPE.md) records the user-approved target.

## Execution batches for throughput

Use the existing dependency graph to group connected behavior into bounded
subsystem batches. Prefer a complete native Rust consumer path using existing
engines; avoid introducing a new N-API adapter for each source. Keep the complete
retained behavior target and existing completion gates. These rules change work
scheduling, not acceptance or approved retirements.

1. Select dependency-ready tasks sharing a Rust crate or runtime boundary. Capture
   their legacy contracts together and define the complete batch deletion set.
2. Implement against ordinary Rust engines and the native CLI/session/SDK first.
   Add a transitional binding only when the batch's identified legacy consumer
   actually needs one. Preserve unmoved consumers and their sources.
3. Use focused checks while editing. The existing `OMP_NATIVE_CARGO_PROFILE=dev`
   override is available for a warm development cache; reserve the candidate
   optimized build for batch acceptance and never call a dev build release or
   performance evidence. Deduplicate the same Cargo/package checks
   across the batch. Rebuild the addon once at the end only if its consumed Rust
   engine or binding changed; xtask-only changes do not rebuild it.
4. Finalize tracked contracts, ledgers and handoff before the final verification
   pass. Run that pass once against the frozen batch; rerun only checks affected
   by a correction. Keep receipts outside the source fingerprint.
5. Record unresolved broad-suite failures without repeatedly rerunning an
   unchanged reproduction. Run full integration at consumer cutovers and final
   acceptance, and whenever a changed shared boundary justifies it. Do not label
   focused checks as a full transition or skip the verification plan's required
   broad checks when claiming that transition passed.
6. Commit and push each coherent verified batch. Never delete shared files until
   their last consumer moves, manufacture parity from generated stubs, or replace
   the promised product with a smaller one to meet a deadline.

Direct implementation is active again at the user's request. The unsuccessful
OpenCode assignment is historical; no external model connection blocks execution.

## Start a fresh session

1. Read root `AGENTS.md`, this file, `SCOPE.md`, and `HANDOFF.md`.
2. Check `git status --short --branch` and `git log -5 --oneline`. Never reset
   someone else's changes. Fetch the recorded baseline commit if missing.
3. Run `cargo xtask migration`, `cargo xtask capabilities`, and
   `cargo xtask playbook status --json`. These describe different kinds of
   progress. File counts do not measure remaining engineering effort.
4. Run `cargo xtask playbook next --json`; read the returned task card and all
   referenced contracts before editing. Resume an `in_progress` task first.
5. Verify the relevant baseline behavior before changing it. If a required
   dependency is missing, record an actionable blocker and work on another ready
   task. Never claim a missing environment, credential or platform check passed.

## Authoritative records

- `source-baseline.json` is immutable, including its original language scope.
- `source-ledger.json` retains every historical row after removal; `task_id`
  identifies the task responsible for that source. New scripts get new rows.
- `capabilities.json` owns retained/retired capability status, the execution
  dependency graph, task states, checks and release acceptance registrations.
- `tasks/` contains the implementation and review instructions. Do not create
  another task list or progress database.
- `HANDOFF.md` is a short live checkpoint; old narratives stay under `archive/`.
- Receipts live under ignored `target/migration/`; CI must preserve receipts and
  logs as artifacts keyed to the tested commit. Machine-local caches are optional.

Use `cargo xtask playbook status --json` for generated counts. Never copy changing
counters into multiple prose documents. Historical receipts remain historical;
new Rust-only acceptance cannot be inferred from an old browser/WASM test.

## Task states and review

States: `planned`, `ready`, `in_progress`, `blocked`, `done`. Ready, active and done
tasks require reviewed instructions and completed prerequisites. A blocked task
must name the failed command or missing requirement and the next corrective action.
Done requires evidence; a done migration task cannot own unfinished source rows.

The initial subsystem review cards are reviewed *review procedures*, not claims
that thousands of source contracts have already been understood. Their associated
migration tasks remain planned with `review: pending`. Complete a review by
creating bounded implementation cards, splitting the existing graph as needed,
and transferring each affected ledger row to its concrete migration task. Keep
the parent planned until every owned source and capability is accounted for.

Every implementation card must name: observable goal; prerequisites; source and
capability IDs; exact inputs, consumers and persisted formats; Rust destination
and interfaces; ordered edits; success/error/cancellation contracts; switch-over
steps; deletion conditions; exact verification commands and expected behavior;
recovery; and required evidence. Do not make a card ready with unresolved product
or architectural choices. New discoveries become explicit prerequisite tasks.

Reviewed retained inert inputs are reported as `verified_inert`, separately from
`verified_removed`. The exact source-audit allowlist, full contract/Rust consumer
review and both behavioral/installed evidence are required. Labeling an arbitrary
executable fixture as data cannot complete a task. This exception does not migrate
any current consumer or turn a retained source into a Rust replacement.

## Implement one behavior slice

1. Review the legacy contract and every connection: imports, native bindings,
   workers, package exports, generators, launchers, installers and persisted data.
2. Capture baseline fixtures from the recorded code/artifact. Keep meaningful
   inputs and failure cases; do not merely copy implementation details into tests.
3. Reuse or extract the ordinary Rust engine. Use existing jailed I/O, approval,
   locks, cancellation, bounded streams and protocol primitives.
4. Connect the Rust behavior. A temporary TS/N-API adapter is allowed only while
   an identified remaining consumer needs it. Track its final removal.
5. Verify the user-visible behavior and installed Rust candidate. Rebuild the
   addon and run affected legacy suites when changing shared engines.
6. Move the last consumers, delete superseded sources, port meaningful tests,
   and remove unused dependencies and build connections. Shared sources stay
   until their last consumer moves. Retirements need removal-contract evidence.
7. Update source/capability/task evidence and the short handoff. Stage intended
   deletions: the deletion gate examines the Git index.
8. Run final checks against the finalized tree, then commit without further
   tracked edits. If checks fail, correct the cause and rerun affected checks.

Use `cargo xtask test -p PACKAGE` for nextest plus doctests, never direct
`cargo test`. Use strict Clippy and formatting for changed crates. During the
transition use the explicit Bun suite/file commands from `AGENTS.md`, never bare
root `bun test`. Shared/security/build changes deliberately widen verification.

`cargo xtask source-audit verify --scope affected --base START_SHA` records a
source-bound transition receipt. It cannot certify complete capability parity.
Receipts must not modify the source they certify. Record test/suite references
before final verification; locate the final commit through Git/CI rather than
trying to write a commit's own hash into its contents.

## Installation, retirement and release

Capability schema v4 separates behavioral verification, candidate installation,
legacy removal and later public promotion. Candidate tests precede default
cutover. Retirement never counts as parity for a retained capability.

`cargo xtask acceptance GATE` rejects missing installed evidence and missing
behavioral check registrations. Empty gate check lists are explicit pending work,
not successful checks. Register real installed workflows as they are implemented;
never use `cargo --version`, a manifest counter, or a placeholder as product proof.

Final completion requires the source deletion gate, retained capability gates,
verified approved retirements, strict non-Rust source audit, all eight supported
installed artifacts, and complete release acceptance. Preserve data recovery,
security, protocol and platform contracts. Report measured performance without
inventing speedup claims. Building a Rust binary is not product completion.

## Continuing with a lighter model

Select ready work, implement and connect it, verify, delete only eligible legacy
sources, update evidence, commit, and continue. Do not repeatedly ask permission
for authorized migration commits. Do not weaken contracts to make a task pass.
If a task repeatedly fails, capture the reproduction and blocker, then continue
independent ready work. Never conceal unfinished behavior with a compiling stub.
