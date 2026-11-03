# application:python-eval-retirement — remove owned Python execution

Prerequisites: `control:playbook`, `review:application` (done). Approved decision:
`SCOPE.md` retires language-specific evaluation, workers and injected helpers.
The broader `tool:eval` retirement stays pending because JavaScript runtime
consumers in browser/computer workers still require native migration.

Inputs: `src/eval/py`, `session/eval-runner.ts`, tool registry/backend selection,
setup CLI, browser/computer Python preludes and their execution tests under
`packages/coding-agent`. Baseline: `2ff0193`. Source membership is the existing
source ledger, assigned to this leaf only for actual reviewed removals.

Destination and connections: native tool diagnostics and `harvest setup python`
return the approved migration result. Transitional TS executor/kernel exports
reject execution without probing, spawning, loading a runner or invoking a user
Python hook. Keep historical Python result/display types and transcript rendering.
Keep generic cancellation/ownership contracts used by JavaScript. The ordinary
shell tool may run user-requested external programs; MCP/launch console helpers
and browser/computer JavaScript engines remain until their consumers move.

Contract: Python cannot be enabled by saved settings, PI_PY or custom interpreter
paths. Never silently dispatch Python to JavaScript. Direct session execution,
kernel tool invocation, backend execution and setup return migration guidance.
`setup python --json` returns structured unavailable/retired data and nonzero exit.
No new Python hook invocation, filesystem mutation or interpreter lookup occurs.
Historical messages, rich display output, shell approval and JS cancellation
continue working. Settings keys remain accepted as deprecated inert inputs but
must not be advertised as active runtime controls.

Steps: remove kernel/runner/HTTP tool bridge and Python prelude payloads; replace
remaining entrypoints with explicit retirement boundaries; remove Python-only
execution tests and replace them with rejection/no-side-effect contracts; retain
the JS and generic parts of mixed tests; verify actual transitional CLI and copied
native candidate, then update source records and commit the complete batch.

Verification: affected Bun tests plus coding-agent typecheck and formatter/lint;
`cargo xtask test -p harvest-omp`, strict affected Rust Clippy/format; copied native
CLI with no language runtime on PATH; source/playbook/capability gates and a
source-bound verification plan. Run the broad legacy workspace suites at this
consumer cutover. Known source-removal/singleton/platform blockers remain visible
and cannot be reclassified as passed from focused checks.

Removal evidence must identify every deleted path's prior role and consumers,
approved retirement reason, replacement diagnostic and behavioral/installed
checks. Boundary TS sources remain pending Rust application cutover and are not
counted as removed. Store receipts under `target/migration/python-retirement-*`.
Recovery: revert this bounded retirement batch; preserve session files and
unrelated migration work. Do not delete JavaScript runtime dependencies before
native browser/computer consumer migration.
