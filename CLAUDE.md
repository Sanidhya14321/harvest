# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`AGENTS.md` is the authoritative rulebook for this repo (package map, code-quality rules, Bun-over-Node conventions, KDL model policy, generated files, security invariants, logging/TUI sanitization, testing guidance, changelog format). It is imported below — follow it.

@AGENTS.md

Current continuation: work directly on `main`; no new branches/worktrees and no
tests during the conversion sweep. Read `docs/rust-migration/HANDOFF.md` and the
common conversion register before writing; existing uncommitted changes belong
to the active shared migration batch. Preserve them.

## Quick command reference

Bun monorepo (TypeScript workspaces in `packages/`) + Cargo workspace (`crates/`, `xtask/`) + Python (`python/`).

| Task | Command |
| --- | --- |
| Fresh setup | `bun setup` (installs workspaces, builds `@harvest/pi-natives`) |
| Run CLI from source | `bun dev` (smoke: `bun dev -- --version`) |
| Rebuild natives after Rust changes | `bun run build:native` |
| Typecheck/lint gate (all) | `bun run check` — never `tsc` |
| Gate for main package | `bun --cwd=packages/coding-agent run check` (types only: `run check:types`) |
| Single TS test file | `bun test <path>` — never bare root `bun test` |
| TS suites | `bun run test:ts`, or `bun scripts/ci-test-ts.ts <suite>` (e.g. `coding-agent-ui`, `coding-agent-runtime`) |
| Rust tests | `bun run test:rs` (nextest + doctests) — never `cargo test` directly |
| Rust check/lint/fmt | `bun run check:rs` / `lint:rs` / `fmt:rs` (via `cargo xtask dev …`) |
| Format / autofix | `bun run fmt`, `bun run fix` (oxlint + oxfmt) |
| Python tests / lint | `bun run test:py`, `bun run lint:py` (ruff) |
| CLI smoke (workers) | `bun run ci:test:smoke` (includes `--smoke-test`) |
| Codegen | `bun run gen:compat` (KDL → `rules.json`), `bun run gen:models` (`models.json`), `bun run gen:nix` (only when `bun.lock` changes) |

## Where to look for architecture

- `packages/coding-agent/DEVELOPMENT.md` maps each `src/` directory to its reference doc under `docs/`; trust `docs/` over other prose, and executable config over docs.
- `ARCHITECTURE.md` and `HARVEST_CODEBASE_REFERENCE.md` at the root give the cross-package overview.
- Entry flow of the main CLI: `packages/coding-agent/src/cli.ts` → `src/main.ts` → `src/sdk.ts`. `cli.ts` is also the single worker host (hidden `__harvest_worker_*` argv selectors).
- **Ongoing TS → Rust migration**: `rust-agent/` (crate `harvest-omp`, binaries `harvest`/`omp`) is an experimental Rust runtime in the root Cargo workspace — *not* yet feature-compatible; the shipped app still launches TypeScript. Status, parity gates, and the continuation checkpoint live in `docs/rust-migration/README.md` and `docs/rust-migration/HANDOFF.md` — read HANDOFF before continuing migration work. Extracted Rust engines (e.g. `harvest-media`, `harvest-tui::mouse`/`highlight`) are exposed to TS via N-API; the TS fallbacks stay until parity is verified, so don't delete them. Scoped Rust tests: `cargo xtask test -p <crate>`.
- `CODE_REVIEW*.md` / audit reports document past security findings that the invariants in `AGENTS.md` guard against.
