# tooling:spoofed-versions — native upstream identity maintenance

Prerequisite: `control:playbook`. Work unit: tooling. Capability:
`surface:codegen` remains partial; completing this script does not complete all
code generation or provider compatibility.

## Contract and inputs

The sole active caller is root `package.json`'s `check-spoofed-versions` script.
The old `scripts/check-spoofed-versions.ts` checks Gemini CLI's default identity
in `packages/catalog/src/wire/gemini-headers.ts` against GitHub's latest stable
release. Default mode reports drift and exits unsuccessfully without writing;
`--update` changes only the matched default version. Failed lookup/parsing must
not overwrite the provider. Preserve unrelated equal version strings.

The saved `xtask/tests/fixtures/spoofed-versions-oracle.json` captures the original
script at d811cb2 in isolated fixtures with mocked upstream responses. It is
behavioral test data, not an upstream-freshness claim.

## Rust interface and connections

Implement `xtask::spoofed_versions` and `cargo xtask spoofed-versions
[--root ROOT] [--update] [--release-json FILE]`. The optional captured JSON input
is explicitly reported as offline evidence, not live freshness. Normal operation
uses GitHub's latest-release endpoint, bounded responses, a deadline, the proper
Accept/User-Agent headers and verified TLS. Use the existing jailed filesystem
and atomic writes. Reject ambiguous source declarations before changing them.

Connect main dispatch, root package command and help; remove the old TS entry
after its replacement passes. The TS provider remains an active consumer-owned
source and is not retired by this narrow task. Its eventual Rust replacement
must move this maintenance target in the same dependency slice.

## Ordered work, verification and recovery

1. Capture the legacy read-only/drift/update/bad-tag/unavailable contracts.
2. Implement the Rust checker and live HTTP contract test; preserve headers,
   endpoint, exit status and source transformation behavior.
3. Exercise the compiled CLI with no Bun/Node executable on PATH against the
   golden fixtures. Check invalid declarations and external symlink refusal.
4. Rewire the caller, delete the TS script, remove only orphaned dependencies,
   and update its source-ledger row and task evidence.

```sh
cargo fmt -p xtask -- --check
cargo clippy --locked -p xtask --all-targets --all-features -- -D warnings
cargo xtask test -p xtask
cargo xtask migration
cargo xtask playbook check
```

Run `cargo xtask spoofed-versions` without `--update` for a live read-only probe
when GitHub API egress is available. Record a denied destination or unavailable
upstream separately; fixtures cannot certify live freshness.

Completion requires parity fixtures, actual native CLI execution, connected
caller and verified eligible source deletion. Recovery: revert this slice's
native entrypoint, caller and removal together; never alter unrelated provider
source or the immutable source baseline.
