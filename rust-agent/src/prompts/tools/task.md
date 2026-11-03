Run ONE subagent synchronously. Execution blocks until work finishes.

# Task Design
- **Agent typing:** Pick the most specific available agent. Read-only research MUST run on `scout`. Omit `agent` when the default (`task`) is the best fit.
- **No overhead:** Each `task` MUST instruct its agent to skip formatters, linters, and project-wide test suites. Run those once at the end.
- **One-pass:** Prefer agents that investigate AND edit in one pass.

# Inputs
- `name`: A stable CamelCase identifier (≤32 chars). Generated automatically if omitted.
- `agent`: The agent type to spawn (`task`, `scout`, `reviewer`, `security-reviewer`, `sonic`). Omitting `agent` selects `task`.
- `context`: Shared project state, constraints, and contracts.
- `task`: Complete, self-contained instructions. One-liners or missing acceptance criteria are PROHIBITED.

Subagent mutating calls (write/edit/bash) still require the parent's approval.
