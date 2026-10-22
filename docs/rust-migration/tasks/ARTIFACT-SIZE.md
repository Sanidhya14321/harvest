# control:artifact-size — preserve the installed binary size contract

Prerequisite: `control:playbook`. Work unit: tooling. No sources are eligible for
removal and no release budget change is approved by this task.

Rust CI run 39775015370, job 119811632401 on a024198, successfully installed
both aliases, ran help/version and the project brain dry-run. It then failed the
existing size gate: installed `omp` was **121,519,040 bytes** against **20,971,520**.
The earlier malformed skill frontmatter no longer blocks installation behavior.
Keep this independent artifact-size failure visible; do not increase the budget
or disable retained AST/tool behavior to make the job green.

Inputs: `.github/workflows/rust-ci.yml` Package and install job, release profile,
`rust-agent`, `pi-edit`, `pi-ast`, tree-sitter grammar registration/linking,
installation/signature/target matrix contracts. Local debug `harvest` symbols
show large `ts_parse_table` objects (up to 16,635,680 bytes); tokenizer assets are
already compressed (~4.2 MiB). This is diagnostic evidence, not a release link-map
measurement. Capture the actual release contribution report before changing
resource ownership or selecting an architecture.

Required contract/destination: keep the 20 MiB binary gate and all retained
language/AST behavior. If native grammar resources become separate or compressed,
first design their Cargo/native-archive installation, offline availability,
version/hash binding, jailed extraction, single-writer recovery, Windows/macOS
loading/signatures and eight-target verification. Third-party native dependencies
are allowed; hidden JS helpers, network-only first-use behavior and silent grammar
removal are not substitutes for parity.

Next actions: reproduce the exact installed release measurement; inspect a link
map/size report; create reviewed bounded resource tasks and prerequisites; then
implement and test them. The task remains blocked until those concrete contracts
and artifact destinations are reviewed. Do not begin an unbounded recursive AST
rewrite or alter provider/model semantics.

Verification: repeat the job's locked cargo install, both aliases' help/version,
brain dry-run, all retained AST contracts and `stat -c %s` <= 20,971,520 on the
installed Linux binary; add equivalent applicable target checks. Compare startup,
first-use and steady-state performance to captured baselines. Evidence must bind
the complete installed artifact/resource set to the tested source. Recovery is
a scoped revert of the eventual resource slice; preserve existing user data and
all measurements.
