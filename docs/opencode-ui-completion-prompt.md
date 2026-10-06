# OpenCode execution prompt: finish Harvest UI and management integration

Copy this entire document into your main OpenCode implementation session.

---

You are the integration owner for Harvest. Complete this assignment in one continuous execution using OpenCode's native agents, subagents, and multiple work sessions. Implement, integrate, test, inspect the rendered UI, and correct regressions before your final handoff. A collection of working leaf components does not complete this assignment: the production application must use them.

The repository is `C:\Users\sanid\Desktop\harvest-2.0\harvest`. The main application is `packages/coding-agent/`; its terminal framework is `packages/tui/`. Keep the configured Muse 1.3 Spark free model and inherit the same configured free model in delegated sessions where supported. Do not silently switch to paid providers or models. Discover the actual available OpenCode session/delegation capabilities; do not invent tool names or assume a particular concurrency limit.

The user authorizes the code changes described here. Complete routine implementation choices yourself. Do not stop after reconnaissance, a plan, worker reports, or a partial fix. Do not ask for approval to implement these already requested changes. Ask only for genuinely unavailable external information or an action outside the authorized scope. If an external dependency prevents a check, finish every independent part and report that exact check as blocked, with evidence; never mark it passed.

## 1. Objective and authority

Finish the connected OpenCode-like terminal experience already being built. Repair the review findings below, complete the session/agent/skill management flows, and finish removing Laya. Preserve the existing runtime, capabilities, and authored user data. Do not restart the UI redesign or replace the terminal framework.

Read before changing code:

- The applicable `AGENTS.md` files, `packages/coding-agent/DEVELOPMENT.md`, and its relevant subsystem documents.
- `docs/agent-ui-opencode-plan.md` for the visual and interaction reference.
- `docs/agent-ui-execution-state.md` and `docs/agent-ui-feature-coverage.md` for previous work and claims to verify.
- Current `git status`, HEAD, diff, and untracked files. The repository already contains substantial uncommitted work. Record the actual baseline; preserve it.

Executable source is authoritative over stale documentation. This assignment explicitly supersedes instructions to retain, install, enable, or troubleshoot Laya. Do not execute the retired `laya-setup` skill. Inert historical settings and old session records may remain for compatibility, but they must not activate Laya behavior.

The established visual reference is the OpenCode terminal UI at commit `907b3bc518fa48e90e8ec24dd327d13eee71c36c`, already linked in the UI plan. Keep Harvest's Bun/TUI/Composer architecture. Preserve the centered Home prompt, anchored session composer, responsive sidebar, bounded dialogs, coherent themes, compact tool output, and keyboard navigation. Verify the existing appearance while repairing its behavior.

Do not commit, push, publish, release, create GitHub issues, or post comments. Do not reset or discard existing changes. Do not remove user checkpoints, caches, environments, histories, credentials, or unrelated processes.

## 2. Multi-agent and multi-session execution protocol

Use one main orchestration session plus separate native work sessions for the workstreams below. Keep their session IDs so interrupted work resumes in the same session. These are OpenCode work sessions, not Harvest sessions created through the broken application tools.

Start with a short parallel read-only reconnaissance wave. Then the integration owner fixes the shared interfaces and assigns exclusive editing ownership. Use up to three implementation workers alongside the owner if actual limits permit. Reduce to one or two workers when rate limits or resource contention occur; continue the assignment without dropping work.

| Workstream | Responsibility | Default editing ownership |
| --- | --- | --- |
| A: session lifecycle and UI | Session facade, visibility, persistence, activity controls, Unicode close geometry, lifecycle regressions | Session facade/tabs/persistence/view-state and relevant management components/tests; registry changes require owner coordination |
| B: managed capabilities and revisions | Sessions/presets tools, revision transactions, safe artifact writes, real evaluations, policy preservation, learning lifecycle | `autolearn/*`, managed tool modules, `task/agents.ts`, their prompts and tests |
| C: palette and Laya retirement | Palette component, remaining active Laya hooks/skill/docs/build references, approval preservation, palette/removal regressions | Palette component, extension approval wrapper, affected AI metadata types, Laya-specific assets and focused tests |
| Integration owner | Production wiring, shared interfaces, session identity mapping, Home submission/focus, complete operator flows, final integration | `interactive-mode.ts`, `composer.ts`, input/selector/focus controllers, `sdk.ts`, tools registry/types, shared runtime/storage/settings interfaces, coverage/checkpoint docs |
| Independent verifier, after integration | Read-only production-flow review, tests, VT captures, regression audit | No implementation writes; return actionable failures to the owning worker |

Adapt this ownership table to actual dependencies before edits. Record the final owners in the checkpoint. One file has one editing owner at a time. Workers submit precise interface/patch requests for shared files; they do not edit them concurrently. A worker may launch an additional bounded read-only subagent for a specific question, subject to the total concurrency limit. Prefer the current shared checkout with exclusive ownership. If using isolated worktrees, ensure they include the current dirty implementation and never create an unrequested commit to transfer work.

The owner must integrate each handoff into the real SDK/TUI paths. Do not accept a helper merely because its isolated tests pass. For every new service or API, record its production callers and verify the public interaction through those callers.

Use the existing execution-state document as the canonical checkpoint. Append a clearly identified correction phase and supersede inaccurate completion claims. The owner alone updates shared ledgers. Each worker handoff must contain:

1. Changed files and public contracts.
2. Actual production entry points wired or shared patches still needed.
3. Regression scenarios, exact commands, and observed results.
4. Remaining failures, risks, and the next concrete action.

Keep context efficient:

- Give each worker only its task, relevant contracts, owned paths, current baseline, and acceptance cases. Use targeted searches and bounded reads rather than repeatedly loading the whole repository or transcript.
- Persist discoveries, interface decisions, session IDs, outstanding work, and test evidence before compaction or switching sessions. Resume by rereading that checkpoint and the current diff.
- Store long logs/captures in task-owned temporary directories. Share concise summaries and paths, not repeated raw output.
- Continue existing sessions instead of spawning replacements that repeat discovery. Close completed worker tasks without losing their handoffs.
- If context or rate limits interrupt execution, resume the same assignment from the checkpoint. Do not silently reduce scope or declare completion because a context window ended.

## 3. Review findings that must be resolved

Line numbers are review-time anchors and may move. Locate the actual symbols and recheck the current implementation.

| ID | Confirmed failure | Starting points |
| --- | --- | --- |
| S1 | Production picker deletion removes a background session's file but leaves its runtime/writer alive. A later rename and `ensureOnDisk()` recreated the deleted JSONL. | `modes/controllers/selector-controller.ts`: picker delete and `#detachActiveSessionBeforeDeletion`; facade delete |
| S2 | Real Home Enter creates a fresh session and submits the correct text, but loses PNG attachments/image links. The fresh session is missing from the tab strip and `/tab list`. | Input controller submit handler; `createSessionFromHomeDetached()` and outer session-transition recording |
| S3 | Clicking the active tab's × with two tabs removes its path but leaves its runtime, transcript, and title selected. Slash close correctly selects the neighbor. | InteractiveMode tab-strip close handler |
| S4 | Model session creation returns a UUID while AgentRegistry uses `tab:<UUID>`. Inspect/rename by returned ID fail. Caller lineage and initial `task` are discarded; the task never runs. | `#registerModelSessionFactory`, live-session factory, `tools/sessions.ts`, SDK registration |
| S5 | `scopeGrant:true` supplied by the model bypasses target ownership checks. It is not a trusted authorization grant. | Sessions schema and `checkTargetAccess()` |
| S6 | SessionManagementFacade has no production consumers. Archive/restore are inaccessible; the Hub hard-codes `archived:false`. Requested-ID reopen, cold listing, writer settlement, and stable-ID persistence also need correction. | Session facade; selector/Hub projection and lifecycle paths |
| S7 | Zero-tab Home retains the closed session's title, private draft, and image. Unicode/wide/combining titles break visible × hit testing. | `enterHomeDetached()`, view-state, SessionTabStrip hit geometry |
| R1 | Production installs neither skill nor preset evaluation executor. Both real evaluate actions throw `No evaluation executor is wired`; tests substitute callbacks. | Managed-skills evaluation, task/agents evaluation, SDK/task-execution integration |
| R2 | Preset architect generation calls `set("autolearn.enabled", false)` on a persistent settings clone. Even a failed generation attempt saved disabled auto-learning into user configuration. | `tools/presets.ts` runArchitect; Settings clone/override APIs |
| R3 | A prompt-only preset update replaces the definition and silently removes existing `tools`, `spawns`, and `model` restrictions. | Presets update; managed preset serialization/write |
| R4 | Promotion/rollback updates the active pointer before materialization. A failed materialization leaves active metadata and live content inconsistent. | Managed skill/preset promotion/rollback and revision store |
| R5 | Revision IDs are accepted as unrestricted filesystem path fragments. Managed preset update/materialization lacks complete leaf/ancestor link protection; a hard-linked preset update changed an external fixture. | Revision path construction; managed preset writes; existing jail/link helpers |
| R6 | Revision retention, run pinning, evaluation budgets, and concurrent writes have leaf helpers but incomplete lifecycle wiring. Concurrent evaluation appends lost records; no automatic prune kept more than twenty revisions. | Revision service, pin/budget helpers, auto-learning and task lifecycle |
| U1 | A listed custom command fails from the palette while normal submission executes it. Palette dispatch still only reaches builtins. | `openCommandPalette()` and shared slash/command dispatch |
| U2 | An open palette resized from 100×45 to 80×10 retains its old list budget and hides the selected command. | Palette overlay geometry/resize ownership |
| L1 | The extension approval wrapper still consumes Laya gating metadata; AI types still expose it. A direct SDK wrapper call behaves differently with legacy Laya metadata. The discoverable setup skill still instructs installation/launch. Some active docs/diagnostics retain Laya behavior or false retirement claims. | Extension wrapper, AI metadata, run diagnostics/event types, `.agents/skills/laya-setup`, active documentation/build corpus |
| L2 | Removal notices claim approval always requires human confirmation, but configured `yolo` permissions still auto-approve ordinary calls. | Builtin removal notice, main startup notice, approval policy |

These are the minimum corrective scope. Recheck adjacent production consumers so equivalent paths do not retain the same bug. Preserve already working visual and runtime behavior.

## 4. Required implementation contracts

### A. One connected session lifecycle

Use one typed SessionManagementFacade over the existing LiveSessionRegistry, SessionTabs, SessionManager/storage, and view state. Wire mouse, keyboard, slash commands, palette, picker, Hub/Activity, model tools, and applicable host adapters through that owner. Replace duplicate lifecycle paths; do not add a second runtime registry or session engine. Avoid nested mutation-queue deadlocks. Serialize transitions and fence stale async completions.

Use the session UUID as the public stable session identity. Maintain an explicit mapping to AgentRegistry/IRC identities where those differ; do not change existing IRC semantics or ask callers to discover alternate IDs. Carry stable IDs through normal tab transitions, persistence, restart restore, list/inspect, and requested-ID reopen. Prefer a warm runtime lookup before requiring a disk file. Include unsaved warm sessions and persisted cold sessions in the relevant lists.

Close, Stop, Archive, and Delete are separate actions:

- Close inactive: hide that tab and keep the selected view unchanged. Close active: select the next open tab to its right, otherwise its left neighbor. Close final: enter clean, detached Home. Closing never stops/disposes/archives/deletes ongoing work. Preserve the closed session's draft, attachments, reading position, and access through Activity/Sessions Reopen and Stop.
- Reopen by ID restores that exact session, not an arbitrary last-closed item. The dedicated Reopen last action may use the bounded closed-tab history.
- Stop targets the authorized run through existing cancellation and settlement; other independent sessions continue. Show truthful completion/failure state.
- Archive requires an idle/settled target, hides it through versioned metadata in the existing session storage abstraction, and remains reversible. Load metadata before projecting lists. Treat the current ad-hoc archive JSON as migration input if present; do not maintain parallel authoritative stores. Restore retains identity, transcript, and history.
- Ordinary Delete rejects busy targets. Busy includes streaming, shell/Eval activity, pending async work, approvals, compaction/deferred persistence, and live owned children. Stop-and-delete must explicitly identify its authorized target/scope, settle applicable work, fence late callbacks, seal/close writers, and only then delete storage/artifacts and remove registry/tab/view/archive references. Never stop unrelated sessions. Handle active and background targets equally. A timeout or settlement failure must reject deletion rather than proceeding. Deletion failure must retain or restore discoverability and report what actually happened.

Home owns independent composer/view state. Save the closed session state under its UUID and display an empty Home composer initially; do not reuse its title, attachments, approval UI, or callbacks. Hidden-session events and approvals remain bound to their originating runtime and cannot hijack Home or another selected session.

Unify Home submission for Enter, Ctrl+Enter, image-only input, and prompt-producing slash-command remainders. Snapshot the entire submission before the first await: text, images, image links, and existing composer payload metadata. Create/select/register/persist a fresh tab through the same production factory and lifecycle owner, then dispatch once to that runtime. Empty input creates nothing. Creation failure preserves the complete Home draft and offers recovery. Commands such as Reopen remain usable without creating a spurious session.

Compute mouse close regions from rendered terminal-cell geometry, including wide characters, combining marks, ANSI styles, clipping, and narrow widths. Preserve existing key remaps and editor bindings: Alt+W closes a tab by default; Ctrl+W remains editor word deletion.

### B. Model-controlled sessions and trustworthy permissions

Replace process-global session factory injection with owner/session-scoped dependencies wired through SDK/ToolSession and the facade. Two concurrent callers must not inherit whichever InteractiveMode happens to be selected last. Snapshot the caller's authorized project, model/settings overrides, permissions, allowed tools, spawn policy, and parent identity. Preserve existing recursion/concurrency limits and increment child depth correctly; reserve capacity atomically so parallel creates cannot oversubscribe it.

Create an independent background runtime without stealing focus. Register lineage and identity before returning its public UUID. If an initial task is supplied, dispatch it exactly once in the background and expose its observable state. The returned UUID must immediately work for inspect, rename, send, and stop under owned-lineage permissions. Parent/child and project checks must use trusted runtime facts. Enforce self-stop/deadlock protections.

Model-provided `scopeGrant` or confirmation booleans are intentions, not evidence of human authorization. Broader targets/actions require the existing trusted permission/approval mechanism bound to caller, target, project, and action. Do not silently bypass it in `yolo` with a fake scope grant. Keep configured ordinary approval behavior and provider-required human safety checks intact. Default SDK/RPC/headless hosts must receive proper owner-scoped wiring where supported; truly unsupported adapters must expose a clear unavailable capability rather than advertising a working create action.

### C. Safe managed skills, presets, and real improvement

Keep generated artifacts in the isolated managed locations. Preserve authored/bundled definitions and reject collisions. Use shared services for operator UI and model tools. Provide discoverable history, draft/evaluation status, evaluate, promote, rollback, and relevant create/update/delete controls through the existing management surfaces/palette. A renderer or tool schema alone is not an operator management flow.

For preset updates, omission of `tools`, `spawns`, or `model` means preserve the existing restriction. An explicitly supplied value means an intentional validated replacement under existing permissions. Do not infer permission widening from absent fields. Build architect/evaluation child settings with runtime-only overrides; neither success nor failure may modify persisted user/project settings. Keep child tools restricted with the real restrictive SDK option, not an empty list that means defaults.

Protect every managed file read/mutation and metadata path through existing jail/safe-file utilities. Validate names and revision IDs against their actual minted grammar; reject path separators, absolute paths, and traversal. Validate artifact identity when loading stored records. Resolve/check existing ancestors and link boundaries before operating, including nonexistent leaves. Reject unsafe symlink/hard-link leaves/roots and use resolved safe targets without check-then-reopen on an unchecked lexical path. Extend central primitives if required; do not copy weaker path checks into each tool.

Revision content, parent links, and provenance are immutable. Persist actual actor/session/run provenance. Evaluation records are append-only in meaning and must not be lost during concurrent updates. Use one per-artifact serialized transaction boundary for all mutation verbs. Compare any supplied `expectedActive` inside that boundary before changing history, content, or pointers; accept `null` as the explicit no-active-revision expectation. Preserve compatible legacy calls while making stale guarded updates reject without partial effects.

Promotion/rollback must keep the active pointer and materialized content consistent. Prevalidate candidate content and safe paths, stage writes, and use atomic replacement plus recovery metadata within the existing storage service. Publish the pointer only after successful materialization; roll back the materialization if pointer publication fails. Recover interrupted transactions before discovery exposes the artifact. Never report a failed operation after leaving a different active revision published.

Rollback restores a previously activated revision; it must not activate an arbitrary failing or never-promoted draft as a shortcut around evaluation. Failed evaluation blocks promotion. Unevaluated activation requires the already established explicit disclosure/approval behavior and must be visibly labeled unevaluated; never label it passed.

Wire real skill and preset evaluators into existing restricted task execution. Execute the supplied task with the candidate pinned, capture its actual run/output, and check the explicit expected observable outcome. Store the verification basis and run/session IDs. A fixed callback, canned summary, nonempty response, or model assertion without observed execution is insufficient. If an outcome cannot be verified, retain unevaluated/failed state and report the reason. Use static Markdown/Handlebars prompt assets for any new prompts.

Wire one candidate plus one evaluation per parent turn into the real auto-learning lifecycle. Evaluation runs cannot trigger recursive learning or claim another parent budget. Pin revisions for each live run with owner-aware references; one run finishing must not release another run's pin. Promotion affects subsequent work, while active runs retain their selected content. Apply the twenty-revision retention rule during real mutations/settlement, keeping older active/pinned revisions. Refresh skill/preset discovery and subscribed UI state after successful writes, safely at turn boundaries, without duplicate subscriptions or silent opt-in changes.

### D. Palette, fallbacks, and Laya retirement

Route palette actions through the same typed production dispatch used for normal command invocation. Handle builtins, extension/custom commands, MCP prompts, skills, file commands, and templates. Preserve argument metadata; commands needing input prepare a complete draft or open their selector. For unknown command metadata, prefer a safe draft over premature execution. Selecting a runnable non-builtin must execute its actual handler once. Preserve focus/draft on Escape and surface actionable failures.

Derive palette height from its actual overlay allocation on open and every resize. Account for borders, search, descriptions, hints, and controls. The selected row stays visible through filtering, paging, first/last selection, and shrinking/growing terminals. Handle tiny terminals with a compact/fullscreen presentation or an explicit usable fallback; do not clip an invisible selected action. Use the central geometry/render utilities.

Remove remaining active Laya gating metadata and behavior, diagnostics transitions, discoverable setup skill, current guidance, and packaging/setup references. Preserve provider computer-safety checks, ordinary configured approvals, plan-mode restrictions, lexical memory retrieval, unrelated tiny inference, speech/ONNX/Transformers, native workers, and historical user data. Retired session events must replay inertly. Keep only necessary compatibility tombstones and explicitly historical documentation. Correct notices to say approvals follow configured permissions; do not claim universal human confirmation when `yolo` still exists.

Audit the feature coverage matrix against actual reachable UI flows. For each affected feature, document its invocation, owner, loading/empty/running/success/error/denied/interrupted state, recovery/fallback, and concrete evidence. Revisit existing `todo`, `open`, or `done*` rows, including attachments/paste/clipboard/editor recovery. A truthful unsupported state is appropriate for genuinely unavailable host capabilities, but cannot substitute for a required feature that should work. Do not rewrite unrelated working features merely to increase diff size.

## 5. Verification and completion gates

Add regression tests for externally observable failures. Use actual production controller/factory flows, real isolated SessionManager/storage fixtures, and VirtualTerminal where appropriate. Helper-only tests and test-injected evaluation callbacks cannot qualify production wiring. Keep tests full-suite safe; do not use `mock.module`, source-grep assertions, placeholder tests, or file-wide leaked environment/module mutations.

Mandatory scenarios:

1. Picker deletes an idle background session; a late rename, callback, or persistence attempt cannot recreate it. Verify active/background targets and stop-and-delete settlement failures.
2. Busy parents/children, pending approval, deferred work, and compaction cannot bypass deletion. Failed deletion remains discoverable and reports failure.
3. Real mouse ×, Alt+W, slash close, and palette close share the same active/inactive/last-tab transition. Subsequent input reaches the displayed neighbor. Closing does not abort background work.
4. Final close shows clean Home. Reopen restores the original session's private draft/images/reading state. Hidden events and approvals retain their owner.
5. Real Home Enter and Ctrl+Enter preserve text, PNGs, image links, image-only submissions, and slash remainders; create a fresh open tab; dispatch once. Inject creation failure and prove complete draft retention. Empty submission creates nothing.
6. Real model create starts its supplied task in the background. Its returned UUID supports inspect/rename/send/stop without an invented alias or additional scope grant. Test two concurrent callers, different permissions/model overrides, depth and capacity boundaries, and denied foreign targets.
7. Forged model scope/confirmation flags do not authorize foreign access or provider-required safety checks. Genuine trusted approvals remain usable.
8. Unsaved warm sessions navigate/reopen without disk files. Restart preserves IDs/tab order/archive state and requested-ID restore. Unicode and combining-title × targets work at visible cell coordinates.
9. Prompt-only preset updates preserve all omitted policy fields. Architect/evaluation success and failure leave persisted parent settings unchanged.
10. Real evaluation executes and records verified pass/fail outcomes. Failing or unevaluated content cannot silently promote; rollback cannot activate a failing never-promoted draft.
11. Bad IDs and linked paths are refused without changing external isolated fixtures. Every managed mutation verb follows the same storage boundary.
12. Failure before/during/after materialization and pointer publication leaves a consistent active artifact; interrupted transactions recover. Concurrent evaluation appends survive; guarded stale mutations reject cleanly.
13. Two runs pinning the same revision keep it protected until both settle. More than twenty revisions trigger actual retention, preserving active/pinned history. Parent-turn budgets and nonrecursive learning are exercised through real lifecycle hooks.
14. Palette-listed custom/extension/MCP/skill/file/template commands execute or draft correctly through production dispatch. Resize an open palette from 100×45 to 80×10 and 60×8; selected command and usable controls remain visible.
15. Legacy Laya metadata/events/settings cannot activate runtime behavior. Ordinary permissions, computer-provider human checks, plan guards, memory, tiny inference, and existing worker-host contracts continue to work.

Run focused checks as you finish each subsystem, then integrate and broaden to the affected package suites. Follow repository commands:

```text
bun --cwd=packages/coding-agent run check:types
bun --cwd=packages/coding-agent run lint
bun --cwd=packages/coding-agent run check
bun test <explicit relevant test paths> --timeout <appropriate bounded timeout>
bun scripts/ci-test-ts.ts <relevant named suite>
bun packages/coding-agent/src/cli.ts --version
bun packages/coding-agent/src/cli.ts --smoke-test
```

Discover the current suite names/scripts rather than inventing them. Run affected TUI/AI suites if those packages change. Use `bun run test:rs` if Rust changes; never direct `cargo test`. Do not run bare root `bun test`, `tsc`, or a repository-wide formatter to hide unrelated drift. Format owned changes only and keep logging/sanitization/path-jail/worker-host/catalog-policy rules from AGENTS.md intact.

The previous baseline had passing type/lint checks, forty-two untouched formatting failures in the package check, a failing Sessions-filter assertion, and several broader Windows/test-cleanup failures. These are observations to recheck, not automatic excuses. Establish the baseline before edits. Fix introduced failures and failures affecting this assignment. For unrelated baseline failures, record the same failing scenario, unchanged ownership, and exact evidence; do not relabel an unexplained failure environmental.

The earlier worker smoke hung without a useful result. Run it with a bounded task-owned process timeout, diagnose the actual stalled subprobe, and capture its diagnostics. Do not infer that downloads are responsible without evidence, disable the smoke, or claim selector/type tests prove compiled-worker execution. Check source and the existing compiled/install path where feasible. If a genuinely unavailable external dependency blocks qualification, state exactly what remains unverified and preserve the rest of the completed work.

Generate current VT JSON/PNG captures and inspect the rendered images, not just test exit codes. Cover 160×45, 121×32, 120×32, 100×30, 80×24, 60×16, and 24×4; also short palette resize, light/dark themes, ASCII, Unicode, tabs, detached Home, Activity, archive/restore, revision/evaluation states, error recovery, and a denied/queued approval. Verify live streaming and rebuilt/parked transcripts where affected. Investigate visible layout defects; do not mark a screenshot inspected solely because its file exists. Measure affected rendering/navigation/startup paths with existing benchmarks and investigate material regressions.

After integration, give an independent verifier a new bounded native session with the current diff, contracts, and repro cases. It must inspect real callers and rerun the important failing flows. Fix its findings and rerun the relevant checks before finalizing.

## 6. Final handoff

Update the execution checkpoint and coverage matrix to reflect observed production behavior. Replace contradictory completion claims with current evidence. Add brief user-facing Unreleased changelog entries in affected packages. Leave changes reviewable and uncommitted.

Your final report must state:

- Which review IDs are fixed, with production entry points and concise scenario/result evidence.
- Which operator controls and model-controlled capabilities now work end to end.
- Actual check/test/benchmark commands and results, including failures and genuinely unverified checks.
- Paths to inspected captures and any remaining limitations.
- That existing user data and unrelated work were preserved, and that no commit/publish/release occurred.

Do not say "all done" if any required flow still depends on a test-only callback, has no production consumer, hides an error, loses an attachment, accepts fabricated authorization, or has unverified lifecycle behavior. Finish implementation and integration in this assignment; report real blockers precisely instead of converting incomplete work into a success claim.
