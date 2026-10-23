# Conversion-first pass: shared ownership and feature register

Active task: **`application:conversion-first`**, state **in_progress**.
Base: `64f3dd7`. Current owner: the active migration agent on `ts-to-rust`.
The latest user instruction changes scheduling: convert connected code first,
then resolve errors and test together. `PLAYBOOK.md` records this across sessions.
Do not revert to per-file test/addon cycles on resume.

## What is written versus what is done

“Written and connected” describes code and consumers. It does not mean behavioral
parity, installation verification, completed migration or eligible source deletion.
The machine manifest retains these distinctions. No feature becomes done from a
compiler result. This checkpoint has no behavioral or installed evidence.

## Reserved interfaces and file ownership

One owner per row. Do not introduce another registry, configuration resolver,
plugin process owner or benchmark scorer beside these paths. Coordinate changes
through this table and the machine task before editing a shared interface.

| Feature | Sole Rust destination | Consumers connected | Remaining work | State |
|---|---|---|---|---|
| Tool schemas/registration | `rust-agent/src/chat.rs`, `tools.rs` | OpenAI/Anthropic model requests; sync/async session tool runtime | schema/collision regressions and dynamic registry integration tests | Written/connected; unverified |
| Native plugin v1 host | `rust-agent/src/plugins/{protocol,host}.rs` | `ToolRuntime`, `AgentSession`, `SessionEngineOptions`, run/RPC `--plugin` | cancellation/exit/queue/approval tests; graceful draining and descendant cleanup | Written/connected; unverified |
| Rust plugin SDK | `rust-agent/src/plugins/sdk.rs`, `examples/native-plugin.rs` | protocol authoring, host callbacks, cancellation, typed UI events | copied external consumer; version/malformed/process-exit tests; installed platforms | Written/connected; unverified |
| Plugin UI | same protocol Event enum; SDK callback | direct host SDK callback | session/TUI forwarding and interactive choice responses | Partial; unfinished |
| Native settings/profiles | `rust-agent/src/configuration.rs`, `config.rs` | sync/async run, RPC, SDK, redacted `harvest config` | legacy YAML/JSONC/schema/external-source parity, settings persistence/UI, credential ownership | Written/connected foundation; unfinished parity |
| Rust edit scoring | `rust-agent/src/edit_benchmark` | native CLI list/compare/apply and ordinary Rust APIs | normalization/patch/file-set bounds tests; actual model runs and usage integration | Written/connected; unverified |
| Benchmark generation/orchestration | same Rust edit-benchmark module and future native metaharness owner | existing TS runner still active | mutation generation, archive handling, shape analysis, orchestration and consumer cutover | Not converted |

## Inputs, connections and deletion rules

Inputs: existing plugin/extension interfaces under
`packages/coding-agent/src/extensibility`, config/config-file/settings sources,
`packages/typescript-edit-benchmark/src/{tasks,verify,in-process-client}.ts`,
metaharness edit adapter, current native tools/provider/session engines.
The source ledger continues owning every retained legacy path through its parent
migration task. This leaf reserves native destination interfaces; it does not
certify the unported parent or move a source row to verified removal.

Implementation order: owned tool schemas; executable protocol/host/SDK; connect
session ownership and host approval; shared configuration/profile selection;
connect native launchers; versioned benchmark loader/apply/scorer/CLI. All are
written before the consolidated stabilization phase. Reuse existing jailed file,
edit, transport, provider and session engines. No new dependency or N-API adapter.

No TS source is deleted in this checkpoint: the production application, plugin
loader, settings implementation and metaharness still consume them. Default
launchers remain on the current product until retained behavior is connected and
installed verification passes. Legacy language plugin loading is retired by the
native ABI; the old TS application's retirement diagnostic/cutover is unfinished.

## Stabilization queue (deferred, not passed)

- [ ] Plugin frame/handshake/version/schema/name/identity bounds and corruption.
- [ ] External Rust SDK example against copied native host; stdout/stderr isolation.
- [ ] Final-argument approval, denied calls, nested jailed mutations, denied escapes.
- [ ] Cancellation before initialization, during reads/writes/host calls; timeout,
      early exit, response truncation, EOF and bounded stderr/queue flooding.
- [ ] Startup interruption, graceful shutdown/draining and descendant cleanup;
      aborting the owned process must not leave threads or children behind.
- [ ] Deliver declarative events to session/TUI clients and define choice responses.
- [ ] Full plugin discovery/install/update/marketplace/reload and TS callback
      migration guidance; do not equate protocol support with the plugin product.
- [ ] Configuration precedence and profile inheritance/cycles across global,
      project and explicit overlays; missing/invalid/secret-containing inputs.
- [ ] Complete legacy YAML/JSONC/schema/config discovery compatibility and settings
      UI/edit persistence; explicit profile and credential/KDL migrations.
- [ ] Regress existing provider, browser, native tool and session/RPC contracts
      against the shared owned-schema/configuration changes.
- [ ] Benchmark CRLF-only normalization, exact file sets, binary/text bounds,
      interrupted/failed/skipped edits, isolated workspace writes, usage unknown
      versus zero, score version isolation, inert historical fixture retention.
- [ ] Complete benchmark generation/archive/orchestration and metaharness cutover.
- [ ] Run consolidated behavioral, installed-platform and full transition checks;
      update receipts and only then update done/verified/removal claims.

## Checkpoint evidence and recovery

One aggregate `cargo check --locked -p harvest-omp --all-targets --all-features`
passes after connecting the complete checkpoint. Its initial missing BufRead
import was corrected. Logs: `target/migration/conversion-pass-connections*.log`.
Formatting ran once for the final tree. No tests, Clippy, native addon rebuild,
legacy suites or installed product verification were run for this checkpoint.
Machine ownership/card validation is metadata validation only.

Future stabilization commands: `cargo xtask test -p harvest-omp`, affected strict
Clippy/format, actual native plugin/CLI installation workflows, and the existing
source/capability verification plan. Receipts remain under ignored
`target/migration/`. Existing inherited-removal/platform/runtime blockers remain
open; no old failed result is converted into a pass.

Recovery: scoped revert of the conversion checkpoint, preserving user changes,
all session/settings files, inert inputs and historical benchmark scores. New
configuration loading never rewrites existing config files. Native plugin launch
is host opt-in, and configuration inspection does not start processes.

Completion: implement every retained contract above, connect the final consumers,
run consolidated checks, record exact evidence, then delete eligible superseded
sources. Until then this task stays in_progress and capabilities stay unverified.

## Full feature snapshot from capabilities.json

Generated from the authoritative manifest for this conversion checkpoint. Update
this section from that manifest, never by estimating percentages. “Partial” means
Rust code exists; it does not certify completeness. Parent-task owners remain
responsible for all outstanding contracts and legacy source ownership. Approved
retirements are listed separately from implemented features.

### storage

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `surface:session-v3` | partial | unverified | `migrate:storage` |
| `surface:session-v1-v2-import` | partial | unverified | `migrate:storage` |
| `surface:prototype-session-import` | absent | unverified | `migrate:storage` |

### catalog-providers

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `surface:streaming-providers` | partial | unverified | `migrate:catalog-providers` |
| `surface:oauth-credentials` | absent | unverified | `migrate:catalog-providers` |
| `surface:kdl-catalog` | absent | unverified | `migrate:catalog-providers` |

### memory

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `tool:memory_edit` | absent | unverified | `migrate:memory` |
| `slash:memory` | absent | unverified | `migrate:memory` |
| `surface:memory-backends` | absent | unverified | `migrate:memory` |
| `surface:automatic-session-learning` | partial | unverified | `migrate:memory` |

### plugins

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `compat:single-engine-owner` | partial | unverified | `migrate:plugins` |

### application

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `command:launch` | absent | unverified | `migrate:application` |
| `command:acp` | absent | unverified | `migrate:application` |
| `command:auth-broker` | absent | unverified | `migrate:application` |
| `command:auth-gateway` | absent | unverified | `migrate:application` |
| `command:agents` | absent | unverified | `migrate:application` |
| `command:bench` | absent | unverified | `migrate:application` |
| `command:cleanse` | absent | unverified | `migrate:application` |
| `command:commit` | absent | unverified | `migrate:application` |
| `command:completions` | partial | unverified | `migrate:application` |
| `command:__complete` | absent | unverified | `migrate:application` |
| `command:compress` | absent | unverified | `migrate:application` |
| `command:config` | partial | unverified | `application:conversion-first` |
| `command:dry-balance` | absent | unverified | `migrate:application` |
| `command:gc` | absent | unverified | `migrate:application` |
| `command:grep` | absent | unverified | `migrate:application` |
| `command:gallery` | absent | unverified | `migrate:application` |
| `command:git` | absent | unverified | `migrate:application` |
| `command:grievances` | absent | unverified | `migrate:application` |
| `command:images` | absent | unverified | `migrate:application` |
| `command:if-bench` | absent | unverified | `migrate:application` |
| `command:install` | absent | unverified | `migrate:application` |
| `command:join` | absent | unverified | `migrate:application` |
| `command:models` | absent | unverified | `migrate:application` |
| `command:plugin` | absent | unverified | `migrate:application` |
| `command:ps` | absent | unverified | `migrate:application` |
| `command:say` | absent | unverified | `migrate:application` |
| `command:share` | absent | unverified | `migrate:application` |
| `command:setup` | absent | unverified | `migrate:application` |
| `command:shell` | absent | unverified | `migrate:application` |
| `command:read` | absent | unverified | `migrate:application` |
| `command:render` | absent | unverified | `migrate:application` |
| `command:ssh` | absent | unverified | `migrate:application` |
| `command:update` | absent | unverified | `migrate:application` |
| `command:usage` | absent | unverified | `migrate:application` |
| `command:tiny-models` | absent | unverified | `migrate:application` |
| `command:token` | absent | unverified | `migrate:application` |
| `command:ttsr` | absent | unverified | `migrate:application` |
| `command:worktree` | absent | unverified | `migrate:application` |
| `command:search` | absent | unverified | `migrate:application` |
| `tool:read` | partial | unverified | `migrate:application` |
| `tool:security_scan` | absent | unverified | `migrate:application` |
| `tool:bash` | partial | unverified | `migrate:application` |
| `tool:edit` | partial | unverified | `migrate:application` |
| `tool:ast_grep` | absent | unverified | `migrate:application` |
| `tool:ast_edit` | absent | unverified | `migrate:application` |
| `tool:ask` | absent | unverified | `migrate:application` |
| `tool:debug` | absent | unverified | `migrate:application` |
| `tool:github` | absent | unverified | `migrate:application` |
| `tool:glob` | absent | unverified | `migrate:application` |
| `tool:grep` | partial | unverified | `migrate:application` |
| `tool:lsp` | absent | unverified | `migrate:application` |
| `tool:checkpoint` | absent | unverified | `migrate:application` |
| `tool:rewind` | absent | unverified | `migrate:application` |
| `tool:task` | absent | unverified | `migrate:application` |
| `tool:hub` | absent | unverified | `migrate:application` |
| `tool:todo` | absent | unverified | `migrate:application` |
| `tool:web_search` | absent | unverified | `migrate:application` |
| `tool:write` | partial | unverified | `migrate:application` |
| `tool:retain` | absent | unverified | `migrate:application` |
| `tool:recall` | absent | unverified | `migrate:application` |
| `tool:reflect` | absent | unverified | `migrate:application` |
| `tool:learn` | absent | unverified | `migrate:application` |
| `tool:manage_skill` | absent | unverified | `migrate:application` |
| `tool:search_code` | absent | unverified | `migrate:application` |
| `tool:sessions` | absent | unverified | `migrate:application` |
| `tool:presets` | absent | unverified | `migrate:application` |
| `hidden-tool:think` | absent | unverified | `migrate:application` |
| `hidden-tool:yield` | absent | unverified | `migrate:application` |
| `hidden-tool:goal` | absent | unverified | `migrate:application` |
| `slash:security` | absent | unverified | `migrate:application` |
| `slash:settings` | absent | unverified | `migrate:application` |
| `slash:setup` | absent | unverified | `migrate:application` |
| `slash:plan` | absent | unverified | `migrate:application` |
| `slash:plan-review` | absent | unverified | `migrate:application` |
| `slash:vibe` | absent | unverified | `migrate:application` |
| `slash:goal` | absent | unverified | `migrate:application` |
| `slash:guided-goal` | absent | unverified | `migrate:application` |
| `slash:loop` | absent | unverified | `migrate:application` |
| `slash:queue` | absent | unverified | `migrate:application` |
| `slash:model` | absent | unverified | `migrate:application` |
| `slash:switch` | absent | unverified | `migrate:application` |
| `slash:fast` | absent | unverified | `migrate:application` |
| `slash:skillful` | absent | unverified | `migrate:application` |
| `slash:extended-context` | absent | unverified | `migrate:application` |
| `slash:computer` | absent | unverified | `migrate:application` |
| `slash:prewalk` | absent | unverified | `migrate:application` |
| `slash:advisor` | absent | unverified | `migrate:application` |
| `slash:export` | absent | unverified | `migrate:application` |
| `slash:trace` | absent | unverified | `migrate:application` |
| `slash:dump` | absent | unverified | `migrate:application` |
| `slash:share` | absent | unverified | `migrate:application` |
| `slash:collab` | absent | unverified | `migrate:application` |
| `slash:join` | absent | unverified | `migrate:application` |
| `slash:leave` | absent | unverified | `migrate:application` |
| `slash:browser` | absent | unverified | `migrate:application` |
| `slash:copy` | absent | unverified | `migrate:application` |
| `slash:open` | absent | unverified | `migrate:application` |
| `slash:todo` | absent | unverified | `migrate:application` |
| `slash:session` | absent | unverified | `migrate:application` |
| `slash:jobs` | absent | unverified | `migrate:application` |
| `slash:usage` | absent | unverified | `migrate:application` |
| `slash:changelog` | absent | unverified | `migrate:application` |
| `slash:hotkeys` | absent | unverified | `migrate:application` |
| `slash:tools` | absent | unverified | `migrate:application` |
| `slash:context` | absent | unverified | `migrate:application` |
| `slash:extensions` | absent | unverified | `migrate:application` |
| `slash:agents` | absent | unverified | `migrate:application` |
| `slash:skills` | absent | unverified | `migrate:application` |
| `slash:git` | absent | unverified | `migrate:application` |
| `slash:hub` | absent | unverified | `migrate:application` |
| `slash:branch` | absent | unverified | `migrate:application` |
| `slash:fork` | absent | unverified | `migrate:application` |
| `slash:tree` | absent | unverified | `migrate:application` |
| `slash:login` | absent | unverified | `migrate:application` |
| `slash:logout` | absent | unverified | `migrate:application` |
| `slash:ssh` | absent | unverified | `migrate:application` |
| `slash:new` | absent | unverified | `migrate:application` |
| `slash:fresh` | absent | unverified | `migrate:application` |
| `slash:clear` | absent | unverified | `migrate:application` |
| `slash:drop` | absent | unverified | `migrate:application` |
| `slash:compact` | absent | unverified | `migrate:application` |
| `slash:shake` | absent | unverified | `migrate:application` |
| `slash:handoff` | absent | unverified | `migrate:application` |
| `slash:resume` | absent | unverified | `migrate:application` |
| `slash:tab` | absent | unverified | `migrate:application` |
| `slash:diagnostics` | absent | unverified | `migrate:application` |
| `slash:pin` | absent | unverified | `migrate:application` |
| `slash:btw` | absent | unverified | `migrate:application` |
| `slash:tan` | absent | unverified | `migrate:application` |
| `slash:omfg` | absent | unverified | `migrate:application` |
| `slash:cleanse` | absent | unverified | `migrate:application` |
| `slash:retry` | absent | unverified | `migrate:application` |
| `slash:debug` | absent | unverified | `migrate:application` |
| `slash:rename` | absent | unverified | `migrate:application` |
| `slash:move` | absent | unverified | `migrate:application` |
| `slash:wt` | absent | unverified | `migrate:application` |
| `slash:add-dir` | absent | unverified | `migrate:application` |
| `slash:remove-dir` | absent | unverified | `migrate:application` |
| `slash:dirs` | absent | unverified | `migrate:application` |
| `slash:exit` | absent | unverified | `migrate:application` |
| `slash:restart` | absent | unverified | `migrate:application` |
| `slash:marketplace` | absent | unverified | `migrate:application` |
| `slash:plugins` | absent | unverified | `migrate:application` |
| `slash:reload-plugins` | absent | unverified | `migrate:application` |
| `slash:force` | absent | unverified | `migrate:application` |
| `slash:live` | absent | unverified | `migrate:application` |
| `slash:pause` | absent | unverified | `migrate:application` |
| `slash:quit` | absent | unverified | `migrate:application` |
| `slash:commands` | absent | unverified | `migrate:application` |
| `slash:sidebar` | absent | unverified | `migrate:application` |
| `surface:settings-profiles` | partial | unverified | `application:conversion-first` |
| `surface:rust-sdk` | partial | unverified | `migrate:application` |
| `surface:extension-process-abi-v1` | partial | unverified | `application:conversion-first` |
| `surface:dynamic-tools` | absent | unverified | `migrate:application` |
| `surface:delegation-goals-advisor` | absent | unverified | `migrate:application` |
| `surface:codegen` | partial | unverified | `migrate:application` |
| `surface:release-installers` | partial | unverified | `migrate:application` |
| `surface:installed-parity-without-legacy-runtimes` | absent | unverified | `migrate:application` |
| `surface:session-tail-recovery` | partial | unverified | `migrate:application` |
| `surface:native-command-completion` | absent | unverified | `migrate:application` |

### terminal

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `surface:terminal-inline-transcript` | partial | unverified | `migrate:terminal` |
| `surface:terminal-input-resize-restoration` | partial | unverified | `migrate:terminal` |
| `surface:native-replay` | absent | unverified | `migrate:terminal` |
| `surface:native-stats` | absent | unverified | `migrate:terminal` |
| `surface:native-collaboration` | absent | unverified | `migrate:terminal` |
| `surface:native-run-dashboard` | absent | unverified | `migrate:terminal` |
| `surface:native-automation-dashboard` | absent | unverified | `migrate:terminal` |

### services

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `slash:mcp` | absent | unverified | `migrate:services` |
| `surface:rpc-v1-v2` | partial | unverified | `migrate:services` |
| `surface:acp-v1` | absent | unverified | `migrate:services` |
| `surface:mcp` | absent | unverified | `migrate:services` |
| `surface:lsp-dap` | absent | unverified | `migrate:services` |
| `surface:eval-ssh` | absent | unverified | `migrate:services` |
| `surface:native-browser-control` | partial | unverified | `migrate:services` |

### browser

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `command:stats` | absent | unverified | `migrate:browser` |
| `slash:stats` | absent | unverified | `migrate:browser` |
| `surface:stats` | absent | unverified | `migrate:browser` |
| `surface:benchmarks` | partial | unverified | `application:conversion-first` |
| `surface:collaboration` | absent | unverified | `migrate:browser` |
| `surface:python-automation` | absent | unverified | `migrate:browser` |

### media

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `surface:media-local-inference` | partial | unverified | `migrate:media` |

### tooling

| Feature | Rust code | Behavior verified | Owning task |
|---|---|---|---|
| `surface:native-installation` | absent | unverified | `migrate:tooling` |
| `surface:performance-baselines` | absent | unverified | `migrate:tooling` |
| `surface:registry-installation` | absent | unverified | `migrate:tooling` |
| `surface:artifact-signatures` | absent | unverified | `migrate:tooling` |

### Approved retirements

| Interface | Removal state | Replacement or removal contract |
|---|---|---|
| `command:browser-relay` | removed | surface:native-browser-control |
| `tool:eval` | pending | remove-language-eval |
| `surface:browser-wasm` | pending | surface:native-replay, surface:native-stats, surface:native-collaboration, surface:native-run-dashboard, surface:native-automation-dashboard |
| `sdk-import:typescript` | pending | surface:rust-sdk, surface:rpc-v1-v2 |
| `sdk-import:python-rpc` | pending | surface:rust-sdk, surface:rpc-v1-v2 |
| `compat:shell-completion-scripts` | pending | surface:native-command-completion |
| `compat:installer-scripts` | pending | surface:native-installation |
