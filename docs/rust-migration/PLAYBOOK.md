# Harvest Rust migration playbook

Work on `ts-to-rust`. Preserve its existing history and user changes. Commit one
coherent migration checkpoint at a time; do not publish, release or merge to `main` as
part of migration work. [SCOPE.md](SCOPE.md) records the user-approved target.

## Full conversion sweep (latest user instruction)

The user again requests the whole codebase be moved first and errors/tests handled
in a consolidated later pass, with routine execution requiring no intervention.
The prior process still produced too many small foundation checkpoints. This
section supersedes their per-feature compiler/test/receipt scheduling. Preserve
SCOPE.md, security/approval/data contracts and honest unverified states.

Use three passes:

1. **Conversion sweep:** implement retained Rust subsystem behavior and connect
   its real callers/entrypoints. Work across the existing subsystem inventory,
   reusing shared engines. Group source review, implementation, consumer moves,
   eligible deletion and ledger/handoff updates by a whole workflow/domain.
   A helper or prerequisite alone is not a normal stopping milestone. Continue
   the same unfinished domain across resumptions instead of switching to another
   small foundation. Do not run cargo/compiler/Clippy/tests/addon/installed loops
   after each helper, file or prerequisite. Uncompiled conversion checkpoints
   are allowed and must say checks not run. Keep remaining connections/errors
   explicit; do not claim syntactic conversion or Rust additions are parity.
2. **Integration repair:** after the conversion sweep has connected the retained
   Rust application graph (or an explicitly recorded broad integration checkpoint
   is needed to resolve shared API uncertainty), run aggregate compiler/connection
   checks and fix groups of errors. Record why an early aggregate check is needed;
   it must not become the ordinary per-helper schedule. Keep independent conversion
   work moving when credentials/platform access block a different workflow.
3. **Verification:** consolidate behavioral, regression, installed, security,
   performance and promised-platform checks by actual user workflow, then run all
   final source/capability/release gates. Only evidence advances verified/done.

Execution granularity: the next domain is native editing as a user workflow,
including per-session edit state, read snapshots/seen-line output, explicit mode
configuration, model schemas, sync/async/RPC/SDK consumers and benchmark guidance.
The existing source/writer engines are code dependencies, not reasons to stop
again. Read the common register/card before selecting interfaces. Finish that
connected implementation domain, or record a genuine architecture/access/resource
blocker, before choosing the next broad domain from the authoritative inventory.

Keep one Markdown ownership/checklist register plus existing ledgers/graph. Update
records once per coherent batch, not a new planning document for every helper.
Keep original baselines and historical evidence. Commit/push recoverable conversion
checkpoints at domain boundaries or actual execution/resource limits, with a short
exact resume action. Cheap diff/ownership hygiene may run before commits; a source
plan or compilation is never behavioral proof. Do not weaken CI or source gates.

Remove legacy files when their last real consumer has moved, record actual removal
with verification pending, and consolidate deletion evidence later. Shared legacy
sources stay until their consumers move. Do not substitute generated stubs,
rename executable scripts as data, embed legacy implementations or introduce a
language runtime to manufacture conversion progress. Default launchers remain
until the final Rust candidate has the required installed evidence.

Routine migration edits, commits and pushes to ts-to-rust are already authorized.
No repeated continue/approval prompts. Use existing scheduled continuation for
resumption; do not spawn overlapping writers or duplicate automations. A scheduled
run is bounded and can fail; do not promise unlimited uninterrupted execution or
completion by a particular time without evidence. Stop only at verified full
acceptance, the user's stop instruction, or an actual run limit with a saved
checkpoint; surface essential unavailable access/product decisions truthfully.

## Conversion-first execution (latest user instruction)

Historical scheduling below is subordinate to the full conversion sweep above.
The user explicitly changed execution order: write and connect Rust across larger
subsystems first, then stabilize errors and run tests together. This supersedes
older per-slice verification scheduling below and in historical task cards.
Maintain [CONVERSION-PASS.md](CONVERSION-PASS.md) as the active Markdown ownership,
connection and unfinished-work register, alongside the authoritative manifests.

1. Work across connected subsystems without stopping for per-file tests, Clippy,
   legacy suites, addon rebuilds or installed verification after each edit.
2. Record exact Rust destinations, consumer connections, remaining TS consumers
   and dependencies for each feature. Reserve shared interfaces before editing;
   a feature has one owner and one implementation, not parallel replacements.
3. Commit/push coherent conversion checkpoints explicitly as **in progress and
   unverified**. A conversion checkpoint may precede behavioral verification.
   Keep machine task states `in_progress`; record code-written/connected and
   verification-pending separately in Markdown. Never inflate completed features.
4. Carry findings into a shared stabilization queue. Run an aggregate compiler
   connection check at a checkpoint if useful; defer broad tests and expensive
   builds to stabilization. Record failures; do not repeatedly rerun unchanged
   checks or pause independent conversion work for a local failure.
5. Preserve the complete target in SCOPE.md. Keep shared legacy sources until
   their consumers move. Stage final deletions after connections and verify them
   during stabilization. Do not replace implementations with generated stubs,
   rename executable scripts as data, or silently remove retained behavior.
6. Stabilization runs affected Rust integration, installation and legacy cutover
   checks in consolidated passes, then updates verified/done/removal evidence.
   Existing final acceptance gates remain mandatory before product cutover.

The user's earlier batching preference still applies: ordinary Rust engines and
native consumers first; no per-file N-API adapter. The native addon development
profile override remains available when a future shared-engine cutover needs it.

Direct implementation is active again at the user's request. The unsuccessful
OpenCode assignment is historical; no external model connection blocks execution.

## Autonomous continuation (latest user instruction)

The user requests minimal interaction and does not want to send `continue` after
each checkpoint. Existing authorization covers migration edits, ledger/card
updates, commits and pushes to `ts-to-rust`. During an active execution, keep
working across dependency-ready batches and push coherent checkpoints; do not
end a turn merely because one small batch is committed. Keep concise progress
updates while working. Choose the next named pending connection from the common
card and inspect its real consumers; mark missing-contract tasks blocked and
continue independent work. Ask only for essential input unavailable from the
checkout or connected sources. Preserve user changes and scope boundaries.

An hourly continuation automation titled "Continue Harvest Rust migration" is
now enabled for the current task. The earlier assumption that no scheduling
facility was available is superseded. Scheduled runs should resume actual coding
from the remote branch and this playbook, update ownership/evidence and push
coherent checkpoints. This is scheduled resumption, not an unlimited single turn.
Verify each run's actual commits and checks; successful schedule creation alone
is not evidence that a coding run executed. Do not create duplicate automations
or overlapping writers. At a resource/environment or execution limit, save a
concrete checkpoint and next actions for the next run. Pause the automation when
the user stops it or complete acceptance is verified. Keep service identifiers
out of this public repository; use the automation service to inspect run state.

Never manufacture completion percentages to satisfy a progress request. Report
verified retained capabilities, partial implementations and verified source
removals separately; percentages must identify their denominators.

## Start a fresh session

1. Read root `AGENTS.md`, this file, `SCOPE.md`, and `HANDOFF.md`.
2. Check `git status --short --branch` and `git log -5 --oneline`. Never reset
   someone else's changes. Fetch the recorded baseline commit if missing.
3. Run `cargo xtask migration`, `cargo xtask capabilities`, and
   `cargo xtask playbook status --json`. These describe different kinds of
   progress. File counts do not measure remaining engineering effort.
4. Run `cargo xtask playbook next --json`; read the returned task card and all
   referenced contracts before editing. Resume an `in_progress` task first.
5. Inspect the baseline contracts before changing them; behavioral execution is
   deferred during the active conversion-first phase. If a required
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
