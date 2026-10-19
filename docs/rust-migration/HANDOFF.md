# TypeScript-to-Rust migration: agent handoff

Prepared on **2026-10-20**, for the user to give to another developer or coding agent.

## Read this first

Continue the existing migration on branch **`ts-to-rust`**. The implementation
checkpoint before this handoff is **`6253f50`** (`mid process`). Preserve existing
commits and user changes; do not reset the branch or restart the implementation.

**The original task is not complete.** The normal Harvest application still
launches TypeScript. The Rust application is experimental and has a substantially
smaller contract. Completed foundations, matching command names, green unit tests,
or a compiling binary must not be represented as full application parity.

The user wants the entire original task completed professionally, with verified
incremental commits. They explicitly authorized commits. They have been frustrated
by repeated partial-completion responses; work autonomously, but report actual
limitations honestly. Their latest instruction is to finish the current slice and
produce this handoff so another person/agent can continue.

Give the next agent this file **and access to the repository/branch**. This file
alone does not contain the implementation. This assistant session has not pushed,
published, created a PR, or released anything; independently confirm which commits
are available on the remote before handing over a clone.

## Exact progress at the checkpoint

Fresh commands at `6253f50` report:

- `cargo xtask migration`: **10 / 5,221 source entries migrated and verified**.
- **5,211 source entries have not passed all migration/removal gates.**
- **5,197 entries still require individual contract review**; 18 entries currently
  have a partial implementation. These are source-ledger states, not feature-weighted
  estimates of effort.
- `cargo xtask capabilities`: **16 / 187 capabilities have partial or complete
  Rust implementations**; **0 / 187 have verified full legacy parity**.
- All 187 capabilities still need their complete parity acceptance satisfied.
  171 do not yet have an implementation recorded in that inventory.
- Tracked extensions: **4,893 `.ts`, 115 `.tsx`, 16 `.js`, 3 `.mjs`, 183 `.py`,
  and 1 `.jl`**; zero `.mts`/`.cts`. Declaration files are included in `.ts`.
  Counts include fixtures/generated material; handling those is part of the task.
- The working tree was clean before writing this document. There is no unfinished
  SIXEL source edit. Only local baseline fixtures for that next slice were captured.

Do not convert these counts into a claimed percentage of whole-project completion.
The remaining provider, UI, lifecycle, service and platform work dominates effort.

### Sources of truth

Read these before continuing:

1. Repository `AGENTS.md` and `packages/coding-agent/DEVELOPMENT.md`.
2. `docs/rust-migration/README.md`: detailed implemented work and limitations.
3. `docs/rust-migration/capabilities.json`: 187 retained capability contracts.
4. `docs/rust-migration/source-baseline.json` and `source-ledger.json`:
   durable file inventory, replacement/retirement destinations and evidence states.
5. `docs/rust-migration/distribution.md` and `brain.md`.
6. Executable registries, build scripts and tests. When prose conflicts with
   executable source, trust the latter and correct the prose.

The ledger deliberately retains original entries after deletion. Do not remove
inventory rows, relax gates, or mark partial work verified to improve the numbers.
Most ledger rows still lack a reviewed contract; that review remains real work.

## What has been implemented

These are verified slices, not a claim that all consumers have migrated.

### Shared runtime and application foundations

- `crates/harvest-runtime`: jailed filesystem access, bounded I/O, cancellation,
  locks, atomic replacement, streaming atomic writes and related security helpers.
- `rust-agent`: an experimental Rust CLI/SDK, shared command aliases, bounded
  OpenAI-compatible HTTP/SSE handling, lossless usage preservation/normalization,
  redacted credential/transport diagnostics, approved shell support in default
  builds, and native traversal-backed grep.
- Session work includes v3 title updates preserving message bytes, legacy import
  paths, interrupted-tail recovery to separate validated files, text-compaction
  replay, context resets, handoffs and branch-summary handling. Read the status
  docs/tests for exact boundaries; this is not the full legacy AgentSession.
- `harvest-brain` has native memory maintenance foundations. The complete memory
  backend set and automatic-learning lifecycle are not replaced.
- Existing mature Rust shell/edit/walker/VCS/AST engines remain available. Reuse
  them; do not build a second weaker implementation beside a sanctioned helper.

### Terminal and tokenizer foundations

- `crates/harvest-tui`: shared keyboard/Kitty protocols, paste decoding, bounded
  input framing and Escape deadlines, output chunking, stalled-consumer policy,
  bounded terminal writer, ANSI/Unicode text geometry, rectangular-fill planning,
  syntax highlighting/stream state, grammar assets and SGR mouse decoding.
- The inline terminal renderer, widgets, capability discovery, lifecycle and host
  scheduling still largely live in TypeScript. Shared primitives do not establish
  full interactive TUI parity.
- The DECCARA planner is an exported SDK helper; it is not currently used by the
  main in-tree renderer. Unsupported/decorated rows retain original content.
- Unpaired-surrogate width now matches replacement cells actually written by the
  terminal; actual Kitty terminal-core checks cover the correction.
- Mouse decoding preserves the existing optional-addon compatibility path. The
  TypeScript parser/routing is retained; no source-removal completion is claimed.
- `crates/harvest-tokenizer`: the existing offline engines/vocabularies are shared
  directly with the SDK and N-API. Eight vocabulary blobs were preserved. Fifteen
  stale Claude fixture counts were corrected against pinned `ctok==1.0.0` after
  checking all 1,479 reference expectations; the tokenizer algorithm was unchanged.
- Small bridge overheads were measured and documented. There is no claimed
  whole-application speedup.

### Most recently completed slice: HTML and SVG

Checkpoint `6253f50` contains `crates/harvest-media`:

- Ordinary Rust `html::html_to_markdown` and `svg::rasterize_svg`, exported as
  `harvest_omp::media`. HTML uses the existing Tier2 backend and rejects
  depth-truncated output. SVG preserves canvas limits, aspect-ratio scaling,
  embedded images and disabled external image resolution.
- N-API adapters retain their existing input ownership and blocking-pool behavior.
- Workspace, Cargo lock, Bazel dependency graph and Rust CI package lists updated.
- **8 media Rust tests + 1 doctest**, **252 combined shared Rust tests**, actual
  **Bazel 8-test execution**, strict Cargo/native/SDK and Bazel Clippy checks passed.
- Standalone installed Rust, with no JS executable on PATH, and rebuilt N-API each
  matched **40 HTML conversions and 20 SVG render/error cases** byte-for-byte.
- **8 native HTML tests**, **26 focused fetch/Git-preview tests**, package gates
  and formatting checks passed.
- All final checks completed successfully, including ones finishing after the
  checkpoint commit. The native addon was rebuilt; 130 existing exports remained.
- Full image loading, fetch transport/fallback orchestration, inference and
  terminal integration remain TypeScript or otherwise unfinished.

### Rust tooling and distribution

- `cargo xtask` owns Rust test/check orchestration (nextest plus doctests),
  checksums, curated release notes, workspace version handling, development
  commands, credential scanning, package/install validation, migration gates,
  Bazel Clippy configuration, embedded theme registry generation, ELF version
  definition repair, and emitted ESM extension repair.
- Theme generation preserves 98 current public names/palettes and handles reserved,
  colliding, quoted and prototype names. The old stale defaults path was fixed.
  Its generated TS registry remains until the frontend migrates.
- The ESM repair tool is wired into the retained package publisher. Its installed
  binary matched 26 actual emitted omptype files/52 rewrites; a Node ESM consumer
  loaded the repaired graph and correctly accepted/rejected schema inputs. The
  helper itself requires no Bun/Node. The enclosing publisher is still TypeScript.
- Distribution work unifies eight intended artifact targets, real `omp`/`harvest`
  aliases, ZIP legal payloads/checksums and installer expectations. Default Rust
  features include shell support; workspace version/path assumptions were fixed.
- Actual GNU Linux x64 and ARM64 application artifacts were built with Bazel and
  checked against glibc 2.17; ARM64 execution was **emulated**, not native hardware.
  These staged artifacts predate the latest foundations; do not claim the newest
  source has been fully rebuilt/verified across shipping targets.
- ARM64 Cortex-A53 erratum protection is preserved, with a controlled link/runtime
  reproducer; it was not dropped to make linking pass.
- A clean installed Nix Linux x64 package build exercised a real auto-patchelf
  relocation. Rust ELF repair corrected `DT_VERDEF`; installed CLI version/help
  passed. This package still contains the legacy application and requires Bun.
- There is still competing legacy release ownership. Cutover gates intentionally
  block shipping the incomplete Rust replacement.

### The ten removed, verified source entries

- `rust-agent/scripts/check_secrets.py`
- `scripts/ci-release-checksums.test.ts`
- `scripts/ci-release-checksums.ts`
- `scripts/ci-release-notes.ts`
- `scripts/fix-dt-verdef.test.ts`
- `scripts/fix-dt-verdef.ts`
- `scripts/fix-emit-extensions.ts`
- `scripts/gen-clippy-bazelrc.ts`
- `scripts/run-rs-task.ts`
- `scripts/sync-themes.ts`

Their replacement/evidence details are retained in `source-ledger.json`.

## Remaining work: full acceptance checklist

Everything below needs contract review, implementation, behavioral evidence,
installed evidence and legacy removal where applicable. Use the inventories for
specific command/tool/provider/file names; this list is not a replacement for them.

1. **Finish file-level mapping.** Document actual behavior, consumers, persisted
   formats, dependencies, Rust destinations, tests and removal paths for every
   remaining source entry. Resolve fixtures/declarations/generated code explicitly.
2. **Finish shared native extraction and security/error/cancellation integration.**
   The latest extraction is HTML/SVG, not all of `pi-natives`. Preserve mature
   editing, traversal, VCS, shell, locking and sandbox behavior across consumers.
3. **Settings, schemas, KDL/catalog and all providers.** Preserve profile precedence,
   migrations, defaults, credentials/OAuth refresh ownership, model identity/policy,
   streaming/error/retry/usage contracts, specialized transports and protocol data.
   The small OpenAI-compatible path is not parity with the retained provider set.
   Explicit protocol roots already replace TS-import-based discovery, but the
   protocol compiler/generator and generated consumers are still largely TS.
4. **Full lifecycle, sessions, compaction and memory.** Complete the actual legacy
   agent/session state machine, hooks, cancellation, queues, retries, persistence,
   context/protocol-block preservation and all retained memory backends.
5. **Rust-compatible plugin/custom-tool SDK.** Complete its process interface,
   discovery, permissions, cancellation, examples and first-party integrations.
   Arbitrary executable JS plugins/tools and JS evaluation are intentionally
   breaking/retired; make migration errors actionable. Do not silently retain a JS
   runtime fallback or claim persistent Rust evaluation parity without proof.
6. **All CLI/tools/protocol modes and the complete terminal frontend.** Include
   command/hidden-tool/slash-command/worker registries, MCP, RPC, ACP, LSP/DAP,
   delegation/goals/advisor, SSH/browser/security operations. Preserve approval
   against final revised arguments, bounded framing and lifecycle semantics.
   Port the renderer/widgets, inline scrollback, streaming previews, Unicode,
   keyboard/mouse/images, resize/backpressure/Windows restoration behavior.
7. **Background services, media and inference.** Replace worker/process paths for
   stats, embeddings, speech, tiny models, brokers and desktop/computer interaction.
   Verify real models, tokenizers, dimensions, normalization, devices, caches,
   downloads and supported platforms. Preserve embedding identity or implement an
   explicit re-embedding migration.
8. **Browser UIs and exported HTML.** Stats, collaboration, metaharness, robomp,
   browser relay and session exports need Rust UI/server logic. Prototype the real
   UX before selecting a framework. Allow only inventoried, narrowly scoped browser
   bootstrap/API glue where the browser platform requires JavaScript.
9. **Python/Julia/automation/benchmarks/examples.** Port robomp queue/database/RPC/
   proxy/dashboard and analysis/project tools. Preserve deduplication, serialization,
   retries, cancellation/draining, worktrees, authorization and HMAC boundaries.
   Preserve benchmark semantics; parser/formatter changes need versioned baselines.
   Foreign-language fixtures must become data assets materialized temporarily when
   literal zero tracked TypeScript files is required.
10. **Complete build/install/release cutover and platform proof.** Finish remaining
    setup/codegen/publishing scripts, Docker/Nix/updaters/installers/Homebrew/WinGet,
    aliases/legal files and documentation. Have one release owner and no Bun/Node
    requirement in normal native build/test/release. Verify current shipping artifacts
    across all eight intended targets, including native platform workflows rather
    than treating emulation/parser checks as hardware proof. Cargo publication
    closure and Windows static-CRT/cross-platform evidence still need completion.

Final acceptance is the original request at the end of this document. Do not
substitute the experimental Rust CLI for the complete product before these gates.

## Immediate next slice that was being investigated

**SIXEL extraction and encoder allocation bounds. No repository source change was
started for it.** The code remains in `crates/pi-natives/src/sixel.rs` (191 lines).

Recommended sequence:

1. Review current encode/decode contracts and callers. Decode already has bounded
   preflight dimensions, payload limits and PNG output limits. Encoding checks zero
   target sizes, but large target dimensions currently reach resize/allocation;
   its source decode also uses broad dependency defaults.
2. Move ordinary encode/decode logic into `harvest-media::sixel`, preserving valid
   frame/PNG bytes and error mapping. Keep a thin N-API adapter while TS uses it.
3. Add explicit source/target/input/allocation bounds **before** decoding/resizing.
   Read the installed `image` crate types; `Limits` is non-exhaustive and its
   allocation limit is not strict for every decoder. Check metadata/dimensions and
   total output bytes before allocating. Do not demonstrate the old bug by asking
   the old encoder to allocate enormous images.
4. Add meaningful Rust boundary/roundtrip tests; verify rebuilt addon, host image
   consumers, normal-output parity, error recovery, Bazel/Cargo/CI wiring and docs.
5. Commit only after passing the appropriate checks. Keep media capability partial
   until its complete scope is verified.

Local-only prepared evidence, if continuing on this machine:

- `/home/namannn04/.cache/harvest/sixel-baseline.rs`
- `sixel-inputs.json`, `sixel-capture.ts`, `sixel-capture.log`, `sixel-oracle.json`
  in that same cache directory.
- The oracle contains **9 encoded frames and 13 decoded frames/errors** from the
  previous addon. It is supporting evidence, not committed CI coverage.
- `media-baseline.node` is a frozen pre-extraction addon suitable for these SIXEL
  comparisons. These cache files are not needed to run the committed Rust tests.

A different machine will not have these caches. Reconstruct baseline comparisons
from Git/the preceding addon build; do not claim local-only artifacts are present
in a fresh checkout.

## Development and verification commands

Run from the repository root. Read `AGENTS.md` for the full rules.

```sh
export PATH="$HOME/.cargo/bin:$PATH"
export RUSTUP_TOOLCHAIN=nightly-2026-08-08

cargo xtask migration
cargo xtask capabilities
cargo xtask test -p harvest-media -p harvest-tui -p harvest-tokenizer -p harvest-runtime -p harvest-omp
cargo clippy -p harvest-media -p harvest-omp --all-targets -- -D warnings
cargo fmt -p harvest-media -p harvest-omp -- --check
cargo xtask clippy-config --check
cargo xtask theme-index --check
```

At the checkpoint, the completion gates below **must fail** because the migration
is unfinished. Never weaken them merely to make a release proceed:

```sh
cargo xtask migration --require-complete
cargo xtask capabilities --require-complete
```

Important rules:

- Use `cargo xtask test` (or mandated `bun run test:rs` for the legacy path), not
  direct `cargo test`; nextest must be followed by doctests.
- Use package `bun run check`/`check:types`; never `tsc`/`npx tsc`.
- Explicit-file `bun test PATH` is allowed. Full TS suites use
  `bun scripts/ci-test-ts.ts SUITE`; never bare root `bun test`.
- Tests must prove observable contracts. No source-grep tests, `mock.module`,
  placeholders or long-lived global environment/Bun mutations.
- Keep filesystem mutations jailed, use pinned/resolved handles, bound child/network
  output, preserve tool-call/result protocol blocks, and keep terminal/RPC output clean.
- Do not hardcode provider/model policy in TS; preserve and port KDL ownership.
- Prompts remain static Markdown resources. Never hand-edit generated catalog data.
- Existing session authorization permits incremental commits. Do not push/release,
  create GitHub issues/comments or message external people without authorization.
- No subagents unless the user or applicable instructions explicitly request them.
- Do not reformat the entire `pi-natives` crate: unrelated audio/AST/crash-handler
  formatting churn exists. Format only changed native files with `rustfmt`.
- Native binding regeneration currently adds four unrelated audio declaration lines
  (`AudioCapture.droppedChunks`, `AudioPlayback.queuedSamples` and their comments).
  Remove only those unrelated generated additions before committing this migration.
- Use `set -e` for sequential verification commands; inspect actual exit codes/logs.

### Local environment notes

The default installed Bun is **1.3.10**, below the repository minimum **1.3.14**.
A verified 1.3.14 binary is cached at:

`/home/namannn04/.cache/harvest/bun-supported/bun`

Use it for TS checks on this machine. It was installed only in the cache; the
user's global Bun was not replaced. The existing `NO_COLOR=true` / `TERM=dumb`
environment is intentional context for tests: use explicit per-test color modes
or subprocess-only environment overrides instead of globally mutating test state.

Native rebuild on this machine:

```sh
export PATH=/home/namannn04/.cargo/bin:/home/namannn04/.cache/harvest/bun-supported:/home/namannn04/.cache/harvest/native-build-tools/cmake/cmake/data/bin:/home/namannn04/.cache/harvest/native-build-tools/ninja/ninja-1.13.2.data/scripts:$PATH
export RUSTUP_TOOLCHAIN=nightly-2026-08-08
export LIBCLANG_PATH=/home/namannn04/.cache/harvest/native-build-tools/libclang/libclang-18.1.1.data/platlib/clang/native
bun run build:native
```

Actual Bazel checks used for the latest media slice:

```sh
/home/namannn04/.cache/harvest/bazel-tools/bazel --output_user_root=/home/namannn04/.cache/harvest/bazel-user test //crates/harvest-media:media_test --jobs=2 --noshow_progress
/home/namannn04/.cache/harvest/bazel-tools/bazel --output_user_root=/home/namannn04/.cache/harvest/bazel-user build //crates/harvest-media:harvest-media --config=clippy-strict --jobs=2 --noshow_progress
```

The host has about 14 GiB RAM and shared user applications. Avoid concurrent
heavy compilers; do not stop user processes. `target/portable/omp` and
`target/portable/omp-arm64` are older staged artifacts, not current-build evidence.
An owned Nix verification container may remain; its metadata is in
`/home/namannn04/.cache/harvest/nix-build-metadata.json`. Do not remove unrelated
user containers or rely on the frozen Nix source snapshot as the current tree.

### Local evidence locations

Supporting logs/probes are under `/home/namannn04/.cache/harvest/`; the most recent
families are `media-*`, `emit-extensions-*`, `mouse-*`, `highlight-*`,
`deccara-*`, `terminal-text-*`, `tokenizer-*`, `theme-generator-*` and `elf-*`.
Prefer `*-final.log` where available: earlier attempts can contain fixed failures.
Notable authoritative records:

- `handoff-migration.log`, `handoff-capabilities.log`: fresh checkpoint counters.
- `media-core-tests-final.log`, `media-shared-tests.log`, `media-bazel-tests.log`,
  `media-bazel-clippy.log`, `media-core-oracle.log`, `media-addon-oracle-final.log`,
  `media-native-build-final.log`, `media-integration.log`.
- `emit-extensions-tests-final.log`, `emit-extensions-clippy-final.log`,
  `emit-extensions-package-proof.log`, `emit-extensions-oracle.log`.
- `elf-nix-installed-proof.log`: natural Nix relocation/repair and installed CLI.
- `elf-runtime-proof-final.log`: controlled x64/emulated ARM64 loader recovery.

These support historical claims. Re-run relevant checks after changes, and obtain
new installed/platform evidence before making new parity or release claims.

## Recent commit landmarks

- `6253f50`: HTML/SVG shared media crate, adapters, SDK, Bazel/CI/docs.
- `1b0339a`: Rust ESM extension repair; old helper deleted.
- `ed48b39`: Rust SGR mouse decoder and compatibility/routing tests.
- `5154dff`: shared syntax highlighter and unchanged grammar assets.
- `6d9941b`: shared conservative rectangular-fill planner.
- `9e8950d`: split-surrogate width matches actual terminal cells.
- `fdea9f1`: shared ANSI/Unicode text geometry.
- `ba0bd01`: shared tokenizer engines/vocabularies and corrected stale fixtures.
- `68dc361`, `5f0a7e8`: Rust theme generator and empty-stem boundary fix.
- `f8e94e9`, `b909faf`: installed Nix proof and bounded Rust ELF repair.
- `c124d63`: Rust Bazel lint configuration generator.
- `556e1b7`, `ad7d7ba`, `5f55899`: terminal writer, output policy and input framing.
- `7d5a4fb`, `7a96bda`, `8c63c76`: GNU portability, erratum protection and packaging.

## Suggested first prompt to the next agent

> Read this handoff, AGENTS.md and the migration inventories. Continue branch
> ts-to-rust from its existing commits. Verify the actual tree first. The normal
> application is still TS and no whole capability has verified legacy parity.
> Finish the original full-workspace migration, committing verified slices and
> deleting legacy sources only after consumer and installed parity. The immediate
> investigated slice is SIXEL extraction/encoding bounds; no SIXEL source changes
> are pending. Do not reset work, weaken gates, claim completion from primitives,
> or silently replace retained features with the experimental CLI subset.

## Original assigned plan (verbatim)

The following is the user-supplied original plan. Historical `/workspace/harvest`
links and the audit commit refer to its starting context; apply it to this checkout
and verify current source instead of assuming the old findings remain unchanged.

---

The plan is to extend the existing Rust workspace until it replaces every first-party TypeScript subsystem, including the CLI, libraries, extensions, browser clients, tests, generators, and release tooling.
Your confirmed compatibility decision is part of this plan: remove the bundled JS runtime and require extensions and custom tools to migrate to a Rust-compatible interface, accepting a breaking release.
This was a read-only audit of /workspace/harvest at commit 80ab442. No files changed, and no test suites or live provider checks were run.
The verified starting point is:
These findings are reflected in the existing [migration status (line 1)](/workspace/harvest/docs/rust-migration/README.md:1), [Cargo workspace (line 1)](/workspace/harvest/Cargo.toml:1), and [capability gate (line 10)](/workspace/harvest/xtask/src/capabilities.rs:10).
1. Establish a complete migration and deletion ledger.
   Expand the existing capability inventory into a map covering every executable entrypoint and tracked source file. Include packages, scripts, workers, project tools, examples, declarations, generated outputs, browser assets, and first-party Python/Julia utilities.
   Each entry should record its current behavior, consumers, persisted formats, external dependencies, Rust destination, verification requirements, and files to delete.
   Track implementation, behavioral verification, installed verification, and legacy removal separately. Record features intentionally retired under the breaking release explicitly. Removing an inventory row must never make unfinished work appear complete.
   Exit condition: every first-party TS file has a documented replacement or justified retirement path.
2. Finish the shared Rust foundations and extract reusable native code.
   Reuse the existing filesystem jail, bounded I/O, cancellation, locks, atomic replacement, shell, editing, parsing, traversal, VCS, and voice engines.
   Some mature implementations remain coupled to N-API callbacks and types. Extract their logic into ordinary Rust libraries with Rust APIs. Keep thin N-API adapters during transition while TypeScript callers still need them.
   The experimental editor currently supports a much smaller contract than the existing [`pi-edit` engine (line 1)](/workspace/harvest/crates/pi-edit/src/lib.rs:1). Wire that engine into the Rust application before replacing TS editing orchestration.
   Expand the existing xtask for migration checks, code generation, and developer commands.
   Exit condition: the Rust application can call the required native functionality directly, with shared errors, limits, logging, cancellation, and security policies.
3. Port settings, schemas, catalog policy, credentials, and providers.
   Replace TypeScript configuration resolution and validation with Rust implementations while preserving profile precedence, settings migrations, defaults, and tool wire schemas. The callable JavaScript schema DSL can retire with the old SDK; its externally observable validation behavior still needs coverage.
   Preserve KDL model policy, .proto definitions, generated catalog data, and static prompts. Port their compilers and generators.
   One concrete deletion dependency needs attention: the current [protocol generator (line 52)](/workspace/harvest/packages/catalog/scripts/generate-protocols.ts:52) discovers required messages by scanning TypeScript imports. Replace that discovery mechanism before deleting its consumers.
   Port provider families individually, including streaming events, usage, retry behavior, OAuth refresh ownership, credential storage, account identity, and specialized transports. The current Rust OpenAI-compatible path does not establish compatibility with the other implementations.
   Exit condition: each retained provider has contract evidence; credential-dependent live checks are reported separately from fixture tests.
4. Complete the agent lifecycle, sessions, compaction, and memory.
   Extend the existing Rust AgentSession to cover scheduling, steering, cancellation, hooks, retries, tool execution, delegation, compaction, and committed events.
   Preserve session-v3 branches, title slots, unknown provider payloads, imports, recovery, ownership, relocation, and concurrent-writer behavior. Distinguish reading a historical session from safely resuming and writing it.
   Treat the two memory systems separately:
   - harvest-brain uses Markdown as canonical storage.
   - Mnemopi uses SQLite, graph/triple structures, embeddings, and its own migrations.
   Port Mnemopi’s storage and retrieval contracts explicitly. Complete automatic brain learning only after committed outcomes, leases, and dependency protection are connected.
   Exit condition: representative existing sessions and memory stores survive migration, resume correctly, and recover from interrupted operations without silent loss.
5. Replace the extension system, SDK, and JS evaluation paths.
   Make a versioned executable plugin protocol the initial extension interface. Provide a Rust SDK for manifests, tools, hooks, commands, provider events, streaming results, deadlines, cancellation, and brokered host actions.
   Plugin UI should use declarative requests rendered by the host. Existing arbitrary JavaScript component and editor objects require a redesigned interface.
   Remove TS/JS module loading, injected SDK shims, npm plugin assumptions, JS workers, transpilation, settings, and examples.
   Retire JavaScript evaluation explicitly. Typed Rust workflows can replace some uses; persistent Rust evaluation needs its own feasibility work before equivalent behavior is promised. WASM plugins can be considered later after validating their required host capabilities.
   Exit condition: first-party plugins and examples use the new interface, legacy modules fail with an actionable migration message, and no hidden JS fallback remains.
6. Port the tools, protocol services, CLI, and terminal UI.
   Use the actual command, tool, hidden-tool, slash-command, and worker registries as acceptance checklists.
   Bring the retained tools onto the shared Rust host, including MCP, RPC, ACP, LSP/DAP, delegation, goals, advisor, SSH, browser automation, and security operations. Preserve framing, cancellation, limits, and approval against final revised arguments.
   Port the terminal renderer and widgets while preserving inline scrollback, Unicode width, streaming previews, keyboard protocols, images, resize, backpressure, Windows behavior, and terminal restoration.
   Evaluate terminal libraries through representative prototypes before selecting one. Generic terminal support does not prove compatibility with Harvest’s current behavior.
   Exit condition: interactive, headless, SDK, and protocol modes exercise the same Rust lifecycle and pass their behavioral workflows.
7. Port background services, media, and local inference.
   Replace Bun workers and subprocess launch paths for stats, embeddings, speech, tiny models, brokers, computer interaction, and terminal output.
   Reuse native engines where available. Verify replacements for npm-based inference dependencies against actual models, tokenizers, dimensions, normalization, devices, cache formats, downloads, and supported platforms.
   Preserve persisted embedding identity. A model change requires an explicit re-embedding migration.
   Exit condition: installed worker/service workflows operate without Bun or Node, with bounded output, cancellation, shutdown, and failure recovery.
8. Rewrite browser clients and generated HTML interfaces in Rust.
   Include stats, collaboration, metaharness, robomp’s dashboard, browser-relay interfaces, and standalone exported sessions.
   Prototype Rust/WASM UI and Rust server rendering against the actual interaction requirements. Leptos is one candidate supporting both approaches; selection should follow measured bundle size, interaction, accessibility, and packaging results. Its [official documentation](https://book.leptos.dev/getting_started/index.html) also confirms that browser WASM applications can require generated JS loading code.
   The [browser relay manifest (line 8)](/workspace/harvest/packages/browser-relay/extension/manifest.json:8) currently requires a JavaScript service-worker entry. Plan Rust/WASM logic with narrowly scoped browser bootstrap/API glue, and validate MV3 loading, CSP, and worker lifetime.
   This browser glue uses the browser’s engine. It does not require bundling Bun or Node with Harvest.
   Exit condition: first-party UI logic is Rust, browser contracts remain covered, and every remaining JS artifact has an exact documented purpose.
9. Port automation, benchmarks, project tools, and remaining languages.
   Include robomp’s service, queue, database, proxy, RPC client, dashboard, analysis scripts, .harvest tools, and executable documentation examples.
   Preserve robomp’s deduplication, per-issue serialization, retries, cancellation, shutdown draining, worktree lifecycle, authorization, and HMAC credential boundary.
   Port the TypeScript edit benchmark implementation while preserving TypeScript as benchmark input data. Its Babel/Flow parsing and Prettier scoring need explicit compatibility investigation. A replacement parser or formatter may change the benchmark and require a versioned baseline.
   For literal zero tracked .ts files, preserve foreign-language fixture bytes in data assets and materialize temporary inputs during tests.
   Exit condition: retained automation runs through Rust implementations; any public Python SDK or external-language guest runtime has an explicit preservation, binding, or retirement decision.
10. Consolidate builds, installation, CI, and releases.
Move setup, checks, codegen, packaging, checksums, versioning, and release orchestration into Rust tooling. Update Docker, Nix, installers, updater logic, Homebrew, WinGet, aliases, and documentation together.
Resolve these source-visible release hazards before publishing:
- Rust release pushes tags that also match the legacy release trigger.
- Rust release retains standalone-project assumptions despite the shared workspace.
- The Rust matrix defines five artifacts; the legacy matrix defines eight.
- Default Rust features exclude shell support.
- Artifact names, checksum names, and installer expectations differ.
These are configuration findings, not claims that a release was executed. The relevant definitions are [Rust release (line 13)](/workspace/harvest/.github/workflows/rust-release.yml:13) and [legacy release (line 15)](/workspace/harvest/.github/workflows/release.yml:15).
Exit condition: one release owner produces and verifies the intended shipping profile across all promised targets.
TypeScript deletion happens within every completed slice. The required sequence is:
1. Capture the observable contract and implement its Rust replacement.
2. Verify success, failure, cancellation, persistence, and relevant platform behavior.
3. Switch consumers, generators, workers, packaging, and documentation.
4. Delete superseded TS/TSX/declarations and port the meaningful tests.
5. Remove orphaned dependencies and configuration.
6. Verify the affected workflow from a clean installation.
Shared files remain until their last consumer moves. This keeps deletions progressive without breaking unfinished migration paths.
The final acceptance criteria should be:
- Zero tracked first-party .ts, .tsx, .mts, .cts, and TypeScript declaration files.
- No handwritten JS replacement for first-party application logic.
- No bundled JS engine or Bun/Node requirement for the native application, standard build, tests, or release tooling.
- Every retained capability verified against the shipping artifact; intentional retirements documented.
- Existing data migration, recovery, security, protocol, and supported-platform workflows verified.
- Browser glue, declarative assets, bootstrap scripts, and necessary third-party native dependencies inventoried explicitly.
- Performance measured against captured baselines, without inventing improvement claims.
Foundations come first; provider, storage, UI, automation, and delivery work can then proceed in parallel where their contracts are stable. Estimates should follow the initial baseline and feasibility prototypes.
I used the cloud environment and plugin management skills and parallel repository audits. No additional plugin was necessary for this planning work. Remaining uncertainties are specific: live provider compatibility, external SDK usage, browser-extension WASM feasibility, benchmark scoring equivalence, inference compatibility, and actual cross-platform release results.
