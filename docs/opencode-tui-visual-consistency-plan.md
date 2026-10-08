# Harvest TUI visual consistency audit and coordinated execution plan

Audit date: **2026-10-08, Asia/Calcutta**. Scope approved by the user: **audit and coordinated plan**.

## 1. Decision and scope

Harvest has a mixed TUI. Home, the workspace, prompt rails, sidebar infrastructure, and some compact tool rows have received redesign work. Many selectors, authentication flows, managers, manual execution views, extension fallbacks, and terminal primitives still retain older presentation. Selecting the new theme alone does not complete the OpenCode-style redesign.

This document is an implementation handoff. **No application source, tests, runtime policy, existing repair ledger, or previous repair prompt was changed by this audit.** The accompanying JSON/PNG files are review evidence, not an implementation.

The other agent is already executing [the production repair prompt](./opencode-ui-production-repair-prompt.md). Keep that assignment active. This plan supplements it with a visual consistency phase; it does not replace its S1–S7, R1–R3, U1–U4 work or its T01–T21 acceptance cases.

Audit snapshot: local HEAD **5c63880 plus substantial uncommitted changes**, including active edits to Composer, InteractiveMode, selector mounting, command palette, tab strip, attachments, Agents Hub, revision views, and their tests. Some files changed during the audit. Findings below describe observed source/render behavior, not a final rejection of the other agent's unfinished integration. Re-inspect its final tree before applying patches.

Repository root at audit time:

    C:/Users/sanid/Desktop/harvest-2.0/harvest

Paths in the implementation tables are relative to that root. The primary package remains packages/coding-agent; packages/tui owns reusable terminal primitives. Follow the root AGENTS.md, any more specific AGENTS.md, packages/coding-agent/DEVELOPMENT.md, and the authoritative docs it links.

## 2. Fixed OpenCode reference

Use the **terminal UI** in anomalyco/opencode at commit **907b3bc518fa48e90e8ec24dd327d13eee71c36c**, the reference already fixed in [the original redesign plan](./agent-ui-opencode-plan.md). Do not silently change reference versions or substitute the desktop/web application.

Verified reference contracts:

| Reference | Relevant contract |
| --- | --- |
| [Home source](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/tui/src/routes/home.tsx) | Centered identity and prompt, restrained supporting rows; normal prompt width cap 75 columns. |
| [Dialog source](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/tui/src/ui/dialog.tsx) | Bounded panel sizes 60/88/116, viewport margin, filled panel background, separate backdrop, managed focus/close behavior. |
| [Dialog selection source](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/tui/src/ui/dialog-select.tsx) | Coherent title/search/list/footer spacing, visible active/current rows, semantic selection surfaces, muted secondary information. |
| [Theme source](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/tui/src/theme/assets/opencode.json) | Separate background, panel, element, text, accent, and status roles. |
| [Session source](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/tui/src/routes/session/index.tsx) and [prompt source](https://github.com/anomalyco/opencode/blob/907b3bc518fa48e90e8ec24dd327d13eee71c36c/packages/tui/src/component/prompt/index.tsx) | Session and prompt references for the executor's complete comparison; preserve the previously agreed Harvest adaptations. |

Use these as design contracts, then implement them in Harvest's existing Bun, Composer, InteractiveMode, and @harvest/pi-tui architecture. Do not transplant OpenTUI/Solid/React into the terminal application. A terminal cannot necessarily reproduce an alpha backdrop; use its supported dimming/solid-surface equivalent without obscuring controls.

## 3. Evidence and its limits

Five isolated captures use the real InteractiveMode and Composer with temporary session storage, in-memory settings, isolated agent directories, and a bundled model definition. They do not submit a prompt or call a provider. These captures cover mounted Home, a conversation containing a user message, the palette, and Settings at 100×30.

A separate gallery rendered 11 actual component families at 100×30 Unicode, 80×10 Unicode, and 24×4 ASCII: Thinking, Queue, Hook Input, Hook Editor, prompt-style editor, manual Login, OAuth selector, Ask, Settings, Move, and Plan Save. That gives **33 direct component renders**. Direct rendering proves component output; it does not prove production mounting, hit testing, callback behavior, or focus restoration.

Evidence is preserved in [tui-visual-audit-evidence-2026-10-08](./tui-visual-audit-evidence-2026-10-08/README.md). The component metrics record rendered height, maximum width, and explicit background-cell counts. Only Settings tab selection painted explicit background cells in that gallery; the other sampled dialog families painted none. This is evidence of missing surface adoption in those sampled states, not a claim that every component has zero background.

The shared rasterizer uses Consolas and a dark default terminal background. Some Unicode/Nerd glyphs rasterize as missing-glyph boxes. **Do not classify those boxes as application defects.** Check captured code points and use a compatible terminal font. Blank cells with terminal-default background also cannot prove the correctness of a light theme.

The source review covered theme resolution/adapters, common overlay chrome, mounting, list/editor primitives, transcript construction, manual execution scaffolds, and the component/auxiliary inventories. Not every feature was invoked end to end. The complete coverage requirement below closes that gap.

The first mounted-capture run hit a Windows EBUSY during removal of its isolated temporary fixture. Capture generation was rerun successfully with fixture retention and explicit manager closure. No user configuration or ~/.harvest data was used. This cleanup limitation is not an application UI finding.

### Observed gaps

| ID | Finding and evidence | Required correction |
| --- | --- | --- |
| V01 | **Effective default theme differs from new theme defaults.** config/settings-schema.ts around 735–752 still defaults theme.dark to titanium and theme.light to light. modes/theme/theme.ts around 117–130 defaults bare initialization to harvest/harvest-light. main.ts around 1613–1620 passes the settings values to final initialization. The mounted default-settings fixture reports titanium. | Make fresh normal startup and standalone TUI entrypoints resolve the intended new defaults consistently. Preserve explicit user theme choices; do not overwrite them as a migration shortcut. |
| V02 | **New surface tokens have no rendering consumers.** screenBg, panelBg, raisedBg, composerBg, and modalBg occur in palettes/schema/loader fallback plumbing, not screen/component paint calls in the audited tree. getEditorTheme in modes/theme/tui-adapters.ts around 273–300 still uses userMessageBg/userMessageText for the editor surface. | Connect semantic tokens to workspace, managers, composer, dialogs, and selected/raised surfaces through central adapters. Retain legacy-theme fallback support. |
| V03 | **Shared overlay chrome still represents the older outline style.** modes/components/overlay-box.ts topBorder/row/splitRow and OverlayPanel supply rounded outlines without a modal/panel surface. Many consumers inherit that layout. Normal command-palette rendering around 537 still uses topBorder/row/bottomBorder and styles only the selected label span. | Introduce or extend common surface presentation; apply full-row selection and a consistent title/query/body/footer hierarchy. A cap helper or theme file alone is insufficient. |
| V04 | **Mounting policy is inconsistent.** selector-controller.ts #showFullscreenMenu uses 100% width/height fullscreen overlays. showSelector replaces editorContainer. Login explicitly documents editor replacement. Small transient decisions and large multi-pane managers therefore use different visual and placement rules without a shared contract. | Classify transient dialog versus full workspace manager; migrate built-ins through one presentation policy while retaining intentional full-screen explorers and extension return/focus contracts. |
| V05 | **ASCII mode remains partial.** Composer rail/field/band/rule/borderless/claude have literal Unicode glyphs in packages/tui/src/components/composer. SettingsList uses a literal vertical separator. Shared scrollbars emitted Unicode in ASCII probes. output-block.ts:272–273 hardcodes successful activity › and ·; tool-execution.ts around 907 uses a literal bullet fallback; Move around 246 uses ▶. | Route application-owned chrome, separators, cursors, rails, ellipses, arrows, and spinner fallbacks through symbol policy. Do not transliterate user/file content. Verify every exposed composer shape and scroll state. |
| V06 | **Shared chrome can exceed tiny widths before compositor clipping.** Direct probes found row/topBorder output exceeding widths 1–3 and splitRow with width 20/sidebar 16 exceeding the requested width. The source reserves fixed border/inset/side-pane cells without a tiny-width branch. | Make central layout helpers width-safe; collapse chrome and secondary panes before body allocation reaches zero. Do not rely on the TUI's emergency truncation as normal layout. |
| V07 | **Short-screen policies are incomplete outside the repaired palette.** Direct gallery metrics include Ask 12 rows at 80×10; Settings 10, Hook Input 8, Hook Editor 11, and Ask 13 rows at 24×4. Settings/Model Hub still have a six-row floor; Ask reserves a larger minimum; Advisor uses a 14-row floor. | Use actual allocated height, with query/selected item or required answer/cancel controls retained first. Reproduce through mounted production flows before calling any particular clipped control a confirmed production defect. |
| V08 | **Execution and fallback cards use divergent visual systems.** Manual bash/eval use full-width DynamicBorder scaffolds in execution-shared.ts:29–49; agent output still contains rounded OutputBlock/CodeCell sections. Skill/custom/hook fallbacks retain padded boxed cards, and TTSR uses a stronger warning treatment. | Use shared inline activity, output panel, notice, and attention roles. Keep intentional expanded code/diff/error detail; remove repeated decoration from routine collapsed output. |
| V09 | **Guest prompts bypass the local prompt presentation.** collab-prompt-message.ts builds a separate author line with literal «…» › and a standalone Markdown bubble; local user rendering/rails are assembled elsewhere. | Share user-message surface/rail/attachment presentation and add a bounded author attribution. Preserve guest identity, message content, image links, and replay behavior. |
| V10 | **Completion placement has separate live and rebuilt contracts.** The transcript audit probe observed live STEP_ONE, STEP_TWO, MODEL_ONE, MODEL_TWO versus rebuilt STEP_ONE, MODEL_ONE, STEP_TWO, MODEL_TWO. event-controller.ts queues endcaps then appends them at turn end around 2004; chat-transcript-builder.ts flushes pending endcaps with usage at intermediate boundaries. | Reproduce with a durable multi-completion fixture, choose one documented association/order, and make live, restored, parked, and focused-agent presentation agree. Preserve orphan/late-tool-result handling. |
| V11 | **Some narrow notices lose content, not just decoration.** compaction-summary-message.ts #divider returns an unbounded bare label in its narrow branch; an ASCII probe at width 20 produced a 31-cell label. Advisor attribution consumes width before note text; a long advisor name made the first note word disappear in a width-40 probe. | Bound the compaction label, and truncate/wrap attribution separately so note text remains reachable. Expanded details must preserve all content. |
| V12 | **Color-free and unsupported-terminal fallbacks need a complete policy.** modes/theme/color.ts only selects truecolor or 256color and emits raw color escapes; the foundation audit found no NO_COLOR branch there. Existing decorative and image paths have separate capability checks. | Reuse central terminal capabilities and honor the project's color-disable contract throughout application-owned styling. Supply meaningful textual substitutes for optional rendering integrations. |
| V13 | **Completion claims do not yet establish visual coverage.** The current repair assignment explicitly requires truthful feature coverage under T21. New components and passing narrow repair tests do not prove that untouched UI families have adopted the same presentation. | Create a visual inventory covering every exposed feature/state and supporting primitive, with production route, fallback, tests, and inspected captures. No unchecked row may become “done” because a related component changed. |

Line numbers are navigation aids from a changing working tree. Search by symbol and verify the final version before editing.

## 4. End-state visual contract

Use the original plan's hierarchy throughout the product, including its deliberate Harvest adaptations:

- **Workspace:** screen background and consistent padding; centered Home prompt capped at 75 columns; scrolling conversation with anchored composer; responsive session context sidebar. Preserve current responsive sidebar rules unless a measured defect requires a scoped adjustment.
- **Composer:** filled composerBg surface, one accent rail, bounded mode/model/effort metadata, attachment rows, separate actionable hints. Busy/retry/approval states share the same geometry and remain visible when height is scarce.
- **Transient dialogs:** viewport-bounded 60/88/116 caps selected by task density; filled modal surface; title/close affordance, query/input, selected row, secondary description, optional footer. At tiny sizes omit decorative rows before controls.
- **Managers:** retain enough room for Settings, Model Hub, Agents/Activity/revision management, Extensions/MCP, and plan/transcript explorers. Use panel/raised/selection roles consistently. Collapse category/sidebar into a switcher or drill-in at narrow widths; do not squeeze every manager into 60 columns.
- **Lists:** full allocated row selection, explicit current/disabled/busy state, bounded labels, quiet descriptions, shared scrolling and mouse mapping. Keyboard and pointer focus must be understandable in light and dark themes.
- **Transcript:** restrained assistant prose; common local/guest user panels; small reasoning and completion metadata; compact ordinary tool activity; purposeful panels for output/code/diffs; clear errors and approvals. Avoid duplicate metrics, titles, frames, or endcaps.
- **Fallbacks:** ASCII affects UI-owned symbols; color-free mode preserves hierarchy through spacing/text; missing image/clipboard/editor/terminal features show a usable text or manual route; unsupported capabilities state what is unavailable and how to proceed.
- **Consistency:** theme change, resize, replay, resume, tab switch, agent focus, and overlay close produce the same visual policy without changing runtime authority or payloads.

“Looks like OpenCode” means matching these observable relationships, density, spacing, surfaces, and interactions across the application. Recoloring old boxes is not the completion criterion.

## 5. Coordination with the running repair agent

### Current protected ownership

The active repair ledger assigns these boundaries. Treat them as reservations until its owner explicitly releases them:

| Owner/workstream | Reserved areas |
| --- | --- |
| Repair owner | sdk.ts; interactive-mode.ts; Composer; input/selector/focus/command controllers; modes/types; existing execution ledger and feature coverage document. |
| Repair A | Session facade, live factory, tabs, persistence/storage/view state, tools/sessions, related session tests. |
| Repair B | Autolearn/revisions/evaluation/recovery; task/agents; presets/manage-skill/learn; related static prompts and tests. |
| Repair C with owner integration | Palette, tab strip, attachments, Agents Hub/Activity, revision components, UI feature tests/captures; Composer proposals applied by owner. |

An unmodified file is **not** proof that it is free for editing. Re-read the latest ownership ledger before every phase. This audit has not sent instructions to or interrupted the external agent.

### Handoff procedure

1. Give this plan to the existing repair owner as a **visual follow-up phase**. It should finish or integrate its current bounded batch before taking new ownership.
2. Read the final repair diff and T01–T21 outcomes. Recheck V01–V13 against that tree; keep existing fixes instead of restoring earlier snapshots.
3. Record the actual starting HEAD, dirty/untracked manifest, and a separate copy of relevant starting bytes using file APIs. Never reset, clean, overwrite, or stash shared work to obtain a comparison.
4. The owner creates a new visual file-ownership table. Only after file release may workers modify overlapping components/controllers.
5. Preserve SDK policy, session UUID/lifecycle, archives/deletion recovery, revision transactions, evaluator cancellation, attachment payloads, and Laya removal. The visual phase must not weaken them.
6. Keep separate visual checkpoints and evidence; owner alone reconciles completed results into the existing feature coverage document.
7. If the repair phase has a real external blocker, record its exact blocked case. Independent released visual files can progress, but no runtime blocker can be covered by a cosmetic “pass.”

## 6. Complete surface inventory and required fallbacks

The source inventory found **115 TypeScript modules under modes/components, 10 under coding-agent/src/tui, and 25 under tui/src/components**. These include helpers/barrels/models, not 150 user-facing screens. Appendix A lists them for reconciliation.

The executor must discover routes from production command/control registries, controllers, tool renderers, extension APIs, and standalone TUI commands. Cross-check the active feature matrix. A filename list is not a feature coverage test.

| Family | Current adoption | Required states, recovery, and entrypoints |
| --- | --- | --- |
| Home/workspace/composer/tabs/sidebar | Partial redesign; overlapping repair-owned work | Empty/detached Home, one/many/overflow tabs, active/inactive/busy/archived/deleted sessions, close/new/switch, draft restoration, quiet mode, warnings, context/todo/changes/agents, narrow/short/ASCII, hidden sidebar and keyboard access. |
| Command palette and quick selectors | Mixed: newer palette budgets, older common chrome | Query/paste, no matches, groups, selected/current/disabled item, unknown command metadata, argument-taking draft, remapped keys, scroll, cancellation and editor restoration. |
| Settings/themes/composer/status previews | Older outlined manager and shared lists | Tabs/search/sections, values, disabled/read-only/warning items, theme live preview/cancel, legacy/custom/invalid themes, all composer shapes and status presets, short-screen preview removal before controls. |
| Models/providers/accounts/auth/setup | Older/mixed | Model Hub/browser/picker, temporary switch versus defaults, roles, discovery loading/empty/failure, missing credentials, OAuth/manual URL/code, custom provider input, logout/account/reset-usage, unsupported browser/callback fallback, cancellation and retry. |
| Session/history/tree/rewind/copy/info/move | Older/mixed, runtime repairs ongoing | Search/filter/empty, trees/branches, selected row, archive/restore/delete feedback, locked/busy failure, copy/link fallback, workspace directory picker, failed switch retaining the prior workspace, pointer/keyboard parity. |
| Agents/Activity/transcript/revisions | Partial/new integration owned by repair team | Foreground/background/live/settled/error agents; roster and focused transcripts; draft/inactive management rows; inspect/evaluate/cancel/pass/fail/promote/rollback/conflict; skills and presets; truthful unsupported capability states. |
| Ask/approval/plan/review/save | Older framed dialogs and explorers | Single/multi-question, recommended/current answer, custom text, note, multi-select, submit/cancel/error, required approval, plan TOC/detail/save/cancel, no silent clipping or accidental approval at small sizes. |
| Assistant/user/guest/skill/custom/hook messages | Mixed new rails and old fallback cards | Streaming/final/aborted/failed, Markdown/code/tables, attachments/reactions, attributed guest/advisor content, custom renderers plus generic fallback, unknown historical message types, expansion, replay/resume ordering. |
| Tool activities and output/diff/code/file/tree | Mixed | Streaming args/in-progress/success/warning/error/cancelled, no output, unknown/custom/MCP tools, expanded/collapsed, ANSI/tabs/long paths/wide glyphs, artifact/truncation links, pending args preserved through live/rebuild paths. |
| Manual shell and eval | Older independent border scaffolds | ! and !!, eval modes, streaming/cancel/exit/error, excluded-from-context status, large output, image output unsupported fallback, transcript restoration. |
| Extensions/plugins/MCP | Older managers and fallback panels | Discovery/config/install/loading/empty/error, list/detail/settings/wizard, live tools, disconnect/reconnect/retry, permission/capability status, extension custom UI return/focus contracts, missing/throwing custom renderer fallback. |
| Notices/maintenance/advisor/usage | Mixed | Usage/context/rate, compaction/handoff/snapcompact, retry/cache/late diagnostics, TTSR/todo reminders, advisor notes, background messages, tiny-model download/pause/timeout/error. Expanded recovery remains reachable. |
| Auxiliary panels | Older/feature-specific | BTW, cleanse, OMFG, transcript outline, snapcompact/composer previews, pause/help, optional fireworks/reactions; consistent close/return and no hidden controls when disabled or unsupported. |
| Standalone terminal tools | Require separate review | setup/model/session pickers, cleanse picker, git TUI, ps TUI/live board, gallery/read/render/config/plugin flows. Intentional plain CLI output stays appropriate; do not alter RPC/JSON output or redesign a browser dashboard as part of this terminal task. |
| Terminal capabilities | Partial shared policies | Truecolor/256/color-free, ASCII/Unicode/Nerd, supported/unsupported images/hyperlinks/cursor/mouse, inline versus fullscreen, multiline paste, external editor/clipboard missing/failed/cancelled. Preserve payload and provide a usable route. |

For **every real feature**, record: production entrypoint; mounted component; available actions; loading/empty/normal/busy/success/error/cancel states as applicable; recovery; terminal fallback; supported mode; focused verification; inspected visual artifact; status. Use “not applicable” with a reason when a state cannot occur. Use “unverified” for missing evidence. Do not invent backend support to fill a UI row.

## 7. Implementation sequence

### Phase 0 — Final-tree reconciliation and contract freeze

Owner reads the final repair handoff, this audit, original redesign decisions, current theme/keybinding/TUI docs, source, and existing tests. Reproduce findings before patching. Separate already-fixed findings from remaining ones.

Produce a visual inventory, file ownership, baseline captures, and a concise style contract. Resolve adapter/presentation signatures centrally before workers branch. Validate that normal startup uses effective settings, not only a direct initTheme fixture.

Exit: current repair work is preserved; released file ownership is explicit; all exposed feature routes are accounted for.

### Phase 1 — Theme adoption and terminal policy

Owner/foundation worker changes fresh settings defaults to harvest/harvest-light, preserving explicit settings and theme discovery. Audit bare initTheme calls in commands/standalone TUIs for consistent effective configuration and prepaint behavior.

Connect screenBg/panelBg/raisedBg/composerBg/modalBg to the central rendering adapters. Fill the **allocated surface**, including padding and row trailing cells, and preserve nested Markdown/diff/selected-row styles through ANSI resets. Background paint must not overwrite images, links, or cursor markers.

Retain loader fallback chains for older custom themes. Extend central color/symbol policy for color-disable and ASCII gaps, including scrollbar rails/thumbs, separators, cursor gutters, compact activity, Move, compaction arrows/ellipses, and all user-selectable composer styles. Do not add per-component environment or terminal-name guesses.

Exit: fresh configured startup and previews agree; custom/legacy themes remain valid; color-free and ASCII application chrome remain readable.

### Phase 2 — Shared dialog/list/manager presentation

Extend existing central helpers or add the smallest shared surface/presentation abstraction if there is no equivalent. Avoid duplicating fit, background reset, wrapping, padding, path shortening, sanitization, key hints, or capability lookup.

The owner defines transient-dialog versus manager layout, including size caps, actual width/height allocation, title/query/body/footer budgets, pane collapse, focus ownership, screen-to-component coordinates, and close restoration. Use the repair's composed-frame geometry contract.

Fix width-1/2/3 and zero-body split allocation in common chrome. Share selected/current/disabled/loading row treatment and scroll indicators. Short-screen fallbacks must keep the query/answer and selected action visible, then recover descriptions/chrome as room returns.

Exit: representative transient selectors and one dense manager share the same surfaces while remaining fully usable through resize and keyboard/mouse navigation.

### Phase 3 — Migrate all selectors, authentication, and managers

Apply the shared presentation to every applicable family in section 6 and Appendix A. Preserve return values, callback-once behavior, async cancellation, borrowed resources, keyboard remaps, and drafts.

Authentication must retain copyable complete URLs/callback values and manual entry; visually truncated labels must not truncate underlying link/payload data. Browser/clipboard failure remains actionable. Setup and temporary model switches must retain distinct persistence semantics.

Dense managers get a narrow category switcher or drill-in, bounded content scroll, and reachable close/back controls. Plan/transcript explorers keep intentional large reading areas with common typography and surface policy.

Exit: no built-in selector or manager remains an unexplained old presentation island; unsupported/headless flows remain truthful.

### Phase 4 — Transcript, tools, notifications, and custom fallbacks

Unify display policy across live event handling, ui-helpers rebuilds, ChatTranscriptBuilder, focused/parked agent viewers, manual shell/eval, guest prompts, and extension/custom fallbacks. Preserve semantic differences such as severity and author attribution.

Use compact success rows for ordinary activity; a shared purposeful output panel for bash/eval/code/diff/large results; explicit error/cancel/approval notices; expansion for additional content. Do not hide a failed tool's cause to make the transcript quieter.

Fix V10's completion association and V11's lost/truncated notice content. Include partial streamed tool args, late tool results, orphan settlement, detached task updates, read grouping, and replay in the verification fixture. Never alter provider protocol blocks to simplify display.

Exit: the same stored conversation has the same order, attribution, tool detail, and expansion behavior in all viewers.

### Phase 5 — Auxiliary and standalone terminal surfaces

Audit all remaining components plus cli/setup, session/model/cleanse pickers, git TUI, ps TUI/live board, previews, optional animations, and capabilities. Apply shared style where it benefits interactive screens; preserve intentional machine-readable and simple exit-only CLI output.

Finish unsupported image/hyperlink/editor/clipboard paths and optional-feature placeholders with meaningful text and a available action. Keep actual Harvest branding and capabilities; do not replace functionality with an OpenCode imitation.

Update authoritative theme/TUI docs, relevant package Unreleased changelog entries, and the owner-managed visual/feature coverage only after evidence exists.

Exit: all inventory rows are implemented, consciously retained under the new contract, or specifically unverified/blocked with evidence.

### Phase 6 — Integrated proof and final handoff

Run relevant package gates and focused contract tests; re-run affected T01–T21 preservation scenarios. Capture the required mounted UI scenes, inspect PNGs and underlying VT data, fix discovered defects, and repeat only affected checks.

Compare the representative screens against the pinned reference and the shared style contract. An independent read-only verifier checks production entrypoints and visual coverage, not just component construction.

Exit: acceptance matrix below has concrete results, all owned workers are settled, the diff contains only intended changes, and final docs state remaining limits accurately.

## 8. OpenCode multi-agent, subagent, and multi-session execution protocol

Use OpenCode's **available native** task/subagent/session facilities. Inspect the installed capabilities before dispatch; do not invent tool names or session IDs. Keep the user's configured model unless separately instructed. If parallel/native session facilities are unavailable, execute the same packets sequentially and report that limitation.

The existing repair owner remains the sole integrator. After its locks release, use this bounded team:

| Role | Exclusive responsibility |
| --- | --- |
| Owner/integrator | Runtime mounting, shared contracts, reserved repair integration, ownership registry, visual checkpoints, gates, and final handoff. |
| Foundation worker | Released theme/adapters/color/symbol/list/composer primitive files plus focused contract verification. |
| Dialog/manager worker | Released selector/auth/manager/standalone surface files; controller changes requested as exact patches to owner. |
| Transcript/tool worker | Released message/output/manual execution/fallback files; event/rebuild integration coordinated with owner. |
| Independent verifier | Read-only final production interactions, inspected captures, requirement/evidence reconciliation. Run after implementation workers settle if capacity is limited. |

Never assign the same file to two writers. A shared primitive change requires an interface announcement and bounded caller migration; an integration conflict returns to owner. Persistent sessions continue their assigned work; extra sessions are not a substitute for durable checkpoints.

Each worker packet contains: objective; current baseline; exact owned files; required dependencies; observable acceptance; preserved contracts; commands already run; expected artifacts; where to report. Workers must inspect current bytes before patching and return modified paths, result, test/capture evidence, and unresolved concerns.

Create a separate **visual execution checkpoint** and **visual coverage document** for this phase. Record session IDs, file locks, stage, completed evidence, unresolved decisions, and next action. Owner alone merges final results into the previous repair documents.

Before compaction/session transfer, save a concise checkpoint. On resume read that checkpoint, this plan, relevant owned-file diffs, and the latest handoff. Do not reread the entire repository, drop the original objective, or rely on a remembered “all fixed” claim. Keep tool output bounded; send focused findings rather than full logs; checkpoint at each milestone and before context is scarce.

One continuous assignment means integrate, verify, repair newly exposed issues, and deliver the completed result. It does not authorize invented success, conflicting writes, commits, releases, GitHub comments, or bypassing the repo's security/runtime invariants.

## 9. Mandatory visual and interaction acceptance

These supplement the existing T01–T21; they do not supersede them.

| Case | Observable acceptance |
| --- | --- |
| A01 — Effective startup | With isolated fresh settings, main startup and interactive standalone entrypoints use intended dark/light defaults; an explicit older/custom theme remains selected. The first paint and later paint agree. |
| A02 — Semantic surfaces | Mounted Home/session/dialog/manager/composer use their resolved surface roles across full allocated rows. Nested resets, selected spans, Markdown, links, and repaint do not produce holes, bleed, or stale background cells. |
| A03 — Legacy/custom theme | A legacy theme lacking new tokens loads through central fallback; invalid theme reports/falls back cleanly; preview/cancel/commit and runtime theme change repaint the same hierarchy. |
| A04 — ASCII/color-free | Every exposed composer style, menu, scrollbar, compact activity, dialog, tab/sidebar, notice, and manager uses the requested UI symbol policy. Color-disable retains selection/focus/status meaning without color SGR; user Unicode content is preserved. |
| A05 — Tiny geometry | Central helpers return lines no wider than allocations at 1/2/3/20/24 columns, including splits and long titles. Mounted short-screen flows retain a reachable selection/input and close/cancel; expansion restores suppressed detail. |
| A06 — Viewport transitions | Test 160×45, 121×32, 120×32, 100×30, 80×24, 60×16, 80×10, 60×8, 24×4 and back to wide. Include long/wide labels. No clipped control responds as an invisible hit target. |
| A07 — Dialog behavior | Open/filter/scroll/select/cancel/nested/back/close works through actual controls; focus/cursor returns to the correct draft once; pointer and keyboard select the same item. Remapped hints match usable bindings. |
| A08 — Managers | Settings, Model Hub, Agents/Activity/revisions, Extensions/MCP, session/tree and plan/transcript explorers have reachable category/navigation controls when secondary panes collapse. No fixed minimum crowds out primary actions. |
| A09 — Auth/setup | Loading/missing credentials/discovery failure/OAuth/manual input/retry/cancel render consistently. Complete underlying URLs and payloads survive narrow display; unavailable browser/clipboard offers usable manual action. No actual login required for deterministic fixtures. |
| A10 — Session/agent preservation | Restyle does not change UUID ownership, requested target, close/new/switch, draft restoration, archive gating, lifecycle settlement, or deletion recovery. Exercise the relevant existing repair cases through the visible controls. |
| A11 — Revisions | Both skill and preset controls show draft/busy/pass/fail/cancel/conflict/active states and truthful promotion disclosure; no visual change bypasses evaluation context, restriction, transaction, or cancellation rules. |
| A12 — Transcript parity | A multi-completion fixture with tool calls, partial args, results, late updates, guest/advisor/custom messages and errors has equivalent order/association after live rendering, rebuild, resume, park, and agent focus. No duplicated endcaps or lost text. |
| A13 — Tool lifecycle | All registered tool renderers and generic custom/MCP fallbacks cover streaming/progress/success/error plus applicable cancel/expanded states. Collapsed successful activity is quiet; detail/error/truncation remains accessible and sanitized. |
| A14 — Manual executions | !, !! and eval keep streaming/cancel/exit/context-exclusion/output semantics while adopting shared panels. Supported images work; unsupported image terminals preserve usable text/artifact information. |
| A15 — Important notices | Compaction labels are bounded; long advisor attribution cannot remove note words; warning/error/approval/retry cause and action remain reachable at narrow widths. |
| A16 — Optional integrations | Missing/throwing clipboard, editor, renderer, image, hyperlink, mouse or optional worker/backend has truthful state plus a usable supported route; text/attachments/focus/payload remain intact. |
| A17 — Performance/terminal safety | Existing differential-render/reference stability, transcript anchors, bounded buffers and sanitization contracts hold. No new console output corrupts TUI/RPC/worker streams. Use existing benchmarks for touched hot paths; investigate a measured regression. |
| A18 — Complete coverage | Every production feature route and inventory module is mapped to a reviewed surface or supporting contract. Each completed row links verification and inspected evidence. No unverified required family, fabricated backend, or “future” required UI is called complete. |

Tests must defend observable behavior/geometry/return values, not imports, source text, constants, prompt wording, or existence. No mock.module, tautological tests, source-grep tests, or broad user-state mutation. Use isolated settings, agent directories, stores, and deterministic transport boundaries. A component stub cannot prove a controller/SDK production route.

Recommended focused commands, adapted to final changed paths:

    bun --cwd=packages/coding-agent run check
    bun test <specific contract test files>
    bun scripts/ci-test-ts.ts coding-agent-ui
    python packages/coding-agent/bench/render-terminal-captures.py <capture-directory>

Inspect packages/tui/package.json for its actual package-local gate before changing shared primitives. Use root/affected TS checks when interfaces cross package boundaries. Do not use tsc/npx tsc or bare root bun test. No Rust gate is needed for a docs/TS-only change unless Rust changes are introduced; if they are necessary use the prescribed bun run test:rs/build:native workflow.

If a broad gate fails, report command, exact failure and changed-path relevance. Do not label failures “baseline” without evidence; do not stash/reset a shared active checkout to gather that evidence. Formatting drift outside owned files is not permission for drive-by edits.

The final report must state: what adopted the contract; which feature routes remain unavailable/unverified; commands/results; inspected capture paths; preserved repair outcomes; known limits. A successful gallery or typecheck alone cannot certify the whole product.

## 10. Launcher for the implementation agent

Give this document to the existing OpenCode repair owner with this instruction:

> Keep implementing the current production repair assignment. Add this document as the subsequent visual consistency phase, reconcile it with the final repair tree, and complete all remaining applicable work in one continuous assignment using your available native agents/subagents/persistent sessions. Preserve the repair's T01–T21 contracts and released file ownership. Complete the entire feature/surface inventory, central theme/presentation adoption, terminal fallbacks, production interaction verification, and inspected visual evidence. Keep durable checkpoints, do not invent completion, and deliver one integrated handoff without committing or releasing.

The user's instruction for this chat was audit/plan only. The implementation instruction above is for the agent to which the user hands this document.

## Appendix A — Module reconciliation checklist

Reconcile the generated module list below against current files before implementation. Mark modules as migrated, consciously retained with a contract, supporting-only, or unverified. Barrels/projection/state helpers do not need their own screenshot, but their consuming production surfaces do. This list is a source inventory, not a completed-feature claim.

<!-- The audited module inventory follows. -->

### packages/coding-agent/src/modes/components (115 modules)

- [ ] [advisor-config.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/advisor-config.ts)
- [ ] [advisor-message.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/advisor-message.ts)
- [ ] [agent-hub-projection.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/agent-hub-projection.ts)
- [ ] [agent-hub-renderer.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/agent-hub-renderer.ts)
- [ ] [agent-hub.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/agent-hub.ts)
- [ ] [agent-transcript-viewer.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/agent-transcript-viewer.ts)
- [ ] [agents-hub.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/agents-hub.ts)
- [ ] [ask-dialog.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/ask-dialog.ts)
- [ ] [assistant-message.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/assistant-message.ts)
- [ ] [attachment-chips.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/attachment-chips.ts)
- [ ] [background-tan-message.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/background-tan-message.ts)
- [ ] [bash-execution.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/bash-execution.ts)
- [ ] [bordered-loader.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/bordered-loader.ts)
- [ ] [btw-panel.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/btw-panel.ts)
- [ ] [cache-invalidation-marker.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/cache-invalidation-marker.ts)
- [ ] [chat-block.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/chat-block.ts)
- [ ] [chat-transcript-builder.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/chat-transcript-builder.ts)
- [ ] [cleanse-panel.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/cleanse-panel.ts)
- [ ] [codex-reset-fireworks.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/codex-reset-fireworks.ts)
- [ ] [collab-prompt-message.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/collab-prompt-message.ts)
- [ ] [command-palette.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/command-palette.ts)
- [ ] [compaction-summary-message.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/compaction-summary-message.ts)
- [ ] [composer-shape-preview.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/composer-shape-preview.ts)
- [ ] [composer-shape-registry.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/composer-shape-registry.ts)
- [ ] [copy-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/copy-selector.ts)
- [ ] [countdown-timer.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/countdown-timer.ts)
- [ ] [custom-editor.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/custom-editor.ts)
- [ ] [custom-message.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/custom-message.ts)
- [ ] [diff.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/diff.ts)
- [ ] [dynamic-border.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/dynamic-border.ts)
- [ ] [editor-top-gap.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/editor-top-gap.ts)
- [ ] [error-banner.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/error-banner.ts)
- [ ] [error-block.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/error-block.ts)
- [ ] [eval-execution.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/eval-execution.ts)
- [ ] [execution-shared.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/execution-shared.ts)
- [ ] [extensions/display-text.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/extensions/display-text.ts)
- [ ] [extensions/extension-dashboard.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/extensions/extension-dashboard.ts)
- [ ] [extensions/extension-list.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/extensions/extension-list.ts)
- [ ] [extensions/index.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/extensions/index.ts)
- [ ] [extensions/inspector-model.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/extensions/inspector-model.ts)
- [ ] [extensions/inspector-panel.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/extensions/inspector-panel.ts)
- [ ] [extensions/live-tool-session.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/extensions/live-tool-session.ts)
- [ ] [extensions/mcp-runtime.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/extensions/mcp-runtime.ts)
- [ ] [extensions/state-manager.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/extensions/state-manager.ts)
- [ ] [extensions/types.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/extensions/types.ts)
- [ ] [footer.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/footer.ts)
- [ ] [history-search.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/history-search.ts)
- [ ] [hook-editor.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/hook-editor.ts)
- [ ] [hook-input.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/hook-input.ts)
- [ ] [hook-message.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/hook-message.ts)
- [ ] [hook-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/hook-selector.ts)
- [ ] [index.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/index.ts)
- [ ] [keybinding-hints.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/keybinding-hints.ts)
- [ ] [late-diagnostics-message.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/late-diagnostics-message.ts)
- [ ] [login-dialog.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/login-dialog.ts)
- [ ] [logout-account-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/logout-account-selector.ts)
- [ ] [mcp-add-wizard.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/mcp-add-wizard.ts)
- [ ] [message-frame.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/message-frame.ts)
- [ ] [model-browser.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/model-browser.ts)
- [ ] [model-hub.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/model-hub.ts)
- [ ] [model-picker.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/model-picker.ts)
- [ ] [move-overlay.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/move-overlay.ts)
- [ ] [oauth-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/oauth-selector.ts)
- [ ] [omfg-panel.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/omfg-panel.ts)
- [ ] [overlay-box.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/overlay-box.ts)
- [ ] [pause-screen.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/pause-screen.ts)
- [ ] [plan-review-overlay.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/plan-review-overlay.ts)
- [ ] [plan-save-overlay.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/plan-save-overlay.ts)
- [ ] [plan-toc.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/plan-toc.ts)
- [ ] [plugin-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/plugin-selector.ts)
- [ ] [plugin-settings.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/plugin-settings.ts)
- [ ] [queue-mode-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/queue-mode-selector.ts)
- [ ] [reaction.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/reaction.ts)
- [ ] [read-tool-group.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/read-tool-group.ts)
- [ ] [reset-usage-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/reset-usage-selector.ts)
- [ ] [revision-views.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/revision-views.ts)
- [ ] [rewind-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/rewind-selector.ts)
- [ ] [segment-track.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/segment-track.ts)
- [ ] [select-list-mouse-routing.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/select-list-mouse-routing.ts)
- [ ] [selector-helpers.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/selector-helpers.ts)
- [ ] [session-account-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/session-account-selector.ts)
- [ ] [session-info-overlay.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/session-info-overlay.ts)
- [ ] [session-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/session-selector.ts)
- [ ] [session-tab-strip.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/session-tab-strip.ts)
- [ ] [settings-defs.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/settings-defs.ts)
- [ ] [settings-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/settings-selector.ts)
- [ ] [show-images-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/show-images-selector.ts)
- [ ] [skill-message.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/skill-message.ts)
- [ ] [skill-revisions.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/skill-revisions.ts)
- [ ] [snapcompact-shape-preview.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/snapcompact-shape-preview.ts)
- [ ] [status-line/component.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/status-line/component.ts)
- [ ] [status-line/context-thresholds.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/status-line/context-thresholds.ts)
- [ ] [status-line/git-utils.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/status-line/git-utils.ts)
- [ ] [status-line/index.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/status-line/index.ts)
- [ ] [status-line/presets.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/status-line/presets.ts)
- [ ] [status-line/segments.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/status-line/segments.ts)
- [ ] [status-line/separators.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/status-line/separators.ts)
- [ ] [status-line/types.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/status-line/types.ts)
- [ ] [stripped-tool-calls-placeholder.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/stripped-tool-calls-placeholder.ts)
- [ ] [theme-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/theme-selector.ts)
- [ ] [thinking-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/thinking-selector.ts)
- [ ] [tiny-title-download-progress.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/tiny-title-download-progress.ts)
- [ ] [todo-reminder.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/todo-reminder.ts)
- [ ] [tool-activity.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/tool-activity.ts)
- [ ] [tool-execution.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/tool-execution.ts)
- [ ] [transcript-container.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/transcript-container.ts)
- [ ] [transcript-outline.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/transcript-outline.ts)
- [ ] [tree-selector.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/tree-selector.ts)
- [ ] [ttsr-notification.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/ttsr-notification.ts)
- [ ] [usage-dashboard.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/usage-dashboard.ts)
- [ ] [usage-row.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/usage-row.ts)
- [ ] [user-message.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/user-message.ts)
- [ ] [visual-truncate.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/visual-truncate.ts)
- [ ] [welcome.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/welcome.ts)
- [ ] [workspace-sidebar.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/modes/components/workspace-sidebar.ts)

### packages/coding-agent/src/tui (10 modules)

- [ ] [code-cell.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/tui/code-cell.ts)
- [ ] [file-list.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/tui/file-list.ts)
- [ ] [hyperlink.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/tui/hyperlink.ts)
- [ ] [index.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/tui/index.ts)
- [ ] [output-block.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/tui/output-block.ts)
- [ ] [status-line.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/tui/status-line.ts)
- [ ] [tree-list.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/tui/tree-list.ts)
- [ ] [types.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/tui/types.ts)
- [ ] [utils.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/tui/utils.ts)
- [ ] [width-aware-text.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/coding-agent/src/tui/width-aware-text.ts)

### packages/tui/src/components (25 modules)

- [ ] [box.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/box.ts)
- [ ] [cancellable-loader.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/cancellable-loader.ts)
- [ ] [composer/band.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/composer/band.ts)
- [ ] [composer/borderless.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/composer/borderless.ts)
- [ ] [composer/box.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/composer/box.ts)
- [ ] [composer/claude.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/composer/claude.ts)
- [ ] [composer/field.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/composer/field.ts)
- [ ] [composer/index.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/composer/index.ts)
- [ ] [composer/pi.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/composer/pi.ts)
- [ ] [composer/rail.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/composer/rail.ts)
- [ ] [composer/registry.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/composer/registry.ts)
- [ ] [composer/rule.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/composer/rule.ts)
- [ ] [composer/types.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/composer/types.ts)
- [ ] [editor.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/editor.ts)
- [ ] [image.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/image.ts)
- [ ] [input.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/input.ts)
- [ ] [loader.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/loader.ts)
- [ ] [markdown.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/markdown.ts)
- [ ] [scroll-view.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/scroll-view.ts)
- [ ] [select-list.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/select-list.ts)
- [ ] [settings-list.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/settings-list.ts)
- [ ] [spacer.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/spacer.ts)
- [ ] [tab-bar.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/tab-bar.ts)
- [ ] [text.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/text.ts)
- [ ] [truncated-text.ts](C:/Users/sanid/Desktop/harvest-2.0/harvest/packages/tui/src/components/truncated-text.ts)


