# Rust migration implementation status

Current instructions: [PLAYBOOK.md](PLAYBOOK.md), [SCOPE.md](SCOPE.md), and [HANDOFF.md](HANDOFF.md).

Generate current counters with `cargo xtask playbook status --json`. The detailed
implementation notes below preserve historical slice evidence; their old counts
and browser-glue policy do not establish the current strict Rust-only target.

The normal application still launches TypeScript. This change establishes the
shared Rust foundations and native brain maintenance; it does not complete the
application rewrite. No capability is marked as legacy-parity complete simply
because a Rust command or tool has the same name.

## Implemented

- Anthropic Messages wire: `rust-agent/src/anthropic.rs` (`x-api-key`, `anthropic-version: 2023-06-01`, system hoisting, `tool_use`/`tool_result` blocks, role-merging, cache-aware usage). Catalog models with `anthropic-messages` and a concrete base URL route to it (`anthropic/claude-*`, z.ai, MiniMax, Vercel gateway, ...). Non-streaming only (text arrives at completion); no thinking, prompt caching breakpoints, images or OAuth.
- Catalog routing: `rust-agent/src/catalog.json` (`bun run gen:rust-catalog`, generated from `models.json` + `CATALOG_PROVIDERS` env vars) lets `--model provider/id` (thinking suffix tolerated) pick the provider base URL and its key env var (`GROQ_API_KEY`, ...). An explicit base URL still wins. Only `openai-completions` models (44 providers) are routed; Anthropic/Responses/Bedrock/Google wires, OAuth, the credential store and custom models are not ported.

- Settings: `bun run gen:settings-schema` emits the 503-entry schema (type,
  default, enum values, credential marker) from `settings-schema.ts` into
  `rust-agent/src/settings-schema.json`. `rust-agent/src/settings.rs` loads
  the global `config.yml`/`config.yaml` from the agent directory, the builtin
  project layer (`.harvest/settings.json`, then `.harvest/config.yml`) and
  strict `--config`-style overlays, deep-merged like `#deepMerge`, with
  schema defaults and type validation. Sessions load it; `grep` reads
  `grep.contextBefore`/`contextAfter` and `bash` applies the
  `tools.maxTimeout` ceiling with its clamp notice. Still pending: setting
  writes and migrations, the `.claude`/`.codex`/`.cursor`/`.gemini`/OpenCode
  project providers, profiles, and wiring the remaining tool settings.

- `web_search` walks the legacy fallback chain over Exa, Tavily and Brave
  (keys from `EXA_API_KEY`/`TAVILY_API_KEY`/`BRAVE_API_KEY`) and keyless
  DuckDuckGo (HTML frontend, redirect unwrapping, bot-challenge error), with
  the legacy request shapes, credit/auth error classification, fallback
  failure summary and `formatForLLM` output; a live DuckDuckGo search works.
  Still pending: the other 18 providers, the credential store, query
  directive parsing and constraint filtering, Exa's keyless MCP path and
  answer synthesis, and the provider order/exclude settings.

- `write` targets archive members (`pkg.zip:src/a.txt`) by rewriting the whole
  zip/tar/tar.gz/tar.zst archive through the workspace capability (new
  `harvest-archive` encoders; read-only formats and `..`/directory targets
  fail with the legacy errors), and SQLite rows (`app.db:table` insert,
  `app.db:table:key` update, empty content deletes) through the primary key
  or ROWID with the column/scalar/query-parameter checks. 13/13 probes match
  TS, final contents included. Row content is parsed as strict JSON (legacy
  accepts JSON5); asar writes are pending.

- `read` opens SQLite databases (`.db`, `.db3`, `.sqlite`, `.sqlite3`, magic
  checked) query-only through bundled `rusqlite`: table listing with bounded
  row counts and `sqlite_stat1` estimates, `:table` schema plus samples,
  `:table:key` rows by primary key or ROWID, `?limit&offset&order&where`
  pages with the comment/terminator/keyword `where` guard, and `?q=` raw
  queries capped at 1000 rows, all through the legacy ASCII-table, vertical
  and row renderers. 16/16 probes match TS. SQLite writes are pending.

- `harvest-archive` (new crate) recognizes every legacy archive extension and
  splits `archive.ext:member` paths like `parseArchivePathCandidates`; it
  indexes ZIP, tar (ustar/GNU long names/pax paths), tar.gz, tar.zst and
  single-stream gz/zst with synthesized parent directories, case-insensitive
  listings, `..` rejection and the legacy size limits. `read` uses it for
  archive listings (500-entry limit notice, offsets, tails), member reads
  (immutable: no tag, no numbers, block-context windows) and the binary-member
  notice; 7/7 probes match TS. Still pending: bz2/xz/Z/lzma tars, rar, 7z,
  iso, cab, cpio, rpm, ar/deb, lzh, arj, asar (explicit unsupported error),
  and archive formats beyond zip/tar for `write`. `grep` searches
  `archive:member` text members in memory with the shared engine (untagged,
  range filters, the binary/non-UTF-8 skip note and the all-unreadable
  error); 7/7 probes match TS.

- `think`, `checkpoint` and `rewind` follow the legacy tools: the scratchpad
  result, the single active checkpoint, and the empty/missing/already-done
  rewind errors. A successful rewind branches the durable session back to the
  checkpoint result entry (the exploration branch stays in the log) and
  commits the rendered `rewind-report` prompt, so the next model turn and a
  reopened session see only the report. The rewind reminder before yielding
  and display-only custom entries are pending. The tool allowlist now derives
  from the tool specs.

- `pi_ast::engine` owns the ast-grep file search, in-memory match and rewrite
  orchestration; the N-API `astGrep`/`astMatch`/`astEdit` exports are thin
  adapters with unchanged bindings (23 TS ast tests pass on the rebuilt addon).
  The Rust `ast_grep` tool uses it with the grep scope resolver: grouped
  snapshot-tagged output, `meta:` bindings, `skip` paging, multi-target merge
  and the legacy parse-issue and limit notices; probes match the TS tool except
  where a plain file in a `;` list is fanned out: TS renders it as
  `# file/` + `## file` (it joins the match path onto the file itself) and
  loses the tag, Rust renders the file directly. `ast_edit` stages a preview
  (byte-identical to TS on three probes) on a session pending-action queue;
  `write xd://resolve` re-runs the rewrite and splices it through the
  workspace capability (stale-preview reporting, fresh tags), `write
  xd://reject` discards it. `xd://propose`, the soft tool requirement and the
  reminder injection are pending.

- `todo` ports the legacy phased task model: all nine ops, `op` inference
  from unambiguous payloads, single in-progress normalization, all-or-nothing
  batches, blocker notes, the summary text and the Markdown round-trip. State
  is session-scoped in memory; persistence into the session log, `/todo`
  editing and the TUI renderer are pending.

- `bash` follows the legacy foreground result contract: `env` (validated
  names), `timeout` (default 300 s, clamped to 1–3600 s with the clamp notice,
  `0` disables), jailed `cwd` with the legacy missing/not-a-directory errors,
  `(no output)`, the wall-time notice, `Command exited with code N`, and the
  timeout/abort wording. Still pending: `pty`, `async`/auto-background jobs,
  the destructive-command barrier, interceptors, skill URL expansion and the
  client-bridge terminal.

- `pi-grep` (new crate) owns the ripgrep-backed search engine: matcher
  fallbacks (brace sanitising, PCRE2, paren escaping, literal), file/dir
  search, per-file caps, context, oversized-prefix handling and cancellation
  via the shared runtime token. The N-API `search`/`hasMatch`/`grep` exports
  are thin adapters with unchanged bindings (`index.d.ts` identical). 38 engine
  tests moved with it; 160 pi-natives tests and the focused TS grep tool tests
  pass after an addon rebuild. No TS source is retired yet.

- The merge-conflict workflow is ported (`rust-agent/src/conflict.rs`): strict
  2-way/diff3 marker scanning, a per-session history with stable ids, the
  window warning `read` appends after a file window (with the full-file count),
  the `:conflicts` index, `read conflict://N[/ours|theirs|base]`, and `write
  conflict://N` / `conflict://*` with `@ours`/`@theirs`/`@base`/`@both` tokens,
  `<id>: @side` directives, content-located splicing with boundary-echo repair,
  CRLF preservation, stale-twin cleanup, hashline-prefix stripping, recovered
  `<file>:` prefixes and snapshot headers. A 17-call read/write sequence over a
  2-way, diff3, long-sided and CRLF fixture produced 19/19 byte-identical
  results and final files against the TypeScript tools. `write` now takes the
  legacy `content` argument (`contents` still accepted). Plain filesystem writes
  now follow the legacy local-file path too: read-selector and `;`-list misfire
  guards, the auto-generated guard, hashline-prefix stripping (incl. a pasted
  `[path#TAG]` target), `chmod +x` for `#!` content with its notice, and a fresh
  snapshot header plus `Successfully wrote N bytes` (UTF-16 length): 21/21
  probes identical, file contents and modes included. Still pending: archive and
  SQLite targets, `local://`/`xd://` and other internal URLs (URI-like targets
  fail with an explicit message), the ACP bridge and LSP writethrough
  diagnostics.

- `pi_walker::workspace` owns the single-pass workspace scan (excluded build/
  cache dirs, depth bound, AGENTS.md discovery below the tree depth);
  `listWorkspace` is a thin adapter with unchanged bindings. `read` of a
  directory renders the legacy tree (`rust-agent/src/tree.rs`): depth 2, 12
  children per directory ranked newest-first with the oldest kept after a
  `… N more` marker, size and relative-age columns, ICU-style name tie-breaks,
  `(empty directory)`, line selectors with the "more lines in listing" notice,
  and the multi-range and `Path '…' not found` errors. 11/12 real-repository
  directory probes are byte-identical (the 12th matches up to the no-session
  output cap); name collation matched `localeCompare` on 3,000 random names, and
  `formatBytes` now rounds exact ties up like `toFixed`. Conflict warnings are
  deferred to a dedicated `conflict://` slice: the warning hands out ids that
  only that subsystem can resolve.

- Bare code reads now return the legacy structural summary: `pi_ast` tree-sitter
  segments with the default fold settings (100-line minimum, 4/6-line body and
  comment thresholds, unfold 50/100), brace-pair merging (`N-M:head … tail`),
  the `[…Nln elided; re-read needed ranges …]` footer and seen-line provenance;
  Markdown/`.txt` files and selector reads skip it. Block context now parses the
  host's original text (new `pi_edit::diff_string::find_block_context_lines_with_text`),
  so nodes that run to EOF keep their last line. 17/17 real-file probes
  (TypeScript, Rust, Python, Shell, Julia, TOML) and the earlier 27/28 read
  probes are byte-identical to the TypeScript tool.

- Tool results now carry their notices separately (`rust-agent/src/output.rs`).
  Inside a persisted `AgentSession`, results over 50 KB spill to
  `<session>/<id>.<tool>.log` (atomic, sanitized names, ids resumed from disk)
  and are replaced by the legacy 20 KB/500-line head + tail window with an
  elision marker and an `artifact://N` notice; reads of artifacts are never
  re-spilled. 22/22 randomized spill cases matched `wrapToolWithMetaNotice`
  byte-for-byte. `read artifact://N[:selector]` follows `#readArtifactFile`
  (plain rows, 300-line default, block-context multi-range, raw-unbounded
  block, storage notice over 50 KB, legacy resolver errors): 15/15 probe cases
  identical. Without a session the host keeps its 20,000-character cap, where
  legacy returns the full text. 107 default-feature and 106 core-feature tests
  pass.

- The Rust host's `edit` is now the legacy hashline contract: one `pi_edit`
  hashline `Session` per call (fuzzy 0.95, auto-generated guard), staged in
  memory with the engine's read-jail re-check, persisted only through a
  `JailedFs` writer, returning the engine's model-facing text. `read` records
  each file in the session-wide `EditStore` and returns `[path#TAG]` plus `N:line`
  rows, with legacy selectors (`:N`, `N-M`, `N+K`, `N-`, `-N`, comma lists,
  `..`, `raw`, compound raw), literal-colon filenames, the 1/3-line range
  context, out-of-range and multi-range notices, and notebook cell views. Tags
  matched the legacy read on all 28 probe cases. A follow-up ports the legacy
  buffered local-file reader (raw LF segments with BOM/CR preserved, the
  300-line `read.defaultLimit`, 512 B/line byte budgets, 768-column cap,
  tree-sitter/lexical block context via `pi_edit`, binary sniff, omitted-line
  and "Showing lines" notices) and the in-memory notebook reader: 27/28 probe
  outputs are now byte-identical to the TypeScript tool. The remaining case is a
  host difference: legacy spills results over 50 KB to a session artifact, while
  the Rust host still hard-caps tool output at 20,000 characters. Still missing:
  structural summaries for bare code reads, merge-conflict warnings, files over
  4 MiB (legacy streams them), directories, archives, SQLite, images, PDFs,
  URLs and internal URIs. 102 default-feature and 101 core-feature tests pass;
  `provider_usage::unsuccessful_json_and_unterminated_sse...` is timing-flaky
  under parallel load (passes alone and on re-run).

- The Rust host gains a `glob` tool (`rust-agent/src/glob.rs`) on `pi_walker::glob`:
  semicolon target lists, base/glob splitting, newest-first merge across targets,
  directory `/` suffixes, prefix-folded grouped output, the 200-result cap and
  limit notice, skipped-missing-path notes, the 5 s timeout notice, and the
  legacy root/ssh/limit/not-found errors, all inside the workspace jail. Six
  tool tests pass. Not yet covered: internal URLs (memory://, local://), streamed
  progress updates and the TUI renderer, so `tool:glob` stays partial.
  A follow-up fixes `JailedFs::relative` for absolute paths through an alias of
  the canonical root (macOS `/var` → `/private/var`), which made
  `native_search::depth_limits_are_visible...` fail on macOS; escapes through
  symlinks remain rejected.

- `pi_walker::glob` owns glob search (pattern normalization, depth bounding,
  mtime ranking, symlink-aware type filtering, heartbeat cancellation); the
  N-API `glob` export and `glob_util` pattern helpers are thin adapters. Five
  walker tests (moved from pi-natives plus a filter-before-limit and heartbeat
  case), 238 pi-walker/pi-natives tests, a rebuilt addon with unchanged
  bindings and 23 focused TS glob tests pass. `tools.test.ts` has 23
  pre-existing macOS `/var` → `/private/var` path-jail failures, identical on
  `pre-rust-migration-backup`. No TS source is retired yet.

- `cargo xtask fix-changelogs` replaces the changelog fixer in `scripts/fix-changelogs.ts`;
  `fix:changelogs` and `scripts/release.ts` call it, and real-repository rewrites are
  byte-identical. The TS file is reduced to the parser/renderer that
  `rewrite-changelog.ts` still imports; its test file is deleted.

- `cargo xtask bench-runs DIR...` replaces `scripts/eval-bench-runs.ts` (deleted).
  Output is byte-identical except that rows whose sort metric is missing (NaN)
  now sort last deterministically instead of in engine-defined order.

- `cargo xtask fix-test-imports [--write]` replaces `scripts/fix-test-imports.ts`
  (deleted); reports and rewritten files are byte-identical on the real test tree.

- `cargo xtask nix-bun-deps [--print-generator]` replaces `scripts/gen-nix-bun.ts`
  and its test (both deleted); `gen:nix` and `scripts/release.ts` call it. A
  real bunx bun2nix run produced byte-identical bun.lock/nix/bun.nix to legacy.

- `cargo xtask merge-pr BRANCH [--pr N] [--dry-run]` replaces
  `scripts/merge-pr.ts` and its test (both deleted); alias it with
  `git config alias.mpr '!cargo xtask merge-pr'`.

- `cargo xtask sync-versions` replaces `scripts/sync-versions.ts` (deleted),
  writing byte-identical manifests on all 16 real package manifests. Like the
  legacy script it also rewrites `catalog:` specifiers, so do not run it on the
  current tree without reviewing that behavior.

- `cargo xtask brew-formula TAG [--out FILE]` replaces
  `scripts/ci-update-brew-formula.ts` and its test (both deleted). Output is
  byte-identical to the legacy renderer; the issue #2398 regressions are
  ported as Rust tests.

- `cargo xtask spoofed-versions [--update]` replaces
  `scripts/check-spoofed-versions.ts` (deleted). It reports Gemini CLI
  User-Agent drift against GitHub releases with bounded HTTP, and rewrites only
  the matched default atomically. Legacy and Rust runs produced identical
  drift reports and exit codes; the binary runs with no Bun/Node on PATH.

- `harvest-media::pdf` owns PDF-to-Markdown conversion (page markers, title,
  OCR-needed pages, encoding-issue flag, `PDF conversion failed:` error prefix)
  and is reachable as `harvest_omp::media::pdf`. `pdfToMarkdown` is a thin
  adapter. The three existing Rust tests moved with the logic; 17 media tests,
  strict Clippy, a rebuilt addon and 9 focused TS PDF converter/read tests pass.
  No TS source is retired.

- `harvest-media::sixel` owns SIXEL encode and bounded decode; the
  `pi-natives` exports are thin adapters with unchanged JS signatures and error
  text. Encoding now rejects zero or oversized targets (8192 px edge, 4 Mi
  pixels) before decoding, and checks source header dimensions (64 Mi pixels,
  64 MiB input) before allocating pixels. Six Rust boundary/roundtrip tests,
  14 media tests plus one doctest, strict Clippy, a rebuilt addon (130 exports)
  and 183 focused TS SIXEL/terminal-graphics tests pass. A byte-for-byte oracle
  against the pre-extraction addon and Bazel execution were not rerun on this
  macOS host. No TS source is retired; `utils/sixel.ts` remains a consumer.

- `harvest-media::{html,svg}` exposes the existing HTML-to-Markdown and SVG/SVGZ
  preview engines through `harvest_omp::media`. N-API only converts/owns inputs
  and schedules the original blocking-pool calls. Standalone installed Rust
  without a JS runtime and the rebuilt addon each match 40 HTML conversions
  and 20 SVG renders/errors byte-for-byte. Eight media tests and one doctest
  cover Markdown structure, explicit image omission, cleanup, depth rejection,
  aspect ratio/pixels, canvas limits and blocked external versus embedded images.
  All 252 shared Rust tests, actual Bazel media tests, strict Cargo Clippy,
  package gates, eight native HTML tests and 26 fetch/Git preview tests pass.
  Rust CI now includes this crate. Full media loading, terminal integration,
  local inference and platform/font parity remain pending; no TS frontend
  source is retired by this extraction.

- `harvest-tui::mouse` decodes SGR clicks, release, drag and vertical-only
  wheel reports directly from Rust. Installed Rust (without a JS runtime),
  rebuilt addon and frontend each match 2,637 legacy reports, including
  malformed reports, zero-based coordinates, huge numeric fields and modifiers.
  The existing frontend decoder remains for older/unavailable optional addons;
  component hit-test routing is still TypeScript. All 100 TUI tests, four
  doctests, 191 combined Rust tests, 97 focused frontend tests, strict Clippy
  and package gates pass. Warmed 20,000-event probes measured old 4.56/5.04 ms
  and shared 17.64/21.51 ms: host bridge overhead is explicit, with no claimed
  speedup. Full interactive terminal parity and legacy removal remain pending.

- `harvest-tui::highlight` shares the existing syntax engine with ordinary Rust
  consumers. All five bundled grammar assets moved byte-for-byte; N-API only
  converts inputs, owns its inline palette and schedules warmup. Palettes are
  borrowed per push without retaining host handles. Installed Rust and rebuilt
  addon probes match 288 whole-code cases and 720 streamed chunks over 32
  language/alias paths, three palette modes and unknown/empty-input boundaries.
  Async warmup and palette overflow/recovery pass. All 332 shared Rust tests,
  96 Bazel TUI tests, three doctests, strict Clippy and package gates pass;
  native alias/stream and colored Markdown/diff checks pass. The diff test now
  supplies an explicit color mode instead of depending on ambient `NO_COLOR`.
  Warmed 200-call probes measured old 48.64/48.83 ms and shared 49.76/50.48 ms;
  this small bridge overhead is recorded without a whole-app performance claim.
  Native Markdown widgets and complete interactive renderer parity remain pending.

- `harvest-tui::deccara` owns the exported rectangular-background planner;
  the TypeScript module is now a thin snapshot/cutoff adapter. This helper is
  currently exported by the SDK, not wired into the main TUI renderer. Native
  and rebuilt-addon probes match 3,745 corrected legacy frame plans and 8,424
  analyses; 3,313 original well-formed frame fixtures are unchanged. The
  independent surrogate-width fix covers the remaining malformed UTF-16 cases.
  Kitty terminal-core emulation exposed and verifies the fix for lost inverse
  and underlined padding. Unsupported/decorated SGR, oversized frames and
  overflowing coordinates retain complete original rows. All 328 shared Rust
  tests/doctests, 127 focused TS contracts, package gates and strict Clippy pass.
  A warmed 100-frame/40-row probe measured old medians 5.82/9.94 ms and shared
  medians 5.94/5.64 ms; this overlapping microbenchmark is not whole-app proof.
  Platform terminal capability discovery and complete renderer parity remain
  pending; the legacy host capability decision and adapter are retained.

- `harvest-tui::text` now owns ANSI-aware width, wrapping, truncation and
  column/overlay slicing. The Rust SDK calls it directly; N-API retains a thin
  UTF-16 adapter and its existing output convention. Unchanged truncation
  borrows its input; huge column ranges reserve against existing text instead
  of requesting gigabytes. All 317 shared Rust tests/doctests, the 81-test
  Bazel TUI harness, strict Clippy, both package gates and 119 focused TUI/
  fallback contracts pass. Installed core and rebuilt-addon probes each match
  3,074 legacy operations, including malformed UTF-16 and terminal sequences.
  Warmed CPU-affined 18,000-operation probes overlap (old 16.98/18.81 ms,
  shared engine 17.39/17.13 ms); no whole-app speedup is claimed. Widget layout,
  renderer lifecycle, restoration and installed terminal parity remain pending.

- `harvest-tokenizer` extracts the existing offline counting engine, embedded
  vocabularies and reference tests from N-API. The Rust SDK exports it directly;
  the legacy addon delegates to the same library. All eight vocabulary blobs
  are unchanged. An installed native probe matches 14,020 UTF-8/UTF-16 legacy
  counts over 701 inputs without Bun/Node on PATH; the rebuilt addon matches
  7,010 counts. All 295 shared Rust tests/doctests, 38 tokenizer tests in the
  Bazel harness, strict Clippy, the native package gate and 24 focused addon/
  agent tests pass. Fifteen stale expectations from five renamed Claude texts
  were corrected against all 1,479 pinned `ctok==1.0.0` references, without a
  runtime algorithm change. Warmed, CPU-affined 10,000-call probes measured
  old medians 100.7/105.3 ms and shared-engine medians 103.4/109.7 ms; this is
  narrow overhead evidence, not an end-to-end performance claim. Catalog
  selection, message/frame accounting and compaction parity remain pending.

- `cargo xtask emit-extensions --directory DIR [--kind types|js] [--check]`
  repairs ESM specifiers in emitted package output. The retained publisher
  invokes Rust for both declaration and runtime output; the old TS helper is
  deleted. All 73 xtask tests, strict Clippy, root tools gate and six publisher
  tests pass. Installed Linux tooling runs without Bun/Node on PATH and matches
  22 fixture rewrites. Real tsgo output matches all 26 omptype files and 52
  rewrites, including paths with spaces; Node ESM loads the repaired runtime
  graph and correctly accepts/rejects schema inputs. Traversal/file/plan bounds,
  lexical escapes, invalid UTF-8, external symlinks and read-only drift checks
  have Rust contracts. The publisher and overall release cutover remain pending.

- `cargo xtask theme-index [--check]` owns bundled theme registry generation,
  including release regeneration and Rust CI drift checks. It fixes the stale
  defaults path and preserves all 98 current exported names and palette values.
  Installed no-runtime generator probes cover identifier collisions, reserved
  names, prototype keys, deleted assets and read-only stale checks; 67 xtask
  tests, strict Clippy, the package gate and 32 theme/startup/export tests pass.
  The old generator is removed; its generated TypeScript registry remains until
  the frontend consumers migrate.

- `cargo xtask fix-elf-verdef --workspace ROOT FILE...` repairs stale GNU
  version-definition pointers through bounded, jailed, atomic streaming writes.
  Invalid metadata, reader failures and unwinds leave the original intact;
  outside hardlink aliases are preserved. All 193 shared Rust regressions and
  19 rebuilt-addon regressions passed. Five installed legacy-oracle cases match;
  controlled x64 and emulated ARM64 glibc 2.17 loader failures recover without
  Bun/Node on the repair command's PATH. The host patchelf chain kept its pointer
  current, so this is deliberate-corruption recovery evidence. The clean Nix
  Linux x64 consumer build passed: the real auto-patchelf chain relocated
  DT_VERDEF, native repair corrected it, and the installed CLI version/help
  checks passed. The old script/test are removed.

- `cargo xtask clippy-config [--check]` owns Bazel lint policy generation.
  Root/release callers and Rust CI use it; the TS generator is removed. Five
  installed legacy-oracle cases preserve generated flags, and missing/stale
  checks leave files untouched without Bun/Node on PATH. All 55 xtask tests,
  strict Clippy and an actual Bazel strict-Clippy build passed. Model catalogs,
  compatibility compilation, browser assets and other generators remain pending;
  the overall codegen capability remains unverified.

- The Rust SDK and legacy TUI now share bounded terminal input framing, paste
  recovery and Escape deadlines through `harvest-tui::input`. The TS adapter
  schedules host timers and delivers events. Shared Rust tests, legacy framing
  and editor suites cover fragmented protocols, same-read paste/Enter delivery,
  watchdogs and discarded OSC tails. Shared output byte chunking and backlog
  progress detection preserve Unicode and keep slow draining frames alive.
  A shared bounded output pump now accepts Rust-owned backends directly, keeps
  in-flight bytes inside its cap and releases detached backend ownership once
  blocked I/O resumes. The legacy Unix adapter uses the same queue.
  Terminal lifecycle, renderer, resize,
  restoration and native interactive installation still require implementation
  and verification; terminal capability parity remains unverified.

- Native distribution uses one eight-target registry for release/CI matrices,
  package names, Homebrew bindings and both WinGet architectures. ZIPs include
  both real aliases, deterministic metadata and legal notices; validation checks
  all targets, alias bytes/modes, headers and legal files. WinGet generation and
  submitted-archive verification run through Rust and reject unknown manifest
  fields or changed downloads before extraction. Canonical bootstrap consumers
  use `SHA256SUMS.txt`. Local fixtures passed 176 combined Rust tests, 90 core
  app tests and shell installer routing/upgrade/refusal checks. A full release
  build completed, and real cargo-binstall 1.26.0 installed both aliases from a
  loopback archive; execution and completion setup worked without Bun/Node.
  PowerShell files passed the real parser and the non-Windows refusal contract;
  native Windows/macOS runs, GNU portability-floor builds and Cargo publication
  dependency closure remain part of distribution parity. See
  [distribution contracts](distribution.md).

- Native settings/override/environment diagnostics omit API keys and endpoint
  URLs. Invalid TOML errors retain only the config filename and byte offset,
  dropping parser source-line/error chains; malformed environment step counts
  no longer echo their input. SDK and real CLI failure contracts cover both
  aliases, no session creation and unchanged config bytes. Eight installed
  credential-bearing parse failures passed without Bun/Node. This does not
  implement credential storage, OAuth or compatible profile/settings migration.

- `cargo xtask release-notes VERSION [OUTPUT]` replaces the old TS release-notes
  script and supplies curated bodies to both release workflows. It merges
  unpublished-tag changelog sections through the target, deduplicates entries,
  and includes native crate/application changelogs as well as legacy packages.
  Automatic floor lookup uses authenticated `gh release list` and fails closed;
  `--floor VERSION`, `--single-version` and `--published-releases JSON_FILE`
  support explicit/offline runs. Seven legacy-oracle cases also passed through
  installed xtask without Bun/Node. All 41 xtask tests, strict Clippy and workflow
  YAML validation passed; no GitHub release was published during verification.

- Native session replay projects text compactions, handoffs, branch summaries
  and durable context resets onto the selected branch without rewriting history.
  Eight legacy-oracle cases match, including reset/compaction precedence and
  sibling isolation. Missing/forward/sibling kept references, active opaque
  compaction payloads and broken tool pairs fail before writing. All 16 installed
  alias/case continuations passed without Bun/Node; app/runtime/brain/xtask passed
  155 tests and the explicit core app passed 85. Custom messages, inferred
  settings, general prompt formatting, provider replay, bitmap archives and
  automatic compaction generation remain pending; session parity is unverified.

- `grep` runs the shared `pi-grep` engine (the same code behind the N-API
  binding) under the legacy tool contract: delimited `path` lists with
  literal-path preference, `file:N-M` range filters, per-file match caps,
  20-file pages with `skip`, round-robin file interleaving, prefix-folded
  grouped output with snapshot-tagged headers (plain `N|line` when a file is
  too large to snapshot), missing-path and 4 MB-window notices, the 50 KB head
  window and column notice, regex/timeout error wording, and the workspace
  jail on every engine root. 44/44 parity probes were byte-identical with the
  TS tool. Still pending: internal/external URLs, archive members, ssh paths
  and the `. ; file` single-walk ordering (Rust fans out per entry instead).
  The Rust-only skip of `.omp`/`.harvest`/`target` directories is gone; the
  engine prunes only `.git` and `node_modules`, like the legacy tool.

- Shared native HTTP request construction returns typed status/transport errors
  without attaching raw URL/status/body diagnostics; `HttpModel` debug output
  omits configured credentials. Real loopback 401/429 failures, invalid schemes
  and rejected JSON/SSE finish reasons exercise both SDK request paths. This
  does not implement credential storage, OAuth refresh or provider retry policy.
  The 72-test app suite, strict Clippy and installed HTTP-failure checks through
  both aliases passed; installed failures preserve the user turn and commit no
  assistant output. The unchanged legacy usage-attribution suite passed 22 tests.

- Native OpenAI-compatible streaming and JSON responses preserve raw usage
  metadata, normalize complete integer token/cache reports and persist them
  before committed events. Unknown numeric metadata remains lossless and stays
  out of replayed model requests. `--request-usage` explicitly enables the wire
  request option; the default preserves compatibility with unsupported hosts.
  Seven accounting cases captured from the legacy helper match both installed
  aliases without runtimes on PATH. Partial/invalid reports retain raw bytes and
  remain unavailable; catalog pricing and other provider families remain pending.
  Explicitly unsuccessful JSON responses cannot commit assistant output.

- `cargo xtask dev <check|fmt|lint|fix|test>` replaces the Bun Rust-task launcher.
  Root aliases, the pooled test runner and CI invoke it. It preserves the
  vendored-fork exclusion, local change skips, CI forcing and Rustup Cargo
  selection; bounded Git failures run gates conservatively. New untracked
  directories are inspected recursively. Real Cargo fixture contracts verify
  repair/recheck and nextest/doctest failure ordering. Installed commands run
  without Bun or Node; remote macOS/Windows execution remains pending.

- Native `session --title TEXT FILE` updates or adds a fixed-width title slot
  under the Rust writer lease, preserving every remaining session byte,
  including unknown signed/provider records. Unicode truncation, clearing,
  malformed/legacy/tree rejection, active-writer exclusion and symlink safety
  are covered. Resume now validates all branches before opening a writer.
  The combined app/runtime/xtask suite passed 99 tests; both installed aliases
  ran without runtimes, and the legacy title reader accepted their output.
  Cached prefix overlays and legacy writer coordination remain pending.

- Both release checksum consumers and the package alias use `cargo xtask
  checksums OUTPUT ASSET...`. The TS generator and its tests are removed.
  The Rust command streams authorized regular files with bounded memory, emits
  deterministic sha256sum-compatible basenames, and rejects duplicate, unsafe,
  escaping or missing inputs before atomic manifest replacement. The installed
  Linux command works without runtimes on PATH; `sha256sum --check` verified its
  output. The 36-test runtime/xtask suite and strict Clippy passed. Remote
  release and cross-platform execution remain unverified.

- `cargo xtask version` reads the root inherited version; `--set VERSION
  --expect CURRENT` changes it atomically under a writer lock while preserving
  TOML comments and unrelated fields. Rust release jobs use root manifest,
  lockfile and target paths, explicit application package selectors, and the
  nextest/doctest gate. Release remains blocked while the legacy release workflow
  owns tags or source/capability parity is incomplete. The crate publication
  dependency graph, installer naming, complete platform matrix and actual remote
  release validation remain pending. Local validation covers 21 xtask contracts,
  strict Clippy, and a two-member fixture whose inherited versions were checked
  through Cargo metadata after running the updater without runtimes on PATH.
  Workflow YAML parses locally; hosted release jobs have not been executed.

- One Cargo workspace, lockfile, toolchain and build profile configuration for
  the existing native engines and the experimental application.
- `cargo xtask check`, `cargo xtask test` (nextest followed by doctests), and a
  capability inventory with a distribution cutover gate. Focused test target
  selectors apply to nextest while doctests retain package/build options;
  `--test session_recovery` no longer fails by mixing `--doc` and target flags.
- Shared capability-relative filesystem operations, bounded reads/appends,
  atomic replacement, root writer locks and cancellation in `harvest-runtime`.
  `pi-shell` retains its cancellation API through re-exports.
- `harvest-brain`: Markdown records, five record kinds, scoped graph/BM25
  retrieval, evidence-bound deterministic admission, exact duplicate merging,
  protected authored/manual/pinned/safety/leased/dependency records, recoverable
  archive/restore/purge transactions, quotas and idle-attempt scheduling.
- Native brain CLI with both organizer spellings, JSON reports and dry runs.
  Explicit user lessons can be admitted with `brain learn`.
- The existing application now uses `harvest-brain` through a small N-API
  adapter. Its duplicate TypeScript scanner, BM25 setup and graph expansion have
  been removed. The legacy SDK message transform remains during migration;
  scope filtering now precedes ranking and applies through graph traversal.
- Bazel native-addon dependency targets include both shared Rust crates and
  their static prompt assets. Cargo native builds are verified locally; Bazel
  cross-builds still need CI verification because Bazel is unavailable here.
- Rust `xtask package` generates Homebrew and WinGet release metadata with
  validated versions, calendar dates and asset checksums. It replaces the two
  standalone Python generators. Secret scanning remains in the existing
  repository-wide Gitleaks security job and the prototype credential scanner.
  The blanket comment ban and arbitrary source-length gate have been removed.
  CI preserves the coverage floor, release tests, cross-target checks, strict
  Rustdoc, feature powerset, packaging and installer verification.
- Rust credential scanning preserves the prototype detection rules, redacts
  matched values and caps manifests, source sizes and findings. Read failures,
  escaping paths and oversized input fail the check. The redundant Python scanner
  and its dedicated lint configuration/job have been removed after Rust contract
  and installed-binary verification. Hygiene now uses the Rust scanner alone;
  repository-wide Gitleaks remains unchanged.
- Both Rust binary aliases share a library. The prototype uses bounded I/O,
  jailed session writes, embedded Markdown prompts, ephemeral retrieval from
  project/user brains and host-owned approvals. Shell execution in `full`
  reuses `pi-shell` with a bounded channel, capped capture and a timeout. Native
  builds now enable `full` by default; `--no-default-features --features core`
  keeps the explicit lightweight build. Default/core application suites passed
  57/56 tests. A default-installed binary completed an approved shell round trip
  through a PTY, evaluated arithmetic in the workspace and durably recorded the
  tool result without runtimes on `PATH`. Other platform shipping checks remain
  pending.
- Native replacement edits now call `pi-edit` directly through capability-relative,
  bounded host reads and atomic writes. BOM/CRLF and notebook preservation,
  generated-file protection, ambiguous matches, cancellation and symlink escapes
  have native application contract tests. Other edit modes, streaming previews,
  LSP/ACP write-through and freshness integration still require host ports.
  Local core tests passed (324); full-profile application/edit/AST tests passed
  (408, one skipped), with doctest passes. Strict linting and formatting passed
  on the changed Rust packages; installed cross-platform parity remains pending.
- A bounded session codec reads title slots and v3 trees, preserves original
  bytes and raw unknown/provider values, and migrates v1/v2 entries in memory.
  `harvest session --workspace <root> --leaf <id> <file>` reports a selected
  branch without rewriting its source. The experimental Rust `AgentSession` now
  replays text/function branches, streams OpenAI-compatible SSE, executes approved
  tools and durably appends v3 messages. Ownership locks, content comparisons,
  generation-specific handles and committed events are tested. Explicit
  `session --import-v3` exports all branches to a new file, rejects malformed records and invalid
  parent chains, preserves source bytes and unknown values, and never overwrites
  a destination. Shared atomic creation publishes fully synced content using
  no-clobber hard links. Explicit `session --recover-to` preserves the raw v3
  prefix and original source, reports the source hash and excluded tail size,
  and rejects interior corruption, broken trees and complete final records.
  Both snapshot commands share the cooperating Rust writer lease; unresolved
  tool calls still block resume. Recovery has seven CLI contracts; the full
  application suite passed 57 tests and strict linting. Installed full-profile
  recovery/continuation passed using a local provider fixture with no runtimes
  on `PATH`. The rebuilt native addon also passed eight legacy edit tests. Signed/multimodal replay, compaction, automatic
  repair, legacy ownership coordination and large-log streaming remain pending. Local installed full-profile `harvest` and
  `omp` passed v1 import, local-provider replay/continuation, durable writes,
  source preservation and overwrite rejection with no runtimes on `PATH`.
  This does not certify live providers or other operating systems.
- Help/version have no setup side effects. Experimental release automation now
  requires capability parity and is explicitly dispatched instead of publishing
  an incomplete application on each source push.

- Protocol generation now uses explicit message/enum roots in
  `packages/catalog/scripts/protocol-roots.json`, replacing Cursor TS-import
  scanning. Cursor/Devin output remains byte-identical when generated in an
  isolated tree containing only the generator, parser, roots and `.proto` files.
  Unknown roots fail before outputs are written. The compiler itself and its
  generated TypeScript runtime still require Rust replacements.

## Remaining before cutover

The manifest includes 41 legacy commands, 29 canonical tools, three hidden tools,
84 slash entries and integration surfaces. Primitive Rust tools are partial:
legacy read/edit/grep/bash behavior and settings still have richer contracts.

Complete provider streaming/OAuth/catalog KDL resolution; remaining session
import/replay adapters, fork/compaction, recovery and legacy ownership integration; RPC/ACP;
the inline terminal; delegation,
goals/advisor; MCP/LSP/DAP; browser/WASM/eval/SSH; collaboration; media/inference;
stats/benchmarks; Python automation; generators and consolidated distribution
still require ports and verification. The Rust library is experimental and does
not yet expose the approved complete `AgentSession` interface or process ABI v1.

Automatic session extraction is **not connected to a production event loop**.
The admission/ledger APIs and one-attempt idle scheduler exist, but the bounded
provider extraction call, committed-outcome adapters, safe inferred global
promotion and legacy skill-management wiring remain implementation work.
Repository identities conservatively unify origin URLs across transport; missing
origins, includes and inaccessible worktree metadata cannot support promotion.
Existing Markdown is indexed non-destructively; legacy revision histories remain
untouched and have not been migrated into the new transaction ledger.

Automatic retirement is limited to generated records with trustworthy receipts.
It never purges. Existing records without receipts are protected, and indexing
incomplete or malformed dependency inventories prevents organization.
Production retrieval does not yet register active-record leases or application
outcomes. Cross-scope dependency protection covers the selected workspace and
user roots; a registry covering other projects' reverse dependencies remains
required before global automatic maintenance can satisfy the complete plan.

## Verification

```sh
cargo xtask test -p harvest-runtime -p harvest-brain -p harvest-omp
cargo xtask test -p harvest-omp --features full
cargo xtask test -p harvest-omp --test session_recovery
cargo clippy --locked -p harvest-runtime -p harvest-brain -p harvest-omp \
  -p xtask --all-targets --all-features -- -D warnings
cargo xtask capabilities
# Intentionally fails until all parity gates are met:
cargo xtask capabilities --require-complete
```

`capabilities.json` is a conservative inventory, not a test-results database.
Schema v2 reports implementation progress independently of compatibility.
Partial entries name Rust paths, contract suites, experimental installation paths
and legacy counterparts. `cargo xtask capabilities` prints both counts. A verified
entry requires complete implementation, contract evidence, default Rust installation
and a removable legacy counterpart; missing evidence fails the gate. These fields
record reviewed evidence, rather than automatically certifying every test run.
The installed full-profile workflows must also pass before distribution
cutover. CI retains cross-platform checks; local measurements alone cannot prove
macOS/Windows parity.

The proposed binary/RSS/paint/retrieval performance targets are not yet certified.
No legacy application entrypoints were removed. Distribution remains blocked
until capability parity is verified; no release has been published.

## Source migration ledger

`source-baseline.json` freezes the tracked TS/TSX/MTS/CTS, JavaScript,
Python and Julia paths at its recorded commit, including declarations, fixtures,
examples and generated sources. `source-ledger.json` retains one entry per path.
The initial inventory contains 5,220 paths. Reviewed entries retain their evidence
while other paths require individual contract review; this enumeration does not
establish that step 1 of the migration is complete.

Run `cargo xtask migration` to validate coverage and deletion evidence, or
`cargo xtask migration --require-complete` to enforce the source cutover gate.
Rust CI runs the coverage gate independently of capability parity. The command
uses the Git index, so stage intended deletions before checking removal evidence.
New tracked source files require ledger entries. Baseline paths must never be
removed just because their source was deleted; retain their completed records.
The gate checks the baseline against the original Git tree, so shrinking both
ledger and baseline cannot hide deletions. Shallow checkouts must fetch the
recorded baseline commit; CI checks and release parity fetch history for this.
Rust release requires both source completion and capability parity before it
can create a release commit or tag.

For each path, review its behavior, consumers, persisted formats, dependencies,
Rust destination and verification requirements. Use explicit `none` where a
review finds no persisted format or dependency; null means unreviewed. Record
replacement, intentional retirement, fixture materialization or browser glue as
separate dispositions. Retirement needs a rationale; all other reviewed entries
need a destination. Record concrete contract suites and installed-workflow evidence
in verification requirements before changing their independent status fields.
Evidence strings are reviewed assertions, not an automatic certification of tests.

Implementation, behavioral verification, installed verification and legacy removal
are independent. Deletion requires completed implementation (or retirement work),
both verification stages and reviewed contract analysis. Fixture sources must move
to data assets and be materialized by Rust tests. Only documented browser bootstrap
JavaScript may remain as browser glue; application logic and TypeScript cannot use
that exception. Capability parity, installed workflows, platform validation and
release consolidation remain separate mandatory gates.

Initial ledger validation: pinned `nightly-2026-08-08` formatting, strict
Clippy and `cargo xtask test -p xtask` passed (11 tests plus the doctest pass).
`cargo xtask migration` reports 0/5,220 completed entries, and the completion
gate rejects that unfinished inventory as intended. CI includes xtask tests in
its cross-platform core/full matrix; those remote runs remain pending.

## November 3 inherited Claude domain integration

The active continuation branch is recorded in HANDOFF.md. Existing Claude
worktree code for native chat/TUI, Mnemopi and managed/session/memory tools is
integrated with its real Rust consumers. The ordinary engine is shared through
the Rust host; retained TS production consumers remain. One aggregate all-target
compiler check passed; behavioral/addon/installed/platform evidence is pending.
See CONVERSION-PASS.md for exact remaining ownership and HANDOFF.md for the next
resume action. No source removal or retained capability parity is claimed.

## November 4 shared-checkout continuation

Native Unix input now connects the existing bounded parser/key owner to the
editor with opt-in strict paste rejection, atomic paste/Enter handling and resumed
history. Native memory lifecycle connects both session hosts with settings gates,
transient recall, acknowledged durable retention and explicit shutdown flushing;
benchmarks opt out of automatic learning. Source inventory handles registered
nested worktrees without hiding ordinary owned sources. Focused input/lifecycle,
playbook and runner regressions passed; see HANDOFF.md for exact commands and
remaining parity/platform checks. No TS deletion or verified-state advance.

## November 4 direct-main conversion sweep

Latest user direction is direct main edits with tests deferred. Native editor
undo/completion/versioned prompt history and shared Markdown display now connect
to real UI consumers. Ordered LSP file resources use the existing writer with
cross-client provenance/cache synchronization; per-file workspace EditorConfig
formatting is connected. Configured memory generation uses the existing model
owner, child tasks share backend ownership, and actual host turn boundaries refresh
file-backed memory configuration. New code is uncompiled/untested; source and
capability states remain partial. HANDOFF.md records exact remaining contracts.

## Native skill invocation and resources (conversion-only, 2026-11-05)

Both Rust session hosts now expand enabled leading/embedded skill commands through
the canonical catalog. Selected instructions persist as a bounded versioned user
content envelope, preserving the original request for TUI/history and automatic
memory. Native `/skills` and completion share the existing worker/catalog. Companion
UTF-8 files and bounded directory listings use the original source jail and canonical
skill-directory containment without edit provenance. Source records remain partial;
legacy consumers remain, and plugin-source/encoded/non-text/legacy-interchange
contracts and consolidated verification are pending. No compiler or tests executed;
see HANDOFF.md for exact source connections and resume instructions.
