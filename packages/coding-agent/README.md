# @harvest/pi-coding-agent

Core implementation package for the `harvest` coding agent in the `harvest` monorepo.

For installation, setup, provider configuration, model roles, slash commands, and full CLI reference, see:
- [Monorepo README (local)](../../README.md)
- [Monorepo README (GitHub)](https://github.com/harvest/harvest#readme)

Package-specific references:
- [CHANGELOG](./CHANGELOG.md)
- [MCP configuration guide](../../docs/mcp-config.md)
- [MCP runtime lifecycle](../../docs/mcp-runtime-lifecycle.md)
- [MCP server/tool authoring](../../docs/mcp-server-tool-authoring.md)
- [DEVELOPMENT](./DEVELOPMENT.md)

## Memory backends

The agent supports three mutually-exclusive memory backends, selected via the `memory.backend` setting (Settings → Memory tab, or `~/.harvest/agent/config.yml`):

- `off` (default) — no memory subsystem runs.
- `local` — existing rollout-summarisation pipeline; writes `memory_summary.md` and consolidated artifacts under the agent dir.
- `hindsight` — talks to a [Hindsight](https://hindsight.vectorize.io) server (Cloud or self-hosted Docker), retains transcripts every Nth user turn, recalls memories on the first turn of a session, and exposes `retain`, `recall`, and `reflect`.

### Hindsight quickstart

1. Run a Hindsight server (Cloud or `docker run -p 8888:8888 ghcr.io/vectorize-io/hindsight:latest`).
2. Set `memory.backend = "hindsight"` and `hindsight.apiUrl = "http://localhost:8888"` (or your Cloud URL).
3. Optional environment overrides (env wins over settings):
   - `HINDSIGHT_API_URL`, `HINDSIGHT_API_TOKEN` — connection
   - `HINDSIGHT_BANK_ID`, `HINDSIGHT_DYNAMIC_BANK_ID`, `HINDSIGHT_AGENT_NAME` — bank addressing
   - `HINDSIGHT_AUTO_RECALL`, `HINDSIGHT_AUTO_RETAIN`, `HINDSIGHT_RETAIN_MODE` — lifecycle
   - `HINDSIGHT_RECALL_BUDGET`, `HINDSIGHT_RECALL_MAX_TOKENS` — recall sizing
   - `HINDSIGHT_BANK_MISSION`, `HINDSIGHT_DEBUG`

Switching backends mid-session immediately replaces the live backend, memory tools, listeners, and system-prompt context. Existing users with `memories.enabled = true|false` are migrated to `memory.backend = "local"|"off"` exactly once on first launch; afterward, `memory.backend` is the sole runtime selector.

## Local Decision Layer (Laya)

Harvest embeds [Laya](https://github.com/convaiinnovations/laya) (`convaiinnovations/laya-typed-decisions`, ModernBERT-large 421M) as a local typed decision layer for high-frequency classifications:

- **Tool-Call Gating**: Intercepts high-risk mutations (`bash`, `write`, `edit`, `ast-edit`, `patch`) and asks a `noul` irreversibility question. Fails **CLOSED** (prompts user for interactive approval) if offline, timed out (300ms), or risky ($P > 0.35$).
- **Model Routing**: Evaluates prompt complexity with a `choice` question to route between `smol`, `slow`, and `default` model tiers. Fails **OPEN** to default tier.
- **Step Completion Checks**: Evaluates execution outputs and stop classifications with `noul` before invoking heavy cloud models. Fails **OPEN**.

### Configuration & Setup
- Configured during onboarding via `harvest setup` ("Configure Laya" page).
- Config keys: `laya.enabled`, `laya.url` (default: `"http://127.0.0.1:8177"`), `laya.autostart`.
- Environment overrides: `LAYA_ENABLED=false`, `LAYA_GATING=false`, `LAYA_SIDECAR_URL`, `LAYA_TIMEOUT_MS`.
- For full architectural details, see the [Decision Layer Guide](../../docs/decision-layer.md) and [Sidecar Documentation](../../decision-sidecar/README.md).
