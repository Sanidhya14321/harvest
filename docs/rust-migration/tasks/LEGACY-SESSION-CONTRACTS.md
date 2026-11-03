# control:legacy-session-contracts — resolve the remaining legacy baseline

Prerequisite: `control:playbook`. This task is blocked pending contract review;
no weakened check or completed parity is authorized.

The source-bound transition receipt on 747421f passes 452 Rust contracts plus
doctests, native rebuild and all 49 workspace chunks. It fails the 94-file
singleton bucket with 22 contract failures. Preserve the failed receipt at
`target/migration/verified-current.json`; the successful portions do not make it
a passing transition. The full names are also captured in
`target/migration/legacy-singleton-failures.json`.

Review bounded families independently: ACP fixture/production writer handover;
canonical and legacy configuration/discovery paths and prompt assembly; settings
coercion; managed-resource symlink errors and authorization; artifact/log labels.
Compare each failure with the recorded d811cb2 baseline under the same environment
before attributing a regression. Determine whether an obsolete fixture or a real
behavioral bug is responsible, then add reviewed leaf cards with exact consumers,
Rust destinations or necessary transitional repairs and checks. Do not disable
writer leases, drop assertions, weaken path authorization, discard legacy user
data or silently accept changed defaults to make the gate green.

Destination and implementation remain unresolved until that review. Keep
production changes bounded; preserve persisted session/configuration formats and
approval contracts. No source deletion is eligible here. Rerun the exact
`bun scripts/ci-test-ts.ts coding-agent-singleton` command for the family fixes,
then the source-bound transition gate. Recovery is a scoped revert of each future
leaf, preserving failed evidence and all user data.

Current failing contracts:

- ACP agent > supports multiple live ACP sessions with model and lifecycle handlers
- ACP agent > replays messageIds and returns turn usage for prompts
- ACP agent > replays assistant tool calls and matching results without duplicating the start
- ACP agent > does not replay internal Hub messages to ACP clients
- ACP agent > preserves tool_use input payloads when replaying assistant tool calls
- ACP agent > does not replay silent-abort marker as agent_message_chunk to ACP clients
- ACP agent > replays todo tool results as ACP plan updates
- ACP agent > refreshes task agent descriptions on ACP /reload-plugins
- autocompleteMaxVisible setting > should coerce submenu string values for live editor updates
- managed-skills primitives > writeManagedSkill > refuses to write through a symlinked skill directory
- managed-skills primitives > writeManagedSkill > refuses to update a SKILL.md that is a symlink
- managed-skills primitives > deleteManagedSkill > refuses to delete through a symlinked skill directory
- skillful setting and /skillful session toggle > lists skills in the system prompt by default and omits them when skillful is false
- skillful setting and /skillful session toggle > rebuilds the system prompt when toggled on an empty transcript without persisting
- skillful setting and /skillful session toggle > appends one hidden notice instead of rewriting the prompt when enabled mid-session
- SYSTEM.md prompt assembly > suppresses discovered SYSTEM.md while preserving the project footer
- SYSTEM.md prompt assembly > prefers project SYSTEM.md over user SYSTEM.md
- workstream B hardening > symlink and hardlink fixtures are refused and left unmodified
- report bundle logs > collects every same-day PID log, not only the current process
- disabledExtensions runtime filtering > can include disabled context files for dashboard-style loads
- structured subagent primitive > reloads model roles before resolving an agent added during the session
- structured subagent primitive > leases temporary artifacts for a retained invocation and registers them for agent URLs

## Runtime/session comparison at the Python retirement checkpoint

`bun scripts/ci-test-ts.ts coding-agent-runtime` ran all 34 groups (334 files):
15 groups passed and 19 failed. The raw report has 43 failing contracts. Three
expected Python-hook/state-execution assertions are superseded by the approved
retirement and their corrected rejection/shell-preservation tests pass.

All other **40 contracts fail on unchanged `2ff0193`**, replaying the exact 19
failing chunk commands in an isolated clean worktree with its own workspace
links. The HTML tool-view resource was rebuilt from baseline sources and the
unchanged verified native addon supplied. Initial baseline attempts with missing
dependencies/generated resources are invalid and retained as failed setup logs;
the final comparison is `python-retirement-runtime-comparison.json`, backed by
`python-retirement-base-chunk-*.log`. Logs are under `target/migration/`.

Do not treat this baseline match as a passing runtime suite or reduce the failing
contracts. Review/repair these families in bounded tasks before migration/release
acceptance. Remaining baseline-matched runtime contracts:

- test/acp-mcp-isolation.test.ts > createAcpSessionFactory TITLE_SYSTEM.md per-cwd resolution (PR #3736) > re-resolves the title prompt for the per-session cwd instead of inheriting the launch cwd's override
- test/agent-session-checkpoint-rewind-branch.test.ts > AgentSession checkpoint rewind branch context > tracks checkpoint and rewind through execute xdev write results
- test/agent-session-model-persistence.test.ts > AgentSession model persistence > marks an incomplete process-exit transcript aborted during SDK resume without dropping history
- test/agent-session-new-session-boundary.test.ts > AgentSession.newSession boundary > does not persist a delayed old-session message into the new session
- test/agent-session-plan-mode-convergence.test.ts > AgentSession plan-mode convergence > T3b: a propose write resets the convergence counter
- test/agent-session-prune-persistence.test.ts > AgentSession per-turn prune persistence > persists the pruned rewrite so a from-disk rebuild matches the live context
- test/agent-session-retry-fallback.test.ts > AgentSession retry fallback > auto-retries Anthropic stream-envelope failures before message_start
- test/agent-session-retry-fallback.test.ts > AgentSession retry fallback > auto-retries Anthropic stream-envelope failures before message_stop
- test/agent-session-retry-fallback.test.ts > AgentSession retry fallback > falls back on mid-stream Anthropic envelope failures without same-model retries
- test/agent-session-retry-recovery.test.ts > AgentSession retry recovery > keeps recovered markers durable across session reload and still excludes them from model context
- test/cli-max-time-flag.test.ts > parseArgs — --max-time flag > reports invalid --max-time values as CLI usage errors
- test/collab/session-replication.test.ts > SessionManager collab replication > ingestReplicatedEntry preserves foreign ids and advances the leaf
- test/custom-provider.test.ts > configureCustomProvider > normalizes baseUrl by appending /v1 when bare host is provided and saves to authStorage
- test/extensibility/legacy-pi-ai-type-remap.test.ts > legacy pi package root remaps (issue #1474) > routes @earendil-works/pi-utils through canonical Bun.resolveSync in non-compiled mode
- test/extensibility/legacy-pi-ai-type-remap.test.ts > legacy-pi @(scope)/pi-ai root `Type` remap (issue #1437) > does not redirect subpath imports such as @harvest/pi-ai/utils/schema
- test/extensibility/legacy-pi-ai-type-remap.test.ts > legacy-pi @(scope)/pi-ai root `Type` remap (issue #1437) > re-exports the legacy model catalog and schema helpers from the root
- test/extensibility/legacy-pi-ai-type-remap.test.ts > legacy-pi @(scope)/pi-ai root `Type` remap (issue #1437) > redirects `import { Type } from "@harvest/pi-ai"` for plugins published against the canonical scope
- test/extensibility/legacy-pi-bundled-virtual.test.ts > legacy-pi bundled virtual module synthesizer (issue #3423) > throws when asked to synthesize a key the bundled modules do not cover
- test/extensibility/legacy-pi-cli-exports.test.ts > legacy shim CLI exports > re-exports parseArgs and CONFIG_DIR_NAME from the legacy package root
- test/internal-urls/docs-tool-coverage.test.ts > omp:// root docs coverage > documents builtin tool presets
- test/internal-urls/docs-tool-coverage.test.ts > omp:// root docs coverage > documents builtin tool search_code
- test/internal-urls/docs-tool-coverage.test.ts > omp:// root docs coverage > documents builtin tool sessions
- test/main-session-resolution-error.test.ts > createSessionManager — missing session (#2084) > rejects --continue followed by an unknown session id instead of falling back to latest
- test/main-session-resolution-error.test.ts > createSessionManager — missing session (#2084) > rejects --fork with SessionResolutionError carrying a usage hint
- test/main-session-resolution-error.test.ts > createSessionManager — missing session (#2084) > rejects --resume with SessionResolutionError carrying a usage hint
- test/main-session-resolution-error.test.ts > createSessionManager — missing session (#2084) > rejects --resume with unknown id instead of falling back to latest persisted session
- test/marketplace/manager.test.ts > MarketplaceManager > installPlugin exposes marketplace package to the runtime loader
- test/marketplace/manager.test.ts > MarketplaceManager > installPlugin keeps same-name local runtime links visible
- test/marketplace/manager.test.ts > MarketplaceManager > installPlugin with force:true → replaces existing
- test/marketplace/manager.test.ts > MarketplaceManager > installPlugin with scope:project exposes the marketplace package to the runtime loader
- test/marketplace/manager.test.ts > MarketplaceManager > installPlugin → plugin in cache + in registry
- test/marketplace/manager.test.ts > MarketplaceManager > setPluginEnabled → persisted in registry
- test/marketplace/manager.test.ts > MarketplaceManager > uninstallPlugin → cache removed + deregistered
- test/plugin-stale-lockfile-entry.test.ts > stale lockfile-only directory plugin is skipped while declared and linked plugins load
- test/session/session-storage-permissions.test.ts > session private permissions > terminal breadcrumb round-trips through a private file
- test/shake.test.ts > AgentSession shake > elide > preserves mixed tool-result images while eliding only recoverable text
- test/shake.test.ts > AgentSession shake > thinking > drops both thinking variants, keeps empty turns empty, and refreshes persisted and runtime state
- test/task/executor-soft-budget.test.ts > runSubprocess soft request budget > a budget hard-abort keeps the kept-alive agent adopted and messageable via irc
- test/workstream-a-caller-ownership.test.ts > trusted caller snapshot propagation (A5) > a restricted background caller begets a restricted child
- test/workstream-c-laya-tombstones.test.ts > laya-setup skill is undiscoverable > never surfaces laya-setup from the repo .agents/skills walk-up scan
