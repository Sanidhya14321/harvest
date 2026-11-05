---
name: rust-migration-slice
description: Carry one TypeScript (or Python/JS) subsystem through the full Rust migration sequence — contract capture, Rust implementation, parity verification, consumer switch, legacy deletion, ledger update, commit. Use for any "port X to Rust" / "continue the migration" request.
---

# Migrate one slice to Rust

Read first (once per session): `docs/rust-migration/HANDOFF.md`, `docs/rust-migration/README.md`, `AGENTS.md`. Executable sources beat prose. Run `/migration-status` if no slice was named.

## Hard rules
- Never mark ledger/capability rows verified without evidence; never delete ledger rows; never relax xtask gates.
- Reuse mature engines (`pi-edit`, `pi-shell`, `pi-walker`, `pi-vcs`, `pi-ast`, `harvest-runtime` jail/bounded IO/atomic writes). Do not write a weaker second implementation.
- Model/provider policy stays in KDL; prompts stay static `.md` (use `include_str!`). Generated files are regenerated, never hand-edited.
- No JS-runtime fallback for retired features — fail with an actionable migration message.
- Tests: `cargo xtask test -p <crate>` (never bare `cargo test`). Tests assert observable contracts; no source-grep tests.

## Sequence
1. **Scope**: pick a slice with a closed set of files. List them and their consumers (`grep -rn` imports/callsites incl. workers in `packages/coding-agent/src/cli.ts`, build scripts, Bazel `BUILD.bazel`, CI workflows).
2. **Contract capture**: document inputs/outputs, error mapping, limits, persisted formats, cancellation. Build an oracle: run the *current* implementation over representative + malformed inputs and save outputs (scratchpad) for byte comparison. Port meaningful existing TS tests as Rust tests.
3. **Implement** in an ordinary Rust crate (existing `crates/harvest-*` or new one added to root `Cargo.toml` workspace, `Cargo.lock`, Bazel `BUILD.bazel`/`MODULE.bazel`, and Rust CI package lists). Explicit bounds *before* allocation/decoding. If TS still calls it, keep a thin N-API adapter (see `/napi-extract`).
4. **Verify**: `cargo xtask test -p <crate>`, `cargo clippy -p <crate> --all-targets -- -D warnings`, `cargo fmt -p <crate> -- --check` (only changed files in `pi-natives` — run `rustfmt` on them, not the crate). Compare oracle byte-for-byte. If natives changed: `bun run build:native`, then focused `bun test <path>` for TS consumers, `bun --cwd=packages/coding-agent run check`.
5. **Switch consumers** (callers, generators, packaging, docs). Delete superseded TS/TSX/declarations only when *every* consumer moved. Remove orphaned deps/config.
6. **Ledger**: update the slice's `source-ledger.json` entries (contract, consumers, rust_destination, verification states, legacy_removal) and `capabilities.json` honestly; add a section to `docs/rust-migration/README.md` with real counts. Run `cargo xtask migration` and `cargo xtask capabilities` to confirm the gates accept it.
7. **Commit** one verified slice (conventional subject, e.g. `refactor(media): move sixel encode/decode into harvest-media`), only if the user authorized commits. Report what is still TS/unverified.
