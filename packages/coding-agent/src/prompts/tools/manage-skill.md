Managed skill: `SKILL.md` in isolated `~/.harvest/agent/managed-skills`; surfaced as a normal skill in future sessions.

Use: repeatable procedures worth codifying — setup sequence, debugging recipe, project-specific workflow.
User-authored skills separate; tool NEVER edits them.

- `action: "create"` — fails if skill exists; needs `description` + `body`.
- `action: "update"` — needs `description` and/or `body`; omitted fields merge from the current revision (prompt-only updates preserve policy fields); fails if skill absent; `expectedActive` rejects on conflict.
- `action: "draft"` — stage a revision without touching the live file; omitted fields merge from current.
- `action: "evaluate"` — run an explicit `task` + `expectedOutcome`; records pass/fail, never promotes.
- `action: "promote"` — activate a revision (failed evaluations block; unevaluated needs `discloseUnevaluated:true`); materializes atomically, pointer publishes only after the file succeeds.
- `action: "rollback"` — reactivate a prior active revision (needs `revisionId`; drafts that were never active are refused).
- `action: "list"` — revisions with state, active marker, eval counts/status, pinned markers.
- `action: "delete"` — fails if skill absent; history retained for audit.

`name`: kebab-case (lowercase letters, digits, hyphens).
`description`: specific; drives discovery.
No frontmatter in `body`; generated from `name` and `description`.
