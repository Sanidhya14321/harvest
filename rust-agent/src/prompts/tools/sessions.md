Managed live sessions: list, inspect, create, rename, message, or stop background agent sessions in the current project.

Sessions are independent runtimes: created in the background, never steal focus, and stay in the current project unless an explicit cross-project grant is given.

- `action: "list"` / `"inspect"` — read-only roster / session detail.
- `action: "create"` — open a background session (optional `task` to run first). Honors `task.maxConcurrency`, spawn policy, and recursion depth.
- `action: "rename"` — set a session title.
- `action: "send"` — deliver a message to a live session without focusing it.
- `action: "stop"` — abort a session's run. Never the calling session itself (refused: no synchronous self-stop).

Targets are limited to the current owned lineage + authorized project; broader targets need a host-provided grant (model-supplied flags never authorize). Busy or foreign sessions are rejected, never force-touched.
