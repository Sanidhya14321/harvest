# Approved migration scope

The revised implementation plan replaces the earlier optional JS compatibility
host. Rust will own the application, both UIs, SDK, extension host, workers,
generators and release tooling. The existing CLI remains TypeScript until the
retained capabilities and installed artifacts pass their cutover gates.

## Intentional interface changes

- JS/TS SDK import paths, object identity, synchronous JS callbacks, extension
  factories and subclasses are replaced with Cargo APIs and versioned Rust/WASM
  plugin interfaces. Consumers must migrate. Retained behavior, including
  injected services, custom providers/tools, hooks, editors, renderers and
  dialogs, remains implementation work in the Rust SDK/plugin capability rows.
- Third-party plugins use portable Wasmtime components with WIT contracts.
  Host-mediated filesystem mutations, credentials, subprocesses and approvals
  retain their policies. Callbacks execute outside session locks; reload drains
  or cancels active work, detaches registrations and rejects stale handles.
- New JS/Bun Eval execution is retired. Python and shell execution, browser
  automation through native services/Python, and display of historical JS Eval
  records remain retained requirements. New JS requests must produce an explicit
  removal error. The current JS worker and Eval implementation have not yet been
  removed; their retirement status remains pending.

The [SDK operation mapping](sdk-mapping.md) identifies existing experimental
Rust entry points and the SDK/plugin replacements still required.

## Source and installation boundary

Generated browser bindings and startup code are allowed. Browser application
logic and rendering move to Rust/WASM. A generated npm/Bun launcher may select a
platform binary and start Rust; it must not contain application logic. Standalone
names, executable aliases, checksums, installation/update/rollback behavior,
protocol framing and storage contracts remain retained requirements.

Authored JS/TS implementations are deleted only after their retained replacement
contracts pass. Transition N-API adapters and missing-addon fallbacks are
currently necessary to keep the TypeScript CLI working. Their presence does not
establish final Rust ownership. Language fixtures and vendored dependencies may
remain. Commits, tags and publishing require a separate user request.

## Capability accounting

Manifest schema 3 defaults entries to `retained`. An entry marked `retired` must
name this approved decision, its reason and the retained replacement capability
IDs, unless it is explicitly JS Eval removal. SDK import retirement does not
retire the operations formerly available through those imports.

Retired entries remain unverified and absent for Rust parity accounting.
Retirement removal is tracked separately: `pending` blocks distribution;
`removed` requires removal contract suites and an explicit removable legacy
record. A manifest containing only retirements cannot establish parity. The
release gate requires every retained entry to have complete Rust implementation,
contract evidence and default Rust installation, and every retired entry to have
verified removal. Local baseline passes cannot certify other target hosts.
