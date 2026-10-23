# Native executable plugin protocol v1

Conversion checkpoint: implementation is connected to the Rust candidate;
behavioral, installed-platform and failure-path verification is pending. The TS
application plugin loader has not switched over. This is the approved breaking
replacement contract, not a claim of compatibility with TS extension callbacks.

Host entrypoints: `harvest run --plugin PATH`, `harvest rpc --plugin PATH`,
`AgentSession::configure_plugins`, and `SessionEngineOptions::plugins`.
Configuration-declared executables require `--enable-plugins`; `harvest config`
only inspects configuration and never starts plugins. Plugins are trusted native
programs, not OS-sandboxed processes. They must use host calls for application
mutations; the host applies its existing jail and final-argument approval.

Frames are UTF-8 JSON objects, one per newline, at most 1 MiB including newline.
Stdout contains frames only. Stderr is drained separately into an 8 KiB tail.
No JavaScript runtime or plugin module loader is started. Language module paths
and known interpreter executables produce migration guidance.

1. Host sends `{"type":"initialize","id":0,"versions":[1]}`.
2. Plugin responds with `initialized`, matching `id`, `version:1`, a name and
   `tools`: ordinary function schemas with name, description and object parameters.
3. Host sends `call` with numeric `id`, registered tool `name` and object
   `arguments`. One call owns the process at a time; different processes may own
   separate sessions. Tool names may not collide with builtins or other plugins.
4. Plugin emits zero or more `event` frames for the active ID, then `result`
   with `value`, or `error` with `{code,message}`. Errors remain structured and
   untrusted diagnostic text is not automatically inserted into model output.
5. A plugin may send `host_call` with a distinct callback ID, `parent_id` equal
   to the active request, and an ordinary `ToolCall`. Host replies `host_result`
   with a string value or structured error. Each nested mutation needs its own
   final-argument approval; plugin declarations never grant blanket approval.
6. Host cancellation sends `cancel` and terminates the process on a failed or
   interrupted exchange. No ambiguous late response is reused. The Rust SDK
   delivers cancellation to the handler while its reader owns stdin.
7. Host sends `shutdown` on disposal and reaps the process. Graceful exit/draining
   behavior and descendant-process cleanup still need stabilization review.

Queues are bounded (8 outbound, 16 inbound frames). Calls have a host timeout
(default 30 seconds, maximum 10 minutes), at most 1024 events, 64 host callbacks
and 16 MiB of response frames. Requests and callbacks must match their owner IDs.

Declarative events are progress, notification and choice data. The host SDK
exposes an event callback. Native session/TUI event delivery and interactive
choice responses are unfinished; no executable UI callback compatibility exists.
Do not advertise those unfinished interaction workflows as supported.

Implement `harvest_omp::plugins::sdk::Handler` and call `sdk::serve_stdio` from a
Rust executable. `rust-agent/examples/native-plugin.rs` demonstrates a jailed
host read through this same protocol. Configuration arguments/environment are
trusted host inputs and omitted from Debug; do not put credentials on argv.
