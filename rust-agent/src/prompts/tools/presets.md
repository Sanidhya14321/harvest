Managed agent presets: versioned task-agent definitions in isolated `~/.harvest/agent/managed-presets`; never authored/bundled dirs.

Use: repeatable agent personas worth reusing — reviewer, planner, domain specialist.

- `action: "list"` / `"inspect"` — read revision history / preset detail with per-revision state, eval counts/status, pinned markers.
- `action: "create"` — generate via an isolated architect session, then save. Fails on name collisions, including authored/bundled claims.
- `action: "update"` — needs at least one of `description`, `systemPrompt`, `tools`, `spawns`, `model`; omitted fields merge from the current file/revision (prompt-only updates preserve policy fields); explicit values are validated replacements; fails if preset absent; `expectedActive` rejects on conflict.
- `action: "draft"` — stage a revision without touching the live file (merge-aware).
- `action: "evaluate"` — run an explicit task + expected outcome; records pass/fail, never promotes.
- `action: "promote"` — activate a revision (failed evaluations block; unevaluated needs `discloseUnevaluated:true`); materializes atomically, pointer publishes only after the file succeeds.
- `action: "rollback"` — reactivate a prior active revision (needs `revisionId`; never-active drafts are refused).
- `action: "delete"` — removes the file; revision history is retained.

`name`: lowercase kebab-case, 2-6 words. `description`: must start with "Use this agent when…".
