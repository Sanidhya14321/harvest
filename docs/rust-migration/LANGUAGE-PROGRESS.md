# Language and deletion audit, 2026-10-23

Snapshot: `bee9356531a64458c11304ddff0ef2443de28b02`, fetched from
`Sanidhya14321/harvest` branch `ts-to-rust`; local HEAD matched FETCH_HEAD.
GitHub repository metadata accessed through the GitHub connector reported
`default_branch: main` and `language: TypeScript`. GitHub's repository language
statistics reflect the default branch. Selecting `ts-to-rust` in the file browser
does not make its commits change the repository-wide language bar. Migration
commits have not been merged to main; that remains a separate action.

Exact live GitHub language bytes/percentages could not be retrieved: the connector
rejects the languages endpoint and the environment's direct `gh api` access
returned Forbidden. Thus the user's displayed 82.8% is not independently certified
here. The Git object audit below independently confirms substantial remaining TS.
No Linguist settings, default branch or language classifications were changed.

| Snapshot | TS + TSX files | TS + TSX blob bytes | Rust files | Rust blob bytes |
|---|---:|---:|---:|---:|
| Original baseline `d811cb2` | 5,015 | 55,942,436 | 607 | 8,816,489 |
| Python retirement `3e38027` | 4,955 | 55,484,591 | 636 | 9,047,026 |
| Audited tip `bee9356` | 4,955 | 55,484,591 | 662 | 9,221,904 |

These are raw tracked blob counts/bytes by extension, including declarations,
fixtures, generated files and tests. They are **not GitHub Linguist statistics**,
a capability completion percentage, or a measure of remaining engineering effort.
The smaller authored-source audit has different exclusions; do not mix them.

Actual TS/TSX deletion paths from `git diff --name-status --diff-filter=D`:

- Baseline `d811cb2..bee9356`: **60**.
- Python retirement `3e38027^..3e38027`: **21**.
- Latest four commits `3e38027..bee9356`: **0**.

The latest four commits added native Anthropic, plugins/configuration/scoring,
MCP and LSP/DAP connections. Their production legacy consumers still exist.
Therefore they have not reduced the tracked TS byte total. Rust additions alone
are small relative to the retained TS tree. Some earlier inherited removals also
lack verified replacement evidence; their existence is not proof of completion.

Reproduce using Git (no worktree, dependencies or build required):

```sh
git fetch origin ts-to-rust
git rev-parse HEAD FETCH_HEAD
git ls-tree -rlz bee9356
git diff --name-status --diff-filter=D d811cb2..bee9356 -- '*.ts' '*.tsx'
git diff --numstat 3e38027..bee9356 -- '*.ts' '*.tsx'
```

Parse each NUL-delimited ls-tree entry into metadata and path at its first tab;
sum the blob size and count for exact `.ts`, `.tsx` and `.rs` suffixes. Report new
checkpoints' additions and deletions separately. Do not imply source reduction
from a Rust code-written checkpoint. Delete superseded implementations when their
last retained consumer moves, with the shared ledger/removal evidence updated.

## Following fixture-tooling checkpoint (base b49ba10)

The edit-shapes package launcher moves to Rust and its standalone TS analyzer is
deleted: **1 file, 355 lines, 13,617 bytes**. No TS sources are added. Net baseline
TS/TSX deletion paths become 61; raw tracked TS/TSX files become 4,954 with
55,470,974 blob bytes. These are source changes, not verified capability progress.
The source ledger records removal with partial/unverified replacement. Remaining
generator and metaharness consumers retain their TS hunk/task modules.
