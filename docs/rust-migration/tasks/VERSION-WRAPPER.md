# tooling:version-wrapper — remove the obsolete Bun version-sync launcher

Prerequisite: `control:playbook`. Source: `scripts/sync-versions.ts`.
Capability: `surface:codegen` stays partial.

The source is now a three-line Bun launcher forwarding every argument to the
already implemented `cargo xtask npm-versions`. Repository-wide search finds no
active caller of this launcher. `scripts/release.ts` already calls the Rust CLI
directly. Historical baseline paths and fixture provenance remain unchanged.

Contract: retain the existing Rust synchronization/release/check interfaces,
manifest format, internal dependency rewrites, private versions, catalog/workspace
protocols, invalid-input preservation, jailed path checks and failure exit status.
The obsolete `bun scripts/sync-versions.ts` invocation is replaced by
`cargo xtask npm-versions`; do not install or publish packages as part of this task.
No asynchronous worker, cancellation protocol or new persisted format exists.

Destination/connections: reuse `xtask/src/npm_versions.rs` and the connected
release caller. Remove only the redundant launcher and document the native
replacement. Bun remains needed by other transitional sources and is not yet an
orphaned workspace dependency.

Steps: inspect the launcher, consumer search, engine and captured fixtures;
exercise the compiled Rust release/check CLI with no language runtime on PATH;
run existing failure/symlink/idempotency tests; stage deletion, update ledger and
task/handoff, and verify source-removal gates. Never alter actual package versions
or live registries during verification.

```sh
cargo fmt -p xtask -- --check
cargo xtask test -p xtask
cargo xtask migration
cargo xtask capabilities
cargo xtask playbook check
```

Expected: fixture manifest rewrites and failure preservation pass; native CLI
runs with a PATH containing no Bun/Node; the staged source inventory accepts the
verified removal. Completion requires those results and a source-bound plan.
Recovery: restore only this launcher and its ledger/task state from the preceding
commit. Keep the Rust engine and unrelated migration history intact.
