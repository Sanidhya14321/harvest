# Architecture (Laya integration — removed)

> **Removed:** the Laya ModernBERT decision sidecar integration described
> below has been removed from runtime, packaging, setup, settings UI, and
> active docs. `laya.*` keys are inert, `LAYA_*` env vars are ignored with
> one deprecation warning, `/laya` reports the removal, and prior user data
> (logs, calibration, model cache) is left in place. Preserved: approvals
> fail closed, brain retrieval is lexical/graph order, model selection is
> explicit. This file is kept because links exist.

## Historical summary (archaeology only)

- **Topology:** CLI agent loop called a local FastAPI sidecar
  (`127.0.0.1:8177`) hosting the single `laya-typed-decisions` checkpoint
  (PyTorch CUDA / MLX / CPU, semaphore-bounded concurrency).
- **Phase 1 — context pruning:** cache-locked relevance scoring of aged
  tool outputs; disabled automatically when local scoring exceeded the
  per-turn latency budget.
- **Phase 2 — subagent selection:** single-choice task classification with
  shadow-mode audit and human-review CLI; never automatic model routing.
- **Calibration:** live benchmarks across representative payload shapes
  derived timeouts and activation thresholds (never hardcoded tables).
- **Installer:** single-command setup with bounded self-healing (8
  enumerated failure signatures, diagnostic bundles, LLM-assisted
  second-tier diagnosis) and fail-open degradation to plain cloud-LLM
  behavior.
