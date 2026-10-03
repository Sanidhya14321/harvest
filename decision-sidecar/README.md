# Harvest Decision Sidecar (Laya Local Decision Layer)

Fast, self-hosted, zero-cloud-cost local decision microservice powered by [Laya](https://github.com/convaiinnovations/laya) and the fine-tuned ModernBERT model [`convaiinnovations/laya-typed-decisions`](https://huggingface.co/convaiinnovations/laya-typed-decisions) (421M parameters, 1024-token context).

The sidecar runs locally as a dedicated Python process listening on `127.0.0.1:8177`, communicating with Harvest's TypeScript/Bun agent loop over low-overhead HTTP.

---

## 1. Overview & Architecture

Harvest delegates narrow classification questions to Laya rather than executing slow, expensive round-trips to the primary LLM:

- **Single-Model Mode**: The sidecar directly invokes `laya.load("convaiinnovations/laya-typed-decisions")` as a module-level singleton. It explicitly avoids the `Router` construct to prevent unintentional background downloads of secondary checkpoints (such as `laya-multilingual`).
- **Language Filtering**: Upstream and in-sidecar guards inspect prompt/state input. If non-English characters or scripts are detected, Laya inference is bypassed immediately and falls back to the main LLM.
- **Strict Fallback Contracts**:
  - **Tool-Call Gating**: Fails **CLOSED** (requires human interactive approval) if the sidecar is offline, times out (300ms default), or predicts irreversible actions ($P > 0.35$).
  - **Model Routing**: Fails **OPEN** (falls back to configured default model tier).
  - **Step Completion**: Fails **OPEN** (falls back to existing full-LLM evaluation check).
- **Temperature-Scaled Calibration**: Pre-calibrated scaling parameters adjust raw overconfident probabilities before decisions reach Harvest's execution gates.

---

## 2. API Reference

The service binds exclusively to `127.0.0.1` (configurable via `--host` / `--port` or environment variables).

### `GET /health`
Returns readiness status, loaded model identity, device, and active port.
The `scheduler` block reports capacity plus stuck-slot observability
(`in_flight`, `oldest_in_flight_ms`, `stuck`, `stuck_threshold_s`,
`worker_model`, `restart_advisory`).

**Response:**
```json
{
  "status": "ok",
  "ready": true,
  "model": "convaiinnovations/laya-typed-decisions",
  "device": "cpu",
  "port": 8177
}
```

### Stuck-slot restart procedure (P0-2)
A timed-out or cancelled inference retains its slot until the torch thread
actually finishes; a permanently stuck op never releases. Detection and
recovery are:
1. Poll `GET /health` → `scheduler.stuck == true` (slot older than
   `LAYA_STUCK_THRESHOLD_S`, default 120s) with a non-null `restart_advisory`.
2. Confirm `scheduler.in_flight > 0` while `/v1/decide` keeps returning 504
   (deadline exceeded) or queue timeouts despite an idle-looking client.
3. Restart the sidecar process (same host/port; systemd/supervisor or
   re-run `python server.py`). No in-flight request survives — callers fail
   open per the contracts above (tool gating fails CLOSED to human approval).
4. Verify `GET /health` → `scheduler.in_flight == 0`, `stuck == false`.

### `POST /v1/decide`
Executes one or more structured questions against the provided `state`.

Authentication: `/v1/decide` requires the per-process secret in the
`x-laya-token` header (or `Authorization: Bearer <token>`). The server logs the
token file location at startup (default `~/.harvest/laya-token`, mode `0600`);
set `LAYA_TOKEN` to use a fixed secret (the TS client sends it from its own
`LAYA_TOKEN` env). Read-only `GET /health` and `GET /v1/hardware` stay
unauthenticated. Set `LAYA_DISABLE_AUTH=1` only for local tests.
Unauthenticated or over-limit requests get `401`/`400`/`413`, all of which the
TS client treats as fallback (tool gating fails CLOSED on fallback).

Request limits (enforced before tokenization): 64 questions max, 500k state
chars aggregate, 100k chars per question chunk, 16k chars per question
definition, 4k chars instructions, ~70k estimated input tokens aggregate.
Inference runs at most 2 concurrent (`LAYA_INFERENCE_TIMEOUT_S`, default 120s),
with at most 8 requests waiting for capacity. A full queue returns 503. Clients
may send positive `metadata.request_timeout_ms` up to the configured server limit;
this budget includes waiting and inference. Harvest sends its remaining client
budget. Expired queued requests never begin inference, and disconnected callers
are checked before dispatch. A timed-out or cancelled running request retains
its slot until synchronous prediction actually finishes: Torch threads cannot be
stopped by cancelling an HTTP waiter. A permanently stuck prediction requires
sidecar restart; it cannot silently oversubscribe inference capacity.

**Request Body:**
```json
{
  "state": {
    "tool": "bash",
    "args": { "command": "rm -rf /tmp/scratch" }
  },
  "questions": {
    "irreversibility": {
      "type": "noul",
      "instructions": "does this call write, delete, publish, or change access irreversibly?"
    }
  },
  "metadata": {
    "call_site": "tool_gating",
    "session_id": "optional-uuid"
  }
}
```

**Supported Question Types:**
| Question Type | Output Shape | Used For |
|---|---|---|
| `noul` | `{ noul: float, confidence: float }` | Binary risk / irreversibility gating ($0.0$ to $1.0$). |
| `choice` | `{ answer: string, confidence: float, probabilities: {...} }` | Selecting among model tiers (`smol`, `slow`, `default`) or roles. |
| `score` | `{ score: float, confidence: float }` | Relevance ratings on 4 levels (0 to 3) for pruning/brain retrieval; see `packages/coding-agent/src/prompts/laya/pruning-criteria.md`. |

**Audit Logging:**
Every request and computed decision is appended to `~/.harvest/agent/logs/decisions.jsonl` by default (a private user-data directory; override with `LAYA_LOG_DIR` or `LAYA_LOG_FILE`) for continuous observability and offline calibration. The log is git-ignored — never commit it; no decision log is tracked in the repo (the former `decision-sidecar/decisions.jsonl` seed was untracked via `git rm --cached` and survives only as local user data).
Logged fields are length-capped (state kept to a 200-char snippet, instructions to 500 chars) and the log rotates size-based (`decisions.jsonl.1..N`).
Requests may pass optional per-question `ground_truth` labels (`question_id -> 0/1`) which are stored in the log record (default `null`) for later calibration.

---

## 3. Installation & Manual Run

### Dependencies
```sh
pip install -r requirements.txt
```

### Running the Server
```sh
python -m uvicorn server:app --host 127.0.0.1 --port 8177
# or
python server.py --host 127.0.0.1 --port 8177
# (positional `python server.py 127.0.0.1 8177` still works)
```

### Environment Variables
- `LAYA_HOST` (default: `127.0.0.1`) — Host interface binding.
- `LAYA_PORT` (default: `8177`) — Port binding.
- `LAYA_DEVICE` — Optional PyTorch device override (`cpu`, `cuda`, `mps`); invalid values fall back to auto-detection with a warning.
- `LAYA_LOG_DIR` — Custom directory for `decisions.jsonl`.
- `LAYA_LOG_FILE` — Exact custom path for `decisions.jsonl` (overrides `LAYA_LOG_DIR`).
- `LAYA_TOKEN` — Fixed sidecar secret (otherwise generated per-process). TS client sends it as `x-laya-token`.
- `LAYA_TOKEN_FILE` (default: `~/.harvest/laya-token`) — Where the generated secret is persisted (0600).
- `LAYA_DISABLE_AUTH=1` — Disable `/v1/decide` auth (tests only).
- `LAYA_INFERENCE_TIMEOUT_S` (default: `120`) — Wall-clock inference guard.
- `LAYA_STUCK_THRESHOLD_S` (default: `120`) — Age after which an in-flight
  inference slot counts as stuck in `GET /health` → `scheduler.stuck`
  (with `oldest_in_flight_ms` and a `restart_advisory` string).
- `LAYA_WORKER_MODEL` (default: `thread`) — Only `thread`
  (`asyncio.to_thread`) is supported. `process` is rejected at startup with
  an explanatory error: the killable-process spike (P0-2) was evaluated and
  rejected because the Laya agent/model state is not safely fork/pickle-able
  per request (full checkpoint reload per child, tokenizer races, new IPC
  protocol) and would risk the fail-open contract. Stuck torch work cannot be
  cancelled in-process; use the stuck flag + restart procedure below.
- `LAYA_LOG_MAX_BYTES` (default: `10485760`) / `LAYA_LOG_BACKUP_COUNT` (default: `3`) — Decision-log rotation.

---

## 4. Temperature Calibration

Laya models can exhibit overconfidence out-of-the-box. The sidecar includes a dedicated calibration engine in `calibration.py`:

```sh
python calibration.py --decisions decisions.jsonl --out calibration_params.json
# Synthetic bootstrap (never touches the real params file by default):
python calibration.py --synthetic
# Overwrite an existing params file (previous content kept as .bak):
python calibration.py --decisions decisions.jsonl --out calibration_params.json --force
# Shareable review copy with free-text state/instructions removed:
python calibration.py --decisions decisions.jsonl --export-sanitized /tmp/decisions.sanitized.jsonl
```

**Calibration Pipeline:**
1. Parses historical or synthetic decisions from `decisions.jsonl`.
2. Computes the pre-calibration **Expected Calibration Error (ECE)**.
3. Fits an optimal temperature parameter ($T$) per call site via negative log-likelihood minimization.
4. Generates calibrated probabilities: $\hat{p} = \sigma(z / T)$.
5. Saves learned parameters to `calibration_params.json`, which the server automatically loads at startup.

**Provenance & safety:** saved params carry a top-level `_provenance` marker
(`synthetic: true/false` plus per-site sample counts) so synthetic bootstraps
are never mistaken for measured calibrations. `--synthetic` writes to
`calibration_params.synthetic.json` by default; overwriting any existing params
file requires `--force` (previous content kept as `<file>.bak`). Note: the
shipped `calibration_params.json` is a synthetic bootstrap (150 samples/site,
seed 42) until real `ground_truth`-labelled logs are collected.

**Acceptance gate for autonomous clears:** the shipped temperatures and the
`noul >= 0.35` / confidence `< 0.75` gating thresholds must NOT be treated as
measured safety evidence, and autonomous approval clears stay OFF until all
of the following hold:
1. A labeled coding-tool dataset from real, consented traces covers
   read-only, reversible-write, destructive, network/publish, and
   permission-change classes (labels arrive as per-question `ground_truth`).
2. False clears, false escalations, timeout rate, and decision latency are
   measured per tool class and hardware on a held-out split.
3. Destructive-class false clears are zero with a non-trivial sample, and
   overall calibrated ECE improves on the held-out set.
4. The resulting params are version-stamped (`_provenance.synthetic: false`)
   and reviewed before replacing the bootstrap.

---

## 5. Release portability and manual gates (Stream-F docs slice)

Docs only — no behavior changed. Stream-E owns the scheduler contract
(`GET /health` → `scheduler`, stuck-slot procedure, `LAYA_*` worker
variables) above; this section only itemizes which release legs are
automated and which stay manual.

Automated (`.github/workflows/release.yml`, all in `publish.needs`):
binary matrix (linux-x64/arm64, linux-musl-x64/arm64, darwin-x64/arm64,
win32-x64/arm64 — each runs `--version` + `--smoke-test` on its target
host), CPU sidecar smoke on Linux + Windows (health identity,
authenticated inference, token rotation, teardown), and the npm-bundle leg
(pack → tarball contents, source-vs-packed version parity, bundled-CLI
boot).

Manual (documented skips — run verbatim on the stated host):

- macOS sidecar (CPU torch wheel unverified on clean mac runners; darwin
  binaries stay gated): `bash scripts/ci-sidecar-smoke.sh`
  (`LAYA_SMOKE_PORT`/`LAYA_SMOKE_DIR` as needed; the script is macOS-safe).
- Full-offline (runner isolation flaky; cold cache needs network):
  cache-only operation is supported — with a warm Hugging Face cache,
  `HF_HUB_OFFLINE=1 bash scripts/ci-sidecar-smoke.sh`. Model absent and
  unreachable ⇒ server stays unready; the TS client fails open (tool
  gating fails CLOSED to human approval).
- Fresh-registry consumer install: the `@harvest` npm scope is
  workspace-resolved, not published, so a clean-registry install stays a
  manual gate.
- Degraded hosts (non-admin Windows, missing Python/native assets,
  unsupported accelerators): setup heals its 8 enumerated modes only and
  logs to `~/.harvest/agent/logs/laya-setup.log`; anything else emits a
  diagnostic bundle and fails open.

## 6. Data handling for sharing and calibration (Stream-F docs slice)

Docs only — no behavior changed. What stays local vs what is shareable:

- `decisions.jsonl` is private user data (`~/.harvest/agent/logs/` by
  default, `LAYA_LOG_DIR`/`LAYA_LOG_FILE` overrides; git-ignored — never
  commit it). Logged fields are length-capped (state 200-char snippet,
  instructions 500 chars) and the log rotates size-based
  (`decisions.jsonl.1..N`).
- The only shareable form is the sanitized review copy:
  `python calibration.py --decisions decisions.jsonl --export-sanitized
  /tmp/decisions.sanitized.jsonl` (free-text state/instructions removed).
- `/v1/decide` requires the per-process secret (`x-laya-token` header or
  `Authorization: Bearer <token>`; persisted at `LAYA_TOKEN_FILE`,
  default `~/.harvest/laya-token`, mode `0600`; `LAYA_TOKEN` pins a fixed
  secret). `GET /health` and `GET /v1/hardware` stay unauthenticated.
  `LAYA_DISABLE_AUTH=1` is tests-only.
- The service binds loopback only (`127.0.0.1`, `LAYA_HOST`/`LAYA_PORT`
  overrides); never expose `/v1/decide` (unauthenticated) off-loopback.
- Text-only consumers (share snapshots, calibration review) cannot inspect
  secrets baked into image pixels — exclude images before sharing when in
  doubt. Per-channel policy lives in `docs/product-decisions.md` (F4
  matrix).
