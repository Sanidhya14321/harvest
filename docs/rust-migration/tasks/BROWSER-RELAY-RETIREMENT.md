# browser:relay-retirement — remove the owned Chrome extension and JS relay

Prerequisite: the verified native engine/CLI connection in `browser:cdp-core` (record
its checkpoint before making this task ready). Approved retirement:
`command:browser-relay`; replacement `surface:native-browser-control`.

Inputs: `packages/browser-relay`, the coding-agent browser relay server/bridge/
daemon/protocol and embedded executable extension assets, CLI command/loader,
relay mode settings, registry branches, target-claim messages and relay tests.
Do not confuse collaboration/IRC relay implementations with this browser relay.

Contract: the retired command, relay settings/env flag and explicit app.relay
requests must give actionable native-browser migration guidance; they must not
install an extension, bind a relay, auto-start its daemon or connect to the user's
ordinary Chrome profile. Users authenticate separately in the owned native
profile. Retain non-relay headless/spawned/connected/cmux behavior until its
consumers migrate. Keep license and third-party notice documents.

Steps: verify the native installed workflow and profile persistence first;
replace relay selection with an explicit retirement error; remove server/daemon
and extension source/assets/build/smoke generators; remove its CLI loader and
package workspace; remove relay-only tests after their now-retired contracts are
replaced by native retirement/ownership tests. Update the CLI help, settings and
migration guide. Regenerate Bun lock/Nix workspace expressions if needed under
the repository's supported commands. Preserve unrelated dependencies still used
elsewhere. Update each affected source row as an approved retirement, not a Rust
behavior-parity replacement. Keep all shared browser/eval/computer consumers pending.

Verification: actual installed Rust `browser` workflow with no JS runtime;
retired CLI invocation and settings produce the migration result; coding-agent
typecheck, targeted retained browser registry/tab/timeout suites and Rust
browser/tool/session suites. Verify source ownership and staged removals. The
full transition's existing singleton failures remain separate and failed.

Recovery: restore this bounded retirement and its command/selection connections
from the preceding commit. Preserve all existing user profiles, tabs and sessions;
never remove a user's already installed extension or browser data during migration.
