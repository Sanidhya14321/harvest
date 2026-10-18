# Core migration audit — 2026-10-16

The application has **not** completed the TypeScript migration. The normal
`harvest` and `omp` executables, session orchestration, providers, integrations,
SDK and extension host still run TypeScript. The existing Rust code is useful
production infrastructure plus an experimental application, not a replacement
that can safely become the default public release.

## Review scope and limits

This report follows the root `AGENTS.md`, `packages/coding-agent/DEVELOPMENT.md`
and the revised scope in this directory. No commits, tags or publishing occurred.

Machine inventory covered every file under the following source roots. The
authored-code candidate count includes `.ts`, `.tsx`, `.js`, `.jsx`, `.rs`, `.py`
and `.wit`, excluding `.d.ts` and names containing `.generated.`. This is a
reproducible coarse classification, not a claim that every candidate is authored
or that every line was semantically reviewed. Generated JSON/KDL assets, prompts,
fixtures and vendor trees must retain their separate ownership classifications.

| Source root | All files | Code candidates | Candidate lines |
| --- | ---: | ---: | ---: |
| `packages/coding-agent/src` | 1,779 | 1,395 | 501,636 |
| `packages/agent/src` | 43 | 28 | 16,193 |
| `packages/ai/src` | 309 | 238 | 107,885 |
| `packages/catalog/src` | 265 | 69 | 34,046 |
| `packages/mnemopi/src` | 66 | 66 | 19,882 |
| `rust-agent/src` | 17 | 16 | 3,180 |
| `crates/harvest-runtime/src` | 8 | 8 | 1,029 |
| `crates/harvest-brain/src` | 9 | 8 | 1,922 |

Fully read: all source and static prompt files in `rust-agent/src`,
`crates/harvest-runtime/src` and `crates/harvest-brain/src`;
`crates/pi-natives/src/brain.rs`;
`packages/coding-agent/src/core/harvest/brain.ts`,
`packages/coding-agent/src/core/agent-session.ts`,
`packages/coding-agent/src/session/session-provider-boundary.ts`,
`packages/catalog/src/build.ts`, and
`packages/ai/src/providers/register-builtins.ts`.
Fully read contract suites: runtime filesystem/cancellation, Rust agent-session,
provider cancellation and session-format integration tests.

Selected consumer sections and public methods were read in the TypeScript CLI,
`main.ts`, `sdk.ts`, session storage/manager/session, agent runtime, provider
dispatch, extension loader, Eval and memory backends. Their remaining lines and
the other inventoried TypeScript files have **not** received complete semantic
review in this audit. The larger native engines were inventoried but not fully
reread. Documentation review includes the subsystem map, migration README,
provider streaming contract, resolution-device runtime, and selected session,
SDK and extension-loading sections. A complete source-review task must preserve
this distinction instead of promoting inventory coverage to review completion.

## Actual ownership and retained contracts

| Domain | Existing Rust ownership | Missing cutover contracts and source evidence |
| --- | --- | --- |
| Default application | Experimental `harvest-omp` library and two executable aliases. | Root `package.json:77` runs `src/cli.ts`; `packages/coding-agent/package.json:19` and `:22` export/run TypeScript. `main.ts:1890` creates the TS session; `:1912`, `:2081`, `:2109` launch ACP, RPC and interactive mode. |
| Session loop | Generation-scoped cancellation, provisional text events, durable committed events and serial primitive tool execution. | `rust-agent/src/agent_session.rs:81` is synchronous; `:173` has a fixed `max_steps` loop; `:190` executes tools serially. No subscriptions, steer/follow-up queues, async jobs, retry/fallback policy, compaction, delegation, reload or shutdown interfaces. Production methods remain in `packages/coding-agent/src/session/agent-session.ts:4252`, `:4489`, `:5332`, `:6017`, `:6689`, `:6708`, `:7575`, `:8800`. |
| Persistence | Lossless bounded JSONL inspection, v1/v2 in-memory migration, title-slot codec, selected-branch traversal, append-only v3 writes, shared writer ownership and content-change detection. | `durable_session.rs:48` rejects legacy-session writes; `:51` rejects damaged/unterminated files; `:82` rejects unadapted entry types; `:146`, `:161`, `:169`, `:183` reject opaque, signed, native-tool and other content replay. Blob stores, foreign imports, compaction/branch-summary reconstruction, remote/indexed stores, session moves/forks/listing and crash repair remain TS. Raw-value preservation by the codec is not provider replay parity. |
| Provider transport | One bounded OpenAI-compatible chat-completions implementation, SSE text/function assembly, cancellation that closes pending network operations, host-thread reentrant callbacks. | `provider.rs:144` always joins `/chat/completions`; `chat.rs:17` represents content as an optional string. No Anthropic/Responses/Codex/Google/Bedrock/Ollama/Cursor/Devin/etc. native wire adapters, image/thinking/signature handling or custom-provider services. `durable_session.rs:236` explicitly records unavailable usage with zero counters. TS `providers/register-builtins.ts` and `docs/provider-streaming-internals.md` describe the retained event/usage/watchdog/retry contracts. |
| Settings, secrets, catalog, OAuth | Basic flags/environment/TOML precedence and agent-directory selection. | `rust-agent/src/config.rs:11` has only model/base URL/API key/max steps; `:125` reads `.omp/config.toml`. This does not implement production YAML/settings profiles, secret obfuscation, credential storage/rotation, OAuth, discovery/cache or KDL resolution. TS `sdk.ts:1346` initializes Settings, `:1362` creates ModelRegistry, `:1389` hydrates credential caches; `catalog/src/build.ts:201` resolves the rule-owned model policy. |
| Tools and approvals | Five partial tools (`read`, `write`, `edit`, `grep`, `bash`), jailed filesystem primitives, deny-by-default mutation policy, explicit terminal approval. Shell uses `pi-shell` only with the `full` feature. | `chat.rs:105` defines only the five primitive schemas. They do not replace existing line/range/image reads, patch/edit/delete modes, streaming tool previews, virtual resolution devices, subprocess jobs, full approval modes or the other tools. `tools.rs:239` rejects shell execution outside `full`; `Cargo.toml:60` defaults to `core`. |
| Markdown brain | Production TS retrieval calls the shared Rust scanner, scoped BM25 and graph engine through N-API; generated-record admission and recoverable maintenance are implemented. | Production `sdk.ts:3452` still owns orchestration and TS message transforms. `CaptureSchedule` is defined in `harvest-brain/src/capture.rs:10` but is not connected to a production loop; evidence recording/leases are not connected to application outcome/retrieval lifecycles. Legacy learned history and reverse dependencies across other project roots are not migrated. SQLite Mnemopi is a separate retained subsystem, still TS (`memory-backend/resolve.ts:20`). |
| Rust SDK | Experimental Rust modules expose basic run/session/model/approval types. | `rust-agent/src/lib.rs:1` explicitly marks the runtime experimental. It does not provide the approved complete session, tool, provider, hook, editor, renderer, dialog or injected-host-service APIs. The TS SDK remains 4,450 lines and constructs TS Agent/AgentSession at `sdk.ts:3587`/`:3772`. |
| Rust/WASM plugins | No first-party component plugin host or versioned WIT contract in the current Rust application. | Workspace members and the fully read Rust application contain no Wasmtime host, plugin registrations, lifecycle draining/reload or stale plugin resource handles. A transitive `wit-bindgen` lockfile entry is not implementation evidence. Existing module discovery/execution remains in `extensibility/extensions/loader.ts`, JS hooks/custom commands and plugin management. |
| MCP/LSP/DAP/SSH/delegation/goals/advisor | Native process/edit/AST/VCS primitives can be reused. | The actual integration clients/managers and policies remain TS: `mcp/manager.ts:200`, `lsp/client.ts:649`, `dap/client.ts:54`, `ssh/connection-manager.ts:734`, `task/executor.ts`, `goals/runtime.ts:117`, `advisor/runtime.ts:265`. Rust application modules do not provide these services. |
| Python/browser execution and JS Eval retirement | Full Rust shell execution exists. | `eval/types.ts:2` still admits `python` and `js`; `tools/eval.ts:12` imports both backends, `:105` advertises both schemas. CLI workers and smoke still invoke JS Eval. Removal is pending, separately from retained Python/browser parity. Delete JS execution only after Rust/Python own browser services, retain historical JS record rendering, and return the explicit removal error for new JS requests. |

The 494-entry manifest currently contains 284 retained capabilities: 6 complete,
14 partial and 264 absent implementations, with **zero verified parity**. Its
210 retired interface rows are absent/unverified; retirement does not certify a
replacement or verified removal. These counts summarize the checked manifest,
not the percentage of source lines migrated or full application coverage.

## Security, data integrity and cancellation gates

1. **New session-file privacy repaired in this audit.**
   `JailedFs::append_checked` previously created files without Unix mode `0600`,
   unlike its atomic-write path. This now specifies `0600` at creation.
   The observable regression test creates a real session file under a permissive
   `umask 0` in an isolated child process and verifies private permissions and
   contents. Production TS storage already enforces private files and directories
   (`session-storage.ts:9`, `:18`, `:133`, `:137`). Existing Rust directory creation
   and permissions on preexisting files still need storage-policy parity review;
   the targeted fix does not claim that entire policy is complete.
2. **Recovery is fail-closed but incomplete.** Rust refuses damaged JSONL,
   unsupported replay and unresolved tool calls instead of silently rewriting
   them. Cancellation/crash after committing an assistant tool batch can leave
   pending results, which `durable_session.rs:290` rejects on reopen. A real
   recovery workflow with explicit interrupted results/ownership handoff is
   required before production sessions can move to Rust.
3. **Workspace jail primitives are substantial, consumer coverage is not.**
   `JailedFs` pins capability-relative parents, validates missing ancestors,
   rejects escape/symlink/hard-link append paths and shares the legacy lock
   namespace (`filesystem.rs:141`, `:177`, `:366`). Future delete/rename/archive/
   SQLite/LSP/native-local mutations must use the same authorization before
   dispatch. The primitive tests do not certify unported integration call sites.
4. **Network cancellation is implemented for the HTTP model path.**
   `transport.rs:84` selects cancellation while waiting for headers and `:93`
   while reading bodies; chunks and queues are bounded. The default `Model`
   implementation wraps a synchronous `complete` call, so arbitrary custom
   implementations cannot be interrupted while inside it (`provider.rs:20`).
   Terminal approval still blocks on a synchronous stdin read (`tools.rs:44`).
   Full runtime cancellation must cover plugin callbacks, approvals, tools,
   subprocesses, drains and shutdown as well as network reads.
5. **Provider compatibility includes event and error meaning.** Rust currently
   exposes only started/text-delta/committed/completed/failed session events,
   and errors as `anyhow` chains. Existing clients consume richer normalized
   block, usage, retry, approval, tool and protocol events. Matching a command
   name or final text does not preserve these contracts.
6. **Large-session append performance is unverified.** Every durable commit
   supplies the full-log expected digest; `filesystem.rs:404` rereads the file
   before appending. Across a growing session this produces quadratic cumulative
   verification I/O. The codec also materializes a bounded 64 MiB/100,000-entry
   document. These are inspectable implementation constraints, not measured
   regressions; paired large-session measurements and a scalable validated
   append strategy are required before cutover.

## Dependency order for one managed execution

Use one tracked program with independently reviewable phases and explicit gates;
do not make a single destructive source-deletion/default-switch operation.

1. Complete the reviewed source/consumer inventory and contract fixtures. Add
   missing retained surfaces before interpreting manifest completion. Freeze
   protocol, storage and extension migration documents plus paired baselines.
2. Finish shared storage/authorization/cancellation engines and their existing
   TS consumers. Port blob/import/recovery/branch/compaction/settings/auth/catalog
   foundations before a production loop reads or writes real session files.
3. Define Rust-owned wire/message/event types, model/provider interfaces and
   host services. Port every provider family and KDL resolution against the same
   deterministic upstream streams, error mappings and persisted fixtures.
4. Replace the fixed-step host with the production asynchronous session engine:
   subscriptions, steer/follow-up queues, scheduling/approvals, retry/compaction,
   delegation, ownership transitions, reload and shutdown. UI and SDK must share
   this owner rather than implementing parallel session engines.
5. Implement Rust SDK and the versioned WIT/Wasmtime plugin host on those same
   services, then bundled extension migrations and adversarial reload/policy
   tests. Retire JS APIs explicitly with migration examples.
6. Port remaining tools/integrations and Python/browser service bridges. Remove
   JS Eval schemas, workers, prompts, dependencies and settings only after the
   retained bridge contracts and historical record rendering are covered.
7. Complete the terminal/browser products against the shared async engine.
   These are separate UI workstreams covered by the UI audit; neither is made
   complete by native primitives or a small embedded matcher WASM payload.
8. For each completed consumer boundary, replace its tests/runners and delete
   authored TS implementations in the same change. Keep generated startup and
   bindings, language fixtures and vendors with explicit provenance. Run the
   required installed-artifact gates before switching the default launcher.

## Verification and public-release decision

Existing tests read in this audit defend useful narrow contracts: durable events
after real append, branch isolation, writer contention, outside-write refusal,
stale cancellation handles, provisional text not persisted, exact signed raw
JSON preservation, and cancellation of stalled headers/body reads. They do not
exercise a complete production runtime, Rust SDK, plugin host or both Rust UIs.

This audit ran strict `cargo clippy --locked -p harvest-runtime --all-targets --
-D warnings` successfully after the privacy fix. The sanctioned
`PATH=/tmp/harvest-tools:$PATH cargo xtask test -p harvest-runtime -p harvest-brain
-p harvest-omp` passed **76 tests, zero skipped**, followed by all three doctest
passes (zero runnable examples). This used the default/core feature set; the
`full` shell feature needs its separate sanctioned run. Local lock sockets and loopback fixtures
required the approved sandbox escalation after the restricted run reported
`EPERM`. The successful log is
`/tmp/harvest-core-audit-tests-2026-10-16.log`; no direct `cargo test` was run.
No fresh macOS, Windows/ConPTY, installed public artifacts,
live providers, registry publish or public-user end-to-end runs were performed.

Public default-Rust release remains blocked by the incomplete domains above,
the required parity/artifact/performance gates, and the unpublished Cargo
dependency closure. `rust-agent/Cargo.toml:71` requires first-party registry
packages; root `Cargo.toml:36` patches `brush-core` to a local fork. A detached
registry consumer must resolve the owned patched-shell dependency without
workspace overrides before Cargo distribution can be certified. Pipeline work
and public-release checks must reject these gaps, not mark them successful.

Current user authorization permits preparing public releases, but this core
audit does not justify publishing or replacing the working public application.

## Follow-up: current updater target compatibility

The bounded updater mismatch identified by the release audit is repaired in
`packages/coding-agent/src/cli/update-cli.ts`. The effective default GitHub
repository is now `Sanidhya14321/harvest`, matching the actual Git remote and
both installer families. Nonempty `HARVEST_REPO` overrides remain authoritative;
an empty value uses the default, matching the installer behavior. Mise derives
its backend from that same effective repository. The default Homebrew formula
is `Sanidhya14321/tap/omp`: the Rust release workflow publishes `Formula/omp.rb`
to the repository owner's `homebrew-tap`, which Homebrew names `owner/tap/omp`.
No new settings or package/channel migration was introduced.

Two related, observable update failures were repaired in the same module.
Release asset selection now retains the public `omp-*` prefix even when the
running application name is `harvest`; all eight target names and rejection of
unsupported targets are tested. Windows script takeover now installs and
retires the alias selected by the active shim (`omp` or `harvest`) and builds
fallback forwarders for that same alias. This preserves the selected launcher's
ownership and rollback behavior instead of switching an `omp.cmd` update to
`harvest.exe`. The existing explicit `binaryName` override is retained.

Verification passed **105 focused updater tests** across
`test/update-cli.test.ts` and `test/cli/update-cli.test.ts`, including real
fixture downloads/hash validation, alias replacement, failed-update rollback,
concurrent updates, package rename behavior, the eight artifact selectors and
fresh-process repository override checks. The run required the approved sandbox
escalation for existing native lock sockets and child output. Two stale test
fixtures were corrected: a filtered `@harvest/pi-utils` cache fixture used
`@oh-my-pi` directory names, and a rollback assertion still expected the old
application display name. Logs are recorded at
`/tmp/harvest-update-routing-tests-2026-10-16.log`.

These are current TypeScript updater repairs, not a completed Rust updater or
evidence that a public registry/release was reached. Future consolidation still
needs one Rust-owned repository/package/channel manifest, the Rust update
implementation, end-to-end old-installed-version migration, signed installed
artifacts, and actual Homebrew/mise installation on their target hosts. The
existing npm metadata/version selection and registry publication gaps remain.

The coding-agent `check:types` gate, focused oxlint/oxfmt checks and whitespace
validation also passed after these changes. No manifests, lockfiles, release
workflows or shared Rust files were changed by this follow-up.
