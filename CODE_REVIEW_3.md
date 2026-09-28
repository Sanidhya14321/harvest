# Code Review Findings

Review date: 2026-09-28

Scope: delta review vs `CODE_REVIEW.md` (2026-09-23) and `CODE_REVIEW_2.md` (2026-09-24), per `CODE_REVIEW_PROMPT.md`. Six parallel harness reviews against current `main`. Only still-valid and new findings are filed below; fixed items are listed once under "Already fixed" so they are not re-filed.

## Already fixed — do not re-file

- CORS wildcard narrowed (sidecar emits no CORS headers; dead import only).
- `LAYA_HOST` / `LAYA_LOG_DIR` / `--decisions` / `--out` drift fixed; calibration import/load wired.
- OAuth `</script>` XSS not vulnerable (`callback-server.ts:592-595` escapes `<>&`; `oauth.html:285-299` uses `JSON.parse(textContent)` + `textContent` render).
- Devin gzip cap enforced (`devin.ts:84-86,248-253,259,326` via `maxOutputLength`).
- Bedrock event-stream cap enforced (`aws-eventstream.ts:27,165-175` + CRC/length checks).
- Ollama `NaN`/negative `contextWindow` rejected (`ollama.ts:86-92`, fallback `128000`).
- omptype `oneOf` throws (`from-json-schema.ts:91-93`); `required`-without-`properties` synthesized (`:188-192`); `__proto__` uses `Object.defineProperty` (`compile.ts:117-122,644-650,917`, `interp.ts:44-50`).
- Activity `slice(-0)` early-returns `[]` (`activity/index.ts:284-286,314-316`).
- Audio queues bounded (`pi-voice/src/audio.rs:44-48,135-160,176` `flume::bounded(256)`; `pi-natives/src/audio.rs:24,48-55` 64-chunk drop counter).
- Workspace walker bounded (`workspace.rs:104,218-231`).
- `browser-page.ts` 4 MiB streaming cap; `runner-cache.ts` 0700 + symlink refusal; `run-experiment.ts` 10 MiB cap + tail re-read; `checkers.ts` 1 MiB per-stream cap.
- Stats `console.*` only in standalone CLI (`stats/src/index.ts`), not server path (allowed).
- Metaharness default bind is `127.0.0.1` (`server.ts:82-93,209`), not `0.0.0.0`.
- Laya TS type claims refuted: `tsgo --noEmit` clean. `AgentMessage` imported from `@harvest/pi-agent-core` (`laya-pruning.ts:22`); `signal` present in `LayaPruningOptions` (`:78`) and forwarded (`:461-466`); `bundle` identifiers are `DiagnosticBundle`; `batchDecide`/`isAvailable` do not exist; `pruningTokenBudget` read at `laya-pruning.ts:510-515`; allowlist filtered at `structured-subagent.ts:280-287` with fallback `:301-313`; case restored via `nameMap`; tool-call pairing preserved (`:604-610,641-647`).

## Findings

### [P0] Sidecar `/v1/decide` unauthenticated with log poisoning and disk-fill

**Locations:** [`decision-sidecar/server.py`](decision-sidecar/server.py#L149-L154), [`decision-sidecar/server.py`](decision-sidecar/server.py#L106-L113), [`decision-sidecar/server.py`](decision-sidecar/server.py#L365-L382)

**Description:**
No auth/token/allowlist on `/v1/decide`. Every request appends `str(state)[:200]` plus instructions verbatim to `decisions.jsonl` with no size cap or rotation. Log already stores hostile payloads (`rm -rf /`, `chmod 777 /var/run/docker.sock`).

**Impact:**
Any loopback process, or a simple-POST from a browser (no preflight for `text/plain`), drives inference and poisons the future calibration corpus, and can fill disk.

**Reproduction / Code Proof:**
```python
# server.py:198 — no dependencies, no auth
@app.post("/v1/decide", response_model=DecideResponse)
async def decide(req: DecideRequest) -> DecideResponse:
...
with open(LOG_FILE_PATH, "a", encoding="utf-8") as f:  # no rotation
    f.write(json.dumps(record, ensure_ascii=False) + "\n")
```

**Recommendation & Fix:**
Require a per-process secret header, enforce a request-body cap, write logs with rotation and sanitize logged fields.

### [P0] English gate fail-open on dict/list input and on exception

**Locations:** [`decision-sidecar/server.py`](decision-sidecar/server.py#L219-L232)

**Description:**
Gate passes `Union[str,dict,list]` directly to `laya.is_english()`. On exception it only `logger.debug`s and falls through to inference. Per-question dict states are never checked per chunk.

**Impact:**
Non-English `tool_gating` inputs receive hallucinated gate verdicts on a safety-critical path.

**Reproduction / Code Proof:**
```python
try:
    if not laya.is_english(state):  # state: str|dict|list
        ...return DecideResponse(..., non_english=True)
except Exception as e:
    logger.debug(f"Language detection check failed: {e}")  # fail-open
```

**Recommendation & Fix:**
Normalize `str(state)` per question chunk before detection; fail closed for gating on detection error.

### [P1] Shipped calibration params are pure synthetic; real logs can never calibrate

**Locations:** [`decision-sidecar/server.py`](decision-sidecar/server.py#L380), [`decision-sidecar/calibration.py`](decision-sidecar/calibration.py#L247-L262), [`decision-sidecar/calibration.py`](decision-sidecar/calibration.py#L187-L190)

**Description:**
Server hardcodes `"ground_truth": None`; calibrator keeps only `rec.get("ground_truth") is not None`. Shipped `calibration_params.json` is exactly 150x3 rows from `generate_synthetic_calibration_dataset(count_per_site=150)` (seed 42). The `--synthetic` branch calls `save()` on the same `--out` with no backup/marker, so one run destroys real params.

**Impact:**
Live confidences (e.g. tool_gating T=2.6386) are scaled by lab-random temperatures, reported as measured.

**Reproduction / Code Proof:**
```python
"ground_truth": None,  # server.py:380
if rec.get("ground_truth") is not None: records.append(rec)  # calibration.py:254
```

**Recommendation & Fix:**
Log real outcomes, keep synthetic bootstrap in a separate output, require `--force` plus provenance marker before clobbering production params.

### [P1] `assertPathJailed` TOCTOU — `resolvedPath` discarded, lexical path re-opened

**Locations:** [`packages/coding-agent/src/core/harvest/security.ts`](packages/coding-agent/src/core/harvest/security.ts#L152-L221), [`packages/coding-agent/src/tools/write.ts`](packages/coding-agent/src/tools/write.ts#L1288-L1294), [`packages/coding-agent/src/edit/index.ts`](packages/coding-agent/src/edit/index.ts#L577-L592), [`packages/coding-agent/src/lsp/edits.ts`](packages/coding-agent/src/lsp/edits.ts#L329-L351)

**Description:**
Check snapshots via `realpathSync` and returns `{jailed, resolvedPath}`. Callers ignore `resolvedPath` and re-open the lexical path (`Bun.write`, `writeFileWithFallback`, `applyTextEdits` double-open). No `O_NOFOLLOW`, no fd pinning.

**Impact:**
File overwrite/delete outside workspace via ancestor swap between check and use.

**Recommendation & Fix:**
Use `jailCheck.resolvedPath`, or open with `O_NOFOLLOW|O_CREAT|O_EXCL` + atomic rename; re-validate after open.

### [P1] `write` archive/SQLite paths skip jail; symlink archive overwrite

**Locations:** [`packages/coding-agent/src/tools/write.ts`](packages/coding-agent/src/tools/write.ts#L1241-L1255), [`packages/coding-agent/src/tools/write.ts`](packages/coding-agent/src/tools/write.ts#L1268-L1284), [`packages/coding-agent/src/tools/write.ts`](packages/coding-agent/src/tools/write.ts#L653-L655)

**Description:**
Archive and `db:table:key` paths enforce plan mode only, never call `assertPathJailed`. Archive path `realpath`s an existing file then `writeArchive(tmp)+rename(tmp,final)` — `ws/link.zip -> /etc/critical` is followed and overwritten. `archiveSubPath` blocks `..` but the outer path is unchecked.

**Impact:**
Arbitrary file overwrite (archive rename) and arbitrary SQLite open+mutate.

**Recommendation & Fix:**
`assertPathJailed` the resolved `absolutePath`/`sqlitePath` before dispatch, same as the regular-file path at `:1291`.

### [P1] `pi-edit` `resolve()` is lexical-only; reads bypass jail

**Locations:** [`crates/pi-edit/src/path_policy.rs`](crates/pi-edit/src/path_policy.rs#L46-L87), [`crates/pi-edit/src/files.rs`](crates/pi-edit/src/files.rs#L115-L129), [`crates/pi-edit/src/session.rs`](crates/pi-edit/src/session.rs#L229-L241)

**Description:**
Plain paths only `lexical_normalize(cwd.join)`; absolute `/etc/passwd` and `~/...` pass through. `local://` rejects only literal `..`, never canonicalizes symlinks. `FileCache::read_resolved` does `std::fs::read` with no containment; `Session::apply` stages+reads before plan-mode `enforce_write`. TS `#write` is the only jail.

**Impact:**
Arbitrary file read into preview/diff (`edit({path:"/etc/passwd"})`, `local://x` where `<sandbox>/x -> /etc/passwd`). Write still blocked; read leaks.

**Recommendation & Fix:**
Enforce containment in `PathPolicy::resolve` / `read_resolved` (canonicalize + `is_within`), or pre-check in `Session::apply` before staging.

### [P1] `pi-iso` walk follows symlink dirs; `pi-vcs` in-repo redirect and read-side follow

**Locations:** [`crates/pi-iso/src/diff.rs`](crates/pi-iso/src/diff.rs#L328-L335), [`crates/pi-vcs/src/git/patch.rs`](crates/pi-vcs/src/git/patch.rs#L1500-L1539), [`crates/pi-vcs/src/git/patch.rs`](crates/pi-vcs/src/git/patch.rs#L1541-L1575)

**Description:**
`walk()` calls `entry.metadata()` (follows links) then tests `meta.is_symlink()` — dead check. Symlink dirs are recursed, escaping roots. Patch validator rejects only outside-root parent/target links; in-repo `a -> .git/` passes, then `create_dir_all + fs::write` follow it. Read helpers (`read_worktree_entry`, `tracked_worktree_map`) join `root/path` with only a `..` check.

**Impact:**
Outside file bytes into diff text; crafted repo + patch corrupts unintended in-repo locations including `.git/objects`.

**Reproduction / Code Proof:**
```rust
let meta = entry.metadata() // follows links
if meta.is_symlink() { ... } // unreachable
```

**Recommendation & Fix:**
Use `symlink_metadata()`, no-follow opens, directory-relative operations, re-check after open.

### [P1] Workpool batch stuck `running` when `register` throws

**Locations:** [`packages/coding-agent/src/task/workpool.ts`](packages/coding-agent/src/task/workpool.ts#L312-L339), [`packages/coding-agent/src/task/workpool.ts`](packages/coding-agent/src/task/workpool.ts#L349-L423), [`packages/coding-agent/src/async/job-manager.ts`](packages/coding-agent/src/async/job-manager.ts#L311-L315)

**Description:**
`#drain` flips items to `running` then calls `#startTurn -> manager.register`, which throws `Background job limit reached` when over cap. No try/catch; `#finishTurn` never runs. `#queueDispatch` catch only fails `queued` items. `#isDrained` never true, `#waitForDrain` hangs. Trigger is real: pool `task.maxConcurrency` and job `maxRunningJobs` (15) are independent gates.

**Impact:**
Pool aggregate job hangs until externally aborted; batch rows lie in `hub jobs`.

**Recommendation & Fix:**
Wrap `#startTurn` in try/catch; on register error revert items to `queued` and fail them, or admit-before-mark ordering.

### [P1] Metaharness `jobName` traversal on launch, zero auth, unguarded `--host`

**Locations:** [`packages/metaharness/src/server.ts`](packages/metaharness/src/server.ts#L392-L399), [`packages/metaharness/src/server.ts`](packages/metaharness/src/server.ts#L484-L517), [`packages/metaharness/src/server.ts`](packages/metaharness/src/server.ts#L766-L769)

**Description:**
`launch()` joins `request.jobName` into `jobsDir`/`logDir` with no `assertSafeJobName()` (only `#destroyRun` has it). `#route()` has no auth on launch/cancel/delete. `--host <anything>` honored with no non-loopback guard.

**Impact:**
Reachable client gets workload execution as server user, PID signaling, recursive delete, and path escape.

**Recommendation & Fix:**
Validate `jobName` on launch, keep `127.0.0.1` default, require token for mutating routes and for non-loopback bind.

### [P2] Batch/token amplification despite caps; semaphore oversubscription

**Locations:** [`decision-sidecar/server.py`](decision-sidecar/server.py#L79-L80), [`decision-sidecar/server.py`](decision-sidecar/server.py#L91-L95), [`decision-sidecar/server.py`](decision-sidecar/server.py#L207-L217)

**Description:**
`MAX_QUESTIONS=64` and `MAX_STATE_CHARS=500_000` bound count/chars, not tokens/compute. Batched path builds up to 64x1024-token sequences; `total_tokens` counted never capped; question keys/instructions depth unbounded; `Semaphore(2)` x `torch.set_num_threads(cpu_count)` oversubscribes. Log shows 37-61s latencies.

**Impact:**
Single request ~64 ModernBERT-large forwards; concurrent batches OOM or break the 300ms gating SLO into fail-closed storms.

**Recommendation & Fix:**
Aggregate token cap, per-question token cap, divide threads by concurrency, inference timeout, bounded inference semaphore.

### [P2] Subagent approval forced `yolo` with parent creds and live MCP

**Locations:** [`packages/coding-agent/src/task/executor.ts`](packages/coding-agent/src/task/executor.ts#L974-L978), [`packages/coding-agent/src/task/executor.ts`](packages/coding-agent/src/task/executor.ts#L885-L939)

**Description:**
Children run with `"tools.approvalMode": "yolo"` regardless of parent mode, delegating to parent live MCP connections and `getApiKey`. Comment claims parent approval is the boundary — one click for an entire unattended subtree over untrusted tool results.

**Impact:**
Privilege downgrade by design; prompt-injected child drives side effects with no confirmation.

**Recommendation & Fix:**
Inherit approval mode; scope `yolo` to read-only agents or document the threat-model exception.

### [P2] Cancel eviction frees slot before settle; filtered delivery bypasses cap

**Locations:** [`packages/coding-agent/src/async/job-manager.ts`](packages/coding-agent/src/async/job-manager.ts#L397-L406), [`packages/coding-agent/src/async/job-manager.ts`](packages/coding-agent/src/async/job-manager.ts#L896-L911), [`packages/coding-agent/src/async/job-manager.ts`](packages/coding-agent/src/async/job-manager.ts#L936-L968), [`packages/coding-agent/src/async/job-manager.ts`](packages/coding-agent/src/async/job-manager.ts#L1074-L1115)

**Description:**
Slot accounting keeps cancelled-but-unsettled jobs counted, but `cancel()` also arms `#scheduleEviction` and `#evictJob` deletes unconditionally. If the timer fires first (guaranteed at `retentionMs=0`), the slot frees early and late settle writes to a detached object, dropping `structured` for the sink. `drainDeliveries({filter})` bypasses the `>=16` loop gate; timeout only rejects the race while the sink promise keeps running; retries have no max attempts.

**Impact:**
Concurrency over-admission after cancel; unbounded hidden sink concurrency; `hub wait` spin until retention eviction.

**Recommendation & Fix:**
Track settlement independently; keep unsettled jobs in capacity until promise completes. Gate the filtered delivery path, add delivery deadline that cancels sink work, cap retry attempts.

### [P2] Abort cannot preempt hung hooks/tools; follow-up turns skip preflight

**Locations:** [`packages/agent/src/agent-loop.ts`](packages/agent/src/agent-loop.ts#L1604), [`packages/agent/src/agent-loop.ts`](packages/agent/src/agent-loop.ts#L2868-L2874), [`packages/coding-agent/src/task/executor.ts`](packages/coding-agent/src/task/executor.ts#L2825-L2904), [`packages/coding-agent/src/task/executor.ts`](packages/coding-agent/src/task/executor.ts#L2537-L2656)

**Description:**
`convertToLlm`/hook phases awaited with no signal/timeout; a hook ignoring its signal blocks the loop. Non-interruptible tools block unwind. Fresh spawns pass `assertDepthAndSpawnAllowed` but revived/continued turns never re-check; protection rests on spawn-time `task` strip. Tool rejections themselves are contained (no deadlock).

**Impact:**
`^C` hangs until hung hook/tool returns; re-enabled `task` post-spawn allows unchecked grandchildren.

**Recommendation & Fix:**
Abort-race plus timeout on hook/convert phases; re-check depth/spawn/blockedAgent on follow-up and IRC-wake turns.

### [P2] vterm CSI repeat-count DoS (length capped, count not)

**Locations:** [`packages/utils/src/vterm/terminal.ts`](packages/utils/src/vterm/terminal.ts#L334-L339), [`packages/utils/src/vterm/terminal.ts`](packages/utils/src/vterm/terminal.ts#L522-L542)

**Description:**
Sequence length capped at 512, but `ESC[<n>S/T` loops `scrollUp/Down` `n` times and `ESC[<n>@/P/X` does `Array.from({length:count})`. `#insertLines/#deleteLines` clamp with `Math.min`; these paths do not.

**Impact:**
Untrusted program output OOMs or hangs the TUI (`ESC[99999999S`, `ESC[10000000@`).

**Reproduction / Code Proof:**
```ts
term.write("\x1b[50000000S");
term.write("\x1b[10000000@");
```

**Recommendation & Fix:**
Clamp `amount` to `rows*4` / `cols*4` matching existing insert/delete clamps.

### [P2] Auth-gateway wildcard CORS with loopback `--no-auth`; vmnet forward oracle

**Locations:** [`packages/ai/src/auth-gateway/http.ts`](packages/ai/src/auth-gateway/http.ts#L223-L240), [`packages/metaharness/src/runner.ts`](packages/metaharness/src/runner.ts#L1271-L1305), [`packages/ai/src/auth-gateway/server.ts`](packages/ai/src/auth-gateway/server.ts#L847-L853)

**Description:**
Empty-token open is correctly loopback-guarded, but gateway answers every response with `Access-Control-Allow-Origin: *`. Under the documented bench default (`--no-auth` loopback) any visited website can `fetch(http://127.0.0.1:4000/...)` and spend broker quota. Runner forwards to `192.168.64.1` with no token check for bench containers.

**Impact:**
Browser-driven credential spend; any vmnet peer consumes gateway quota.

**Recommendation & Fix:**
Drop `*` CORS when unauthenticated, or require token for non-`host.docker.internal` peers.

### [P2] Laya daemon spawn unguarded with PID discarded and 180s blind poll; subagent `signal` dropped

**Locations:** [`packages/coding-agent/src/core/harvest/laya-service.ts`](packages/coding-agent/src/core/harvest/laya-service.ts#L810-L841), [`packages/coding-agent/src/core/harvest/laya-subagent-selection.ts`](packages/coding-agent/src/core/harvest/laya-subagent-selection.ts#L44-L56), [`packages/coding-agent/src/task/structured-subagent.ts`](packages/coding-agent/src/task/structured-subagent.ts#L293-L299)

**Description:**
`Bun.spawn(detached, stdio ignore)` handle discarded, no PID file or log capture. Readiness poll burns up to 180s even if the child exited instantly. Two concurrent setups double-spawn; `resolvePortConflict` drifts 8178-8185. Pruning forwards `signal` to `client.decide`; subagent selection options lack `signal` and the `structured-subagent.ts:131` signal is dropped at `:293-299`.

**Impact:**
Orphan daemons, port drift, slow opaque startup; cancelled turns block to timeout (up to `SANITY_TIMEOUT_CEILING_MS` 15000) before fail-open.

**Recommendation & Fix:**
PID file + singleflight + kill-on-failed-probe + log capture; propagate parent `signal` through `selectSubagentWithLaya` to `decide`.

### [P2] Session breadcrumb and mnemopi aux paths use ambient perms; `writeText` transient 0644

**Locations:** [`packages/coding-agent/src/session/session-paths.ts`](packages/coding-agent/src/session/session-paths.ts#L231-L232), [`packages/coding-agent/src/session/session-storage.ts`](packages/coding-agent/src/session/session-storage.ts#L317-L322), [`packages/mnemopi/src/core/banks.ts`](packages/mnemopi/src/core/banks.ts#L30-L37), [`packages/mnemopi/src/cli.ts`](packages/mnemopi/src/cli.ts#L136-L138)

**Description:**
`db.ts:82,85,98` is correct (0700/0600 with re-chmod), but breadcrumb dir/file, bank/blob/cache/export dirs, JSON memory exports, and `Bun.write(path,{createPath:true})` (chmod applied after) inherit umask (`0755`/`0644`).

**Impact:**
Local disclosure of session paths, memory excerpts, backups; transient world-readable transcript window on crash/reader race.

**Recommendation & Fix:**
`mode:0o700/0o600` at create plus reuse of `enforcePrivateDir/File`; use the `writeTextSync`/`writeTextAtomic` 0600-at-create path.

### [P3] Residual docs/CLI drift, bucketing gating, and hygiene nits

**Locations:** [`decision-sidecar/server.py`](decision-sidecar/server.py#L411-L414), [`decision-sidecar/test_bucketing.py`](decision-sidecar/test_bucketing.py#L106-L219), [`packages/snapcompact/src/snapcompact.ts`](packages/snapcompact/src/snapcompact.ts#L1743), [`packages/omptype/src/compile.ts`](packages/omptype/src/compile.ts#L226-L227), [`packages/coding-agent/src/memories/index.ts`](packages/coding-agent/src/memories/index.ts#L1439-L1443)

**Description:**
`LAYA_DEVICE` and `--host/--port` documented but unread/positional. `test_bucketing.py` has real `TestCase` coverage for shapes but the numerical-equivalence benchmark copies server math instead of importing it and runs only under `--benchmark`/`RUN_LAYA_BENCHMARK`. `preserveData` and omptype optional-prop checks use `in` instead of `Object.hasOwn` (shape guard at `snapcompact.ts:202` is correct). `encodeProjectPath` uses 64-bit non-crypto `Bun.hash` plus lossy `[/\\:] -> -` folding (`/a/b:c` vs `/a/b/c` collide).

**Impact:**
GPU override silently ignored; bucketing drift undetected; negligible-until-polluted `in` edges; cross-project memory mixing on hash/path collision.

**Recommendation & Fix:**
Implement or undocument the env/CLI flags; import the production bucketing helper into a `TestCase` and gate equivalence in CI; switch to `Object.hasOwn`; use escaped path plus stable hash with migration.

## Scope and verification

- Compared against `CODE_REVIEW.md` and `CODE_REVIEW_2.md`; re-verified each cited location on current `main`.
- `bunx tsgo --noEmit` clean for the Laya TS surface; `oxlint` reports only dead imports on touched paths.
- Python checks are static plus log/behavior evidence (`decisions.jsonl` latencies and payloads); no checkpoint download or daemon spawn was performed for this delta.
- Severity: P0 = active workspace escape / RCE / credential spend / safety-gate bypass; P1 = unauth control, jail bypass, protocol corruption, typecheck failure; P2 = OOM, slot leak, ignored setting, synthetic contamination; P3 = docs drift, hygiene, unreachable-path uniformity.
