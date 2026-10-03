# Laya Local Decision Layer

Harvest integrates [Laya](https://github.com/convaiinnovations/laya) and the fine-tuned ModernBERT checkpoint [`convaiinnovations/laya-typed-decisions`](https://huggingface.co/convaiinnovations/laya-typed-decisions) as a fast, local, zero-API-cost typed decision layer. It handles narrow deterministic classifications directly on the developer's machine without polluting the primary LLM reasoning stream or incurring remote latency.

---

## 1. Architectural Role & Decision Points

The decision layer operates at two wired interception points in the agent loop, plus one available helper:

```
                                  [Agent Loop Turn]
                                          │
    ┌─────────────────────────────────────┼─────────────────────────────────────┐
    │                                     │                                     │
    ▼                                     ▼                                     ▼
[1. Tool Gating]              [2. Model Routing — helper]          [3. Completion Check]
• High-risk tools:            • NOT in the turn loop;              • Unexpected-stop only
  bash, write, edit, ast-edit   opt-in helper, fail-open             • Question: noul (premature stop?)
• Question: noul (irreversible?)   • Question: choice (tier/role)     • See §1.3 for the live contract
• Timeout: ~300ms                  • Timeout: ~300ms                  • Timeout: ~300ms
• Contract: FAIL CLOSED            • Contract: FAIL OPEN              • Contract: FAIL OPEN
  (requires human approval)          (falls back to default tier)       (falls back to full LLM)
```

### 1.1 Tool-Call Gating (`laya-gating.ts`)
Before dispatching any mutation command from the high-risk tool set (`bash`, `exec`, `write`, `edit`, `ast-edit`, `patch`), Harvest constructs a `noul` query with the versioned instructions in `packages/coding-agent/src/prompts/laya/tool-gating.md`.

- If the computed irreversibility score exceeds `0.35`, or model confidence is below `0.75`, the call is tagged with `providerMetadata.layaGatingRequired = true`.
- Harvest's tool wrapper immediately stops execution and prompts the user for manual confirmation in the TUI.
- **Fail-Closed Guarantee**: If the local sidecar service is offline, unresponsive, or times out (300ms), the tool call fails **CLOSED** and prompts the user for confirmation. Read-only tools (`read`, `grep`, `glob`) bypass gating checks with zero overhead.

### 1.2 Model Tier Routing (`laya-routing.ts`)
Status: available as tested, fail-open helpers — NOT wired into automatic
per-turn model selection. Harvest never silently switches your explicitly
chosen model; routing stays out of the loop until a product-approved
single selection boundary lands with user-model precedence proven.
When invoked directly, user prompts are evaluated against the available model tiers
(tier descriptions in `packages/coding-agent/src/prompts/laya/model-routing-criteria.md`):
- `smol`: Simple syntax fixes, localized queries, single-file edits.
- `slow`: Multi-file architecture, deep algorithmic reasoning, complex refactoring.
- `default`: General software engineering workflows.

- Uses a `choice` question to classify the prompt in ~300ms (the client default decision budget).
- **Fail-Open Guarantee**: If the sidecar is unreachable or confidence is low, routing immediately falls **OPEN** to the user's configured default model tier.

### 1.3 Step & Task Completion Evaluation (`laya-completion.ts`)
Classifies unexpected assistant stops; it does not replace general
step-result evaluation. In production only the `unexpected_stop` question
(`packages/coding-agent/src/prompts/laya/completion-unexpected-stop.md`) is
sent, with the turn's abort signal propagated — the `step_success` question
(`completion-step-success.md`) exists for the general step-evaluation
contract, which has no production caller yet.
- Evaluates the terminal message text using a single `noul` query.
- **Fail-Open Guarantee**: If unclassified or low-confidence, falls **OPEN** to standard full-LLM evaluation.

---

## 2. Guardrails & Technical Invariants

1. **Strict Single-Model Checkpoint**:
   - The sidecar loads `convaiinnovations/laya-typed-decisions` directly via `laya.load(...)`.
   - It explicitly forbids constructing a `Router` instance, ensuring secondary checkpoints (such as `laya-multilingual`) are never downloaded to the developer's machine.
2. **Upstream Language Filtering**:
   - Upstream client checks detect non-English characters before dispatch.
   - Non-English prompts and tool arguments immediately bypass Laya and fall back to the primary LLM, preserving checkpoint accuracy.
3. **Localhost Isolation**:
   - The sidecar binds strictly to `127.0.0.1:8177`. No external network interfaces are exposed.
4. **Preserved Cloud Fallbacks**:
   - The primary cloud LLM is never removed from any call site. Laya acts purely as an acceleration and safety layer.
5. **Circuit Breaker & Decision Statistics** (`laya-circuit.ts`):
   - After 5 consecutive sidecar failures, optional decision points (pruning, reranking, subagent selection, routing, completion) fail fast for a 30s cooldown instead of spending more timed-out round trips.
   - Tool gating always attempts: its fallback requires human approval, so the breaker can never turn an unavailable sidecar into permission.
   - Per-call-site calls, fallbacks, timeouts, and p50/p95 latencies are queryable; detail already flows into the decision logs.

---

## 3. Interactive Setup Wizard Integration

Harvest's onboarding wizard (`harvest setup` or fresh install cold launch) includes a dedicated **Configure Laya** scene (`packages/coding-agent/src/modes/setup-wizard/scenes/laya.ts`):

1. **Prompt**: Asks the user whether to configure Laya locally.
2. **Automated Lifecycle (`laya-service.ts`)**:
   - Probes system Python 3 (verifying version $\ge 3.9$).
   - Verifies and installs missing Python dependencies (`laya`, `fastapi`, `uvicorn`, `torch`).
   - Ensures model weights are cached in single-model mode.
   - Launches or connects to the local daemon at `http://127.0.0.1:8177`.
   - Persists settings to `config.yml`:
     - `laya.enabled = true`
     - `laya.url = "http://127.0.0.1:8177"`
     - `laya.autostart = true`
3. **Opt-Out**: If the user skips, `laya.enabled` is set to `false`, and Harvest runs using standard cloud LLM evaluation exclusively.

---

## 4. Configuration & Environment Reference

### Settings (`config.yml` / `/settings`)
| Setting Key | Type | Default | Description |
|---|---|---|---|
| `laya.enabled` | `boolean` | `false` | Master toggle for local typed decision layer. |
| `laya.url` | `string` | `"http://127.0.0.1:8177"` | HTTP URL for the local decision sidecar. |
| `laya.autostart` | `boolean` | `true` | Automatically spawn sidecar subprocess if inactive. |

### Environment Variables
| Variable | Description |
|---|---|
| `LAYA_ENABLED` | Set to `"false"` to force-disable Laya regardless of config settings. |
| `LAYA_GATING` | Set to `"false"` to bypass tool-call gating checks specifically. |
| `LAYA_SIDECAR_URL` | Override sidecar URL (e.g. `http://127.0.0.1:8177`). |
| `LAYA_TIMEOUT_MS` | Override per-call timeout in milliseconds (default: `300`). |

---

## 5. Temperature Calibration (`calibration.py`)

Raw Laya probabilities are calibrated to prevent overconfidence:
- Logged decisions in `decision-sidecar/decisions.jsonl` are analyzed with `decision-sidecar/calibration.py`.
- Temperature scaling parameters ($T$) are optimized to minimize the Expected Calibration Error (ECE):
  - **Tool Gating**: $T = 2.6386$ (ECE: 0.4667 $\to$ 0.3021, -35.3%)
  - **Model Routing**: $T = 1.1333$ (ECE: 0.0819 $\to$ 0.0598, -27.0%)
  - **Completion Checks**: $T = 1.4002$ (ECE: 0.1842 $\to$ 0.1750, -5.0%)
- Calibrated parameters are saved to `decision-sidecar/calibration_params.json` and dynamically loaded by the sidecar server.

---

## 6. Provenance and release pointers (Stream-F docs slice)

Docs only — no behavior changed. The explicit contracts live in
`docs/product-decisions.md`: the F4 per-channel share-redaction matrix
(which channels redact, which ship raw, and the image-embedded-secret
limitation), the F7 retrieval-provenance surfacing contract (brain
citations, Mnemopi correction workflow, scope controls), and the F9
automated-vs-manual release legs. Sidecar portability and data-handling
detail lives in `decision-sidecar/README.md` (§5–§6).

Laya-side provenance already recorded in this repo: synthetic-vs-measured
calibration stamps (`_provenance` in `calibration_params.json`, acceptance
gate keeping autonomous clears off until measured), versioned Laya prompt
assets binding calibration data to an exact prompt revision, length-capped
rotated decision logs in git-ignored user data with a sanitized export,
and per-decision-point fail-open / fail-closed contracts (§1–§2 above).
Pre-share preview and unified provenance UIs remain open by design.
