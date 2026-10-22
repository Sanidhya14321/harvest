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
