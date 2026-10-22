# control:portability — restore platform foundation contracts

Prerequisite: `control:playbook`. No capability parity is certified by this task.

Inputs: Rust CI run 39728439649, Windows/macOS core-contract job logs,
`harvest-runtime` jailed filesystem, `harvest-brain` retrieval/store,
`xtask` ZIP writer and artifact CLI tests, tokenizer rustdoc, the repository's
`tool-prompt-optimization` skill frontmatter, and the registered workspace
format check. These failures predate the extracted SIXEL engine.

Contract: an explicitly opened root alias and its canonical name authorize the
same suffix inside the original directory capability. Retargeting the alias
must not redirect writes outside that directory. Unrelated absolute roots,
parent traversal and escaping symlinks remain denied. Persisted memory paths
use forward slashes on Windows; Unix literal backslashes remain literal.
Release ZIP entries preserve Unix executable bits on every build host. Invalid
archives must still fail their specific verification. Rustdoc links resolve in
non-test builds; skill frontmatter parses without changing its description.

Destination/connections: retain the existing Rust modules and public APIs.
Update the root spelling retained by `JailedFs`, its direct consumers, shared
memory path serialization, ZIP producer and the corrupt-archive fixture
producers. Fix formatting with the pinned toolchain. No legacy sources or
runtime adapters are eligible for deletion in this repair.

Steps: reproduce failing contracts; implement bounded fixes; add alias
retargeting regression; run existing escape, memory, archive and rustdoc checks;
then verify the same checks on Windows and macOS CI. Do not waive failures.

```sh
cargo fmt --all -- --check
cargo xtask test -p harvest-runtime -p harvest-brain -p xtask
cargo clippy --locked -p harvest-runtime -p harvest-brain -p harvest-tokenizer -p xtask --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --locked -p harvest-tokenizer --no-deps
cargo xtask playbook check
```

Also rerun `harvest-omp` native search contracts and installed `brain --dry-run`
checks on CI, where the original temp-root aliases were observed. Linux success
alone does not establish Windows/macOS success. Completion requires those CI
results. Recovery: revert this repair's constructors, serializers and archive
options together; never repair data by moving it outside its authorized root.

Follow-up from CI 39775015370/job 119811632870: Windows core passed its debug
contracts, then the release `provider_usage` fixture failed with Winsock 10035.
Its accepted socket inherited the listener's nonblocking mode. Explicitly return
accepted fixture streams to blocking mode before applying the existing bounded
read timeout. Preserve the 10-second accept deadline, request limits and all
assertions. Rerun the five provider-usage tests and Windows CI; this task remains
in progress until that result is available.
