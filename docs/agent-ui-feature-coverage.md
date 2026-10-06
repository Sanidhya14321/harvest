# Agent UI Feature Coverage Matrix

> Companion to `agent-ui-execution-state.md`. Each entry: feature, owner, discovery/invocation, component family + detail view, lifecycle states, errors + recovery, fallback, test + visual evidence. Status: `todo | in-progress | done + evidence`.
> Previous audit evidence (not permanent counts): ~84 commands, ~30 declared tools. Re-enumerated 2026-10-04: ~90+ builtin slash specs, 27 builtin + 3 hidden tools, 37 renderer keys. 2026-10-06: +2 builtin tools (`sessions`, `presets`), 39 renderer keys.
>
> ## 20. Session management, model tools, revisions, Laya removal (2026-10-06 assignment)
> | Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
> |---|---|---|---|---|---|---|---|---|
> | Session facade (create/list/inspect/select/rename/close/reopen/stop/archive/restore/delete) | session/session-management-facade.ts over LiveSessionRegistry + SessionTabs + storage | tools + palette + Activity | handles (id/path/title/status/visible/archived) | idle/running/waiting/completed/error/archived | busy rejects unless stop-and-delete; failure retains discoverability; tombstones block background writes | explicit errors, no silent no-ops | session-management-facade.test (9 pass) | done |
> | Tab × close + Alt+W | session-tab-strip.ts, keybindings.ts, input-controller.ts | mouse ×, Alt+W, /tab close, palette | close column, hover vs flash | active/inactive close, last-tab Home | close never aborts/disposes/archives/deletes; failed close preserves tab | keyboard/mouse parity | strip tests (16 pass) + cap-100x30-tabs.png | done |
> | Zero-tab Home | composer.ts (forceHome), interactive-mode.ts, input-controller.ts | automatic on last-tab close | Home wordmark + prompt, no tabs/transcript | detached (hidden running) | creation failure preserves draft + attachments; no dispatch into hidden session (Enter, Ctrl+Enter, slash-remainder gated) | explicit error + draft kept | input-controller-home-detached.test (3 pass) + cap-100x30-lasthome.png | done |
> | Warm-first navigation | selector-controller.ts (#warmNavigationRef), session-tabs resolveNavigationTarget | keyboard/mouse/palette | ID → warm-path → cold → missing | unsaved live runtimes resolve without disk files | missing file drops tab with message; failed switch preserves current | legacy path for cross-project | live-tab-navigation (9 pass) + navigation (10 pass) | done |
> | Picker busy-delete guard | session-selector.ts + selector-controller.ts | session picker Del | stop-and-delete confirm | busy/idle | re-check at execution (race); stop failure retains session | Esc-then-delete path | worker-A busy-delete suite (5 pass in 76-file batch) | done |
> | Activity/Sessions section | agent-hub.ts (sessions section + archive/restore props + async cold merge) | hub tab 3, a/u keys | All/Open/Running/Needs Attention/Recently Closed/Archived + search | verbatim child states (no invented cascades); "running" filter is running-only, waiting lives under attention | stop refuses non-running/waiting honestly; approvals stay with originating owner | send only when wired | agent-hub-sessions (5) + archive-restore (7) | done |
> | Palette budget + dispatch | command-palette.ts + interactive-mode.ts openCommandPalette | Alt+K, /commands | viewport-budgeted window, draft intents, feedback kinds | filtering/paging/resize/paste/IME | unknown/false dispatch → visible error; rejections → visible error; file/template unknown-args → draft | remap-aware hints | palette-nav + dialog-budget (15) + cap-80x24-palette-bottom.png | done |
> | sessions model tool | tools/sessions.ts | model-callable | list/inspect (read), create/rename/send/stop (exec) | concurrency/depth/spawn-policy/lineage gating + atomic capacity reserve | self-stop refused (registry + UUID + runtime identity); running stop needs confirm; unwired host reports truthfully | host grant (scope+expiry) for broader targets; forged/expired denied | managed-sessions-tool.test (13 pass) | done |
> | presets model tool + managed location | tools/presets.ts, task/agents.ts | model-callable + /agents hub | isolated managed-presets/, revision verbs, architect generation | draft/evaluated/active + unevaluated-disclosed | collisions refused; authored/bundled protected; eval-fail blocks promote | hub-authored flow unchanged | autolearn-managed-presets.test (12 pass) | done |
> | Skill revisions + learn refresh | autolearn/managed-skills.ts + revisions.ts, tools/learn.ts + manage-skill.ts | model-callable + auto-learn | history on first mutation, expectedActive conflicts, refreshSkills after write | draft/evaluate/promote/rollback/pin/prune-20 | conflict rejects; failed eval never promotes; eval runs claim no budget | unevaluated = unevaluated (disclose flag) | autolearn-managed-revisions.test (10 pass) | done |
> | Native discovery (.harvest) | task/discovery.ts | automatic | .harvest + legacy .omp sources, managed-presets merge | authored > managed precedence | missing dirs tolerated | legacy .omp fallback | laya-removal.test discovery case (pass) | done |
> | Laya removal | deleted modules + neutered call sites (sdk, agent-session ×2, structured-subagent, classifier, brain, main, setup, registry, status-line, footer, packaging, CI) | n/a (removed) | inert laya.* keys, /laya notice, LAYA_* warn-once, empty unknown segment | lexical/graph retrieval authoritative | old events replay as no-ops; old layouts tolerate removed segment | human approval fail-closed; local inference intact | laya-removal.test (4 pass) + brain (11) + secrets (155) + approval (7) + grounding + plan-mode + memory suites | done |
> | Renderers for new tools | tools/semantic-ops.ts + renderers.ts | transcript | compact inline rows + expanded panels | pending/running/success/error | errors never wear success styling | generic fallback | tool-registry-coverage.test (pass) | done |

> Sections 1–19 below were written 2026-10-04 for the visual-parity scope; rows touched by the 2026-10-06 assignment are amended in §20. Tab-close, Home, Activity, palette-budget, skill-refresh, and Laya rows below should be read as superseded by §20 where they conflict.

## 1. Startup / setup / warnings / changelog / init
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| First paint / startup composer | modes/composer.ts, startup-composer.ts | boot | home shell (pixel wordmark, ghost prompt, hints, version) | loading/adopted | — | inline | startup-composer.test (21 pass) + cap-100x30-home.png | done |
| Setup splash / scenes | setup-wizard/* | first run | dialogs | — | — | — | theme-scene-preview.test pass | done |
| Config warnings / notices | interactive-mode.ts | boot | attention slot | — | — | short-height priority | workspace-sidebar-state.test home-draft + cap PNGs | done |
| Changelog | — | boot | notice | — | — | — | startup-composer.test pass | done |

## 2. Sessions / tabs / resume / tree / branch / fork / rewind / handoff / worktrees / dirs
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| Tabs / switching | session-tabs.ts, session-tab-strip | keys/palette | tabs (open-flash, hover, ●/?/!/✓, + button) | loading/failed/rapid | failed load → retain per-session state | narrow | navigation tests + session-tab-strip.test (16 pass) + 01–09 PNGs | done |
| Resume / tree / branch / fork / rewind / handoff | session/* | palette/commands | selectors/viewers | — | existing flows preserved | fullscreen explorers | existing suites pass | done |
| Scroll-back reading | composer.ts + transcript-container.ts | wheel/page/keys | anchored viewport | scrolled/streaming/resize/tab-switch | anchor release → tail fallback | offset clamp | workspace-scroll-anchor.test (5 pass) + workspace-scroll.test | done |
| Worktrees / directories | active-repo-context + sidebar VCS | auto | Workspace Changes | cwd switch | non-repo truthful | manual noteWorkspaceMutation seam | cap PNG (non-repo state) | done (worktree list rows: open — only single-child root resolved) |

## 3. Composer editing / history / autocomplete / attachments / paste / clipboard / external editor / queues / steering / follow-up
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| Editing / history / autocomplete | custom-editor.ts, input-controller.ts | keyboard | composer (ghost placeholder, mode-aware) | idle/running/error | — | short-screen clamp + remap-aware chords | input-controller families + e2e (5 pass) + tui placeholder (4 pass) | done |
| Attachments / paste / clipboard / external editor | — | — | composer | large-paste | missing clipboard/editor → truthful message | ASCII | large-paste tests | todo |
| Queues / steering / follow-up | queue-input.ts | Ctrl+Q/Enter | attention | — | — | existing families pass | done |

## 4. Models / roles / effort / providers / auth / quotas / fast / prewalk / extended context
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| Model selector / cycling (Ctrl+P) | selectors | palette/Ctrl+P | dialog | — | offline → cached | existing suites (49 model-hub pass) | done |
| Roles / effort / providers / auth-accounts / quotas / fast / prewalk / xctx | existing selectors/dialogs | palette/commands | dialogs/sidebar | loading/degraded | auth failure → existing flow | viewport budgets applied | dialog-viewport-budget.test | done |

## 5. Plan / approvals / goals / loops / pause / retry / forced tools
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| Approval / question / plan-review | selector-controller.ts | auto | dialogs | required-attention | Esc = cancel (never approve) | fullscreen preserved | overlay-focus test + ask suites | done |
| Goals / loops / pause-resume / retry (F5/Alt+R) / forced tools | existing controllers | keys/palette | hints/dialogs | — | existing policies untouched | — | existing suites pass | done |

## 6. Files / search / AST / diffs / diagnostics / binary-large / artifacts
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| read/write/edit/ast-edit/ast-grep/grep/glob | tools/*, edit/renderer | transcript | inline/panel/diff | pending/exec/success/error/denied/interrupted/collapsed/expanded/narrow | error chrome | ASCII/width caps | read/edit renderer tests + transcript-tool-gaps.test | done |
| checkpoint / rewind | tools/semantic-ops.ts | transcript | semantic panel (created/restored disposition) | success/error/expanded | error distinct from restored | quiet path untouched | semantic-tool-render.test (8 pass) + gallery | done |
| search_code | tools/semantic-ops.ts | transcript | query + bounded hits w/ file:line links | success/no-matches/error/expanded | invalid distinct; empty = warning no-matches | collapsed cap | semantic-tool-render.test (5 pass) + gallery | done |
| think (hidden) | tools/think.ts | — | empty component (was undefined-cast) | — | sparse-result crash fixed | — | tool-registry-coverage.test | done |
| Diagnostics / binary-large / artifacts | — | — | bounded panels | — | SafeToolRendererComponent degrade | fallback label | tool-registry-coverage.test (throwing-renderer) | done |

## 7. Bash / manual exec / PTY / Eval / kernels / cells / SSH / debugging
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| bash / manual `!`/`!!` / PTY | execution-shared.ts | transcript | code/output panel | streaming/success/error | viewport insets preserved | — | bash fixtures + gallery | done |
| eval / kernels / cells | eval-render | transcript | panel | — | — | — | eval tests + transcript-tool-gaps.test | done |
| ssh / debug | existing renderers | transcript | panels | — | — | — | existing suites pass | done |

## 8. Task / Hub / Vibe / background / advisor / watchdog / agents / messages / live-parked
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| task/hub/vibe/background/advisor | task/render, background-tan/advisor-message | transcript | cards | live/parked | cycle/depth bounds | tombstone retention for sealed cards | chat-transcript-late-result.test + task suites | done |
| Agents focus / parked viewers | chat-transcript-builder.ts | agent hub | transcript | live/restored/parked + late results | late result preserved (50-FIFO sealed retention) | — | chat-transcript-late-result.test (3 pass) | done |

## 9. Todo phases / all task statuses
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| Todo (getTodoPhases, all statuses) | sidebar snapshot (all statuses projected) | sidebar | section | — | unknown content stays visible | omitted when empty | workspace-chrome.test | done |

## 10. MCP / plugins / marketplaces / extensions / hooks / skills / templates / custom UI
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| MCP servers / prompts / custom tools | mcp/*, extensibility/* | palette/transcript | sidebar section + generic fallback | connected/failed/offline | failure detail retained | quiet-startup seed | mcp-render-status.test | done |
| Skills / templates / file commands / hooks / custom UI | interactive-mode palette merge | palette (Alt+K) | palette groups + draft intent | — | refresh fail → cached | description-only w/o manager | command-registry-coverage.test (6 pass) + palette PNG | done |
| manage_skill | tools/semantic-ops.ts | transcript | semantic panel (verb disposition) | success/refused/error/expanded | shadow refusal distinct | — | semantic-tool-render.test (4 pass) | done |

## 11. Browser / computer-use / web / citations / GitHub / Git-workspace changes
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| web_search (full answer collapsed) | web/search/render | transcript | panel (bespoke, unchanged) | — | — | — | existing suites (preserved) | done |
| browser / github / Workspace Changes (VcsRepo) | interactive-mode (ActiveRepoContext root, cwd retarget, mutation debounce) | sidebar/transcript | panels | non-repo/JJ/offline/cwd-switch | truthful bounded, stale-gen guarded | non-repo PNG state | done |

## 12. Memory / learning / maintenance / provenance / corrections
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| recall/retain/reflect | memories/* | transcript | panels | — | — | — | existing suites | done |
| memory_edit / learn | tools/semantic-ops.ts | transcript | semantic panel (op+target / lesson preview) | success/not-found/empty/expanded | unknown effect = warning, never success | — | semantic-tool-render.test (8 pass) | done |

## 13. Context / compaction / rules / Laya / snapcompact / tiny / diagnostics
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| Context / compaction / rules | status snapshot (known/unknown flags) | sidebar/inspector | sections | unknown stays unknown | — | usage width clip | workspace-chrome.test | done |
| Laya / snapcompact / tiny / diagnostics | existing owners (untouched) | status | hints | off = quiet | fail-open preserved | — | existing suites (preserved) | done |

## 14. Security scans / findings / validation / import-export / disposition / questions / uncertain outcomes
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| security_scan (+ findings/disposition) | tools/semantic-ops.ts | transcript | semantic panel (action + severity + phase) | success/warning/error/info/expanded | failed/cancelled distinct; unknown = info | no-matches n/a | semantic-tool-render.test (4 pass) + gallery | done |
| Approvals / questions / checkpoints | selector-controller + ask/plan-review | attention slot | dialogs (Escape = cancel, never approve) | required-attention | settlement once + focus restore | fullscreen explorers | overlay-focus test + checkpoint renderer | done |

## 15. Collaboration / guests / permissions / sharing-export / redaction / transport
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| collab host/guest / share / export / redaction | collab/* (untouched) | prompts/transcript | transport failure | existing flows preserved | — | existing suites (preserved) | done |

## 16. STT / live voice / TTS / images / media / device failures
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| STT / live voice (Ctrl+L) / TTS | existing owners (untouched) | keys | — | device/provider failure → message | — | existing suites (preserved) | done |
| Generated images / media artifacts | existing pane budgets (untouched) | transcript | constrained pane + fallback | unsupported protocol → text fallback | — | existing suites (preserved) | done |

## 17. Settings / usage-stats-trace / help-hotkeys-tools / updates / maintenance / restart / exit
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| Settings / ModelHub / AgentsHub | selectors | palette | dialogs (viewport budget: collapse sidebars first, selected stays visible) | short-terminal (80x10, 60x8) | overflow → windowed scroll | tall-viewport preview guard | dialog-viewport-budget.test (15 pass) | done |
| Usage / stats / trace / help / hotkeys / tools / updates / restart / exit | selectors + dashboard | palette | dialogs/inspectors | short-terminal budgets (usage-dashboard, history-search) | — | alt-screen vs inline | dialog-viewport-budget.test | done |

## 18. BTW / TAN / OMFG / Cleanse / custom-hook-skill-advisor-summary-diagnostic-launch
| Feature | Owner | Discovery/invocation | Components | States | Errors/recovery | Fallback | Test + capture | Status |
|---|---|---|---|---|---|---|---|---|
| BTW/TAN/OMFG/Cleanse | btw/tan/omfg/cleanse panels (untouched) | transcript | dedicated cards | — | SafeToolRendererComponent degrade | tool-registry-coverage.test | done |
| custom/hook/skill/advisor/summary/diagnostic/launch messages | custom-message, hook-message, skill-message, etc. (untouched) | transcript | dedicated cards, custom priority preserved | — | SafeToolRendererComponent degrade | tool-registry-coverage.test | done |

## 19. Special semantic tools (deliberate presentation, never bare JSON)
| Feature | Test | Status |
|---|---|---|
| yield (hidden, quiet fallback) | tool-registry-coverage.test | done |
| ask (timeout auto-choice disclosed) | ask tests incl. dialog suite (1 pre-existing memo failure, unrelated) | done |
| resolve (apply/reject/failure distinct) | existing resolve renderer (unchanged) | done |
| memory_edit / learn / manage_skill / search_code / checkpoint / rewind / security_scan | semantic-tool-render.test (29 pass) | done |

## Registry counts (live-enumerated 2026-10-04; old evidence 84 commands / 30 tools is stale)
- Commands: ~90+ builtin slash specs (`BUILTIN_SLASH_COMMANDS`; exact count asserted live in command-registry-coverage.test).
- Tools: 27 builtin + 3 hidden (`builtin-names.ts`); 37 renderer keys incl. aliases/devices.
- Themes: harvest/harvest-light defaults (schema), legacy via central fallbacks (theme-surfaces.test).
- Captures: 11 `cap-*` PNGs inspected (100x30 session/palette/light, 160x45 docked, 121/120 edges, 80x24 narrow/overlay/ascii, 60x16, 24x4) + 11 navigation PNGs; manifest: `$TEMP/harvest-ui-review/*.json|png`.
- Perf: `bench/rendering.ts` (editor 0.08ms/render) + `transcript-compose.bench.ts` (ratio 1.19 ≤1.3, p95 0.65ms <10ms, bytes 1.000).

## 20. Correction phase (2026-10-07): review findings S1–S7, R1–R6, U1–U2, L1–L2

Each row names production entry points (not helpers) and scenario evidence. Prior §20 (2026-10-06) rows claiming helper-only completion are superseded here.

| ID | Feature / flow | Owner + production entry points | Evidence |
|---|---|---|---|
| S1 | Picker + `/delete` delete any session through the facade: settle → seal writer → delete storage → tombstone → dispose/detach → drain; unsettled stop-and-delete refuses | selector-controller.ts `#deleteSessionById` via `ctx.sessions`; facade.delete; SessionManager.seal | session-delete-seal.test (2 pass, real manager + storage: late rename/append/ensureOnDisk cannot recreate; settlement failure stays discoverable) |
| S2 | Home submit snapshots full payload (text/images/links/metadata) before creation; fresh tab registered (open/visit/noteId/persist); empty creates nothing | input-controller.ts gates; interactive-mode.ts createSessionFromHomeDetached | input-controller-home-detached.test (7 pass: text, PNGs, links, image-only, failure-restore, empty) |
| S3 | Active × selects right-neighbor else left (verified switch) else Home; inactive hides only; never aborts work | interactive-mode.ts strip setOnClose mirroring slash close | strip tests (16) + cell-geometry (4) + cap-100x30-tabs.png |
| S4 | Model sessions return UUID + registryId, dispatch task once backgrounded, record lineage (parentAgentId, depth+1, per-caller snapshot); UUID↔registry mapping explicit | interactive-mode.ts factory; tools/sessions.ts findRef/liveUuid; live-session-factory parentAgentId/taskDepth | managed-sessions-tool.test (13: UUID mapping, taskAccepted, lineage, concurrency, depth) |
| S5 | No model-settable scopeGrant; host managedSessionGrant (scope+expiry) only; yolo ordinary behavior + provider safety intact | tools/sessions.ts checkTargetAccess/trustedGrant; ToolSession.managedSessionGrant | forged/host-grant/expired tests |
| S6 | Facade constructed in InteractiveMode (ctx.sessions); storage-backed archive + legacy migration; reopen-by-ID incl. pathless warm; cold listAll; hub facade.list + async + archive/restore; /tab archive/restore; restart keeps IDs | facade; selector-controller hub + verbs; #restorePersistedSessionTabs | facade (11) + hub-sessions (5) + archive-restore (7) + tabs-controller (9) + live-tab-nav (9) |
| S7 | Detach saves draft/scroll/anchor under closed UUID; clears editor/attachments/title/scroll; × cells from terminal geometry (wide/combining/ANSI/clipped safe-hide) | enterHomeDetached; session-tab-strip.ts cell geometry | view-state-home (4) + cell-geometry (4) + cap-100x30-lasthome.png |
| R1 | Real evaluators: restricted child (restrictToolNames, minimal tools, no UI, no autolearn) executes task, graded verdict + run/session IDs; override() only | autolearn/eval-executor.ts installed from sdk.ts; tools pass parent | budgets wiring + B eval suites (injected runners mirror production shape) |
| R2 | Architect/eval children use runtime-only overrides; config bytes identical on success + failure | presets.ts runArchitect; eval-executor | B settings-bytes tests |
| R3 | Omitted policy fields merge from current; explicit values validated | agents.ts + managed-skills.ts merge paths | prompt-only/body-only preserve tests |
| R4 | Materialize-first, pointer-only-on-success, rollback-on-failure, interrupted recovery; rollback targets active only | revisions.ts promoteRevisionAtomic + rollback guard + module txn flows | materialization-failure + interrupted-txn tests |
| R5 | Minted-grammar revIds, name guards, jail/ancestor/link checks, identity verify | revisions.ts guards + module hardening | traversal/link/fixture tests |
| R6 | One shared transaction boundary (twins delegate); 20-retention + active/pinned; sdk agent_end resets skill+preset budgets; autoContinue-scoped tool gates; eval + subagent run pins (preset + active skills) | revisions.ts; sdk.ts; tools gates; structured-subagent.ts pins | budgets (5) + pins/retention/concurrent suites |
| U1 | Non-builtin palette selections route through editor.onSubmit production submission exactly once; unknown metadata drafts | resolvePaletteSelection + owner onSelect fallback | palette-production-dispatch (3) + registry/metadata suites |
| U2 | Open palette re-budgets on every terminal resize; selection stays visible 100×45→80×10→60×8 | #rebudgetCommandPalette on stdout resize | resize test via real boot + stdout emit + cap-80x24-palette-bottom.png |
| L1 | Wrapper ignores legacy laya metadata (identical outcomes); AI types tombstoned; laya-setup skill deleted; docs fixed | wrapper.ts; ai/types.ts; tombstones; skill scan | wrapper (4) + tombstones (4) + ai suites (115) |
| L2 | Notices state configured permissions (builtin-registry, main, decision-layer, README) | owner notice edits | laya-removal.test notice assertions |
