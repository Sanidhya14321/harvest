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

## Confirmed batch and implementation boundary

The native input bridge used Rust Instant while the host scheduled timers with
its performance clock. Bun fake timers advance performance.now but not Instant;
therefore a delivered host timer did not advance native framing deadlines. Select
an explicit host-clock domain at construction and pass monotonic elapsed time
through the adapter before input/deadline operations. Validate finite bounded
nonnegative timestamps and reject backward movement or switching an active native
clock. Ordinary Rust InputBuffer keeps its existing explicit Duration API; direct
native consumers keep Instant by default. Do not copy framing logic into TS.

The resize capture selected the immediate absolute-addressed heading-only probe,
although this contract waits for the settled full paint and inspects CRLF spacer
rows. Select that full paint; retain every glyph-column/erase assertion. The four
resize cases pass with this fixture correction alone. Validate the combined input
and resize batch with the full native suite; keep failure-cleanup cascades visible.
One addon rebuild covers the whole batch. Recovery restores bridge and host clock
connection together; no user/session data changes.

## Completed connected batch

The addon exposes optional hostTime selection and monotonic setTime validation;
the StdinBuffer adapter supplies elapsed performance.now time before input and
timer operations. Native callers retain Instant by default. A clock cannot be
switched after construction, move backwards, or accept non-finite/negative values.
An executable binding test covers deadline expiry, I/O deferral, subsequent arrow
framing and clock errors. The renderer fixture selects the last settled full paint;
its glyph-column/erase assertions all remain. No keyboard flags were weakened.

One optimized addon rebuild serves the entire batch. All 141 focused host tests,
all four full native chunks, 127 Rust terminal tests plus four doctests and both
TUI/coding-agent typechecks pass. The previous six failures are resolved together.
The strict whole-addon Clippy still fails with the same 14 prior messages outside
the changed bridge; this task does not certify that separate lint gate or whole
terminal/application parity. Logs: `target/migration/terminal-batch-*`.
