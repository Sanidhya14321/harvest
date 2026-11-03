# @harvest/pi-metaharness

One manager for repository benchmarks. Harbor, TypeScript edit, and SnapCompact
runs use the same experiment → run → trace model, SQLite store, REST/SSE API,
and dashboard. Benchmark-native artifacts remain on disk; adapters normalize
their live progress, scores, token usage, costs, and traces.

```bash
# Dashboard + API on :4700; launch every benchmark from the same “new run” form
bun run serve --port 4700
```

## How Harbor runs execute

1. **Local omp, not npm.** By default the runner bind-mounts the repo
   read-only into each task container (`--install source`) and runs omp
   straight from `packages/coding-agent/src/cli.ts` — TS edits apply to the
   next trial with no rebuild. A cached linux `node_modules` tree (built once
   per lockfile change inside `oven/bun`, stored in `<jobs-dir>/_bench/_deps/`)
   shadows the host's darwin one, and a linux `bun` binary is mounted at
   `/opt/omp/bin` — so trial setup needs zero outbound network. Alternatives:
   `--install local` (pack a tarball per run) or `--binary` (prebuilt
   `dist/omp-linux-*` self-contained binaries).
2. **Auth never enters containers.** A generated `models.yml` routes provider
   `baseUrl`s at the host pm2 auth-gateway; the gateway resolves credentials
   host-side.
3. **Harbor owns trials.** The runner/serve layer polls each trial's
   `result.json` for progress, spend, and outcomes.

## Server

- `GET /` — experiments, runs, normalized traces, and a launch form for every benchmark.
- `GET /api/experiments[?q=]` — experiment summaries across all benchmark types
  (`q` filters by id/goal substring).
- `POST /api/experiments` — register an experiment before its first arm. Body
  `{ "id": "sb2", "goal": "..." }`; the id is the dash-free token job names
  group under (`sb2-n8` → experiment `sb2`).
- `GET /api/experiments/:id` — arms, per-task matrix, and calibrated projections.
- `PUT /api/experiments/:id` — update the goal and per-run role/note/label.
- `POST /api/experiments/:id/arms` — launch a comparable arm; sample + config
  inherited from a sibling.
- `DELETE /api/experiments/:id` — delete every arm (DB rows **and** job dirs)
  plus the goal row; rejected while any arm is running.
- `GET /api/runs[?experiment=&status=&benchmark=]` — uniform run rows with
  benchmark, score, progress, spend, and tokens.
- `POST /api/runs` — launch through a benchmark adapter. Body:

  ```json
  {
    "benchmark": "edit",
    "model": "anthropic/claude-opus-4-8",
    "tasks": 20,
    "concurrency": 4,
    "attempts": 2,
    "jobName": "edit-baseline",
    "role": "baseline",
    "goal": "compare edit strategies"
  }
  ```

  `benchmark` is `harbor`, `edit`, or `snapcompact`. Harbor uses `dataset`,
  `include`, `timeoutMultiplier`, and `prewalk`; edit uses `include` as task IDs;
  SnapCompact uses `conditions` and treats `tasks` as the passage limit.
- `GET /api/runs/:name` — `{ run, traces }` (syncs native artifacts on read).
- `POST /api/runs/:name/cancel` — cancel a manager-launched run.
- `DELETE /api/runs/:name` — permanently delete a finished run (DB row **and**
  job dir; a surviving dir would be re-discovered on restart); rejected while
  the run is live.
- `POST /api/runs/:name/resume` — resume an incomplete harbor run in place:
  completed trials (and their spend) are reused, interrupted/pending trials
  re-run, and errored trials retried (body `{ "filterErrorTypes": [...] }`
  overrides the retry set, which defaults to every exception type in the job's
  `result.json`). The runner recovers the original launch flags from
  `_bench/<name>/runner-config.json` (snapshotted at launch) or the run's
  `manager.json` — nothing needs re-specifying.
- `GET /api/runs/:name/traces/:trace[?raw=1]` — normalized or native trace.
- `GET /api/events` — SSE stream of run-list snapshots (sent on change).

State lives in `<jobs-dir>/_manager/metaharness.sqlite`; the filesystem
stays the source of truth and historical CLI runs are auto-discovered.

## Native Terminal-Bench 2.1 runner

`bench:tb` bypasses Harbor and Docker. It boots each task's published OCI
image as an x86_64 KVM microVM on a Vibemon host, streams omp's `--mode rpc`
protocol through `vmon exec --pipe`, runs the verifier in the same mutated VM,
and writes resumable epochs plus artifacts under `runs/tb`.

```bash
bun run bench:tb \
  --dataset /path/to/terminal-bench-2-1 \
  --concurrency 4 --forever --budget 25
```

For the measured workstation defaults (seven-model pool, `:floor`, concurrency
20), use the resumable wrapper:

```bash
bun --cwd packages/metaharness run bench:tb-floor
```

It writes to `runs/tb-floor`; rerunning resumes an incomplete epoch or starts
the next epoch after completion. Environment overrides: `TB_JOBS_DIR`,
`TB_CONCURRENCY`, `TB_DATASET`, `TB_BUDGET_USD`, `TB_ATTEMPTS`, and
`TB_FOREVER=1`. Extra CLI flags pass through and win over wrapper defaults.

The default pool is Ling 3.0 Flash, both DeepSeek V4 Flash routes (0731 and
unversioned/0423), Nemotron 3.5 Lightning, Laguna S 2.1, Tencent Hy3, and Step
3.7 Flash through OpenRouter. Requests default to OpenRouter's `:floor`
cheapest-provider routing; use `--openrouter-variant default|nitro|online|exacto`
to override it. Repeat `--model provider/id` to replace the pool.

Defaults target the workstation's `xeon.internal` KVM host and the local
`/work/vibevmm` checkout. The runner cross-builds and caches a patched `vmon`
binary when the remote copy lacks raw pipe support, owns a privileged TAP
broker for its lifetime, and reverse-tunnels the loopback omp auth gateway so
provider credentials never enter task VMs. Trial agents use a lean terminal
tool allowlist, `edit.mode: replace`, and the same low-cost
`openrouter/qwen/qwen3.7-flash` vision role; general orchestration tools are
omitted from the benchmark prompt. Override infrastructure with
`--vmon-host`, `--vmon-source`, `--vmon-bin`, `--vmon-home`,
`--vmon-kernel`, `--vmon-agent`, and `--gateway-url`.

## Harbor runner options (excerpt)

| Option | Default | Notes |
|---|---|---|
| `-m, --model <provider/model>` | `anthropic/claude-sonnet-4-6` | Repeatable |
| `-l, --tasks <N>` | `20` | Max tasks |
| `-n, --concurrency <N>` | `4` | Concurrent trials |
| `-k, --attempts <N>` | `1` | Attempts per task (pass@k) |
| `-d, --dataset <name>` | `terminal-bench@2.0` | Any Harbor dataset id |
| `-i/-x, --include/--exclude <glob>` | — | Task filters (repeatable) |
| `--timeout-multiplier <x>` | — | Scales task agent/verifier timeouts |
| `--agent-arg <arg>` | — | Extra arg forwarded verbatim to the in-container omp CLI (repeatable) |
| `--env <KEY[=VALUE]>` | — | Forward env into the omp container (repeatable); `KEY` alone forwards the host value |
| `--binary <path>` | — | Prebuilt omp binary (repeat for arm64+x64) |
| `--install <source\|local\|published>` | `source` | `source` = repo bind-mount, `local` = tarball pack, `published` = npm `@harvest/pi-coding-agent` |
| `--environment <docker\|apple-container>` | `docker` | `apple-container` runs trials via Apple's `container` CLI (no Docker); source/deps mounts go through `harbor --mounts` and the gateway is auto-forwarded from `192.168.64.1:4000` to the loopback-bound gateway |
| `--gateway-url <url>` | `http://host.docker.internal:4000` | `http://192.168.64.1:4000` under `--environment apple-container` |
| `--no-gateway` | off | Pass host provider keys into containers instead |
| `-o, --jobs-dir <path>` | `<repo>/runs/harbor` | Shared with the server |
| `--resume <name\|path>` | — | Resume that job dir via `harbor job resume`; original flags recovered automatically |
| `--filter-error-type <T>` | `CancelledError` | With `--resume`: also re-run completed trials that errored with exception type `T` (repeatable) |
| `--dry-run` | off | Print the harbor command + models.yml and exit |

## Outputs

- `<jobs-dir>/<jobName>/` — Harbor trial dirs (`result.json` per trial).
- `<jobs-dir>/_bench/<jobName>/report.md` — markdown summary table.
- `<jobs-dir>/_bench/<jobName>/harbor.log` — full Harbor output.
- `<jobs-dir>/_manager/logs/<jobName>.log` — runner output for API-launched runs.

## Trace reports

Native `harvest trace-report` turns one normalized run trace into a numbered
Turn Log, parallel per-turn model summaries, then a Story Arc and failure analysis.
It also reads actual persisted session JSONL through the shared Rust branch codec.
No transcript tool/code is executed. Model prose is analysis, not proof of success.

```bash
cargo run --quiet --locked -p harvest-omp --bin harvest -- trace-report <run> <trace> --workspace . --config trace-models.toml --tiny-profile trace-tiny --synth-profile trace-synth --out report.md
# Offline normalized input, or a selected native/compatible persisted branch:
harvest trace-report 'run|trace' --workspace . --input trace.json --metadata run.json --tiny-profile trace-tiny --synth-profile trace-synth
harvest trace-report run trace --workspace . --session session.jsonl --leaf ENTRY_ID --tiny-profile trace-tiny --synth-profile trace-synth
```

`bun run trace:report -- ...` is the package convenience entrypoint to the same
Rust CLI. Model selection uses native profiles, with explicit endpoints/API and
credential sources (literal, environment, or opt-in command), rather than assuming
that a provider/model string establishes catalog or credential policy. For example:

```toml
[profiles.trace-tiny]
model = "inclusionai/ling-2.6-flash"
base_url = "https://openrouter.ai/api/v1"
api_key = "OPENROUTER_API_KEY"

[profiles.trace-synth]
extends = "trace-tiny"
model = "openai/gpt-oss-120b"
```

Native configuration precedence remains profile < environment < flags; unset
OMP_MODEL/OMP_BASE_URL overrides when selecting different profile models. Use
`--config FILE` for a native-schema overlay; no old auth database is modified.
Set OPENROUTER_API_KEY before using the example (the shared value resolver uses an
exact environment name, with literal fallback). `--allow-credential-commands`
explicitly authorizes command-backed secrets.
`--base` defaults to http://localhost:4700; `--concurrency` defaults to 8 (1..32),
`--timeout-seconds` bounds the whole job (default 600, max 3600), `--focus` supplies
reviewer notes, and `--json` includes separate map/reduce usage and diagnostics.
Output/input paths stay inside `--workspace`; output is atomic and only written
when the whole report succeeds. Unknown usage/pricing stays unavailable.

The server fetch requests its maximum 2,000-entry tail; upstream normalization
may already truncate result bodies. Unsupported question/answer/conversation
formats and orphan results fail explicitly. Offline/session input avoids the
server dependency but remains bounded; a branch containing an orphan result must
be repaired/selected correctly, never guessed into an assistant turn.

**Credential compatibility pending:** the standalone `scripts/trace-report.ts`
and explicit `trace:report:legacy-auth` entrypoint remain for consumers using
stored/OAuth credentials in the legacy auth database. They are transitional TS,
not a native fallback or completed removal. Native profiles do not yet import or
refresh those credentials. The native CLI never silently launches Bun. Shared
credential migration and its installed checks must pass before deleting that
last legacy path. See `docs/rust-migration/tasks/METAHARNESS-TRACE-REPORT.md`.

## Caveats

- **Network policy.** On Harbor's local Docker backend only **public**
  registries work; task containers reach models via the host gateway.
- **`--install source` reflects local TS changes** with no rebuild, but Rust
  natives load from the in-tree `packages/natives/native/pi_natives.linux-*.node`
  prebuilds — rebuild those when Rust changes (the loader skips the version
  sentinel for workspace loads, so a stale `.node` runs silently).
- **Source mode is single-arch.** The deps tree matches the docker daemon's
  native arch; trials on emulated images (e.g. x64 tasks on an arm64 host)
  fail setup with an arch-mismatch error — use `--binary` for those.
- **The repo is visible (read-only) inside task containers** in source mode;
  fine for curated benchmarks, but don't point it at untrusted tasks.
- **Apple Container specifics.** `--environment apple-container` needs
  `brew install container && container system start` (macOS 26+, Apple
  silicon). `--host-network` and `--cleanup*` are docker-only, and bind
  mounts are read-write (the backend ignores `read_only`).
- **`--install local` reflects local TS changes** (inlined into `dist/cli.js`),
  but **not** uncommitted Rust natives — rebuild `packages/natives` per target
  first (the version sentinel must match).
