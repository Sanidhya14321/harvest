# Decision Layer (Laya removed)

> **Removed:** the Laya local decision sidecar (ModernBERT
> `convaiinnovations/laya-typed-decisions`, `decision-sidecar/`,
> `laya-*.ts` core modules, `prompts/laya/`, `/laya` command, setup-scene,
> CLI command) has been removed from runtime, packaging, setup, settings
> UI, and active docs.
>
> What replaces each behavior:
> - Tool gating → existing human-approval policy (`tools.approval*`);
>   approvals follow configured permissions (a `yolo` configuration still
>   auto-approves ordinary tiers; provider-required human safety checks stay
>   mandatory).
> - Brain retrieval → deterministic lexical/graph order (BM25 + one-hop
>   `depends_on`/`relates_to`); no pruning or rerank step.
> - Model/subagent selection → explicit user/agent selection; no automatic
>   routing.
> - `laya.*` settings keys are parsed but inert (no UI); `LAYA_*`
>   environment variables are ignored with one deprecation warning.
> - User data is left in place: `~/.harvest/agent/logs/decisions.jsonl`,
>   `laya-setup.log`, `laya-calibration.json`, HuggingFace cache.
>   Delete manually if unwanted.
>
> `/laya` now reports the removal. This file is kept because links exist.

## Historical contract (archaeology summary)

- Three interception points: tool-call gating (`noul` irreversibility,
  fail-closed, ~300ms), model-tier routing helper (fail-open, never
  automatic), completion classification for unexpected stops (fail-open).
- Guardrails: single-checkpoint pin, localhost-only bind
  (`127.0.0.1:8177`), circuit breaker (optional decisions fail fast;
  gating always attempts), cloud-LLM fallbacks preserved.
- Setup wizard provisioned Python ≥3.9, dependencies, weights, daemon,
  and `laya.*` config; temperature calibration per decision point.
- Decision history lives in `docs/product-decisions.md` (F6/F9 items,
  marked superseded where applicable).
