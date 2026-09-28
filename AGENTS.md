# Development Rules

## Default Context

This repo contains multiple packages, but **`packages/coding-agent/`** is the primary focus. Unless otherwise specified, assume work refers to this package.

**Terminology**: When the user says "agent" or asks "why is agent doing X", they mean the **coding-agent package implementation**, not you (the assistant). The coding-agent is a CLI tool — questions about its behavior refer to code in `packages/coding-agent/`, not your current session.

### Package Structure

| Package                         | Description                                                        |
| ------------------------------- | ------------------------------------------------------------------ |
| `packages/coding-agent/`        | Main CLI application (primary focus): `src/cli.ts` → `src/main.ts` → `src/sdk.ts` |
| `packages/agent/`               | Agent runtime: state machine, execution loop, compaction           |
| `packages/ai/`                  | Multi-provider LLM client with streaming support                   |
| `packages/catalog/`             | Model catalog: bundled models.json, provider descriptors, KDL rules |
| `packages/tui/`                 | Terminal UI library with differential rendering                    |
| `packages/natives/`             | N-API bindings for native text/image/grep operations               |
| `packages/stats/`               | Local observability dashboard (`harvest stats`)                    |
| `packages/omptype/`             | ArkType-compatible schema validation with a lazy JIT runtime       |
| `packages/utils/`               | Shared utilities (logger, streams, temp files)                     |
| `packages/mnemopi/`             | Local SQLite memory engine                                         |
| `packages/metaharness/`         | Benchmark orchestration harness                                    |
| `packages/wire/`                | Shared collab live-session protocol types                          |
| `packages/collab-web/`          | Browser guest client + local relay for collab sessions             |
| `packages/snapcompact/`         | Bitmap-frame context compression                                   |
| `packages/browser-relay/`       | Extension relay for Eval browser API                               |
| `packages/harvest-memory/`, `packages/harvest-schema/` | Memory backend + shared schema                       |
| `packages/typescript-edit-benchmark/` | Edit benchmark suite                                         |
| `crates/pi-*` (`crates/harvest-*` mirrors) | Rust natives: shell, ast, iso, voice, walker, edit, vcs, diff |
| `decision-sidecar/`             | Laya local decision microservice (Python/FastAPI, see below)       |
| `python/robomp/`                | GitHub issue/PR lifecycle automation                               |

Authoritative subsystem map: `packages/coding-agent/DEVELOPMENT.md` links each `src/` directory to its reference doc under `docs/`. Trust `docs/` over prose elsewhere; if docs conflict with scripts/config, trust the executable source.

**Catalog import convention**: code in this repo imports catalog _values_ (bundled models, model-thinking helpers, identity, descriptors, model manager/cache) from `@harvest/pi-catalog/<module>` — never via `@harvest/pi-ai`. The pi-ai barrel re-exports only the model/effort _types_ its own signatures use (`Model`, `Api`, `ThinkingConfig`, `Effort`, …); type-only imports of those from `@harvest/pi-ai` are fine.

## GitHub

Unless user tells you exactly what to write:

- **Never comment on GitHub** (issues, PRs, discussions).
- **Never create issues on GitHub**.

## Code Quality

- No `any` unless absolutely necessary.
- Prefer explicit type names over inline type queries where clarity benefits.
- Use top-level imports for maintainability.
- Check `node_modules` for external API types instead of guessing.
- **Barrel exports**: prefer `export * from "./module"` over named re-exports, including `export type { ... } from`. In pure `index.ts` barrels, use star re-exports even for single-specifier cases. If stars create ambiguity, remove the redundant export path; do not keep duplicates.
- **Class privacy**: standard TypeScript `private`/`protected` or ES `#private` fields are permitted; keep member visibility clean and minimal.
- **Prompts**: never build prompts in code (no inline strings, template literals, or concatenation). Prompts live in static `.md` files; use Handlebars for dynamic content. Import them via `import content from "./prompt.md" with { type: "text" }` — not `readFile`. (`bunfig.toml` maps `.md`/`.py`/`.lark` to the text loader, which is why this works.)
- **Worker scripts**: workers re-enter the CLI entrypoint; never spawn separate worker entry modules. `cli.ts` declares itself as the worker host at startup (`declareWorkerHostEntry()` from `@harvest/pi-utils/env`) and dispatches hidden argv selectors (`__harvest_worker_stats_sync`, `__harvest_worker_tab`, `__harvest_worker_js_eval`, `__harvest_worker_tiny_inference`) before loading the command registry. Spawn sites use:
  ```ts
  import { workerHostEntry } from "@harvest/pi-utils";
  const hostEntry = workerHostEntry();
  const worker = hostEntry
  	? new Worker(hostEntry, { type: "module", argv: ["__harvest_worker_<name>"] })
  	: new Worker(new URL("./<worker>.ts", import.meta.url).href, { type: "module" });
  ```
  When the process was started from the harvest CLI — source `cli.ts`, npm-bundle `dist/cli.js`, or compiled binary — `workerHostEntry()` is `Bun.main` and the worker re-enters the single entry module, so no per-worker `--compile` entrypoints or bundle entries exist. Outside a CLI host (`bun test`, SDK embedding, standalone `omp-stats`) it returns `null` and the direct-module fallback loads the worker source. New worker kinds MUST add their selector to the dispatch table in `cli.ts` and keep the fallback branch.
  History: `with { type: "file" }` only copied the entry as a raw asset (workers crashed silently in compiled binaries — issues #1011, #1027), and the later literal-path + extra-entrypoint pattern required keeping spawn literals and two build scripts in sync (issue #1150). The smoke probe below is the live validation of this contract.
  Validate any new worker with the dedicated smoke probe: `harvest --smoke-test` spawns the stats sync worker and the tiny-model subprocess, pings them, and exits — it's wired into `ci:test:smoke` and `scripts/install-tests/run-ci.sh` so binary, source-link, and tarball installs all exercise it. Add a sibling smoke if the new worker is on a different module graph.

## Central Utilities

Before writing a helper, check whether one already exists — `packages/coding-agent/src/utils/`, `@harvest/pi-utils`, `@harvest/pi-tui`, and the domain modules next to your callsite. This applies to **everything**: VCS wrappers, formatting/truncation/path-display helpers, image handling, clipboard, streams, temp files, caching. The central versions carry hardening a fresh copy always loses (timeouts, output caps, non-interactive env, lock avoidance, caching, TUI sanitization).

- Search first: `grep` for the operation before implementing it. Two implementations of the same thing is a bug even when both work.
- Examples of the pattern: `src/utils/git.ts` and `src/utils/jj.ts` are the only sanctioned way to run git/jj (`import * as git from "../utils/git"` — never hand-spawn via `$`/`Bun.spawn`); rendering goes through the helpers in TUI Sanitization below (`replaceTabs`, `truncateToWidth`, `shortenPath`, `PREVIEW_LIMITS`) rather than ad-hoc string math.
- Missing capability? Extend the central helper (new option, new sub-function on the namespace) and call it — don't fork its logic locally.

## Bun Over Node

Use Bun APIs where they provide a cleaner alternative; fall back to `node:*` only for what Bun doesn't cover. **Never spawn shell commands for operations with proper APIs** (e.g., don't `Bun.spawnSync(["mkdir", "-p", dir])` — use `mkdirSync`).

### Quick reference

| Operation       | Use                                       | Not                                |
| --------------- | ----------------------------------------- | ---------------------------------- |
| File read/write | `Bun.file()`, `Bun.write()`               | `readFileSync`, `writeFileSync`    |
| Spawn process   | `` $`cmd` ``, `Bun.spawn()`               | `child_process`                    |
| Sleep           | `Bun.sleep(ms)`                           | `setTimeout` promise               |
| Binary lookup   | `$which("git")` from `@harvest/pi-utils` | `spawnSync(["which", "git"])`      |
| HTTP server     | `Bun.serve()`                             | `http.createServer()`              |
| SQLite          | `bun:sqlite`                              | `better-sqlite3`                   |
| Hashing         | `Bun.hash()`, `Bun.password.*`, WebCrypto | `node:crypto`                      |
| Path resolution | `import.meta.dir`, `import.meta.path`     | `fileURLToPath` dance              |
| JSON5           | `Bun.JSON5.parse()` / `.stringify()`      | `json5` package                    |
| JSONL           | `Bun.JSONL.parse()` / `.parseChunk()`     | `text.split("\n").map(JSON.parse)` |
| String width    | `Bun.stringWidth()`                       | `get-east-asian-width`, custom     |
| Text wrapping   | `Bun.wrapAnsi()`                          | custom ANSI-aware wrappers         |

### Process execution

Prefer Bun Shell (`` $`cmd` ``) for simple commands:

```typescript
import { $ } from "bun";

const result = await $`git status`.cwd(dir).quiet().nothrow();
if (result.exitCode === 0) {
	const text = result.text();
}

$`do-stuff ${tmpFile}`.quiet().nothrow(); // fire and forget
```

Methods: `.quiet()`, `.nothrow()`, `.text()`, `.cwd(path)`.

Use `Bun.spawn`/`Bun.spawnSync` only for: long-running processes (LSP, kernels), streaming stdin/stdout/stderr (SSE, JSON-RPC), or process control (signals, kill, complex lifecycle).

When using `pipe` mode, cast the stream:

```typescript
const child = Bun.spawn(["cmd"], { stdout: "pipe", stderr: "pipe" });
const reader = (child.stdout as ReadableStream<Uint8Array>).getReader();
```

### Node module imports

Always use **namespace imports** for `node:fs`, `node:path`, `node:os`:

```typescript
import * as fs from "node:fs/promises";
import * as path from "node:path";
import * as os from "node:os";
```

- Async-only file → `node:fs/promises`.
- Needs both sync and async → `node:fs`, then `fs.promises.xxx` for async.

### File I/O

Prefer Bun:

```typescript
const text = await Bun.file(path).text();
const data = await Bun.file(path).json();
await Bun.write(path, data); // auto-creates parent dirs
```

Use `node:fs/promises` for directory ops (`fs.mkdir`, `fs.rm`, `fs.readdir`) — Bun has no native directory APIs. Avoid sync APIs in async flows; use sync only when forced by a synchronous interface.

**Anti-patterns:**

- `existsSync`/`readFileSync`/`writeFileSync` in async code → `Bun.file()` APIs.
- `mkdir(dirname(path), …)` before `Bun.write(path, …)` → redundant; `Bun.write` handles it.
- `if (await file.exists()) { await file.json() }` → two syscalls plus race. Use try-catch with `isEnoent`:
  ```typescript
  import { isEnoent } from "@harvest/pi-utils";
  try {
  	return await Bun.file(path).json();
  } catch (err) {
  	if (isEnoent(err)) return null;
  	throw err;
  }
  ```
- Multiple `Bun.file(path)` handles for the same path (including across `checkX`/`loadX` helpers).
- `Buffer.from(await Bun.file(x).arrayBuffer())` → `await fs.readFile(path)`.
- Existence check + try-catch around the same read → drop the existence check.

### Streams

Prefer centralized helpers:

```typescript
import { readStream, readLines } from "./utils/stream";
const text = await readStream(child.stdout);
for await (const line of readLines(stream)) {
	/* ... */
}
```

Manual reader loops only when the protocol requires it (SSE, streaming JSON-RPC).

### Misc

- **Sleep**: `await Bun.sleep(ms)`, never `new Promise(r => setTimeout(r, ms))`.
- **Password hashing**: `Bun.password.hash(pw, "bcrypt")` / `Bun.password.verify(pw, hash)`.
- **String width**: `Bun.stringWidth(text, { countAnsiEscapeCodes?: false })`.
- **Wrapping**: `Bun.wrapAnsi(text, width, { wordWrap, hard, trim })`.

## Model/Provider Policy Lives in KDL

**NEVER hard-code model- or provider-conditional policy in TypeScript.** No `id.includes("claude")`, no model-name regexes, no per-model lookup tables (effort ladders, pricing, context windows, modalities, API routing, quirk flags). All of it belongs in the KDL rule tree at `packages/catalog/src/compat/rules/`, compiled by `bun run gen:compat` into the committed `rules.json` and resolved at build time via `resolveModelPolicy`/`buildModel`.

Ownership strata (see `src/compat/rules/README.md`):

- `taxonomy/*.kdl` — identity: class membership, families, revision extraction, reviewed overrides, suffix collapse.
- `classes/*.kdl` — model-lineage truths (behavior inherent to a model line, on any host).
- `providers/*.kdl` — deployment contracts (behavior a host imposes), plus documented exact-id residue.
- `runtime/behavior.kdl` — heuristics that run before/outside exact model lookup (`api-routes`, `model-limits`, `exclude-models`, `pricing-peer`, hosted defaults).

Rules for TS code:

- Branching on model identity in TS is allowed **only** through structured facts from `classifyModel()` (`class`/`family`/`revision`/effort facts) — never through string matching on ids, and prefer a KDL axis when one can express the policy.
- Discovery mappers map authoritative upstream fields as reported; seed neutral values only for fields the upstream omits or misreports **and** KDL explicitly owns via a correction axis (`input-modalities`, `cost-patch`, `limits-patch`, `context-window-floor`, thinking axes). Assert rule-owned corrections through `buildModel`; raw discovery specs remain the right assertion surface for parsing/normalization contracts.
- An id that no selector can isolate gets an exact-id `models` residue rule with a comment — never a special case in TS.
- Equal-rank rule overlaps throw `AmbiguousOverlapError` at resolve time; fix with an explicit `priority=` in KDL, not code.
- After editing rules: `bun run gen:compat` and commit `rules.json` alongside the `.kdl` change.

## Generated Files

**NEVER edit `packages/catalog/src/models.json` directly.** It is generated from upstream sources (stencil.so, provider catalog discovery, OpenCode docs) by `packages/catalog/scripts/generate-models.ts` and the descriptors/resolvers in `packages/catalog/src/provider-models/`. Hand-edits get overwritten on the next regen. The same applies to `packages/catalog/src/compat/rules.json`, compiled from the KDL tree by `bun run gen:compat`.

To change an entry, fix the source:

- **Model/provider policy** (identity, thinking ladders, wire quirks, modality/limit/pricing corrections, API routing, roster exclusions) → the KDL tree in `packages/catalog/src/compat/rules/` (see the section above).
- **Provider catalog entries** (default model, discovery factory/flags) → the `CATALOG_PROVIDERS` table in `packages/catalog/src/provider-models/descriptors.ts`.
- **Discovery/request plumbing** (endpoint shapes, auth, response parsing) → the mappers in `packages/catalog/src/provider-models/openai-compat.ts`.
- **Generator wiring** (upstream merges, premium multipliers, post-processing order) → `packages/catalog/scripts/generate-models.ts`.

Regenerate with `bun run gen:compat` and/or `bun run gen:models` and commit the generated files alongside the source change. Add a regression test against the **rule/descriptor/mapper**, not the bundled JSON, so it survives upstream metadata shifts.

## Laya Decision Sidecar (hard invariants)

`decision-sidecar/` is a Python/FastAPI microservice on `127.0.0.1:8177` (fallbacks `8178-8185`) serving `POST /v1/decide` to the TS agent loop. Full contract: `decision-sidecar/README.md` + `ARCHITECTURE.md`.

- **Single checkpoint only**: load via `laya.load("convaiinnovations/laya-typed-decisions")`. Never import `Router` — it triggers multi-GB secondary downloads.
- **Fail-open, except gating**: sidecar offline/timeout/error → fall back to defaults (unpruned context, heuristic routing). Exception: tool-call gating fails **CLOSED** (require human approval).
- **Bind loopback only**; never expose `/v1/decide` (unauthenticated) on `0.0.0.0`. Same rule applies to metaharness, auth-gateway/broker, and stats servers — keep `127.0.0.1` defaults, require a token for any non-loopback bind.
- **Pruning must preserve protocol blocks**: never strip `toolCall` blocks while leaving `toolResult` messages orphaned — providers reject the replay with 400. Exclude mixed messages from candidacy or preserve non-text blocks.
- Setup heals 8 enumerated failure modes only (torch wheel, symlink restriction, port conflict, corrupt checkpoint, router invocation, disk space, stale calibration, missing python) and logs to `~/.harvest/agent/logs/laya-setup.log`. Never invent new remediation classes; unrecognized failures emit a diagnostic bundle and fail open.

## Security Invariants

From prior code reviews (`CODE_REVIEW*.md`) — re-verified gaps agents reintroduce:

- **Workspace jail**: every file-mutation entry point (write, edit incl `op === "delete"`, archive/SQLite subpaths, LSP `workspace/applyEdit` URIs, native `local://` sandbox) must pass `assertPathJailed` (or the Rust `PathPolicy` equivalent) *before* branching by operation. For nonexistent leaves, resolve + validate the deepest existing ancestor — `realpath` on the full target fails and falls back to a lexical check that symlinked parents defeat. Reuse `jailCheck.resolvedPath`; check-then-reopen on the lexical path is a TOCTOU.
- **No `console.*` on TUI/RPC/worker paths** (see Logging below); no unbounded buffers on network/child-output paths — cap response bodies, log tails, CSI sequences, and checker output.

## Logging and CLI Output

Code that may run while the TUI, RPC, SDK, workers, or background runtimes are active MUST NOT use `console.log`/`error`/`warn`; it corrupts rendering or protocols. Use the centralized logger:

```typescript
import { logger } from "@harvest/pi-utils";

logger.error("MCP request failed", { url, method });
logger.warn("Theme file invalid, using fallback", { path });
logger.debug("LSP fallback triggered", { reason });
```

Logs go to `~/.harvest/logs/harvest.YYYY-MM-DD.log` with automatic rotation. Standalone CLI commands that exit without entering the TUI MAY use `console.*` or process streams for intentional user-facing output. Keep structured stdout clean. This exception is semantic, not filename-based; shared code must use `logger` or an explicit output sink.

## TUI Sanitization

All text displayed in tool renderers must be sanitized. Raw content (file contents, error messages, tool output) breaks terminal rendering: tabs → visual holes, long lines → overflow, paths → leak home directory.

**Rules:**

- **Tabs → spaces** via `replaceTabs()` (from `@harvest/pi-tui` or `../tools/render-utils`).
- **Truncate** lines with `truncateToWidth()` / `ui.truncate()`. Use `TRUNCATE_LENGTHS` constants.
- **Shorten paths** with `shortenPath()` (replaces home with `~`).
- **Preview limits** from `PREVIEW_LIMITS`. No ad-hoc numbers.

**Apply to every render path**, not just the happy one:

- Success output (file previews, command output, search results).
- **Error messages** — these often embed file content (e.g., patch failure messages include unmatched lines). If a message contains file content, it needs `replaceTabs()`.
- Diff content (added and removed).
- Streaming previews.

### Streaming tool previews

Tool-call previews can have **multiple render paths**. If you add preview-only fields or depend on partially streamed args, update every path — not only the final renderer. Streamed argument buffers decode into display args via `decodeStreamedToolArgs` / `ToolArgsRevealController` (`modes/controllers/tool-args-reveal.ts`); both the live event path and transcript rebuilds must go through them — never spread provider-parsed `arguments` next to a raw `__partialJson` (parsed args lag the stream by a throttled parse window).

For the bash tool specifically:

- The pending preview may need raw `partialJson`, not just parsed `arguments`. Parsed args lag until a JSON object closes, which makes inline env assignments appear only at the end.
- Preserve preview-only fields (e.g. `__partialJson`) through `event-controller.ts`, transcript rebuilds in `ui-helpers.ts`, and merged call/result rendering in `tool-execution.ts`. Missing one path causes inconsistent previews.
- `ToolExecutionComponent.#buildRenderContext()` for bash must work even before a result exists — the renderer uses call args plus render context to show the command preview while streaming.
- Verify both live streaming and rebuilt transcript paths after any bash preview change. A fix in one path does not fix the other.

## Commands

- NEVER commit unless asked.
- Never use `tsc`/`npx tsc` — the typecheck gate is `bun run check` (root: `check:ts` + `check:rs`; in `packages/coding-agent/`: oxlint + oxfmt + `tsgo -p tsconfig.json --noEmit` via `check:types`).
- Never run `cargo test` directly for Rust tests — use `bun run test:rs`. It runs `cargo nextest run` (config: `.config/nextest.toml`) followed by a `cargo test --doc` pass, because nextest does not execute doctests. The doctest pass currently executes nothing (pi-natives is a `cdylib`, which rustdoc skips; pi-builtins' examples are `ignore`d vendored uutils docs) and exists so the first runnable doctest added to a lib crate is actually run.
- Fresh clone: `bun setup` (installs workspaces + builds `@harvest/pi-natives`), then `bun dev`. Non-interactive smoke: `bun dev -- --version`. Re-run `bun run build:native` after changing Rust crates or `packages/natives`.
- Focused verification: `bun --cwd=packages/coding-agent run check` (gate), `run check:types` (types only), `run lint`, `bun test <path>` (single file). Full TS suites go through `bun scripts/ci-test-ts.ts <suite>` — never bare root `bun test` (it walks the whole tree; `bunfig.toml` only prunes known scratch dirs). New worker kinds: validate with `harvest --smoke-test` (wired into `ci:test:smoke`).
- Codegen after editing sources: `bun run gen:compat` (KDL → `rules.json`), `bun run gen:models` (catalog → `models.json`), `bun run gen:nix` only when `bun.lock` changes (never edit `nix/bun.nix` manually). Python: `bun run lint:py` / `fix:py` (ruff).
- Merge commits follow standard conventional commit subjects: `Merge PR #<number>: <conventional PR subject>`.
- PRs (per `CONTRIBUTING.md`): one logical change, no drive-by refactors; every PR body needs a human-written sentence explaining what/why (generated summary alone fails review); verify the changed path yourself and report the scenario + result (`bun run check` passing is not proof). If you intend to implement work yourself, do not file an issue for it first — robomp treats actionable issues as pickup work. Major features/architecture changes need prior discussion before implementation.
## Rust Build Profiles

Profiles live in the root `Cargo.toml`; `.cargo/config.toml` carries what Cargo.toml cannot express. Both are committed — no local `~/.cargo/config.toml` needed.

| Profile | Use |
| --- | --- |
| `dev` | Default. Line tables for our crates, no debuginfo for deps, deps at `opt-level = 2`. |
| `release` | Shipping: fat LTO, 1 codegen unit, stripped. |
| `local` | Fast local release iteration: thin LTO, 16 codegen units, incremental. |
| `profiling` | `release` codegen with symbols kept, for `perf`/`samply`/Instruments. |
| `ci` | Thin LTO, no debuginfo, stripped. |

Gotchas: never `split-debuginfo = "off"` on a profile with debuginfo (Mach-O backtraces silently lose `file:line`); keep `embed-metadata = false` in sync with `rust-toolchain.toml`. Rejected with measurements — do not relitigate: sccache (slower, can't cache incremental/proc-macro), mold (ELF-only), `panic = "abort"` on `dev` (doubles dep graph, 131 → 214 MB).

## Testing Guidance

Test the contract the system exposes — not the easiest internal detail to assert.

- Every new test must defend one **concrete, externally observable contract**: behavior, output shape, state transition, error mapping, or a regression-prone parsing boundary. If you cannot name the contract, do not add the test.

### Good vs. bad test filter

- **Name the failure mode.** Every test MUST state what a consumer observes if it regresses. Cannot name one? NEVER add it.
- **Good: transformation.** One fixture MAY prove parse/render/normalize/encode/resolve behavior when output is computed, not echoed.
- **Good: branch or boundary.** Distinct inputs, empty values, malformed input, version/provider routing, and state transitions MUST prove distinct outcomes.
- **Good: external contract.** Exact bytes/shape MAY be asserted when a provider, parser, protocol, or persisted consumer reads them.
- **Good: precedence or negative contract.** Keep explicit `false`/override-wins assertions and required absence only when they prevent a documented leak, downgrade, 400, or incompatible wire field.
- **Good: regression.** A repro MUST trigger the prior real failure path and assert the corrected observable result.
- **Bad: static echo.** NEVER test a constructor/builder merely copied a fixture or baked constant into an in-memory config/metadata field.
- **Bad: success passthrough.** NEVER assert `fn(x) === x` when `x` was already supplied/declared valid; assert a transform, rejection, or downstream effect instead.
- **Bad: wording/defaults.** NEVER assert prompt/UI boilerplate, a default literal, object existence, non-empty output, or length growth without a consumer contract.
- **Bad: duplicate rows.** Parameterized/loop rows MUST each cover a distinct branch, provider/model path, or consumer contract; delete same-path duplicates.
- **Metadata exception.** Exact metadata, identity, ordering, or `undefined` MAY remain only when a downstream consumer depends on it and the test establishes branch, precedence, negative-contract, wire, or regression evidence.
- **Termination exception.** For cyclic/large inputs, assert a bounded output, surfaced error, or state change; bare `not.toThrow()` is insufficient.
- No placeholder tests, tautologies, or "the code ran" assertions (`expect(true).toBe(true)`, bare `not.toThrow()`, non-empty string checks, length-grew checks, "prompt exists" checks without semantic assertion).
- Prefer contract-level tests over implementation details. Avoid asserting internal helper wiring, field assignment, singleton identity, incidental ordering, prompt boilerplate, or passthrough option forwarding unless another component depends on that exact detail.
- Don't duplicate coverage across abstraction levels. If an integration test already proves the behavior, drop the narrower unit test that restates it through mocks.
- Tests **must be full-suite safe**, not just file-local safe. No long-lived file-wide mutations of `Bun.*`, `process.platform`, `process.env`, or `Bun.env` when a narrower seam exists. Prefer per-test `vi.spyOn(...)` with `vi.restoreAllMocks()` in `afterEach`. A test that passes alone but poisons later files is broken.
- **Never use `mock.module()`**. Bun's `mock.module()` mutates the global module registry and leaks across files ([oven-sh/bun#12823](https://github.com/oven-sh/bun/issues/12823)). Use `spyOn` on the imported module object instead. For pass deps, import the pass and spy on `.run`. For package deps, namespace-import and spy on the exported function.
- For lifecycle/stateful code, prefer one test per invariant or transition over several tiny tests asserting one field each from the same transition.
- For error handling, trigger the real failure path and assert the surfaced contract — don't instantiate error classes directly or inspect internal metadata.
- Smoke tests are acceptable only when they catch a failure mode narrower tests would miss. "Package boots" or "command starts" alone is not enough.
- Assert exact strings, ordering, and formatting only when downstream code parses or depends on the exact bytes. Otherwise assert semantic content.
- Compile-time guarantees → type checks/type tests, not runtime placeholders.
- **Never source-grep.** A test that reads an implementation file (`.ts`/`.rs`/build script) and asserts on its _text_ — `expect(src).toContain("someCall()")`, `.toMatch(/import .../)`, `.not.toContain("oldName")`, or "comment must say X" — is banned. It tests how code _looks_, not what it _does_: it breaks on harmless refactors (comment reflow, rename, import reorder) and passes while the behavior is broken. Assert the observable contract instead (run the code, check output/state/error), use the runtime smoke probe for wiring you cannot exercise in-process, and enforce structural invariants (no value-import of X, no self-import) with a type test or an oxlint rule — never a string scan of the source. (Reading a file your code _wrote_ — apply-patch result, generated bundle, temp fixture — and asserting on that output is fine; that is behavior, not a source grep.)
- Don't add tests for tiny low-risk changes unless they protect a real contract or fix a regression-prone edge case.
- Prefer focused package-local verification for the changed area.

## Changelog

Location: `packages/*/CHANGELOG.md` (per package).

**Format** — sections under `## [Unreleased]`:

- `### Breaking Changes` (first if present)
- `### Added`
- `### Changed`
- `### Fixed`
- `### Removed`

**Rules:**

- New entries always go under `## [Unreleased]`.
- Entries are one line, brief, and user-facing: lead with what the user will see or can now do. Root-cause narration and implementation detail belong in the commit/PR, not the changelog.
- Never modify already-released sections (e.g., `## [0.12.2]`) — they are immutable.
- Don't flag changelog section order or formatting in reviews or PRs — `bun run release` runs `fix-changelogs` which normalizes everything automatically.

**Attribution:**

- Internal (from issues): `Fixed foo bar ([#123](https://github.com/harvest/harvest/issues/123))`.
- External contributions: `Added feature X ([#456](https://github.com/harvest/harvest/pull/456) by [@username](https://github.com/username))`.

## Releasing

1. Ensure all changes since last release are in each affected package's `[Unreleased]` section.
2. Run `bun run release`.

The script handles version bump, CHANGELOG finalization, commit, tag, publish, and adding new `[Unreleased]` sections.
