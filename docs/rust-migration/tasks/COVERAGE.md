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
