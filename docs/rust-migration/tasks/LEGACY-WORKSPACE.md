# control:legacy-workspace — bounded transitional suite execution

Prerequisite: `control:playbook`. Existing owned sources remain pending Rust
migration: `scripts/ci-test-ts.ts` and its tests. This repair does not certify
provider parity or advance source-removal counts.

Inputs: failed transition receipt `target/migration/verified-foundations.json`,
workspace rerun logs, the existing coding-agent chunk planner and the eight
workspace package suites. Cgroup memory events confirmed worker OOM kills. The
AI suite retained too much VM/native state even with one worker over 414 files.
The utils wall-time assertion passed in isolation but failed under file fan-out.

Contract/destination: keep every test and assertion, the existing deadlines and
credential-dependent skips. Run utils with one file VM; run AI's complete test
registry as fresh ten-file process chunks with one file VM each. Reuse the
existing collector and shared budgeted executor. Preserve both `workspace` and
`local-ts` entrypoints, failure propagation, watchdogs and explicit arguments.
A planner regression independently enumerates the test files and rejects omitted
or duplicate coverage. Do not hide failing assertions or increase thresholds.

Steps: reproduce memory/timing failures and isolate them; bound worker/process
lifetimes; verify the entire registry exactly once; run all eight workspace
suites. Global FORCE_COLOR is not appropriate for silent-stderr subprocess tests.
The earlier failed receipt remains historical evidence; a passing workspace run
alone does not certify the entire transition or release.

```sh
bun test scripts/ci-test-ts.test.ts
OMP_TEST_CONCURRENCY=1 bun scripts/ci-test-ts.ts workspace
```

Run the repository formatter/linter and TypeScript checks for affected tests.
Completion evidence: seven planner/watchdog contracts and all 49 workspace
chunks pass; no test file is dropped. Recovery: revert this bounded scheduling
repair and keep failed diagnostics. Retire this Bun-specific orchestration after
its last legacy suite moves to the Rust-owned test runner; do not preserve it in
the final Rust-only product.
