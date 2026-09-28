---
name: laya-setup
description: Autonomous installation, hardware self-calibration, connection, and troubleshooting for the local Laya ModernBERT decision sidecar in Harvest.
---

# Laya Decision Layer: Autonomous Setup & Connection Guide

Laya is Harvest's local typed decision microservice powered by `convaiinnovations/laya-typed-decisions` (ModernBERT-large). It runs locally on loopback (`http://127.0.0.1:8177`) with zero API token cost, providing:
1. **High-Risk Tool Call Gating (Fail-Closed)**: Evaluates commands (`rm -rf`, `chmod`, `git push --force`, schema drops) in <30ms and triggers approval prompts if dangerous.
2. **Subagent Selection**: Dispatches tasks to the optimal specialist (`scout`, `reviewer`, `coder`) with hardware-aware latency budgets.
3. **Step Completion & Context Pruning**: Evaluates whether tool outputs succeeded and prunes stale context on capable hardware.

---

## 🚀 Autonomous One-Command Setup

Whenever the user asks to setup, install, connect, or repair Laya, run:

```bash
harvest laya setup --yes
```

This autonomously performs all required steps without requiring user intervention:
1. **Python Environment**: Discovers system Python 3.9+ or bootstraps Python via system package managers (`winget` on Windows, `brew` on macOS).
2. **Virtual Environment Isolation**: Creates and manages a dedicated isolated virtualenv under the Harvest agent config directory (`getAgentDir()/laya-venv`), protecting the system Python environment.
3. **Fast Dependency Installation**: Upgrades pip and installs `laya>=0.3.5`, `fastapi`, `uvicorn`, and `torch`. On non-NVIDIA machines (CPU or Apple Silicon), automatically selects the lightweight **~180MB CPU PyTorch wheel** (`--extra-index-url https://download.pytorch.org/whl/cpu`) instead of downloading 5GB+ of unused CUDA runtimes, cutting install time from ~50 minutes down to ~30 seconds.
4. **Model Checkpoint Verification**: Probes local HuggingFace cache for `convaiinnovations/laya-typed-decisions` (single 842MB `model.safetensors` checkpoint); streams download percentage and speed in real-time, with automatic fallback to `https://hf-mirror.com` if `huggingface.co` is throttled or unreachable.
5. **Port Conflict Auto-Reclamation**: Checks port `8177`; if occupied by an unresponsive or stale process, terminates the zombie process and safely binds the daemon.
6. **Daemon Startup**: Launches the background sidecar process and polls `/health` with an extended timeout window (up to 180s for cold CPU model warmup).
7. **Hardware Self-Calibration**: Measures latency on the machine's hardware tier (NVIDIA CUDA, Apple Silicon MPS, or CPU) and saves empirical timeout budgets to `getAgentDir()/laya-calibration.json`.
8. **Settings Connection**: Automatically sets `laya.enabled = true`, `laya.url = "http://127.0.0.1:8177"`, and `laya.autostart = true` in Harvest settings.

---

## 🔍 Verification & Inspection

### Check Laya Status
```bash
harvest laya status
```
Outputs the current connection state, detected hardware tier, device name, and derived latency settings.

### Query Health Directly
```bash
curl -s http://127.0.0.1:8177/health
```
Returns JSON with `"status": "ok"`, `"ready": true`, `"hardware_tier"`, and `"device_name"`.

### Run Hardware Calibration
```bash
harvest laya calibrate
```
Runs representative benchmark shapes (warmup + 3 runs each) to update derived timeouts and pruning enablement based on real device speed. If the sidecar is offline, `calibrate` automatically launches setup first.

---

## 🛠 Troubleshooting & Recovery

| Issue | Autonomous Fix |
|---|---|
| 5GB PyPI CUDA download on CPU | Setup automatically detects non-NVIDIA hosts and uses PyTorch CPU index (`https://download.pytorch.org/whl/cpu`, ~180MB). |
| HuggingFace download stall/block | Setup streams live download progress and automatically retries using `https://hf-mirror.com` if primary HF endpoint stalls. |
| Port 8177 already in use | Setup reuses a healthy sidecar or selects an available fallback port; it preserves foreign listeners. |
| Python 3.9+ missing | Setup attempts autonomous installation via `winget --scope user` (Windows) or `brew` (macOS) with zero UAC elevation blocking. |
| PEP 668 `externally-managed-environment` | Setup uses an isolated virtual environment under the Harvest agent config directory and reports an error if isolation cannot be created. |
| Slow CPU load | ModernBERT-large takes ~70–120s on CPU on cold load. Setup polls `/health` for up to 180 seconds with live elapsed status. |
| Corrupted installation | Run `harvest laya setup --reinstall` to wipe the virtualenv and reinstall all packages fresh. |
