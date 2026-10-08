# OpenCode execution prompt: finish Harvest's production UI integration

Prepared from the 2026-10-07 review. Copy the assignment below into the main OpenCode session, or tell that session to read this entire file and execute it.

---

You are Harvest's implementation and integration owner. Complete this assignment in one continuous execution using OpenCode's available native agents, subagents, and multiple persistent work sessions. Implement, integrate, reproduce the real user flows, inspect terminal captures, fix the independent verifier's findings, and finish with evidence. Do not stop after planning, creating helpers, adding mocked tests, or receiving worker completion summaries.

Repository: `C:\Users\sanid\Desktop\harvest-2.0\harvest`.
Primary application: `packages/coding-agent/`.
Terminal framework: `packages/tui/`.

Use the user's currently configured free OpenCode model; the earlier requested configuration was Muse 1.3 Spark free. Discover its actual installed identifier rather than guessing a model name. Workers inherit the configured model where supported. Do not silently switch to a paid model/provider or make billable evaluation calls for verification. Use deterministic provider/credential seams while keeping the application runtime and UI under test real.

The user authorizes implementation of this assignment. Resolve routine choices yourself. Continue through integration and verification without asking for approval between ordinary phases. Request input only for genuinely missing external information or an action outside this assignment; finish independent work while such a question is pending. A failed or unavailable check is not a passed check.

## 1. Objective and source of truth

Deliver a connected, usable, OpenCode-like Harvest terminal experience, including management of sessions, background work, agents, managed presets, skills, and the remaining user-facing capabilities. Complete the latest production defects below and the feature coverage gaps. Keep the existing Bun, `@harvest/pi-tui`, Composer, InteractiveMode, session, tool, extension, and revision architecture. Extend its existing owners instead of creating another session registry, lifecycle engine, revision engine, or UI framework.

Read applicable `AGENTS.md` files, `packages/coding-agent/DEVELOPMENT.md`, and the relevant reference documents it links before editing. Read:

- `docs/agent-ui-opencode-plan.md` for the agreed appearance and interaction reference.
- `docs/agent-ui-feature-coverage.md` for feature inventory and evidence gaps.
- `docs/agent-ui-execution-state.md` for prior decisions and checkpoints.
- `docs/opencode-ui-integration-repair-prompt.md` for historical requirements and preservation context.

This assignment supersedes earlier prompts' stale defect descriptions and completion claims. Treat executable source and reproduced behavior as authoritative. Locate symbols in current source; review-time line numbers are navigation hints, not invariants.

At review time, HEAD was `5c63880634691da001b8963524018e8510bf00de`, with 24 modified and eight new files containing the repair implementation. The checkout repeatedly reset and restored during review. Begin by recording the actual HEAD, status, tracked diff, relevant untracked files, and hashes of files being changed. Preserve the existing uncommitted implementation. Never assume unpushed means unavailable locally, or that a clean HEAD contains the repairs.

Keep a task-owned backup of the starting diff and relevant new files using normal file APIs. Do not use reset, checkout/restore of user files, stash, clean, or an unrequested commit to establish a baseline. If source changes unexpectedly during a test or edit, invalidate that affected result, checkpoint the observed versions, and reconcile the current diff before continuing. Never overwrite a concurrent change with an older snapshot.

Do not commit, push, publish, release, create GitHub issues, post comments, delete user data, or terminate unrelated processes. Keep test sessions, credentials, repositories, settings, and captures in isolated task-owned temporary directories. Do not use the user's real history, configuration, clipboard, or caches as fixtures.

Laya remains retired. Preserve inert historical settings/events and removal notices where compatibility needs them; do not reinstall or reactivate it. Preserve configured permissions, human-approval behavior, normal configured `yolo` semantics, provider-required safety checks, jail boundaries, plan restrictions, and unrelated local inference/workers. Versioned skill/preset improvement means configuration iteration and evaluation, not model-weight fine-tuning.

## 2. Native multi-agent and multi-session execution

Use one persistent integration-owner session and separate resumable work sessions for the three workstreams below. Discover the actual capabilities exposed by the installed OpenCode version. Do not invent delegation tools, session commands, CLI flags, model IDs, or concurrency limits. If native subagent tasks create resumable sessions, use those IDs and state that mechanism honestly. OpenCode work sessions are distinct from the Harvest sessions being tested.

Start with bounded parallel reconnaissance. Before implementation, the owner locks shared interfaces, dependencies, and file ownership in the checkpoint. Run up to three workers alongside the owner when supported. Reduce concurrency when host resources or free-provider limits require it; preserve the full scope. Workers may delegate bounded read-only reconnaissance within the total available limit. After integration, use a fresh independent read-only verifier session when a slot is free.

| Work session | Scope | Editing ownership |
| --- | --- | --- |
| Owner | Cross-workstream contracts, production wiring, SDK caller/evaluator context, navigation integration, final acceptance | `sdk.ts`, `interactive-mode.ts`, shared input/selector/focus controllers and context types, shared ledgers and final coverage documentation |
| A: session lifecycle | Child authority/factory contract, deletion recovery, archive coordination/policy, neighbor selection, existing persistence contracts | `session/session-management-facade.ts`, `session/live-session-factory.ts`, `session/session-tabs.ts`, relevant persistence/storage/view-state modules, `tools/sessions.ts`, focused session tests |
| B: revision lifecycle | Retention transactions, cancellation commit boundary, recovery history, production evaluation service contracts | `autolearn/*`, `task/agents.ts`, preset/manage-skill/learn service and tool paths, static prompt resources, focused revision/evaluator tests |
| C: operator UI | Final rendered hit geometry, management catalog and revision controls, attachments, worktree/host flows, truthful UI evidence | Composer, tab strip, Agents Hub and Activity components, existing/new revision management components, attachment components, feature-specific UI tests and captures |
| Verifier | Independently reproduce acceptance cases on integrated production paths | Read-only; return defects to their editing owner |

One file has exactly one editing owner at a time, including tests and documentation. A worker needing a shared-file change sends a precise typed interface or patch request; the owner applies it. Record ownership transfers explicitly. Do not have several agents edit `interactive-mode.ts`, `selector-controller.ts`, `sdk.ts`, or `task/agents.ts` simultaneously.

Prefer disjoint editing in the existing shared dirty checkout. If the installed system uses worktrees, first verify they include the current repair state; a fresh checkout from HEAD omits the uncommitted implementation. Do not transfer work through an unrequested commit. Assign clear integration responsibility before workers start.

Agree on these dependencies before parallel edits:

1. Owner + A: a typed trusted caller snapshot and owner/session-scoped creation/delivery binding, obtained from actual SDK/runtime authority.
2. Owner + C: one final rendered geometry contract that translates screen mouse coordinates to tab/control coordinates after layout.
3. B + C + owner: authoritative management enumeration for inactive artifacts, revision operation APIs, and evaluation context/cancellation passed from the owning runtime.
4. A + owner: recoverable deletion transition and one archive/reopen policy through the existing facade.
5. B: shared transaction ownership for mutation, recovery, pin changes, metadata append, and pruning through completion.

## 3. Context management and uninterrupted execution

Append a new latest-review correction phase to `docs/agent-ui-execution-state.md`. Only the owner writes that shared checkpoint and the coverage matrix. Preserve historical notes, and supersede stale completion claims explicitly.

The checkpoint must contain:

- Objective, starting commit/status, current source versions, and preserved changes.
- Native work-session/subagent IDs, current status, file ownership, and dependencies.
- Issue IDs below with `unverified`, `reproduced`, `implementing`, `integrated`, `verified`, or `blocked` status.
- Settled interfaces and decisions; production caller chains for each repair.
- Exact verification commands, exits, scenario results, capture/log paths, and baseline failures.
- Current blockers, outstanding verifier findings, and the next concrete action.

Send each worker a compact task packet: owned issue IDs/files, necessary contracts, preservation requirements, acceptance cases, and handoff format. Workers report changed files, behavior, interfaces requested, commands/results, unresolved risks, and next action. Keep large logs and detailed captures in task-owned directories and reference their paths.

Checkpoint before compaction, session switching, interruption, or handoff. Resume existing sessions by reading that checkpoint and current diff; do not repeatedly restart discovery or paste the entire repository into every worker. Use targeted searches and bounded reads. Never abandon scope because a context window ends, and never claim unlimited context or agents/sessions that were not actually used.

Execution order: preflight and reproduction -> interface agreement -> disjoint implementation -> production integration -> independent verification -> repair findings -> focused re-verification -> final report. Worker success is not integration success. Finish all available work in this assignment; do not hand back a list of ordinary implementation steps for the user.

## 4. Preserve the fixes that already work

These were verified in the reviewed repair snapshot. Recheck the current state and preserve them:

- Saved cold UUID deletion removes storage and discovery entries; unknown IDs do not create false tombstones.
- Archive migration publishes durable replacement metadata before removing migration input, preserves project ownership, and survives reconstruction.
- Actual controller construction/transitions persist UUIDs for every open tab, including background sessions and recently closed records.
- Creators can inspect, rename, send to, and stop their own children immediately by returned UUID; foreign-lineage denial and self-stop protection remain enforced.
- The active close handler avoids re-entering its own navigation queue; later navigation still settles.
- Detached Home submission targets a fresh visible session, retains drafts/attachments on failure, and never dispatches into a hidden runtime.
- Closing hides a view while background work remains reachable; Stop, Archive, Restore, and Delete have distinct effects.
- Public history listing does not mistake an in-flight promotion for crash recovery.
- Credential/model resolution, runtime-only settings overrides, unique evaluation leases, append coordination, and pending-evaluation promotion guards already implemented are retained.
- Palette unknown metadata drafts safely; selected actions remain visible down to 24x4 and across expansion; ASCII uses central symbols.
- Late callbacks cannot resurrect a successfully deleted session; historical Laya behavior stays inert.

Do not rewrite working subsystems simply because an older prompt describes them as broken.

## 5. Remaining defects: required implementation

Paths below are relative to `packages/coding-agent/src/` unless stated otherwise. Every item requires production integration and an observable regression test.

### Session and navigation workstream

**S1 — Home mouse targets use the wrong geometry.**

Relevant: `modes/composer.ts`, `modes/components/session-tab-strip.ts`, `modes/interactive-mode.ts`.

Reproduced at 100x30 with First, Second, Third: the visible Third `x` at zero-based screen row 9, column 44 did nothing. Clicking blank row 0, column 32 instead closed Third. Composer centers/insets the Home group, while mouse handling passes screen coordinates to the strip's local hit map.

Derive hit regions from the final composed frame, including horizontal/vertical offsets, headers, clipping, quiet/non-quiet Home, narrow layouts, and sidebar allocation. Translate once at the owning boundary. Hidden or clipped controls have no interactive hit target. Apply the same geometry to close, select, new-session, and hover behavior. An overlay must intercept its own input. Do not hard-code the reproduced coordinates or assume tabs begin at screen row zero.

**S2 — Real restricted children inherit zero tools.**

Relevant: `tools/sessions.ts`, `session/live-session-factory.ts`, `sdk.ts`, actual runtime/session types.

A real parent with tools `[read, sessions, task]` and restricted SDK options created a child with `[]`. `callerPolicy` forwards `restrictToolNames` without the allowed names; snapshot code reads optional policy fields that real AgentSession does not expose.

Capture the actual effective authority through a typed sanctioned SDK/runtime seam. Carry effective permitted tool names, restrictions, spawn policy, MCP limits, approval policy, project/jail scope, settings/model, authorized credentials/model registry, owning agent registry, lineage, and task depth. Preserve valid tools while intersecting caller and requested preset limits. No omitted field or model-supplied flag may widen authority. Background callers inherit their own snapshot, regardless of foreground selection. Do not solve this with guessed properties, unsafe casts, an empty allowlist, or an unrestricted default.

**S3 — Production factory ownership is still process-global.**

Relevant: `InteractiveMode.#registerModelSessionFactory`, `setSessionToolDeps`, `ToolSession.openManagedSession`, SDK/live factory wiring.

Initializing owner B made owner A's next creation attempt fail against B's cwd. Merely adding an optional scoped seam to the tool does not repair a UI that still installs the global factory.

Bind creation and delivery to the actual owner/session in every supported production path, including newly created background runtimes. Preserve the caller's private registry and auth/model ownership. Any legacy global fallback must be limited to explicitly supported adapters, never used as the normal interactive binding. Unknown callers must not silently inherit foreground authority. Unavailable headless capability must return a truthful error. Preserve atomic capacity/depth/spawn limits, exactly-once initial task dispatch, queue/delivery semantics, originating approval callbacks, and no focus steal.

**S4 — Failed live deletion leaves a terminally sealed manager.**

Relevant: `SessionManagementFacade.delete`, `SessionManager.seal`, storage and owner runtime transitions.

Deletion seals the manager and clears view state before storage removal. On injected unlink failure, the file, tab, and discovery row remained, but later appends/renames plus `ensureOnDisk()` changed no durable bytes.

Treat deletion as an owned lifecycle/storage transaction. Preserve identity, draft, reading anchor, transcript, and runtime configuration until success. Block new work during the operation and fence old writers. On failure, recover a usable durable runtime/manager under the same UUID, using the existing lifecycle owners; do not merely unseal the old generation or leave a normal-looking tab with silently disabled persistence. Handle partial artifact removal with a truthful recovery path. Publish tombstones and irreversible view cleanup only at the correct success boundary. Successful deletion must retain late-write fencing.

**S5 — Different owners lose each other's archives.**

Relevant: facade archive loading/add/remove, authoritative archive storage and migration coordination.

Load two facades for the same project before either writes; A archives UUID1, then B archives UUID2. A fresh facade sees only UUID2. Cached per-instance state plus atomic rename is insufficient.

Coordinate by the canonical authoritative store using existing bounded storage/locking facilities. Read current metadata while holding coordination, apply the mutation, publish atomically, then update caches. Support additions and removals; a union-only merge must not resurrect restored entries. Preserve unrelated IDs and project ownership. Where several CLI processes can write the store, use the appropriate shared-store coordination rather than only an instance-local queue. Coordinate legacy input consumption across projects with consistent lock ordering; failed writes preserve durable input and truthful cached state.

**S6 — Activity bypasses archive policy for warm sessions.**

Relevant: `SelectorController.showAgentHub` reopen callback, `agent-hub.ts`, facade reopen/restore.

Direct facade reopen rejects archived B, but Activity/Sessions -> B -> Enter selected B, opened its tab, retained `archived=true`, and reported success.

Route warm, cold, and unsaved targets through the same authoritative archive/ownership policy before selecting them. Warm lookup is an optimization, not a bypass. Require Restore before ordinary reopening and update list/filter state, selection, tab persistence, focus, and detached Home coherently. Enforce this through every actual UI/API entry point.

**S7 — Rightmost close wraps to the first tab.**

Relevant: `SessionTabs.neighbor` and close consumers.

Closing Third from First/Second/Third selects First because the cycling helper wraps. Close selection must choose the next tab on the right, otherwise the left neighbor. Keep intentional keyboard next/previous cycling separate from close-neighbor policy. Inactive close preserves selection; final close enters detached Home; failed selection preserves a usable visible tab. Keep the repaired navigation queue ownership.

### Revision and evaluation workstream

**R1 — Pruning can delete a newly active or newly pinned revision.**

Relevant: `autolearn/revisions.ts::pruneRevisions`, public skill/preset prune wrappers, pruning callers.

Pause retention immediately before unlinking an older revision, publicly promote that revision, then release retention. Promotion succeeds, but the active pointer names a revision that `readRevision` cannot find.

Serialize pruning with mutation/recovery and pin acquisition through the shared artifact transaction, keyed by resolved agent directory, kind, and name. Hold ownership through the deletion's completion; do not rely on a stale pointer/pin snapshot. Acquire a run pin only after atomically establishing that its selected immutable revision exists. A pin request after deletion must fail or explicitly select another valid revision, never protect missing history. Audit all call sites, including draft creation, promotion, pin release, and public wrappers. AsyncLocalStorage reentrancy must not let a fire-and-forget nested prune outlive the outer lock or share its authority after release. Reentrant authority expires when the owning transaction completes, including for inherited delayed callbacks. Preserve bounded retention: newest twenty revisions plus active and live-pinned revisions. Never hold an artifact lock through provider execution. Avoid introducing nested-lock deadlocks or a second transaction engine.

**R2 — Abort while result append is queued still persists a pass.**

Relevant: skill/preset evaluation append callbacks and shared transaction boundary.

Hold the artifact transaction; let the evaluator return a pass; abort while its append waits; release the transaction. The operation returns `passed:true` and writes a passing record despite the aborted signal.

Check cancellation after acquiring the serialized commit callback and after asynchronous preparation, before publishing an evaluation, as well as during execution/grading. Define commit-versus-cancel ordering coherently. A queued-aborted run surfaces the existing cancellation mapping and appends neither a passing nor failing result; any queued work left behind must remain unable to commit later. Do not report a committed result as unrecorded if cancellation arrives after the commit point. Keep immutable revision identity, per-run lease ownership, child settlement/disposal, listener cleanup, and pin release correct for skills and presets. Preserve existing pending/failed-evaluation promotion guards.

**R3 — Interrupted rollback destroys legitimate publication history.**

Relevant: `resetFalseActiveRevisionToDraft`, skill/preset recovery and transaction journal.

Publish V1 then V2; simulate interrupted rollback to V1 before pointer publication. Recovery restores V2, but resets previously published V1 to draft. A later valid rollback to V1 rejects it as never active.

Distinguish a never-published failed candidate from a historically published rollback target. Preserve operation and original target state/publication facts in the existing versioned journal or authoritative revision history before mutation, and recover them correctly. Do not relabel every non-current active-state record as falsely active. Keep pointer, materialized content, history, evaluations/provenance, and journal coherent across each failure boundary. Failed never-active candidates remain invalid rollback targets; previously published revisions remain legitimate history. For legacy journals with insufficient evidence, preserve the marker and surface actionable recovery failure rather than guessing or erasing history. Mutation callers must not swallow unresolved recovery and continue. Retain recovery evidence if coherence restoration itself fails.

### Operator UI and coverage workstream

**U1 — New managed drafts cannot reach their own revision controls.**

Relevant: `AgentsHub.#reload`, managed save flow, authoritative preset management enumeration, `task/agents.ts` discovery.

Creating an inactive revision succeeds, but real discovery and Hub rendering omit its row because no active agent file is materialized.

Enumerate inactive artifacts for management through the existing revision service, independently of active-only runtime spawn discovery. Saving a draft must immediately expose its row, history, unevaluated state, and available actions in the same Hub. Derive managed/authored identity from authoritative ownership, not a file-path substring. Draft visibility must not activate it for execution. Preserve existing active revisions, authored/bundled collision protection, and explicit project/user authored-edit flows.

**U2 — Real Hub evaluation lacks its parent context.**

Relevant: Hub evaluate flow, selector/context wiring, installed `autolearn/eval-executor.ts` runner.

Actual Hub keyboard navigation -> revisions -> evaluate -> task/outcome reports `Preset evaluation needs a parent session context.` and records zero evaluations.

Pass the owning production ToolSession/equivalent trusted evaluation context and AbortSignal through the component/controller boundary. Include actual cwd, settings, registry, credential resolution, model context, restrictions, and ownership; do not construct an incomplete fake parent or cast AgentSession into ToolSession. Reuse the production restricted child executor and borrowed auth/model ownership. Render running, passed, failed, cancelled, unavailable/unverifiable states with useful recovery. Closing or changing the manager must fence late callbacks from affecting another artifact. Neither success nor failure/cancellation may change persisted user/project settings.

**U3 — Attachment admission ignores the inter-card gap.**

Relevant: `modes/components/attachment-chips.ts` and its compositor allocation.

Two text chips produce six 30-cell rows at widths 28 and 29. Include the gap before admitting a card: `x + (x > 0 ? CARD_GAP : 0) + CARD_COLS <= width`. Use actual terminal cell widths and central sanitization. No row may exceed its allocation. Do not reserve empty card rows when none fit. Preserve keyboard access to omitted attachments, payload ownership, preview/removal, and dense image-marker remapping.

**U4 — Every-feature UI coverage and evidence remain incomplete.**

The matrix still calls skill revision UI future and worktree listing open. New Hub tests/captures fabricate the missing discovery row and substitute evaluation; an image-only test calculates its own empty-submit predicate rather than invoking the controller. The Laya skill-discovery test resolves two parents from `packages/coding-agent/test`, scanning `packages/.agents/skills`; an empty scan can falsely pass.

Complete discoverable operator controls for managed skill and preset history/inspection, draft/edit, evaluate/cancel, promote, rollback, conflicts, and applicable model/tool/spawn restrictions through existing managers/palette. Prefer shared revision components. Model-callable tools alone are not operator coverage. Unevaluated promotion must use the existing explicit disclosure/authorization contract, never pretend evaluation passed.

Finish real worktree/directory listing and selection using the current executable repository's central VCS/workspace helpers. Retarget session/editor ownership, tool cwd/jail, repository context, sidebar/status, completions, and relevant background services consistently. Support real temporary repositories/worktrees, normal directories, missing targets, unsupported hosts, and failed transitions without losing the prior usable workspace.

Exercise attachments, image-only submission, large paste, clipboard failure, and external-editor failure through the actual input/controllers. Preserve drafts, payloads, focus, terminal restoration, and owned temporary-file cleanup. Use deterministic host-backend seams; do not modify the user's clipboard or depend on their installed editor.

Re-enumerate the live command/tool registries, managers, events, and host capabilities. Each feature gets a concrete invocation path, owning component/detail view, lifecycle states, error/recovery, keyboard access, and narrow/ASCII/unsupported-host fallback. Cover sessions, workers/subagents, presets/skills, models/providers/auth/usage, tools/files/search/diffs, memory, browser, MCP/extensions, settings, approvals/questions, plans/goals, queues/steering, voice/media, collaboration, diagnostics, and supported workspaces. Reuse component families; do not invent backends or add decorative controls with no operation.

Replace misleading tests with production-path contracts. Correct the retirement root calculation and establish positive discovery of a real active skill before asserting the removed skill's absence. Keep historical events/metadata inert through real replay/wrapper behavior. Update active docs and coverage only after evidence. No required implementation row remains future/TODO; genuinely unsupported capabilities have a tested truthful fallback and explicit reason.

## 6. Mandatory acceptance cases

Use real managers, storage, registries, controllers, callback wiring, and terminal input. Stub only the narrow external provider, credential, filesystem failure, clock, or host-backend boundary needed for deterministic tests. Do not replace the behavior being tested with a passing high-level runner or manually fabricated management catalog.

| Case | Required observable contract |
| --- | --- |
| T01 / S1 | Locate the visible tab/control cells in a real VT frame and send SGR reports at those screen coordinates. Close/select/new/hover work on Home and conversation layouts across resize, offsets, headers, quiet modes, and sidebar states. Clicking the old invisible hit region does nothing. Clipped/overlay-covered controls do not respond. |
| T02 / S7 | With First/Second/Third, close Third -> Second; close an active middle tab -> right neighbor; inactive close retains selection; final close -> detached Home. Background runtimes continue, drafts restore, and subsequent navigation settles without queue deadlock. |
| T03 / S2 | Real SDK + InteractiveMode parents with `[read,sessions,task]`, restricted options, bounded spawns, and private registries create usable children with the intended allowed tools and no forbidden execution. Cover foreground/background creators, policy intersections, MCP/approval/jail restrictions, depth, and foreign lineage. Do not assert only factory arguments. |
| T04 / S3 | Two real owners with different projects/registries/settings create children before and after selection changes. Neither uses the other's cwd/authority/callbacks; initial tasks dispatch once without focus steal. Their children can create permitted descendants through correctly scoped wiring. Unsupported headless creation reports unavailable capability. |
| T05 / S4 | Inject live deletion failure before unlink and at an artifact boundary. No false tombstone or lost draft/anchor; recover the same UUID, append new work/rename, flush, and reconstruct it with old/new durable data. A later retry succeeds, and callbacks from the old generation cannot recreate deleted files. |
| T06 / S5 | Load two owners before mutations; archive different IDs concurrently, restore one through the stale owner, and reconstruct fresh owners. Preserve unrelated archives without resurrection. Failed publication preserves metadata; cross-project migration preserves unmigrated input and ownership. |
| T07 / S6 | Real Activity/picker/tab callbacks cannot reopen an archived warm or cold target until Restore. After Restore, reopen the requested UUID with truthful filters/status/focus. Also test archiving the selected/final tab. |
| T08 / R1 | Controlled barriers race public prune against public promotion and pin acquisition. Promotion either publishes an extant correct revision or rejects without changing the committed state; pin acquisition never succeeds for missing history. Independent pins survive another owner's release. Verify eventual retention, automatic pruning completion, expired inherited reentrancy, and independent stores/artifacts. No timing-only race test. |
| T09 / R2 | Hold the artifact transaction, let the real evaluation pipeline finish at a deterministic provider seam, abort while append waits, then release it. Both skill/preset operations surface cancellation and persist neither a passing nor failing result. Cancel one of two runs sharing a caller run ID without losing the other's record/lease. Child settlement, pin/listener cleanup, and promotion rejection are observable. |
| T10 / R3 | Publish V1/V2, interrupt rollback to V1 before publication, recover V2, then successfully roll back to V1. Separately fail a never-active candidate's publication and prove it remains ineligible. Interrupt after pointer publication and preserve that committed revision. Cover both artifact kinds, legacy ambiguity, and missing-history/recovery failure retaining its journal. |
| T11 / U1 | Generate/save a wholly new managed draft through the actual Hub and a deterministic architect transport. The same Hub refresh exposes its real history/actions; runtime discovery does not offer it for spawning. No injected discovery rows. Existing authored bytes and an existing managed active revision remain unchanged until intended publication. |
| T12 / U2 | Invoke the installed production evaluator through actual Hub controls. A deterministic transport exercises a real restricted child runtime and grading: pass, fail, missing credential/model, and cancellation. Verify record provenance, no settings persistence, safe borrowed-resource disposal, and no parent-context error. Do not substitute `createAgentSession` or the evaluation runner as proof of the end-to-end case. |
| T13 / U4 | Through user-facing controls complete preset AND skill inspect/draft/evaluate/promote/rollback, including stale expected-active conflict and failed/unevaluated promotion policy. The chosen effective restrictions match executed behavior. |
| T14 / U3 | Two chips at widths 28/29 never create 30-cell rows; test successive card-plus-gap boundaries, omitted chips, wide labels, and actual compositor allocations. Keyboard removal/preview and submitted payloads remain correct. |
| T15 / U4 | Actual input-controller submission supports image-only/mixed drafts, attachment removal, payload/link retention, detached Home creation failure, and bounded large-paste transport. Assert downstream payloads rather than duplicating the implementation's empty predicate. |
| T16 / U4 | Real clipboard/editor controller paths handle injected missing backend, throw/nonzero exit, cancellation, and successful return. Preserve text/attachments, terminal mode/focus, and owned-file cleanup without using host user state. |
| T17 / U4 | Real workspace selection switches between isolated repositories/worktrees and ordinary directories. Assert tool/file resolution and session ownership, not just a sidebar label. Missing/failed selection preserves the previous usable workspace; verify narrow/ASCII/unsupported-host recovery. |
| T18 / preservation | Preserve actual cold deletion, durable migration, all-tab UUID persistence, requested-ID reopen, creator-child UUID management, foreign-lineage denial, busy settlement, self-stop aliases, detached Home, and late-write suppression. |
| T19 / preservation | Actual palette owner projection/selection handles unknown command metadata safely and keeps query/selected action visible through 100x45 -> 80x10 -> 60x8 -> 24x4 -> expansion, empty search, long descriptions, paste, and ASCII. |
| T20 / U4 | Correct real repository walk-up skill discovery includes a positive active-skill result and excludes retired Laya setup. Historical replay/wrapper behavior remains inert while configured approvals/provider safety and unrelated workers retain existing contracts. |
| T21 / U4 | Every live feature matrix row links its production invocation, component, state/error/recovery/fallback, focused test, and inspected visual evidence where relevant. Unsupported capabilities remain truthful; no fabricated backend, future required UI, or untested done claim. |

Use a small number of meaningful tests per distinct contract. Do not add static echo, source-grep, constructor-copy, boilerplate wording, non-empty-output, or duplicate passthrough assertions. Follow AGENTS testing guidance. No `mock.module()`, permanent global spies, or long-lived mutation of Bun/process/environment state. Restore narrow spies/settings after each test and await pending work before cleanup. Use `TempDir.createSync("@...")` for external task temp directories and isolate the agent directory before SDK/discovery initialization.

## 7. Verification, visual evidence, and completion rules

Run focused regressions appropriate to each change, including existing relevant suites. Settlement/lifecycle tests previously needed explicit 15-30 second deadlines on this host; use appropriate deadlines and recorded bounded waits, not silent skips or unbounded sleeps. Once checks pass, repeat them only for new changes or unresolved concerns. Run relevant full TS buckets through the registered CI runner, not bare root test discovery.

Commands from the repository root:

```text
bun --cwd=packages/coding-agent run check
bun --cwd=packages/coding-agent run check:types
bun --cwd=packages/coding-agent run lint
bun test <explicit-relevant-test-paths> --timeout 30000
bun scripts/ci-test-ts.ts <actual-relevant-registered-suite>
```

Discover suite names from the executable CI script/config; do not guess them. Never use `tsc`/`npx tsc`. If Rust changes are truly needed, use `bun run test:rs`, rebuild natives as required, and follow the dedicated worker smoke contract if adding a worker kind. Prefer avoiding unnecessary Rust or worker changes for this assignment.

At review time, focused tests passed in batches of 71 session, 66 revision, and 80 UI tests, with one skipped; production probes still failed. Type checking passed. The full package gate stopped on formatting in 42 unchanged files. Treat these as prior evidence, not permanent facts. Re-run current gates, record exact results, classify any unchanged baseline failure without resetting the user's checkout, and require new/changed code to pass its applicable lint/type/format checks. Do not mass-format unrelated files or label a red full gate green.

Capture actual final VT cells through real controller flows. Include Home tab interaction, active/inactive/final close, Activity archived/restore state, newly saved inactive draft, real evaluation states/cancellation/failure, skill revision controls, attachment boundary widths, worktree selection/failure, palette resize/ASCII, and narrow fallback. Use task-owned capture paths. Render with the existing `bench/render-terminal-captures.py` where appropriate and inspect the generated images. A capture generated from invented rows is not evidence. Keep visuals consistent with the agreed OpenCode reference and Harvest's central layout/theme/sanitization helpers.

The independent verifier must read the integrated diff and run production acceptance, specifically challenging zero-tool children, global factory overwrite, failed-delete durability, stale archive writes, warm reopen bypass, screen/local mouse offsets, stale prune snapshots, queued cancellation, legitimate rollback history, fake draft discovery, and fake evaluator success. Give the verifier issue IDs and acceptance contracts; have it return concrete reproductions rather than reassuring summaries. Repair all actionable findings, then re-run affected checks and re-inspect changed captures.

Follow repo conventions throughout: reuse central utilities; keep prompts in static Markdown; use explicit types/top-level imports and Bun APIs; sanitize all rendered output; use the centralized logger on active TUI/RPC/worker paths; preserve the workspace jail and protocol blocks; keep model/provider policy in catalog/KDL; do not manually edit generated catalog files. Do not introduce new authentication, settings, or VCS implementations beside existing hardened ones.

Completion requires all S1-S7, R1-R3, U1-U4 to be integrated and verified, T01-T21 to have concrete outcomes, preserved contracts to remain intact, and the coverage matrix/checkpoint to describe the actual result. An external blocker may leave a specifically named check unverified, but it cannot become a done claim. No ordinary repair may be deferred to the user or another future assignment.

Finish with a concise report containing:

1. What changed and how the real user behavior improved.
2. Per-issue completion/evidence, plus any precisely bounded remaining limitation.
3. Actual native agent/subagent/session IDs and their responsibilities.
4. Exact commands/results, baseline failures, and inspected capture paths.
5. Updated checkpoint/coverage paths and final source version/status.
6. Confirmation that existing changes were preserved and no commit/push/release/billable verification occurred.

Begin with preflight and bounded parallel reproduction, then carry this assignment through implementation, integration, and independent verification in this run.
