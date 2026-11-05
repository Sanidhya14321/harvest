---
name: harvest-verify
description: Choose and run the correct verification commands for whatever changed in this repo (TS packages, Rust crates, natives, KDL/catalog, Python). Use before reporting work as done or before committing.
---

# Verify changes

Determine changed paths: `git status --porcelain` and `git diff --name-only main...HEAD`. Run with `set -e`; report actual exit codes and failing output.

| Changed | Run |
| --- | --- |
| `packages/coding-agent/**` | `bun --cwd=packages/coding-agent run check`; `bun test <changed/related test files>` |
| other `packages/<pkg>/**` | that package's `check` script; focused `bun test <path>`; full suite via `bun scripts/ci-test-ts.ts <suite>` |
| `crates/**`, `rust-agent/**`, `xtask/**` | `cargo xtask test -p <crates>`; `cargo clippy -p <crates> --all-targets -- -D warnings`; `cargo fmt -p <crates> -- --check` (pi-natives: rustfmt only changed files) |
| `crates/pi-natives/**`, `packages/natives/**` | additionally `bun run build:native` and TS consumer tests |
| `packages/catalog/src/compat/rules/**` | `bun run gen:compat`, commit `rules.json` |
| catalog descriptors/generator | `bun run gen:models` |
| `bun.lock` | `bun run gen:nix` |
| `python/**` | `bun run lint:py`, `bun run test:py` |
| workers / `cli.ts` dispatch | `bun run ci:test:smoke` |
| migration ledger/capabilities | `cargo xtask migration`, `cargo xtask capabilities` |
| themes / clippy config | `cargo xtask theme-index --check`, `cargo xtask clippy-config --check` |

Never: `tsc`, bare root `bun test`, bare `cargo test`. A green check is not proof — also exercise the changed path (e.g. `bun dev -- <args>` or `cargo run -p harvest-omp --bin harvest -- <args>`) and state the scenario and result.
