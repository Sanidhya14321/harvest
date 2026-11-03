Agent coordination and supervised processes. In this runtime only the process ops are available:
- `start`: launch `application` with `args` (optional `cwd` inside the workspace, `env`, `name`); output is captured to a capped log.
- `ps`: list processes started this session.
- `logs`: last `lines` (default 100, max 1000) of `name`'s output; `grep` filters by regex.
- `stop`: terminate `name`.
Peer messaging (`send`/`wait`/`inbox`/`list`) and job control (`jobs`/`cancel`) need the TypeScript agent registry and fail with an error here. Requires approval.
