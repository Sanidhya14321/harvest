# Audit Progress Tracker — 2026-09-30 Handoff (48 items: 38 P/U + 10 F)
Branch: `codex/session-manager-laya-review` | HEAD: `db336df` | Updated: 2026-10-03

Source: `CODEBASE_AUDIT_HANDOFF_2026-09-30.md` Third-pass table (lines 15-68). This file supersedes older counts.
`AUDIT_REPORT.md` Batch 1-4 ledgers are historical context only.

## Tally
- Fixed: 37/38 P/U fully (incl. cheap slices P0-1/P2-4 whose full validation stays gated) + P0-2 spike (stuck observability DONE, killable-process switch deferred by design) — all 38 accounted; P0-2 is the 1/38 not fully closed
- Partial (needs human/machine traces, not code): P0-1 dataset, P0-2 process model, P2-4 model-backed equivalence — all documented with Gates
- F proposals: 3/10 DONE (F2,F5,F9), 1/10 PARTIAL (F8), 6/10 OPEN by design (F1,F3,F4,F6,F7,F10) per `docs/product-decisions.md`; no Stream-F code slices remain
- Dirty tree at tracker creation: `CODEBASE_AUDIT_HANDOFF_2026-09-30.md`, `CHANGELOG.md`, `brain.ts` (2-line log), `ci-sidecar-smoke.sh`, `docs/product-decisions.md` (untracked)

## Fixed (37+spike) — verify, don't regress
P0-3, P1-1, P1-2, P1-3, P1-4, P1-5, P1-6, P1-7, P1-8, P1-9, P1-10, P1-11, P2-1, P2-2, P2-3, P2-4 (cheap slice), P2-5, P2-6, P2-7, P2-8, P2-9, P2-10, U1, U2, U3, U4, U5, U6, U7, U8, U9, U10, U11, U12, U13, U14, P0-1 (cheap slice: provisional docs, no safety claim)
+ P0-2 SPIKE (stuck observability + restart advisory; process switch deferred by design)

## Partial → work assigned
| ID | Gap per handoff | Owner | Status |
|----|-----------------|-------|--------|
| P0-1 | shipped params synthetic, thresholds hard-coded `laya-gating.ts:47` | Stream-B (calibration) | DONE (cheap slice: thresholds documented PROVISIONAL in code, values unchanged — no measured-safety claim; server temps confirmed not wired to TS rule; labeled coding-tool dataset + held-out bar still needs human traces) |
| P0-2 | worker still `to_thread`, no killable process | Stream-E (scheduler) | SPIKE-DONE 2026-10-03: production stays `to_thread`; `LAYA_WORKER_MODEL=process` rejected with rationale, `snapshot()`+`/health.scheduler` now expose `oldest_in_flight_ms`/`stuck`/`restart_advisory` (`LAYA_STUCK_THRESHOLD_S`, default 120s), README restart procedure; 53/53 `test_inference_scheduler.py`+`test_sidecar.py` pass, fail-open preserved |
| P0-3 | sole caller dispose `agent-session.ts:4765`, no rewind/compact invalidation | Stream-C (pruning lifecycle) | DONE — prompt-rev in lock key + `invalidatePruningLocksOnHistoryRewrite` on rewind/compact/restore |
| P1-3 | `/laya status` misreports defaults as overrides `laya-cli.ts:126-154` | Stream-B | DONE (`getExplicitSetting` + exported `resolveLayaStatusSettings` precedence explicit→calibrated→schema-default; `laya-cli.ts` + `builtin-laya.ts` `/laya status` both fixed; `test/laya-status.test.ts` pins contract) |
| P2-3 | `decisions.jsonl` still tracked in git | Stream-A (hygiene/release) | DONE 2026-10-03: `git rm --cached` (seed kept as local user data, preserved on disk); `.gitignore` covers file + rotations (verified via `check-ignore`); `server.py` + `calibration.py` `default_log_path` already user-data (`~/.harvest/...`, existing `test_default_log_path_is_user_data_not_repo` — no duplicate); README "frozen artifact" sentence fixed in consistency pass (no decision log tracked; user-data default + gitignore stated) |
| P2-4 | equivalence test opt-in only | Stream-B | DONE (cheap slice: verified `test_bucketing.py` uses production `bucket_items_by_length`, startup probe covered in `test_sidecar.py`; model-backed equivalence stays opt-in behind `RUN_LAYA_BENCHMARK` — needs 421M checkpoint + human GPU/CPU traces, not runnable in unit CI) |
| P2-5 | subagent/calibration questions still inline | Stream-B | DONE (new `prompts/laya/subagent-selection.md`, `subagent-criteria.md`, `calibration-sanity-check.md` + leaf `laya-prompt-assets.ts` with strict throwing parsers; `BENCHMARK_PAYLOADS` reuses production assets; `test/laya-prompt-assets.test.ts` pins asset↔production equivalence; `laya-pruning.ts` untouched — Stream-C active there) |
| P2-10 | no source-vs-npm/macOS-sidecar/offline legs | Stream-A | DONE 2026-10-03: `npm-bundle` gate job (pack → tarball contents/version-parity/booted-CLI checks) wired into `publish.needs`, verified end-to-end locally (`18.1.14`, CLI `harvest/18.1.14`); macOS sidecar + full-offline CI documented skips with manual gates (CPU wheel unverified; cold cache needs network); smoke script macOS-safe (`seq`→brace); `@harvest` scope unpublished (registry 404) so fresh-registry install stays manual |
| U2 | no idempotency keys, no read/write split | Stream-D (MCP/RPC) | DONE 2026-10-03: per-call key (`_toolCallId`) sent via `params._meta` + HTTP `Idempotency-Key`; server-echoed key on errors gates write replay; `readOnlyHint` reads may retry once; writes without a server key keep `outcomeUnknown` |
| U5 | no run/generation binding | Stream-D | DONE 2026-10-03: `RpcRunBinding` epochs; `abort.runGeneration?` + `get_state.runGeneration`; bound abort for a superseded run answered without touching the live run; unbound aborts unchanged |
| U7 | no stdin backpressure | Stream-D | DONE 2026-10-03: `readRpcInputFramesWithBackpressure` pauses stdin pulls while the serial queue is full (`queuedSerialCount`/`awaitQueueBelow`), with pause/resume accounting + shutdown debug log; `packages/utils` untouched |
| U12 | no shared prune+rerank budget, no prompt-rev in key | Stream-C | DONE — `LayaContextBudget` prune→rerank remainder via `sdk.ts`, prompt-rev in prune lock + rerank cache key |
| F2,F5,F9 | DONE per `docs/product-decisions.md` (tier-first gating; U2 outcomeUnknown; release/portability gates with documented macOS+offline+registry skips) + Stream-F slices 2026-10-03 (F2 `mcp-approval-policy.ts` per-server override fail-closed; F5 `mcp-outcome-ledger.ts` last-100 ledger, no replay change; contract tests `mcp-approval-policy` + `mcp-outcome-ledger`) | Streams B/D/A + Stream-F | DONE (incl. Stream-F docs slice 2026-10-03: F9 automated-vs-manual legs in `docs/product-decisions.md` F9 section + sidecar README §5 — docs only, no behavior change) |
| F8 | PARTIAL per `docs/product-decisions.md` (`/diagnostics` run stage + per-transform measurements; unified provenance view still open) | — | OPEN (unified view needs UX design) |
| F1,F3,F4,F6,F7,F10 | OPEN by design — needs UX/product, do not half-implement (F4/F7 have per-channel seams but no unified preview/provenance UI) | — | OPEN (documented; Stream-F docs slice 2026-10-03: F4 per-channel matrix + F7 provenance contract in `docs/product-decisions.md`, sidecar README §§5–6, `docs/decision-layer.md` §6 — docs only, no behavior flips, preview/provenance UIs stay open) |

## Streams (parallel, non-overlapping files)
- Stream-A: `decision-sidecar/decisions.jsonl` untrack + `.gitignore`, `scripts/ci-sidecar-smoke.sh`, `.github/workflows/release.yml`, `docs/product-decisions.md` F9 bit
- Stream-B: `decision-sidecar/calibration.py`, `packages/coding-agent/src/core/harvest/laya-gating.ts`, `packages/coding-agent/src/**/laya-cli.ts`, prompts for subagent/calibration
- Stream-C: `packages/coding-agent/src/core/harvest/laya-pruning.ts`, `packages/coding-agent/src/session/agent-session.ts` (pruning hooks only), `packages/coding-agent/src/core/harvest/brain.ts` (budget/key only)
- Stream-D: `packages/coding-agent/src/mcp/tool-bridge.ts`, `packages/coding-agent/src/modes/rpc/*`, `packages/utils/src/stream.ts` (stdin/backpressure only)
- Stream-E: `decision-sidecar/inference_scheduler.py`, `decision-sidecar/server.py` (process model spike only, fail-open)
- Stream-F: F docs only (statuses recorded in `docs/product-decisions.md`), no behavior flips

Rules for all streams: never commit; reproduce consumer-visible effect first; contract tests not source-grep; `bun --cwd=packages/coding-agent run check:types`; focused `bun test <path>`; preserve `decisions.jsonl` user data; no GitHub comments/issues.

## Verification log (Streams A-E regression battery, read-only)
- 2026-10-03 — `bun --cwd=packages/coding-agent run check:types`: PASS (tsgo --noEmit, 0 errors).
- 2026-10-03 — `bun test laya-pruning + markdown-brain + laya-status + laya-prompt-assets + mcp-tool-delivery + rpc-input-frame --timeout 120000`: PASS — 101 pass, 0 fail (401 expects, 6 files).
- 2026-10-03 — `python -m pytest decision-sidecar/test_sidecar.py decision-sidecar/test_inference_scheduler.py -q`: PASS — 53 passed, 3 pre-existing Starlette deprecation warnings (`test_sidecar.py:177,190,202` HTTP_413 constant rename, non-blocking).
- 2026-10-03 — `git status --porcelain`: no commits made (HEAD still `db336df`); working-tree M = Streams A-E files only; `decision-sidecar/decisions.jsonl` staged `D` = intended P2-3 `git rm --cached`, preserved on disk (240633 bytes, 460 lines, Test-Path True), `.gitignore:64` covers it (`check-ignore` confirms). No user data loss. No regressions.
