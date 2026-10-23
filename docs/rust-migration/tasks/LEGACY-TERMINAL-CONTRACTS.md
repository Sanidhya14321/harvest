# control:legacy-terminal-contracts — resolve full native consumer failures

Prerequisite: `control:playbook`. The DOCX/host continuation's full native check
`bun scripts/ci-test-ts.ts native` passes natives, collaboration and benchmark
chunks but fails the TUI chunk: 1,464 passed and six failed. The exact six failures
also reproduce with the two affected files run directly. This is separate from
the previously recorded 22 coding-agent singleton failures. The old cached addon
has only 117 exports and lacks the portable terminal classes, so it cannot serve
as a valid regression baseline. Do not claim these six are new or baseline-matched
until the corresponding source-bound baseline is actually exercised.

Inputs: `packages/tui/test/{terminal-appearance,issue-8318-repro}.test.ts`,
`packages/tui/src/{terminal,stdin-buffer}.ts`, ordinary Rust terminal/input engines
and `crates/pi-natives/src/portable_terminal.rs`. These sources did not change in
the DOCX/concurrent checkpoint merge, but source equality does not prove parity.

Observable failures:

- OSC 66 reserved row disappears during resize repaint.
- Partial OSC 11 buffering swallows an unrelated up-arrow.
- Abandoning private CSI reassembly splits the unrelated escape.
- ConPTY receives alternate-key pushes despite disambiguation-only requests.
- Existing parent reporting is duplicated with the wrong ConPTY flags.
- Shutdown does not balance the single kitty push.

Next action: run each failure in an isolated process to distinguish primary
failures from leaked listeners after earlier assertions. Compare timers, parser
ownership and terminal shutdown with the last relevant Git source/build. Inspect
whether fake JavaScript timers and native parser deadlines share an explicit
clock contract; this is a hypothesis, not a concluded fix. Create a bounded
implementation card for the first confirmed defect. Keep all assertions and
files in the full native suite. Never split a suite merely to hide leaks.

Checks: `bun scripts/ci-test-ts.ts native`; direct two-file reproduction; each
isolated failure; Rust terminal contracts and rebuilt-addon checks for any actual
engine edit. Logs: `target/migration/docx-final-native-tests.log`,
`docx-terminal-recheck.log`, `docx-terminal-isolated-*.log`. Recovery reverts the
confirmed engine/consumer change together; no session or terminal data is deleted.
Completion requires the full native suite, not only file-local success.

Initial isolated observations: the partial-OSC up-arrow case still fails alone;
ConPTY disambiguation passes alone. Preserve the complete group failure and
investigate failed-test cleanup cascades before changing keyboard production
flags. No baseline equivalence claim is made from these observations.
