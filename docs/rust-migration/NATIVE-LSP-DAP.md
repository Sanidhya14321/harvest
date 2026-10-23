# Native LSP/DAP conversion checkpoint

Language-server/debugger code is written and connected to the Rust candidate.
Behavioral, installed, performance and platform checks are deferred during the
conversion-first pass. The default TS application's implementations remain in
use; this checkpoint does not establish full language-tool parity or removal.

## One process and registry owner

`process_transport.rs` now supports bounded Content-Length framing in addition
to JSONL. LSP and DAP reuse its child, writer/reader queues, stderr tail, timeout
polling and direct-child shutdown. Plugins and MCP continue through JSONL on the
same owner. Headers are bounded to 8 KiB, JSON payloads to less than 1 MiB; reject
missing/duplicate/invalid lengths, truncation and malformed bodies before decoding.
No per-service worker script, JS shim or second process implementation is added.

`language_services::{lsp,dap}` provide ordinary Rust clients;
`language_services::Runtime` registers the `lsp`/`debug` tools in the existing host
registry. Their names are reserved against plugin/MCP collisions. Sync sessions,
async sessions, RPC and direct SDK users share these APIs and approval boundaries.

## Native host configuration and entrypoints

Add explicitly selected server/adapter configurations to native TOML:

```toml
[[lsp]]
name = "rust"
command = "/path/to/rust-analyzer"
language_id = "rust"
extensions = ["rs"]
timeout_ms = 30000

[[dap]]
name = "lldb"
command = "/path/to/lldb-dap"
timeout_ms = 30000
```

Optional args/env/cwd belong to host configuration. LSP initialization_options
and settings, and DAP launch_defaults/attach_defaults, are structured data. Debug
output omits these potentially sensitive settings, argv and environment values.
The common profile/overlay resolver owns these arrays and validates duplicate
names/limits. Config inspection reports counts and launches no programs.

Run/RPC require `--enable-lsp` and/or `--enable-debug` to expose these tools.
Servers/adapters start lazily on actual calls. Rust hosts use
`AgentSession::configure_language_services` or `SessionEngineOptions::{lsp,dap}`.
One owned runtime keeps debugger state and open language documents across calls.

`harvest lsp --arguments JSON` and `harvest debug --arguments JSON` use the same
configuration/profile selection and runtime. Add `--interactive` in a terminal to
keep ownership while entering further JSON requests; `exit`/`quit` closes it.
One-shot mode disposes its clients after the response. Use a persistent session,
SDK owner or interactive mode for multi-command debugging. Mutations/debugger
requests use existing host approval; no blanket approval flag is introduced.

## Written LSP behavior

Initialize with workspace/root and UTF-16 capabilities; reject a different server
position encoding. Open/synchronize bounded jailed text documents with versions,
full/incremental updates and configuration delivery. Respond to supported server
configuration/workspace/progress requests; unsupported methods return -32601.
Request IDs are monotonic and bounded. Error messages retain numeric codes and
omit untrusted server error text. Interrupted/ambiguous exchanges close the owner.

Native actions: hover, definition, type_definition, implementation, references,
symbols, diagnostics, rename, code_actions, format, request, reload, status and
capabilities. Select a server explicitly when file-extension matching is ambiguous.
Positions use 1-based tool lines plus 0-based UTF-16 characters, or an unambiguous
symbol substring with an optional #N occurrence. Raw requests accept structured
or JSON-string payloads. Code-action application needs a unique query (a title
substring or zero-based index) and a supported edit-only action.

Pull diagnostics use the advertised provider; push results are bounded and report
availability/version rather than treating an empty cache as a clean document.
Formatting uses explicit options or content indentation detection with the legacy
2-space fallback. An encountered .editorconfig requires explicit options until
its full resolver is ported; do not silently overwrite its indentation policy.

Workspace text edits validate local URIs/jail, UTF-16 boundaries, surrogate pairs,
overlap, insertion order, duplicate ranges, document versions and byte/count
limits before mutation. Ask for approval of the actual returned edits using a
fresh approval identity, then recheck sources. Individual file replacements are
atomic. Return exact changed files and partial-failure file index; do not claim a
multi-file transaction. Changed open documents/watch events are notified, with
synchronizationPending retained when notification delivery fails. SDK hosts can
inspect applied-edit reports and notifications.

Create/rename/delete resource operations reject before any writes. Combined
edit/command code actions and rename_file remain unconverted; no portion of an
unsupported resource edit is silently applied.

## Written debugger behavior

DAP uses the same Content-Length transport, initialized capabilities, monotonic
sequence numbers, bounded pending-response correlation and event/output queues.
Launch/attach defaults are merged as JSON and included in final host approval.
Initialization/configurationDone and out-of-order launch responses have a shared
request owner. Reverse terminal/start-debugging requests are denied and are not
advertised. No model-selected shell wrapper is started by the host.

Native actions cover launch/attach, source breakpoint set/removal, continue,
step-over/in/out, pause, threads, stack_trace, scopes, variables, evaluate,
read/write memory, disassemble, modules, loaded_sources, custom_request, output,
terminate, disconnect and sessions. Thread operations require an explicit
thread_id. Cached source breakpoint sets update only after successful responses.
Structured adapter failure is distinguished from a corrupted/closed exchange.
Cancellation attempts the advertised cancel request and closes ambiguous state.
Disconnect cleanup distinguishes an attached target from a target this client
launched; complete descendant cleanup remains unverified.

## Deferred conversion/stabilization contracts

- Legacy LSP/DAP defaults, config discovery, root markers, server/adapter selection,
  multiplexing/pooling/restart, project warmup, .editorconfig resolution and linters.
- Complete diagnostic waiting/workspace batching, stale/version-less diagnostics,
  idle event delivery, file write-through hooks and dynamic capability management.
- Resource operations/rename_file, code-action commands, advanced rename workflows,
  cross-file journaling/recovery, cooperating writer locks and filesystem races.
- Coordinate edge cases, edit ordering/UTF-16 bounds, cancellation/CPU budgets,
  large files/responses and non-UTF-8/platform URI/path handling.
- Adapter TCP/Unix socket modes, discovery, reverse terminal/child sessions,
  instruction/data/function breakpoint wrappers, adapter-specific defaults and
  prelaunch configuration. Explicit custom requests expose the protocol but do
  not certify the missing higher-level workflows.
- Debug stopped/running/error transitions, stale frame/variable references,
  startup interruption, process exit, backpressure, cancellation and target cleanup.
- Native session/TUI events and complete installed workflows; all retained TS
  language/debug consumers and source deletion remain pending.

Consolidated checks must exercise actual native LSP/debugger workflows, then the
existing shared provider/plugin/MCP/config/browser/session integration suites.
This checkpoint uses aggregate cargo check plus ownership metadata validation;
no behavioral tests, Clippy, addon rebuild or installed certification is claimed.
