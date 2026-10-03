# Product decisions (audit follow-up F1–F10)

Date: 2026-10-03. Each proposal from the codebase audit was decided
deliberately: implemented where a safe mechanical slice existed, otherwise
recorded with rationale. Items marked DONE have regression tests; items
marked OPEN need UX/product design and must not be half-implemented.
Status: 3 DONE (F2, F5, F9), 1 PARTIAL (F8), 6 OPEN (F1, F3, F4, F6, F7, F10)
— mirrors `AUDIT_PROGRESS_TRACKER.md`.

## DONE

- **F2 — Trust-aware MCP capability classification.** DONE. Gating
  eligibility follows each call's structured approval tier
  (`resolveToolTier`), so MCP/extension writes are classified even though
  their wire names are unlisted; undeclared tools default to `exec`.
  Per-tool user policy (`tools.approval.<tool>: allow|deny|prompt`) already
  provides audited allow/deny overrides with tests. Stream-F slice (2026-10-03):
  per-server overrides via `tools.approval.<server>.<tool>` (nested or flat
  dotted key, see `mcp-approval-policy.ts`) take precedence over
  `tools.approval.<tool>`; invalid values are ignored fail-closed and
  tool-owned deny still wins. No UX redesign.
- **F5 — Action outcome reconciliation.** DONE via U2. Uncertain
  deliveries report `outcomeUnknown` with "verify the remote state"
  guidance instead of replaying; the transcript distinguishes completed
  effects from rolled-back local work. Stream-F slice (2026-10-03):
  an in-memory outcome ledger (`mcp-outcome-ledger.ts`, last 100 outcomes
  keyed by idempotency key: committed/unknown/failed) annotates result
  details additively; replay policy is unchanged.
- **F9 — Release and portability qualification.** DONE. Release gates
  smoke-test every binary target, the packed npm tarball (source-vs-npm:
  tarball contents, version parity, bundled-CLI boot), and a CPU sidecar
  proof (health identity, authenticated inference, token rotation, teardown)
  on Linux and Windows. macOS sidecar and full-offline CI are documented
  skips with manual gates (CPU-wheel availability unverified on clean mac
  runners; cold-cache model download needs network) — darwin binaries stay
  gated, and the smoke script is macOS-safe so the manual gate runs
  unmodified. A fresh-registry consumer install stays manual too: the
  `@harvest` scope is workspace-resolved, not published to the registry.
  Automated-vs-manual legs are itemized in the F9 section below.

## PARTIAL

- **F8 — Explicit context pipeline controls and provenance.** PARTIAL.
  `/diagnostics` explains run stage, gating verdicts, and failures, and each
  transform (pruning tokens/latency, rerank cache hits, scheduler counters)
  is measured. A unified per-transform provenance view is still open.

## OPEN (deliberate, with rationale)

- **F1 — Project trust before executable discovery.** OPEN. The repo
  documents committed definitions as trusted (`docs/mcp-config.md`) and
  `tools.approval` gives per-tool deny. Flipping to default-distrust would
  break that configured contract; it needs a UX design (first-open prompt,
  headless policy), not a silent behavior change.
- **F3 — One session/task budget.** OPEN. Spend is reported (stats, goal
  budgets) but no kill-switch exists by design: hard-stopping at a dollar
  limit risks corrupting runs, and provider usage/pricing is delayed or
  unknown, so exact enforcement cannot be promised. Needs estimate UI plus
  ask-before-raise flow.
- **F4 — Privacy preview per sharing channel.** OPEN. Redaction seams
  exist per channel (provider obfuscation, share snapshots) but no unified
  pre-share preview UI. Needs design; image-embedded secrets stay a
  documented limitation. The explicit per-channel policy is itemized in the
  F4 matrix below — only the unified preview UI stays open.
- **F6 — Unified integration health view.** OPEN. MCP statuses, `/laya`
  diagnostics, backend statuses, and smoke probes each report separately
  and correctly; merging them into one view is UI work with no stubbed
  halfway state worth shipping.
- **F7 — Retrieval explainability and correction.** OPEN. Scoped
  recall plus update/forget/invalidate APIs exist and cited brain retrieval
  names its sources; a "why was this injected" surface is UI work. The
  surfacing contract (what is cited today, correction workflow, scope
  controls) is documented in the F7 section below — only the unified UI
  stays open.
- **F10 — Consistent cancellation and recovery UI.** OPEN at the UI
  layer. The protocol work is done (immediate abort lane, per-session
  approval ownership, guest-request settlement, owner-aware Stop). A
  second-Stop force behavior and per-operation owner display need TUI
  design; inventing force-kill semantics here would risk state loss.

## F4 — Per-channel redaction matrix (contract, Stream-F docs slice 2026-10-03)

Status: policy DOCUMENTED below; pre-share preview UI stays OPEN. No
behavior changed — this matrix describes the current seams so reviewers and
users know exactly what leaves the machine on each channel.

| Outbound channel | Secret redaction | Preview before send | Reference |
| --- | --- | --- | --- |
| `/share` → share server (sealed AES-256-GCM blob, 1 MB cap, link `<base>/<id>#<key>`) | Yes, when `share.redactSecrets` (default `true`) and `secrets.enabled` with configured secrets: typed per-field walk over header, system prompt, tool descriptions, entry summaries/labels, and message text including tool-result output and `@file` mentions. Opaque payloads that cannot be walked field-by-field (`providerPayload`, `redactedThinking`, `compaction.preserveData`, extension `details`/`data`, `mode_change.data`, output schemas) are dropped, not shipped. | None — upload is immediate. Check the setting and watch for the "trimmed to fit" notice. | `packages/coding-agent/src/export/share.ts`, `src/slash-commands/builtin-collaboration.ts`, `src/commands/share.ts` |
| `/share` → secret gist (`share.store: "gist"`, needs authenticated `gh`, falls back to the server; 5 MB cap) | Same snapshot as above (`buildShareSnapshot` runs before sealing either way). | None — same as above. | Same as above (`sealToFit`, `GIST_FILENAME`) |
| `/collab` live replication (`welcome` + `snapshot-chunk`, `entry`/`event`/`state`/`bus`/`agents` frames, `fetch-transcript`; guest replica at `~/.harvest/collab/<roomId>.jsonl`) | No — guests receive the raw live transcript including the back-transcript. The boundary is link possession (full 48-byte key + write token vs view-only 32-byte key, host-verified) plus AES-256-GCM in flight; scope is limited to the shared session and its subagent descendants (advisors, other sessions, orphans excluded). Do not host collab on a session containing secrets you would not show the guests. | None — starting `/collab` is the consent step; use `/collab view` for read-only guests. | `docs/collab.md`, `packages/coding-agent/src/collab/` |
| `/export` HTML file | No — raw local archive (the explicit local-archive option from the audit). | N/A (local file). | `AgentSession.exportToHtml` |
| `/dump`, clipboard, LLM-request JSON (`harvest-llm-request-*.json` in the OS tmp dir) | No — raw by design, with an on-screen warning that the file persists and may contain secrets. | N/A (local). | `formatSessionAsText`, `dumpLlmRequestToTmpDir` |
| Provider / advisor outbound | Secret obfuscator over provider context and advisor rendering, gated on `secrets.enabled` with the `secrets.*` config. | N/A. | `src/secrets/`, `src/advisor/runtime.ts` |
| `/diagnostics`, run record, status-line `run` segment | N/A by construction — stage/tool/timing/gating-verdict metadata only, no message content. | N/A. | Run-stage record, `/diagnostics` |
| Stats dashboard (local SQLite) | Local-only; no outbound channel. | N/A. | `packages/stats/` |
| Calibration data (`decisions.jsonl`) | Length-capped snippets (state 200 chars, instructions 500 chars), size-rotated, git-ignored user data (`~/.harvest/agent/logs/`). The only shareable form is `calibration.py --export-sanitized` (free text removed). Never commit the raw log. | N/A — never leaves the machine except via the sanitized export. | `decision-sidecar/README.md`, `decision-sidecar/calibration.py` |
| Images on any channel | Text-only obfuscation cannot inspect secrets baked into image pixels — documented limitation. Oversized share blobs strip images first (`[image omitted from share]`). When in doubt, exclude images before sharing. | Exclude-before-share is the control. | `share.ts` `stripImagePayloads` |

Notes: redaction resolves the session's own project secrets (settings load
against the session cwd, not the invoking cwd). With `share.redactSecrets`
off, `secrets.enabled` off, or no secrets configured, the snapshot ships
unredacted. Raw local archives (`/export`, `/dump`) remain an explicit
option and must be treated as secret-bearing.

## F7 — Retrieval provenance surfacing contract (Stream-F docs slice 2026-10-03)

Status: surfacing contract DOCUMENTED below; unified "why was this
injected" UI stays OPEN. No behavior changed.

### What the model and user see today

- **Markdown brain:** the injected block is headed as reference data, not
  instructions ("Retrieved brain context … not new user instructions …
  Project knowledge applies to this project; user knowledge describes
  cross-project preferences"). Every page cites `scope` (`project`/`user`),
  `title`, `heading`, `Source: <filePath>`, and `Page: <id>`, and instructs
  reading the cited file before relying on omitted details
  (`src/prompts/brain/context.md`). Documents with
  `superseded: true` frontmatter are excluded from the index; `depends_on` /
  `relates_to` expand one hop with bounded fan-out while `contradicts` /
  `supersedes` links are recorded but never expand retrieval. Scope
  exclusion (`scopes` option, `brain.scopes`) is enforced before graph
  expansion and reranking, so excluded knowledge cannot leak back in through
  neighbors or model scores; rerankings are bound to their exact prompt
  revision (`BRAIN_RERANK_PROMPT_REVISION` in the cache key).
- **Mnemopi recall:** each recalled row renders as
  `- <content> (id: <id>) [<source>] (<date>) c:<score>`; recall previews
  clip content (`truncated`) and the full row is inspectable via
  `memory://<id>` (first hit wins across retain → recall → global banks).
  The `search` API returns `id`/`content`/`source`/`timestamp`/`score`;
  `stats`/`diagnose` report banks, counts, and integrity.

### Correction workflow (existing APIs — no second store)

- `memory_edit` `update` / `forget` / `invalidate` resolve in the same
  first-hit bank order as reads. Facts-table rows are read-only and report
  `not_editable` (never silent `not_found`); `update`/`forget` against
  non-`working` stores report `not_found` with bank/store context;
  `invalidate` accepts an optional `replacementId`.
- The latest correction wins: current user messages and tool output take
  precedence over recalled memories per the memory developer instructions,
  and superseded brain documents leave the index. Corrections go through
  these APIs — no parallel memory store is introduced.

### Scope controls

- `brain.rerank` / `brain.scopes` bound Markdown-brain injection;
  `mnemopi.*` scoping (`recallBanks`, project/global banks) and
  `memory.backend` bound the Mnemopi backend. Turning memory off or
  excluding user knowledge must explain both controls — they are distinct
  switches, not one flag.

### Still open

A unified per-injection surface (score/reason per page/fact, conflict
display, one-click correct/exclude) needs TUI design and must not be
half-implemented as ambient text.

## F9 — Release legs: automated vs manual gates (Stream-F docs slice 2026-10-03)

Status: DONE (gates) + manual gates DOCUMENTED below. No behavior changed.

### Automated (`publish.needs` in `.github/workflows/release.yml`)

- **Binaries (8 targets):** linux-x64/arm64, linux-musl-x64/arm64,
  darwin-x64/arm64, win32-x64/arm64. Each runs `--version` plus
  `--smoke-test` (worker-host probe) on its target host.
- **CPU sidecar smoke (Linux + Windows):** health identity, authenticated
  inference, token rotation, teardown, with the pinned checkpoint cached.
- **npm bundle (source-vs-npm):** pack → exactly one tarball carrying
  `dist/cli.js`, `dist/docs-index.generated.txt`, and the tool-views
  bundle; source-vs-packed version parity; bundled CLI boots and reports
  the release version.

### Manual (documented skips with exact commands)

- **macOS sidecar:** CPU torch-wheel availability on clean mac runners is
  unverified, so the sidecar matrix stays Linux + Windows while darwin
  binaries stay gated. Manual gate on a clean mac host (script is
  macOS-safe): `bash scripts/ci-sidecar-smoke.sh`
  (`LAYA_SMOKE_PORT`/`LAYA_SMOKE_DIR` as needed).
- **Full-offline CI:** runner network isolation is flaky and a cold model
  cache needs network, so offline is not gated. Cache-only operation is
  supported — warm HF cache plus
  `HF_HUB_OFFLINE=1 bash scripts/ci-sidecar-smoke.sh`. Absent and
  unreachable model ⇒ server stays unready and the TS client fails open
  (tool gating fails CLOSED to human approval).
- **Fresh-registry consumer install:** the `@harvest` scope is
  workspace-resolved, not published (registry 404), so a clean-registry
  install stays a manual gate, not an automated one.
- **Degraded environments (non-admin Windows, missing Python/native
  assets, unsupported accelerators):** the setup wizard heals only its 8
  enumerated modes and logs to `~/.harvest/agent/logs/laya-setup.log`;
  unrecognized failures emit a diagnostic bundle and fail open, and
  unsupported hardware degrades with a precise message (device-override
  fallback, calibration-derived effective settings) rather than a silent
  skip.
