# browser:relay-retirement — remove the owned Chrome extension and JS relay

Prerequisite: the verified native engine/CLI connection in `browser:cdp-core` (record
its checkpoint before making this task ready: `1786b93`). Also requires
`control:embedded-sources` for the text-wrapped extension payload ownership. Approved retirement:
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

Checked connections: the CLI command table/help now omit the relay; argv routing
returns retirement guidance rather than forwarding it as a prompt, including
behind launch flags. The native CLI retains a hidden rejection command for old
invocations. The registry kind/acquisition/disposal branches and private target
claim are removed. Enabled raw relay configuration, explicit app selection and
the env opt-in are rejected before dispatch. Disabled historical settings are
inert. Remaining connected/headless/spawned/cmux modes keep their existing APIs.

Exact verification for this slice:

```text
cargo fmt -p harvest-omp -- --check
cargo clippy --locked -p harvest-omp --all-targets --all-features -- -D warnings
cargo xtask test -p harvest-omp --no-default-features --features core
cargo run --locked -p harvest-browser --example verify_cli -- COPIED_NATIVE_BINARY NATIVE_CHROME
HARVEST_TEST_BINARY=COPIED_NATIVE_BINARY cargo xtask test -p harvest-omp --no-default-features --features core --test cli
bun run --cwd packages/coding-agent check:types
bun test packages/coding-agent/test/tools/browser-schema.test.ts packages/coding-agent/test/cli-argv-routing.test.ts packages/coding-agent/test/tools/browser-dispose-timeout.test.ts packages/coding-agent/test/tools/browser-open-lease.test.ts packages/coding-agent/test/tools/browser-tab-timeouts.test.ts packages/coding-agent/test/tools/browser-profile-cleanup.test.ts packages/coding-agent/test/tools/browser-run-cancellation.test.ts
bun test packages/coding-agent/test/tools/browser-tab-worker-startup.test.ts
bun test packages/coding-agent/test/tools/browser-attach.test.ts packages/coding-agent/test/tools/browser-cmux-kind.test.ts packages/coding-agent/test/tools/browser-cmux-release-mid-run.test.ts packages/coding-agent/test/tools/browser-op-tracking.test.ts packages/coding-agent/test/tools/browser-lifecycle-leak.test.ts packages/coding-agent/test/tools/browser-orphan-registry.test.ts packages/coding-agent/test/tools/browser-stealth-acquire-debugerror.test.ts
```

The copied binary runs with an empty PATH and an absolute Chrome ELF. Native
CLI retirement calls must fail with migration guidance; no extension installer
or service is launched. Loopback static HTML replaces `data:` fixtures for the
viewport contracts because managed Chrome blocks that scheme on this host.
The original failed probes stay in the ignored receipts directory. The attach
retry check now counts only its main-frame target navigation, excluding favicon
and other background requests while retaining the exact one-attempt assertion.
Bun lock and Nix expressions are regenerated with the supported tools, keeping
Puppeteer dependencies for their still-pending consumers. Eighteen ledger rows
are approved retirements, not claims of retained-capability parity.
