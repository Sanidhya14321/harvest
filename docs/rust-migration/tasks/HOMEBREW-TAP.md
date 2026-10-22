# tooling:homebrew-tap — native tap formula staging

Prerequisite: `control:playbook`. Source ID/path:
`rust-agent/scripts/update_homebrew_tap.sh`; linked capability `surface:release-installers` remains
partial. The baseline shell blob is `1c108d73978ba7b60061bd17bc2de6cac4253c2e`
at checkpoint `838c33f`. Its selftest verifies first commit, unchanged no-op and
an updated formula; capture it before deleting the script.

Destination: `xtask/src/homebrew_tap.rs`, public CLI
`cargo xtask tap-update --formula FILE --tap CHECKOUT --tag TAG`. Reuse jailed
filesystem writes/locks and bounded local process capture. Git remains an allowed
external native program. The formula is opaque bounded data, not evaluated here;
no new formula format, network request, push or release is authorized. The writer
lock lives in Git-resolved metadata, including linked worktrees.

Contract: copy byte-identical formula content to `Formula/omp.rb` and commit it with normal Git attributes/line-ending semantics, use the same
bot identity/message, and leave HEAD unchanged for identical input. Preserve
unrelated work. Reject invalid tags, non-repository roots, dirty target files,
preexisting staged changes, oversized input and escaping/symlink destinations
before mutation. Use a single writer lock, atomic replacement and argument arrays.
Suppress hooks/signing for this maintenance commit and scope the commit to the
formula. Unlike the old helper, do not rewrite the checkout's identity config.
A failed Git operation returns failure; retain inspectable changes for recovery.
Cancellation may leave a staged formula; report it and inspect before retrying.

Connections: replace all three script calls in `.github/workflows/rust-ci.yml`
(selftest is subsumed by Rust contracts; installed tap uses the native CLI) and
`.github/workflows/rust-release.yml` (staging only; existing separate push stays).
No addon rebuild is needed for a tooling-only change. Preserve the original
baseline and historical shell path in the ledger.

Steps: capture baseline; implement native CLI; port baseline transitions and add
failure-preservation contracts; run the compiled CLI with only Git on PATH;
connect workflows; stage script deletion; finalize evidence and source-removal
records; run checks before committing. Remove no other installer or runtime.

```sh
cargo fmt -p xtask -- --check
cargo clippy --locked -p xtask --all-targets --all-features -- -D warnings
cargo xtask test -p xtask
cargo xtask playbook check
cargo xtask migration
cargo xtask capabilities
```

Completion requires native CLI success without shell/Bun/Node, unchanged/updated
HEAD and exact content checks, refusal preservation, no remaining active script
consumer, and staged source-removal validation. Whole-product/release acceptance
remains pending. The full transition is independently blocked by the recorded
legacy singleton failures; preserve its failed receipt.

Recovery: revert this CLI, workflow connections and ledger/task row together,
restoring the script from the preceding commit. Do not alter an external tap or
push during rollback. Inspect any failed fixture's Git status before cleanup.

Verified locally on Linux x86_64: five Rust tap integration contracts pass,
including the compiled CLI with Git as the only PATH entry and deliberately
misleading ambient Git directory/index overrides. All 117 xtask tests plus
doctests and strict Clippy pass. The registered CI/release callers now use the
native helper. Platform CI still supplies additional Windows/macOS evidence;
no live tap, push, publication or whole-release completion is claimed.
