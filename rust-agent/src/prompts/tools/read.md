Read a UTF-8 text file inside the workspace via `path`.

## Selectors — append `:<sel>` to `path` (e.g. `src/foo.ts:50-200`, `src/foo.ts:raw`)

- `:50` / `:50-` — from line 50 | `:50-200` — inclusive | `:50+150` — 150 lines from 50 | `:-60` — last 60 lines | `:5-16,960-973` — multiple ranges
- `:raw` — verbatim, no anchors/prefixes | `:2-4:raw` / `:raw:2-4` — range + verbatim

## Output

`[foo.ts#1A2B]` snapshot header + `N:line` rows. Copy `[FILENAME#TAG]` for anchored edits; NEVER fabricate the tag.
