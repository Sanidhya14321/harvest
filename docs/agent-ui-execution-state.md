# Agent UI Execution State (integration owner ledger)

Objective: Make `packages/coding-agent/` interactive terminal UI match OpenCode quality/layout/spacing/surfaces/dialogs/transcript/interaction. Cover EVERY user-facing feature. Previous implementation incomplete (M1–M6 code substantially done; captures, M5 height budgets, M7 incomplete).

Locked decisions:
- Retain Bun, `@harvest/pi-tui`, Composer, InteractiveMode, session model, tools, extension contracts. No OpenTUI/React/Solid.
- Geometry: sidebar 42, dock >120, home prompt 75, dialogs 60/88/116. Single contract in `modes/workspace-layout.ts`.
- Themes: `harvest` dark / `harvest-light` light defaults; legacy themes via central surface fallbacks.
- Collapsed success = quiet rail/inline; expanded/error/running keep framed chrome.
- Completion endcap aggregated once at deferred usage/turn boundary, never per AssistantMessageComponent.
- Palette via `executeBuiltinSlashCommand`, never shell strings; close via `focusActiveEditorArea`.
- No commit/push/publish/release/GitHub comments. Preserve concurrent work. No `mock.module`, no source-grep tests, no `tsc`, no bare root `bun test`, no direct `cargo test`.
- Free model only (Muse Spark 1.3 Free); no model overrides on subagents.

Baseline:
- HEAD `adb0f9e` (feat: new interactive mode UI layout/theming/controllers) — swept workstream + unrelated work. Do NOT treat as clean.
- Dirty at start (11 files, uncommitted): `.gitignore`, `modes/composer.ts`, `modes/interactive-mode.ts`, `modes/workspace-layout.ts`, `modes/components/chat-transcript-builder.ts`, `modes/setup-wizard/scenes/theme.ts`, `session/session-view-state.ts`, `test/modes/theme-scene-preview.test.ts`, `test/modes/workspace-sidebar-state.test.ts`, `test/transcript-tool-gaps.test.ts`, `test/usage-row-placement.test.ts`.
- Pre-existing failures (classified): `welcome-history-resize.test.ts` 7 fail identically with diff stashed and at `cca60c0` (not caused by UI work); `run check` oxfmt drift in ~60 untouched files + 1 oxlint warning in `laya-pruning.ts`; full `coding-agent-ui` bucket times out on host.
- Reference: anomalyco/opencode @ `907b3bc5`, packages/tui/src/.

Phase: 6 — OpenCode-look animation/polish pass ( screenshots reviewed 2026-10-04).
Done: pixel HARVEST wordmark (accent/muted halves, ASCII+narrow fallback); Editor.placeholder + mode-aware ghost (Ask/Run command/Run Python); `key label` hint style on home + new session hint row; home version stamp; tab open-flash (hover highlight until select, mouse wins); `working` spinner type (KnightRider blocks, preset-aware + schema) driving the working row with `esc interrupt` trailer; submitted-prompt + home captures inspected. Deferred with reason: metadata-inside-box (status ownership is distributed; adjacent row kept), full-screen screenBg fill (TUI BCE contract; light themes expect light terminals).
Pre-existing (re-verified by stash): tui autocomplete-ellipsis 1 fail; event-controller approval-preview 1 fail (edit engine); main-interactive-input 3 fails (title/showError arity, untouched files).
Next: handoff. No commits.

Server-restart recovery notes (2026-10-04): three parallel impl worker spawns aborted on restart, but their landed files survived in-tree: impl-B2 semantic renderers (semantic-ops.ts, semantic-tools.ts fixture+index, semantic-tool-render.test.ts — reviewed + accepted by owner, 29 pass) and partial impl-B3 enumeration scratch (enum-*.ts — deleted). impl-B3/impl-D1 never executed: owner implemented registry coverage tests + capture suite directly instead.

Workers (final):
- impl-B1 DONE: tombstone + usage-row width (3 pass, types clean).
- impl-C1 DONE: palette window/paste/clamp (3 pass, types clean).
- impl-C2 DONE: 5 dialog budgets + 15 tests (102 related pass; 11 unrelated failures verified pre-existing).
- impl-B2 LANDED→ACCEPTED: 7 semantic renderers + fixtures + 29 tests (owner-reviewed, gallery-checked).
- impl-B3/D1 NOT RUN: owner-authored command/tool-registry-coverage.test.ts (12 pass) + ui-captures.test.ts (11 PNGs inspected).
- Integration owner (self): all shared-file edits above + think empty-component fix + SafeToolRendererComponent export + light-theme investigation (reverted fill; BCE contract documented).

Shared interface decisions (owner-controlled):
- `modes/types.ts` `InteractiveModeContext`: added `noteWorkspaceMutation()` (VCS invalidation seam).
- `transcript-container.ts`: added `anchorForOffset()` provenance walk (bounded offset+viewport).
- `composer.ts`: scroll-back paints from pinned anchor; home via `#homeIntro` priority; remap-aware hints via `setKeyHintSource`.
- Test stub `helpers/interactive-mode-context.ts`: added `noteWorkspaceMutation` mock (mirrors contract).

Workers:
- impl-B1 COMPLETED ses_efa40013affekkau6KXgJCRSgo: tombstone (50 FIFO) + usage-row width; 3 pass; types clean.
- impl-C1 COMPLETED ses_efa400138ffe7e4ljYvKoGb823: palette window/paste/clamp; 3 pass; types clean.
- impl-C2 COMPLETED ses_efa400135ffexg2ulq3hH7N6pH: 5 dialog budgets + 15 tests; 102 related pass; 11 unrelated failures verified pre-existing (ask-dialog memo etc., untouched components).
- impl-B2 (general, editing): semantic renderers for checkpoint/rewind/security_scan/memory_edit/learn/manage_skill/search_code/yield + coverage. Owns: src/tools/renderers.ts, src/tools/*-specific, test/semantic-tool-render.test.ts (new), cli/gallery-fixtures/semantic-tools.ts (new) + index one-line export. MUST NOT touch interactive-mode/composer/palette/transcript-container/tool-execution.existing-behavior.
- impl-B3 (general, editing): registry coverage checks. Owns: NEW test files only (test/tool-registry-coverage.test.ts, test/command-registry-coverage.test.ts). No src edits. MUST NOT edit existing tests.
- impl-D1 (general, editing): capture seam scenarios. Owns: bench/render-terminal-captures.py + bench fixtures only. PNGs to task temp dir; owner inspects.
- Integration owner (self): owns interactive-mode.ts, composer.ts, transcript-container.ts, tool-execution.ts, types.ts, theme/*, settings-schema, session-view-state, event-controller.ts, input-controller.ts, test/helpers/interactive-mode-context.ts during Phase 3.

Completed (owner, this session): A awaited-refresh+merge; E groups; I todos+extensions; F quiet-alloc+query-keys; H schema harvest-defaults, sidebar theme.symbol markers, composer remap-aware hints; D focus arm + hide-toggle unify + editor handler; G composer anchor wiring + anchorForOffset; I VCS retarget/rewatch/mutation-debounce; C home priority + hint truncation + regression test.

Shared interface decisions (owner-controlled):
- `modes/workspace-layout.ts`: sole geometry owner.
- `modes/theme/*`: surface tokens + fallbacks owner.
- `modes/components/workspace-sidebar.ts` + `command-palette.ts` + `overlay-box.ts`: sidebar/palette/dialog chrome owners.
- `modes/composer.ts` (shell) + `modes/interactive-mode.ts` (wiring): single owner at a time — currently integration owner.
- `tui/output-block.ts`, `tui/code-cell.ts`, `tools/render-utils.ts`: transcript block owners.
- `modes/components/chat-transcript-builder.ts` / `transcript-container.ts` / `controllers/event-controller.ts` / `utils/ui-helpers.ts`: transcript parity owners.

Workers:
- recon-A (explore ses_efa44cfccffevM2jg0u9JB9Lwr, read-only): shell/composer/sidebar/nav/geometry. Status: COMPLETED. C FIXED-mostly, D FIXED, G split-ownership, H defaults harvest/* (schema stale).
- recon-B (explore ses_efa44cfabffe3iPVXyYzkV3yFO, read-only): transcript/tools/parity. Status: COMPLETED. Registry 28+3=31 tools. B CONFIRMED-narrowed, F mostly-fixed+2 remnants, G fixed-verify-resize.
- recon-C (explore ses_efa44cfa9ffeTUD12p4DvIfXQx, read-only): dialogs/settings/fallbacks. Status: COMPLETED. A CONFIRMED, E CONFIRMED-all, I CONFIRMED-most. Registry ~90+ builtin specs. Dialog inventory done.
- impl-B1 (general, editing): chat-transcript-builder late-result tombstone + usage-row width. Owns: chat-transcript-builder.ts, usage-row.ts, related test. MUST NOT touch interactive-mode.ts, command-palette.ts, tool-execution.ts. Status: pending launch.
- impl-C1 (general, editing): command-palette render-window/paste/clamp. Owns: components/command-palette.ts + its test only. MUST NOT touch interactive-mode.ts. Status: pending launch.
- impl-C2 (general, editing): dialog short-terminal budgets + autocomplete collision + mouse guards. Owns: selectors/model-hub/agents-hub/settings-selector only. MUST NOT touch interactive-mode.ts/composer.ts. Status: pending launch.
- Integration owner (self): owns interactive-mode.ts, composer.ts, tool-execution.ts, workspace-layout.ts, theme/*, settings-schema, session-view-state during Phase 2. All worker cross-file needs come as patch requests.

Completed: baseline ledger + coverage skeleton created; prior M1–M6 implementation preserved (see summary in checkpoint).
Remaining defects/gaps (updated 2026-10-04): A–I all fixed with regression tests (overlay auto-open added during capture review; screenBg fill investigated and reverted per TUI BCE contract — light themes expect a light terminal). Captures: 11 cap-* PNGs inspected + 11 navigation PNGs; dynamic scenarios (streaming/diff-dialog/approval/error-retry) covered by focused tests, not PNGs. `welcome-history-resize` 7 pre-existing fails unchanged; ask-dialog 1 pre-existing memo fail; `run check` oxfmt drift in untouched files only + 1 pre-existing oxlint warning. Perf: ratio 1.19, p95 0.65ms, bytes 1.000. Genuinely unverified: Windows Terminal keyboard/paste/resize smoke, IME/clipboard/fonts, `harvest render --timing` before/after, full coding-agent-ui bucket (host timeout).

Verification commands and actual results (2026-10-04, same machine):
- `bun --cwd=packages/coding-agent run check:types` → 0.
- `bun --cwd=packages/coding-agent run lint` → 0 errors, 1 pre-existing warning (laya-pruning spread).
- `run check` → red ONLY from pre-existing oxfmt drift in untouched files (69 files incl. laya/memories/web trees) + owned files formatted via `bunx oxfmt --write` on 11 owned paths.
- Focused suites all green: startup-composer 21, navigation 10, composer-cache 3, scroll 4+5, sidebar-state 9, layout 8, theme-surfaces 4, reading-anchor 4, overlay-focus 8, theme-scene 2, late-result 3, palette-nav 3, dialog-budget 15, semantic-render 29, registry coverage 12, transcript-gaps 7, usage-placement 10, e2e 5, model-hub/agents-hub/browser 72, history-search 3, mcp-render 3, read-renderer 7, tool-execution 6, captures 2.
- Pre-existing failures (unchanged, not caused here): welcome-history-resize 0/7 (inline-history path; identical at cca60c0), edit-renderer 9 fails (hashline engine owned by committed edit/index.ts +216 in adb0f9e; zero diff from this task in edit/**), ask-dialog 1 memo fail (untouched component).
- Captures inspected: 11 cap-* + 11 nav PNGs in $TEMP/harvest-ui-review.
- Perf: transcript-compose ratio 1.19/p95 0.65ms/bytes 1.000; rendering.ts editor 0.08ms/render.
- Gallery: checkpoint/search_code/security_scan success states render, exit 0, no `render failed`.

Captures/reports: to be listed here as produced. Worker reports: concise handoffs in subagent sessions.

Blockers/continuation: none yet. If rate-limited, reduce to 1–2 concurrent subagents. Resume: reread this ledger + coverage matrix + current diff before continuing.

---

## 2026-10-05 — Integration-owner phase (new assignment)

Objective: complete terminal UI session management, agent-controlled management capabilities, skill/preset versioning+evaluation, full Laya removal. Supersedes older UI-plan restrictions on session mgmt/persistence/Laya.

Starting HEAD: adb0f9e. Dirty: 43 changed files (prior UI workstream, preserved) + untracked facade/revision/text-extract additions in progress.

Phase: 0→1. Recon A/B/C completed (ses_ef2b2c941ffeD54e34jioN2LGe, ses_ef2b2c93bffeKS8JQ0h806AraJ, ses_ef2b2c937ffeuE64F3EJUNjHF1).

Defect verification vs current tree:
- D1 picker-delete bypass: CONFIRMED (selector-controller.ts:1997-2005 vs guard :2269).
- D2 .omp vs .harvest: CONFIRMED, owner-fixed (discovery.ts now accepts .harvest + legacy .omp).
- D3 keyboard/mouse warm-runtime: unified entry, residual path-keyed risk (selectById unused, tabs path-keyed).
- D4 palette dispatch: fixed for builtins, gap for file/template metadata.
- D5 palette budget: CONFIRMED (PAGE_SIZE=10 fixed, no viewport budget).
- D6 tab close: CONFIRMED missing (no ×, no binding; flash exists).
- D7 learn refresh: CONFIRMED, owner-fixed (learn.ts now calls refreshSkills best-effort).
- Activity Reopen/Stop for closed-running: NOT IMPLEMENTED. Last-tab→Home: refused instead.

Shared contracts (owner-owned):
- src/session/session-management-facade.ts (new): Create/List/Inspect/Select/Rename/Close/Reopen/Stop/Archive/Restore/Delete, stable IDs, Serial+generation, tombstones, archive via agentDir/session-archive.json.
- src/autolearn/revisions.ts (new): draft/active revisions, expected-revision conflicts, eval records, atomic pointer, 20-retention+pinning, discloseUnevaluated.
- src/core/harvest/text-extract.ts (new): relocated extractMessageText/createScoringExcerpt/estimateTextTokens from laya-pruning.
- Keybinding to add: app.session.tab.close, default Alt+W (preserve Ctrl+W word deletion).
- Palette: maxRows/viewport budget param; file/template intent metadata default draft.

File ownership:
- Owner (self): interactive-mode.ts, composer.ts, types.ts, workspace-layout.ts, theme/*, settings-schema.ts, session-view-state.ts, event-controller.ts, tools/index.ts, builtin-registry.ts, task/discovery.ts, sdk.ts, session/agent-session.ts, core/agent-session.ts, brain.ts, session-storage/manager, facade, revisions, text-extract, laya call-site neutering, docs/AGENTS.md plan/state/coverage.
- impl-A: session-tabs.ts, session-tab-persistence.ts, session-tab-strip.ts, session-selector.ts, command-palette.ts, agent-hub.ts, input-controller.ts (tab-close wiring only), keybindings.ts (additive only).
- impl-B: autolearn/managed-skills.ts, tools/manage-skill.ts, tools/learn.ts, tools/sessions.ts+presets.ts (new), task/agents.ts + agents-hub save flow coordination, autolearn/controller.ts (display wiring only).
- impl-C: core/harvest/laya-*.ts (delete), prompts/laya/*, commands/laya.ts, cli/laya-cli.ts, slash builtin-laya.ts, setup-wizard scenes/laya.ts, scripts/laya-sidecar-payload.ts + bundle/compile defines, decision-sidecar/, ci-sidecar-smoke.sh, install.sh/ps1 laya lines, release.yml sidecar-smoke job, laya tests, docs laya mentions.
- Shared-file needs from workers come as patch requests to owner; no concurrent shared edits.

Accepted changes: discovery .harvest fix, learn refresh fix, facade/revisions/text-extract new files.
Tests executed: check:types pass (2026-10-05). Baseline failures: welcome-history-resize 7 (pre-existing, inline-history), edit-renderer 9 (pre-existing hashline), ask-dialog 1 memo (pre-existing); oxfmt drift in untouched files.
Pending: impl-A/B/C workstreams; owner integration (facade wiring into interactive-mode/composer, palette dispatch, Home safety, model tools registration, live-settings, Laya call-site neutering + settings inert + status segment removal, packaging/CI, captures, coverage updates).
Next: await impl-A/B/C handoffs; owner integrates shared wiring; focused tests; visual qualification 80×24/100×30/160×45 + ASCII.

---

## 2026-10-06 — Owner integration complete (awaiting final verification)

All three workstreams landed and are integrated by the owner. No commits.

**Accepted:**
- A (session UI): {id,path} tabs + v2 persistence, × close column + hit-testing, Alt+W binding, closeLastTabToHome, resolveNavigationTarget, palette viewport budget + draft intents + dispatch feedback, agent-hub Sessions section, busy-delete guards.
- B (revisions/tools): skill/preset revision history + draft/evaluate/promote/rollback + pinning + one-per-turn bounds + eval-run guards; sessions + presets model tools with approvals/concurrency/depth/lineage/self-stop rules; effective-state display wiring.
- C (Laya removal): all 13 laya-*.ts + prompts + CLI/commands/slash/scene + decision-sidecar + payload script + ci smoke + 25 tests deleted by worker; owner neutered all call sites (sdk, agent-session ×2, structured-subagent, unexpected-stop, brain lexical-only, main autostart, setup, registry + /laya notice, cli-commands, status-line segment + presets, footer, gallery fixtures, bundle/compile defines, settings-schema inert, release.yml job, install.sh/ps1, LAYA_* warn, markdown-brain tests rewritten).
- Owner extras: SessionManagementFacade (+dispose/detach settlement, agentDir seam), revisions.ts, text-extract.ts, .harvest discovery fix, learn refresh fix, adoptBackground (no focus steal), interactive model-session factory registration, Home forceHome + submit gates (Enter + Ctrl+Enter + slash-remainder), palette wiring, hub Sessions wiring, picker busy wiring, /session delete predicate alignment, sessions/presets renderers, tool registration + eval-run gating, managed-preset discovery merge, hub identifier unification.

**Tests (this machine, 2026-10-06):**
- check:types: clean. lint (oxlint): clean. check: red ONLY from pre-existing oxfmt drift in 42 untouched files (none touched by this work; verified by name).
- New: session-management-facade 9 pass; laya-removal 4 pass; input-controller-home-detached 3 pass; ui-captures 3 pass (17 captures incl. new tabs/lasthome/palette-bottom/ascii-home).
- Workers: revisions 10 + presets 12 + sessions-tool 11 + effective-state 7 pass; palette/registry/semantic suites pass; live-tab-navigation 9 + session-tabs-controller 9 pass (after adding new ctx contract members + v2 expectation).
- Surviving: secrets 155, mcp-approval 7, grounding 1, plan-mode 14+5, memory 60+, brain 11, navigation 10 (alone; batch runs flake under load — environmental).
- Baseline failures (pre-existing, unrelated): welcome-history-resize 7, edit-renderer 9, ask-dialog 1 memo, tiny-worker-env logPath 1 (Windows path semantics), full coding-agent-ui bucket host timeout.
- Introduced-then-fixed: harness ctx stubs (new contract members), persistence v1→v2 expectation, missing sessions/presets renderers, 9 lint findings from call-site removals (eligibleAgents, subagentStartTime, emitGatingDecision, resolveToolTierSafe, unused imports, prefer-const).
- Smoke probe: `bun src/cli.ts --smoke-test` exceeded 120s foreground with no output; background rerun killed after ~10 min, exit 1, no output. DIAGNOSED (2026-10-07, per-subprobe runner with 45s bounds): 10/12 subprobes pass in milliseconds (sync, statsActivity, tinyTitle, stt, jsEval, computer, tts, mnemopi, daemonBroker, terminalOutput). `lspMux` and `blobBroker` stall: worker subprocess spawns and stays alive with empty stderr, but the socket probe never connects (blob uses `fetch({unix: <tmpdir>/*.sock})` even on win32; lspMux pipe probe likewise silent). Pre-existing Windows IPC environmental failure in code this assignment never touched (no diff in blob-broker/, lsp/, subprocess/). Full --smoke-test therefore cannot go green on this host; worker-host reentry contract itself validated by worker-selector tests + check:types.

**Captures inspected:** 17 VT JSON + PNGs in $TEMP/opencode/harvest-ui-2026-10-06(-png): tabs with × affordance, last-tab Home (no tabs/transcript, hidden session running), palette selection mid-window at 80×24 with descriptions, ASCII home, plus prior matrix (session/palette/light/docked/edges/narrow/overlay/short/minimal/submitted).
**Perf:** transcript-compose ratio 0.655 (≤1.3), p95 1.31ms (<10ms), bytes 1.000; rendering.ts editor ~1.08ms/frame; startup `cli.ts --version` ~239ms source boot.
**Compatibility:** laya.* keys parse with defaults, no UI; brain.rerank default false (deprecated/ignored); old session events (laya_gating_decision) replay as no-ops; custom status layouts with "laya" render empty; /laya + `setup` flags + installer flags answered with removal notices; user data (checkpoints/caches/venvs/logs) untouched.

**Pending:** full `coding-agent-ui` bucket (host timeout — run scoped instead); Windows Terminal keyboard/paste/resize smoke + IME/clipboard (genuinely unverified); `bun run check` full-green blocked only by pre-existing untouched-file drift.
**Next:** final report. Resume: reread this ledger + coverage + `git status --short`.

---

## Correction phase close (2026-10-07)

- Verifier ses_ef087d1e: 12/12 FIXED confirmed, 119/119 tests. Filed D1/D2/D3 — all fixed + tested (empty-Enter gate, sessions.md wording, skill run pins in runStructuredSubagent).
- Smoke: per-subprobe diagnosis — 10/12 pass in ms; lspMux + blobBroker stall (alive, empty stderr, win32 socket probe never connects). Pre-existing environmental failure in untouched code; full --smoke-test cannot go green on this host.
- Captures: 18 VT JSON + PNGs in $TEMP/opencode/harvest-ui-2026-10-07(-png), inspected (new: hub-sessions with live + archived rows + action footer).
- Perf: transcript-compose ratio 1.042, p95 0.48ms, bytes 1.000; CLI --version boots (harvest/18.1.14).
- Gates: check:types clean, lint clean, check red only from 42 pre-existing untouched-file oxfmt drift + 15 pre-existing ai-package drift files (none touched).
- Baselines re-verified unchanged: welcome-history-resize 7, edit-renderer 9, ask-dialog 1 memo, tiny-worker-env 1.
- No commit/push/publish/release. User data preserved (one self-inflicted test leak into ~/.harvest/agent removed exactly; tests now isolated).

---

## Correction phase (new assignment, supersedes §9 plan restrictions on Laya retention)

Owner ran read-only recon as 3 parallel explore sessions (all read-only, no edits):
- Recon-S S1–S7 (ses_ef219a7d0ffeCXpoJvwHa5trvy): ALL STILL BROKEN (S7 geometry partially mitigated). Full file:line evidence in session handoff.
- Recon-R R1–R6 (ses_ef219a792ffeoZg7M4A3nIuIEU): ALL STILL BROKEN (R5 skills hardened, presets+revIds not).
- Recon-U/L U1–U2–L1–L2 (ses_ef219a78cffe2EljCHYoKIMR66): ALL CONFIRMED.
Prior "done" claims in §20 coverage for facade/tools/revisions/palette/Laya are therefore corrected to "helper-complete, production-wiring incomplete" until this phase re-verifies each mandatory scenario.

Delegation status: 3 parallel implementation-worker spawns failed with provider rate-limit errors (ses_ef2162b95 / b9e / ba3, "Rate limit exceeded"). Per protocol, owner proceeds with shared-interface implementation directly and retries workers ONE at a time. B (ses_ef0c6d09), A (ses_ef0b3d1b), C (ses_ef0a846e) completed; verifier (ses_ef087d1e) confirmed + filed D1/D2/D3 (fixed). Final owners (adapted): owner = interactive-mode, composer, input/selector/focus controllers, command-controller, sdk, tools/index types, ToolSession seam, session-manager/storage guards, settings, notices, ledgers. A = tabs/persistence/strip/selector/hub/view-state + tests. B = autolearn/*, tools/manage-skill+learn+presets+sessions, task/agents, prompts + tests. C = command-palette component, wrapper/hook wrappers, AI metadata types, event/diagnostic tombstones, laya-setup skill, active docs/build leftovers + tests.

Correction progress (2026-10-07):
- S1: picker + /delete via ctx.sessions (seal-first, tombstone, dispose+detach, drain); verified by session-delete-seal.test with real manager+storage.
- S2: input snapshot-before-create (text/images/links/metadata); fresh tab registered (open/visit/noteId/persist).
- S3: active × selects right-neighbor else left (verified) else Home; inactive hides only.
- S4: factory returns {UUID, registryId, taskAccepted}, once-dispatch, lineage → parentAgentId, depth+1; UUID↔registry mapping; triple-identity self-stop.
- S5: scopeGrant removed; host managedSessionGrant only.
- S6: facade in InteractiveMode (ctx.sessions); storage-backed archive + migration; reopen-by-ID incl. pathless; listAll; hub facade.list + async + archive/restore; /tab archive|restore; restart keeps IDs.
- S7: enterHomeDetached saves under closed UUID, clears editor/attachments/title/scroll.
- R1: eval-executor.ts production runners installed from sdk; override() only.
- R6: native withRevisionTransaction (twins delegate); revId/name guards; rollback rejects non-active; sdk agent_end budget reset; autoContinue-scoped tool gates; eval + subagent run pins.
- U1/U2: resolvePaletteSelection + onSubmit fallback; #rebudgetCommandPalette on resize.
- L2: configured-permissions wording everywhere.
- B: R2/R3/R4/R5 + 21 hardening tests green. A: geometry, hub actions, selector tests, view-state, TAB sources (21 tests). C: draft fail-safe, wrapper, tombstones, skill deleted, docs (17 tests).
- Verifier follow-ups fixed: D1 empty-Enter (isEmptySubmit + test), D2 sessions.md wording, D3 skill run pins + test.
- Smoke: 10/12 subprobes pass; lspMux + blobBroker stall on win32 socket probe (pre-existing, untouched code).
- User-data incident: test artifacts leaked into ~/.harvest/agent (5 skills + histories + 1 preset); removed exactly those; budget tests now isolated via setAgentDir(temp).

---

## Latest-review correction phase (2026-10-08, HEAD 5c63880, clean tree)

Recon (3 parallel explore, read-only, completed 2026-10-08):
- Recon-A ses_ef05baeb6ffeVwC4u68zCpy3dE: A1-A7 ALL CONFIRMED BROKEN (full file:line in handoff).
- Recon-B ses_ef05baeafffeiIdmDVvtkVMbr1: B1-B7 ALL CONFIRMED BROKEN (B5c retention triggers FIXED but depend on broken pins).
- Recon-C ses_ef05baea9ffeNy8JKNSuMVxKEU: C1-C5 ALL VERIFIED (C1 projection defeats leaf; C2 budget/ASCII; C3 hub bypass; C4 matrix gaps; C5 memory.md + test path + echo test stale; notices already correct in src).

Ownership lock (one file = one owner):
- Owner: interactive-mode.ts, sdk.ts, input/selector/focus/command controllers, composer.ts, modes/types.ts, docs ledgers.
- A: session facade/tabs/persistence/view-state/live-session-factory.ts, tools/sessions.ts + focused session tests. Registry changes need owner coordination.
- B: autolearn/*, task/agents.ts, tools/presets.ts + manage-skill.ts + learn.ts, static prompts/tests. SDK changes via precise patch requests to owner.
- C: palette/management components (command-palette.ts, agents-hub.ts), feature UI tests, docs/memory.md + coverage rows for owned flows. Shared-controller changes via patch requests.
- Verifier (later, read-only): production-flow audit.

Interface decisions pending (owner settles before/during implementation):
- Navigation: single Serial owner; internal transition for in-queue close (A1).
- Facade delete/archive: cold resolution + tombstone-after-success + per-project stores + coordinated migration (A2/A3).
- Tab IDs: noteId at every transition incl. model/cold; no invented IDs (A4).
- Caller snapshot: explicit trusted snapshot incl. tool/spawn/permission/scope/registry/depth; background caller uses own (A5/A6/A7).
- Eval: centralized credential contract + model resolver reuse + unique lease pins + signal threading + transaction-coordinated recovery (B1-B7).
- Palette: preserve undefined metadata; budget from actual allocation + compact fallback + ASCII (C1/C2).

Workers dispatched: A ses_* / B ses_* / C ses_* (record on launch). Next: implement → integrate → verify → correct → handoff.

---

## Latest-review implementation (2026-10-07, owner)

Workers (all completed, no commits):
- A ses_eeb868f71ffe14VY035xN4UTge: A2 cold delete + A3 atomic migration + A4 ensureTabId/backfill + A5 snapshot/policy + A6 callerRegistryIds + A7 session-scoped seam + dispatchInitialTaskOnce; 4 new test files (22 tests). Pre-existing tree already held partial A2/A3/A4/A5 which worker kept+verified.
- B ses_eeb78520effe6ON8ZLI96q3wiy: fixed committed HEAD type breakage (missing hasLiveSkillEvalPin/serializeSkillEvalAppend — half-applied B5 edit in dirty tree); AsyncLocalStorage txn re-entrancy (fixed 5-append collapse); raceWithAbortSignal + createToolEvalSignal; unique eval leases; B1 credential fallback; B6 abort threading; B7 shared model resolver; B3 merge-inside-txn; agentDir threading; 23-test lifecycle suite. Gray edit: discovery/builtin.ts loadManagedSkills recovery (+16 lines).
- C ses_eeb342038ffe4VZVPoEdb157ej: agents-hub managed-preset operator flow (isolated architect + revision manager + setAgentsHubArchitectRunner seam); projectPendingSlashCommands + paletteOverlayRows component helpers; tombstones test corrected to repo walk-up; echo test replaced with real inert assertions; composer attachment tests; memory.md historical; 65/65 across 11 owned files.

Owner applied (this session):
- A1: close-tab op now calls internal #resumeSessionView instead of public handleResumeSession (Serial deadlock fixed).
- A P1: factory uses snapshotTrustedCaller + openLiveAgentSessionFromSnapshot + ensureTabId + dispatchInitialTaskOnce (trusted callerId/callerTaskDepth/callerPolicy inputs).
- A P2: recordSessionTransition + /tab open (match.session.id) + both resume paths + constructor record UUIDs via tolerant #noteActiveTabId helper (no invented IDs; old stubs without getSessionId keep path-only behavior — fixed 8 regressions in session-tabs-controller.test).
- A type-compat: TrustedCallerSource.session getAgentId widened to string|null|undefined.
- C patch #1: pendingSources via projectPendingSlashCommands (unknown stays undefined → draft fallback active).
- C patch #2: overlay budget via paletteOverlayRows at open + rebudget (no min-8 overstatement).
- B suggestion accepted as-is (lease pins already order prune-vs-append); no change made.

---

## Latest-review close (2026-10-07)

Verifier ses_eeb085cdcffe5hCugGRC9noKtf (read-only): ALL IDs FIXED, ~238 pass / 0 fail across 20 suites. Two minor defects filed — both fixed + re-verified:
1. Optimistic tab close on failed delete → tab now closes only after storage removal succeeds (facade.delete reorder; 17/17 delete suites pass).
2. Coverage short-name ambiguity → exact test-path mapping note added under §21.
Gates: check:types 0 errors; lint 0/0; check red ONLY from 42 pre-existing untouched-file drift (verified zero overlap with changed files). Captures: 18 VT JSON + PNG in $TEMP/opencode/harvest-ui-2026-10-07b (tabs × + hub Sessions inspected; ○ rasterizes as "o", not a defect). Perf: transcript-compose 0.945 / p95 0.44ms / bytes 1.000. User dirs verified clean (no test leaks). No commit/push/publish/release/issues/comments. Changes uncommitted (31 entries).

---

## Independent re-check (2026-10-07, owner, post-handoff audit)

Re-ran everything from scratch against the current tree (HEAD 5c63880 + 32 uncommitted entries):
- `check:types` exit 0; `lint` 0 warnings/errors; `check` red only from the same 42 pre-existing drift files.
- Focused suites, all EXIT:0 — A×4 (22), B eval-lifecycle (23), B hardening/revisions/presets/budgets (48), C×5 (35), facade/delete-seal/sessions-tool/laya-removal/home-detached (37), captures/dispatch/budget/tabs-controller/live-tab-nav (31).
- Captures regenerated with `HARVEST_TERMINAL_CAPTURE_DIR` set (`harvest-ui-check`, 21 JSON → PNG) and PNGs inspected: 24×4 ultra-compact shows selected action + query (C2); hub revision manager shows draft + passing eval + actions (C3); tabs × + hub Sessions rows clean.
- Wiring spot-checks: A1 internal `#resumeSessionView` (interactive-mode.ts:1996); snapshot factory + `dispatchInitialTaskOnce` + `ensureTabId`; `projectPendingSlashCommands` + `paletteOverlayRows` at open + rebudget; facade seal → delete → tombstone → close-tab order; zero active Laya refs in `src`.
- Singleton bucket (`ci-test-ts.ts coding-agent-singleton`, clean run): 74 unique failing tests, every one classified — ACP×9 identical at stashed baseline; managed-skills symlinks×4 identical at baseline (Windows privilege); single-test repro of `python-cleanup` fails identically alone at baseline (warmup-dependent flake); all remaining fails in files this diff never touches (tunnels, ttsr, SYSTEM.md, MCP parity, edit-engine, PTY/broker, `*_test` infra). With-changes vs baseline 6-file sample: 12 vs 11 fails, delta = the flaky python-cleanup timeout (26s wall-clock vs 10s test cap under load).
- `coding-agent-runtime` bucket launched in background; `coding-agent-ui` + full smoke remain host-blocked per baseline (ui timeout; blob/LSP win32 socket stalls in untouched paths — re-documented, not re-run since paths unchanged).
- Incidental: untracked `docs/opencode-ui-production-repair-prompt.md` appeared 22:34 during verification (not created by this work; no source references it) — left untouched.
- No commits/pushes/releases/issues/comments; user agent dirs verified empty of test artifacts.

---

## Production integration assignment (2026-10-08, HEAD 5c63880, 33 entries)

Preflight: baseline recorded in `harvest-baseline-20261008/` (HEAD, starting.diff 462KB, starting.stat, 8 new test files). No reset/stash/clean used for baseline (file APIs only; stash used solely for read-only baseline comparisons, always popped; tree verified 33 entries after each).

Recon (3 parallel explore, read-only, all REPRODUCED with file:line):
- Recon-S ses_ee8a1136effedKI33YJgQcq8j8: S1 mouse geometry untranslated; S2 toolNames dropped (AgentSession has no surface); S3 global factory still normal binding; S4 seal-before-delete with no recovery; S5 stale-cache clobber; S6 warm reopen bypass; S7 neighbor wrap.
- Recon-R ses_ee8a112d5ffeQqyyl6LmEiVNxS: R1 prune unlocked/stale snapshots/fire-and-forget outliving lock/ALS stale authority/pins-without-existence; R2 append commits after abort (no post-queue cancel check); R3 recovery resets historically-published V1 to draft (journal lacks op/published evidence; callers swallow via .catch(()=>{})).
- Recon-C ses_ee8a112a1ffe3TalGQ9xb4SSOY: U1 drafts invisible (file-scan discovery, path-substring identity); U2 Hub evaluate throws parent-context, zero evals; U3 gap ignored at 28/29 (wide-label truncation already OK; omitted-chip keyboard + empty-row OK); U4 matrix future/TODO rows, image-only test uses own predicate, tombstones root resolves to packages/ (vacuous), worktree listing overclaimed (single-child resolve + file Changes + CLI list only).

Ownership lock (one file = one owner):
- Owner: sdk.ts, interactive-mode.ts, input/selector/focus/command controllers, composer.ts, modes/types.ts, ledgers + coverage.
- A: session-management-facade.ts, live-session-factory.ts, session-tabs.ts, persistence/storage/view-state, tools/sessions.ts + focused session tests.
- B: autolearn/*, task/agents.ts, presets/manage-skill/learn + static prompts/tests.
- C: composer.ts(shared with owner — C proposes, owner applies), tab strip, agents-hub/activity, revision components, attachment-chips, feature UI tests/captures, memory.md line.
- Verifier (later, read-only).

Settled interfaces:
1. Owner+A: typed trusted caller snapshot from real SDK/runtime authority (must solve AgentSession exposing no toolNames/restrict — sanctioned seam, no guessed props/casts); owner/session-scoped creation binding replacing global as normal path (global fallback only for explicitly supported adapters).
2. Owner+C: single composed-frame geometry contract (screen→control translation once at owning boundary; hidden/clipped = no hit target).
3. B+C+owner: management enumeration independent of spawn discovery (inactive drafts listed); revision op APIs; eval gets owning ToolSession + AbortSignal through component/controller boundary.
4. A+owner: deletion as owned lifecycle/storage transaction (same-UUID usable recovery, tombstone at success boundary); one archive policy with shared-store coordination (stale owners cannot clobber; union must not resurrect).
5. B: one artifact transaction for mutation/recovery/pins/appends/pruning (no lock through provider execution; reentrant authority expires with owner).

Bucket classification (this session): singleton 74 fails all pre-existing/environmental (ACP×9, symlinks×4, python warmup flake — each stash-proven identical; rest in untouched files). Runtime 36 fails: sdk-credential-disabled 8 identical at baseline; rest timeout-shaped in untouched areas (RpcClient workers, cloud network, model discovery). `check` red only from 42-file pre-existing drift. coding-agent-ui + smoke remain host-blocked (ui timeout; blob/LSP stalls, paths unchanged).
