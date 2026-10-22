# control:embedded-sources — account for text-wrapped executable code

Prerequisite: `control:playbook`. Discovery during browser replacement: extension
assets `*.js.txt`, injected Puppeteer scripts, the aria snapshot bundle, eval's
JS prelude and the generated stats-client archive are executable payloads despite
their text/container suffix. Their import/installation consumers establish that
role. License documents and tips remain ordinary data.

Contract: audit and source-removal gates classify source-named text wrappers and
these reviewed executable containers consistently. Unowned new wrappers must
fail ownership, and final release must reject them. Preserve the immutable
baseline; append existing omitted sources to the same ledger with pending owners.
Do not mark them migrated, reclassify them as inert, or change language-statistics
configuration. Wrapped HTML with script tags remains a review candidate.

Destination: shared classification in `xtask/src/source_audit.rs`, reused by the
source ledger gate in `xtask/src/migration.rs`; existing execution graph paths.
Steps: inspect actual import/build consumers; add narrow plain-text executable
container recognition and recursive source suffix handling; append missing
ownership rows; test generated runtime fixtures, including ordinary text/license
nonmatches and wrapped markup; run  xtask contracts, strict Clippy and state gates.
No source is deleted by this accounting fix. Recovery is a scoped revert; retain
the discovery and prevent release from claiming strict completion.

```sh
cargo fmt -p xtask -- --check
cargo clippy --locked -p xtask --all-targets --all-features -- -D warnings
cargo xtask test -p xtask
cargo xtask migration
cargo xtask playbook check
cargo xtask source-audit status
```

Completion requires the new executable containers to appear in the non-Rust
inventory and authoritative ledger, with owners and pending status. Historical
completed removals remain separate from the expanded denominator.
