# tooling:gen-nix — verify and complete the inherited Rust generator

Execution owner: direct implementation by the primary agent, per the latest
user instruction. The OpenCode attempt below is historical and cancelled. Prerequisites: `control:playbook` and
`tooling:host-detect`. Source IDs: `scripts/gen-nix-bun.ts` and
`scripts/gen-nix-bun.test.ts`. Both were deleted at `6eefffc` without sufficient
verification. This bounded task removes them from the inherited-removal backlog
only after the replacement is verified. No complete installation capability is
claimed.

## Inputs and contract

Read AGENTS.md, MIGRATION_CHECKLIST.md, PLAYBOOK.md, SCOPE.md, HANDOFF.md and
CONCURRENT-REMOVALS.md. Read `xtask/src/gen_nix.rs`, `xtask/src/main.rs`,
`scripts/release.ts`, root `package.json`, `flake.lock`, `bun.lock`, `nix/bun.nix`
and the original source/tests using `git show 043709e:PATH`.

Preserve generator selection: executable `bun2nix` on PATH, otherwise executable
`nix develop --command bun2nix`, otherwise the pinned `bun2nix@2.1.2` portable
fallback. Preserve each argument, working directory, failure propagation, lockfile
version handling and canonical output. Version 1 is byte-preserved; version 2
changes only its version stamp to 1; missing stamps and versions above 2 fail
before invocation. Generated Nix ends with exactly one LF, preserving other bytes.
The original normalizes bun.lock before launching the selected generator; capture
that failure behavior explicitly rather than guessing a new transaction contract.
A failed selected generator must not silently fall through to another generator.

Review concrete inherited gaps: executable lookup currently tests only `is_file`,
not executable permissions; the stamp regex dropped the legacy line anchor. Use
observable failing cases before deciding whether either changes the used contract.
Do not rely on incidental matches inside another JSON string. Keep the existing
native CLI and release/package connections to `cargo xtask gen-nix` working.
No unbounded output capture, global environment mutation in tests or language
runtime fallback hidden inside the Rust implementation. The portable generator
fallback remains explicitly transitional until Bun workspace retirement.

## Destination, edits and connections

Ordinary orchestration remains in `xtask/src/gen_nix.rs`; make bounded internal
seams for executable discovery/process invocation if needed. Prefer stdlib and
existing dependencies. Cover root `package.json` gen:nix and release invocation;
retain external generator semantics. Add meaningful Rust tests for the original
normalization and precedence/failure cases. Do not port static echo tests or
merely assert command names exist.

Exercise the compiled xtask from a clean temporary repository-like directory:
feed bun.lock, run an external generator, check invocation/exit and actual output.
A small Rust-only generator fixture compiled by rustc is acceptable; put fixture
source under xtask/tests/fixtures and keep it clearly identified as test support.
Its filename may stand in for bun2nix/nix in a temporary PATH. Never add shell,
JS or Python fixture programs or embed those languages in Rust strings. Such a
fixture proves native orchestration, not correctness of third-party bun2nix.
Per-child env/PATH changes are allowed; process-global env writes are not.

Use an isolated worktree based on the supervising checkpoint. Limit changes to
this engine, its necessary CLI connection, Rust tests/fixtures, this card, the
two source ledger rows, task state and concise handoff/checklist pointers. Do not
change media, terminal, providers, source baseline or unrelated deletion claims.
Existing legacy sources stay deleted after their replacement checks pass. If a
missing dependency appears, record a bounded prerequisite instead of expanding
into the whole release system. Do not commit, push, publish or send messages;
return the diff and exact verification results to the supervisor for review.

## Verification and evidence

```text
cargo fmt -p xtask -- --check
cargo clippy --locked -p xtask --all-targets --all-features -- -D warnings
cargo xtask test -p xtask
cargo xtask playbook check
cargo xtask migration
cargo xtask source-audit verify --scope affected --base 3e730cb --plan
```

Use the existing compiled wrapper if rebuilding the test orchestrator is
unnecessary; never direct cargo test. Provide the Rust integration test command
for the copied native CLI/fixture with no Bun/Node/Python on PATH. The global
migration command is expected to remain failed on other inherited removals;
record its actual failure and do not make it green by weakening the validator.
Do not rebuild the native addon for this xtask-only change. CARGO_BUILD_JOBS=2;
share the supervising target cache and avoid adding dependencies or broad builds.

Completion: all scoped contracts and installed orchestration checks pass; both
rows have complete contracts, consumers, persisted formats, dependencies, Rust
destination and evidence; update this card/handoff and task state honestly.
Keep full-native TUI, whole-addon Clippy and singleton failures separate.
Recovery: revert this coherent generator/consumer slice or restore the two legacy
sources and their caller together from Git; preserve unrelated user work/data.

## OpenCode execution checkpoint

OpenCode 1.19.4 was installed outside the repository and invoked with
`opencode/deepseek-v4.1-flash-free` in the isolated `opencode/gen-nix` worktree
at `/workspace/harvest-opencode` (base `2419a56`). The supervisor supplied this
complete card, denied commits/pushes/destructive Git operations, disabled external
plugins/LSP, and shared only the Rust build cache for scoped checks.

The actual first request failed before edits with
`Proxy refused to open a tunnel: 403 Forbidden` (session
`ses_e98a0ea38ffei7LJqyF20APVxl`, CLI exit 1). No model output or implementation
completion is claimed. Logs and the exact prompt are under
`/tmp/harvest-opencode/gen-nix-*`; those machine-local files are optional aids,
not authoritative task records. No paid-provider credentials or preconfigured
OpenCode authentication were found. `opencode.ai` is not in the environment's
allowed network hosts. Do not bypass the proxy or copy unrelated credentials.

Resume after the environment permits OpenCode's API or the user supplies an
accessible authenticated OpenCode server/provider. Keep architecture/review with
the supervisor and code changes with OpenCode as requested; inspect the diff and
independently rerun the card's checks before advancing the two source rows.

## Direct implementation completion

The user cancelled external-model delegation. The direct Rust implementation
restores the legacy line anchor, preserves CRLF lockfile bytes except the version
stamp, rejects future/missing stamps, checks executable permissions on Unix and
retains deterministic generator precedence. Two normalization contracts and three
compiled-CLI integration contracts cover all three invocation branches, native
fixture output, selected-generator failure, no silent fallback, existing-output
preservation, pre-invocation future-version refusal and non-executable shadowing.
The fixture is Rust-only and subprocess PATH contains only its native aliases.

`cargo clippy --locked -p xtask --all-targets --all-features -- -D warnings`
and `cargo xtask test -p xtask` pass (125 tests plus doctest pass). The legacy
source/tests remain deleted only after their meaningful contracts have Rust
coverage. This task verifies Linux orchestration; third-party generator semantics
and promised platform release checks remain separate. Global migration remains
failed on other inherited deletions. No addon or unrelated legacy suite rebuild
is required for this xtask-only slice.
