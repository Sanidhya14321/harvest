# control:coverage — account for owned transitive engine coverage

Prerequisite: `control:playbook`. This is an explicit release/CI blocker, not a
completed migration or a request to lower the quality gate.

Inputs: `.github/workflows/rust-ci.yml` coverage job and run 39728439649,
job 119657680928. The measured 106,981 lines included owned `pi-builtins`,
`pi-edit`, `pi-shell`, `pi-walker` and other dependency engines, while nextest
selected only the portable application crates. Overall line coverage was
20.81% against the existing 65% requirement. Excluding vendored brush-core alone
would leave approximately 21.47%, so vendor exclusion cannot solve this gap.

Contract/destination: retain the 65% floor and meaningful first-party coverage.
Keep reports and the floor computed over the same explicitly documented scope.
Connect retained owned engine suites to the coverage invocation and repair real
missing tests. Third-party coverage may be reported separately, but no owned
module may be excluded merely because it is insufficiently tested.

Ordered work: inspect the complete report and owned dependency graph; establish
which existing native suites are supported in coverage; add those suites;
measure again; split remaining behavioral gaps into bounded task cards. Review
new checks before making this card ready. Do not expand into an uncontrolled
rewrite. No source deletion is authorized by this task.

Verification: run the coverage job's exact `cargo llvm-cov nextest` command,
followed by `cargo llvm-cov report --lcov --output-path lcov.info` and
`cargo llvm-cov report --fail-under-lines 65`. Record exact package selection,
platform and report as evidence. The command must succeed without weakening
the floor or hiding first-party code. Recovery: restore the prior measurement
configuration, preserve failing reports, and keep this task blocked.

## Reviewed additive suite selection

The existing first-party engine suites provide over 2,500 Rust test cases across
`pi-ast`, `pi-builtins`, `pi-diff`, `pi-edit`, `pi-iso`, `pi-shell`, `pi-vcs`,
`pi-voice` and `pi-walker`. Add all nine packages to the existing measurement
command, retaining every previous package and the unfiltered report/floor. No
filename exclusion or reduced threshold is introduced. The transitional N-API
adapter remains covered by its existing rebuilt-addon host contracts; this job's
metric remains the selected Rust product/ordinary engine code. Run the exact new
workflow command and require both its tests and unchanged 65% report to pass
before completion. Keep any real native-engine regressions visible and repair
them before claiming success.

## Verified result

At source checkpoint `1a22bd5` plus this additive workflow selection, Linux x86_64
and nightly-2026-08-08 run 2,925 passing contracts (five existing skips). The
unfiltered LCOV/report contains 144,597 executable lines, 107,698 hit: **74.48%**.
The exact unchanged `cargo llvm-cov report --fail-under-lines 65` command exits
successfully. Fresh collection had 12 GB of disk headroom; the earlier disk-full
collection was discarded. Logs and LCOV live under `target/migration/coverage-*`.
No prior package, owned module or third-party report entry was excluded. CI
reproduces this package selection; this is not a whole-product parity claim.
