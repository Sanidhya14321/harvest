# Native evidence-based brains

Project data lives under `.harvest/brain`, `.harvest/skills/learned` and
`.harvest/brain-state`. User data uses `brain`, `skills/learned` and `brain-state`
under the configured agent directory. Existing project `.agents/skills` and user
`managed-skills` are indexed too. Markdown is authoritative; BM25 postings and
one-hop graph edges are rebuildable in-memory caches.

```sh
cargo run -p harvest-omp --bin harvest -- brain --scope project learn --title "Verification" --file lesson.md
cargo run -p harvest-omp --bin harvest -- brain --scope project retrieve "verification" --json
cargo run -p harvest-omp --bin harvest -- brain --scope both organize --dry-run --json
cargo run -p harvest-omp --bin harvest -- /organize-skills --scope both --json
cargo run -p harvest-omp --bin harvest -- brain restore RESTORATION_ID
cargo run -p harvest-omp --bin harvest -- brain purge RESTORATION_ID
```

Use `--workspace` and `--agent-dir` to select existing roots. User defaults to
`HARVEST_CODING_AGENT_DIR`, `PI_CODING_AGENT_DIR`, the prototype's
`HARVEST_AGENT_DIR`, then `~/.harvest/agent`, in that order. Named profiles and
XDG settings still require the compatible settings port; pass `--agent-dir`
explicitly for those installations. Learning requires explicit single-scope
selection. Selecting user scope for `brain learn` expresses cross-project intent.
Rust callers record evidence through a trusted host API; evidence recording is
not exposed as a model tool. Future automatic proposals must reference committed
ledger events bound to their exact content/conditions revision.

Admission weights evidence 30, reliability 25, applicability 20, observed utility
15 and compactness 10. Admission needs 70 points plus attributable explicit intent
or two independent verified sessions. Scores below 50 are discarded, otherwise
unqualified records remain candidates and are excluded from retrieval. Exact
normalized duplicates merge evidence. Paraphrases never justify deletion.

Generated content is capped at 256 records/4 MiB per scope; archives at 4 MiB.
Capacity pauses admission rather than evicting records. Organization protects
manual edits and required dependencies, even across scope boundaries. Otherwise
retirement needs explicit contradiction/supersession, unsafe content, or 30 days
plus five relevant presentations across three sessions with no confirmed benefit.
Age and low retrieval frequency alone never authorize retirement.

Maintenance uses writer locks, content hashes and a persisted recovery journal.
A pending transaction must recover before indexing. Restore reinstates exact
original bytes and refuses collisions. Purge is explicit and irreversible.
Archive metadata includes the reason, original receipt and evidence snapshot.

The privacy/authority matchers are conservative heuristics, not semantic proof
that arbitrary text is safe. Retrieval cannot grant permissions; the host owns
approval and filesystem access. Verified application success is evidence of an
outcome, not proof that a lesson caused that outcome. Inferred global admission
needs three distinct trusted repository identities. Identity uncertainty blocks
promotion. Production automatic extraction and its event-loop integration remain
pending; the CLI currently learns explicit user-supplied lessons.
