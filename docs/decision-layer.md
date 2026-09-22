# Laya Local Decision Layer

Harvest integrates [Laya](https://github.com/convaiinnovations/laya) and the fine-tuned ModernBERT checkpoint [`convaiinnovations/laya-typed-decisions`](https://huggingface.co/convaiinnovations/laya-typed-decisions) as a fast, local, zero-API-cost typed decision layer. It handles narrow deterministic classifications directly on the developer's machine without polluting the primary LLM reasoning stream or incurring remote latency.

---

## 1. Architectural Role & Decision Points

The decision layer operates at three critical interception points in the agent loop:

```
                                  [Agent Loop Turn]
                                          │
    ┌─────────────────────────────────────┼─────────────────────────────────────┐
    │                                     │                                     │
    ▼                                     ▼                                     ▼
[1. Tool Gating]                   [2. Model Routing]                 [3. Completion Check]
• High-risk tools:                 • Evaluates prompt                 • Evaluates command output
  bash, write, edit, ast-edit        complexity & domain                and stop conditions
• Question: noul (irreversible?)   • Question: choice (tier/role)     • Question: noul (done/clean?)
• Timeout: ~300ms                  • Timeout: ~300ms                  • Timeout: ~300ms
• Contract: FAIL CLOSED            • Contract: FAIL OPEN              • Contract: FAIL OPEN
  (requires human approval)          (falls back to default tier)       (falls back to full LLM)
```

### 1.1 Tool-Call Gating (`laya-gating.ts`)
Before dispatching any mutation command from the high-risk tool set (`bash`, `exec`, `write`, `edit`, `ast-edit`, `patch`), Harvest constructs a `noul` query:
> *"Does this call write, delete, publish, or change access irreversibly?"*

- If the computed irreversibility score exceeds `0.35`, or model confidence is below `0.75`, the call is tagged with `providerMetadata.layaGatingRequired = true`.
- Harvest's tool wrapper immediately stops execution and prompts the user for manual confirmation in the TUI.
- **Fail-Closed Guarantee**: If the local sidecar service is offline, unresponsive, or times out (300ms), the tool call fails **CLOSED** and prompts the user for confirmation. Read-only tools (`read`, `grep`, `glob`) bypass gating checks with zero overhead.

### 1.2 Model Tier Routing (`laya-routing.ts`)
During session initialization or turn transitions, user prompts are evaluated against the available model tiers:
- `smol`: Simple syntax fixes, localized queries, single-file edits.
- `slow`: Multi-file architecture, deep algorithmic reasoning, complex refactoring.
- `default`: General software engineering workflows.

- Uses a `choice` question to classify the prompt in under 400ms.
- **Fail-Open Guarantee**: If the sidecar is unreachable or confidence is low, routing immediately falls **OPEN** to the user's configured default model tier.

### 1.3 Step & Task Completion Evaluation (`laya-completion.ts`)
Replaces speculative sub-queries to cloud models when verifying whether a command completed cleanly or whether an unexpected stop occurred.
- Evaluates recent diagnostic output tails using batched `noul` queries.
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
