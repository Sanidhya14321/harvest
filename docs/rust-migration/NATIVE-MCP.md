# Native MCP conversion checkpoint

The native stdio and streamable HTTP client, tool bridge, configuration and CLI
are written and connected. Behavioral, installed and platform checks are deferred
under the conversion-first instruction. This does not complete the full MCP
product or switch the TS application over.

## Connections and configuration

`harvest_omp::mcp::Client` owns one initialized server connection. The native
`ToolRuntime` registers its tool schemas alongside builtins/plugins; `AgentSession`
and `SessionEngineOptions::mcp` share that owner across sync/async/RPC consumers.
Every model-originated remote call requires host approval for its final original
arguments. Remote readOnlyHint/annotations cannot grant permission. Server roots
and ping requests are supported; sampling, elicitation and other unadvertised
server-to-client methods return -32601.

Native TOML configuration supports named servers:

```toml
[[mcp]]
name = "files"
type = "stdio"
command = "/path/to/native-mcp-server"
args = []
timeout_ms = 30000

[[mcp]]
name = "remote"
type = "http"
url = "https://mcp.example.org/mcp"
timeout_ms = 30000
```

Run/RPC select native-configured servers with `--enable-mcp`. Explicit
`--mcp-config PATH` loads enabled entries from a strict JSON `mcpServers` file.
Such a file defaults missing `type` to stdio and respects disabledServers over
enabledServers within that file. Server names must be unique across selected
files; cross-source conflict resolution/discovery remains pending. Optional args,
env, cwd, headers, requestIdFormat and positive timeout values are supported.
Environment values are literal: no shell evaluation or hidden expansion occurs.
Explicit native configurations and executables are trusted host inputs.

`harvest mcp --server NAME [--mcp-config FILE]` offers `status`, `tools`,
`resources`, `templates`, `prompts`, `read URI`, `prompt NAME --arguments JSON` and
`call NAME --arguments JSON`. Otherwise it uses the same native profile/overlay
resolver (`--profile`, `--config`, `--agent-dir`). It connects only the selected
server. An explicit CLI call is a host action; model calls use Approval.
Redacted `harvest config` reports configured server counts without starting them.

## Written protocol behavior

- Initialize with 2025-11-25 and negotiate a known returned version; send
  notifications/initialized before discovery. Preserve server info/capabilities.
- Numeric request IDs stay within JavaScript's exact integer range. String IDs
  include a connection owner and monotonic counter. Reject mismatched, ambiguous
  or malformed responses. Structured RPC errors expose numeric codes without
  server-provided secret text and do not themselves invalidate the connection.
- Bound JSONL frames, queues, stderr tails, SSE frames, total response bytes,
  callback/notification counts, list entries, cursor repetition and pagination.
- Tools/resources/templates/prompts use bounded cursor pagination; retain original
  tool definitions and parameters. Namespaced host aliases include a stable digest
  and collision checks; calls map back to exact original server names.
- Resource reads/subscription requests and prompt retrieval preserve structured
  results. Tool isError data remains distinct from JSON-RPC transport failure.
- Stdio cancellation notifies and closes the ambiguous process connection; HTTP
  cancellation drops pending I/O and attempts a bounded cancellation notification.
  Do not retry potentially mutating operations automatically after interruption.
- HTTP supports application/json or bounded SSE responses, session/version headers,
  origin-locked redirects, owned header protection and bounded DELETE cleanup.
  Session IDs cannot silently change. Credentials/URLs are absent from errors.
- Plugins and MCP reuse `process_transport.rs` for one child owner, bounded JSONL
  queues, cancellation polling, separate stderr and bounded graceful-EOF shutdown.
  Providers and MCP reuse the existing async/cancellable HTTP owner and SSE reader.

## Unfinished conversion and stabilization

OAuth/credential storage and refresh, external config-source discovery, JSONC,
nonliteral environment expansion, disabled timeout semantics, legacy SSE GET/
endpoint transport, reconnect/idempotency/retry policy and pooling remain pending.
Unsupported OAuth/SSE inputs fail explicitly; no runtime fallback starts JS.

Subscriptions and notifications have request/SDK hooks, but background event
consumption, HTTP GET resumption/Last-Event-ID, session/TUI delivery, automatic
registry refresh and hot reload are unfinished. SDK clients can explicitly refresh
tools and receive notifications during active requests. Multimodal native tool
rendering remains limited by the current text-oriented session boundary.

OS-specific process/session/console ownership, descendant cleanup, process startup
interruption and shared-transport regression checks need stabilization. The shared
owner currently drains on stdin close for at most 100 ms before reaping its direct
child. This is not evidence of complete cross-platform process-tree cleanup.

Consolidated future checks must cover stdio/HTTP JSON/SSE, all pagination and
server callback paths, session identity/headers/redirects, cancellation at every
stage, resource/tool errors, duplicate names, final-argument approval, queue/output
limits, process exit and native installed workflows. Run the existing provider,
plugin, browser, config and session suites against these shared-interface changes.
Keep all legacy MCP sources until their remaining consumers and contracts move.
