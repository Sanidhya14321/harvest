# Project and user Markdown brains

Harvest's main SDK agent loop retrieves relevant Markdown knowledge and skills before each model request. Files are durable source material; the section-page index, inverted term index, and graph are disposable session caches. No embedding service or additional model download is required.

## Storage and format

| Scope | Directories |
| --- | --- |
| Project knowledge | `<cwd>/.harvest/brain/**/*.md` |
| Project skills | `<cwd>/.harvest/skills/**/*.md`, `<cwd>/.agents/skills/**/*.md` |
| User knowledge | `<agentDir>/brain/**/*.md` |
| User skills | `<agentDir>/skills/**/*.md` |

`agentDir` is Harvest's configured agent directory (normally `~/.harvest/agent`). User files are shared across projects; project files are selected by the active session working directory. Changing that directory replaces the project index. Missing directories are harmless. Create ordinary Markdown files in these directories to enable retrieval; no files or directories are created automatically. Existing memory backends keep their own lifecycle and storage.

```markdown
---
id: database-migrations
title: Database migrations
depends_on: [user:verification-preferences]
relates_to: [release-checklist]
---
## Apply changes
Run migrations against a staging database before production.

## Verify
Check both forward migration and rollback behavior.
```

```markdown
---
id: verification-preferences
title: Verification preferences
---
## Reporting
Report the command run, its result, and any unverified scenario.
```

An `id` is scoped as `project:<id>` or `user:<id>`. Without an explicit ID, the path relative to its indexed directory is used. Use unique explicit IDs within a scope when linking between files. Bare link targets resolve within the source scope; cross-scope links require a prefix. Relationships use YAML string values or lists: `depends_on`, `relates_to`, `supersedes`, `contradicts`. Only the first two expand retrieval. Conflict and supersession edges record relationships without automatically deciding which claim is true. Mark obsolete documents `superseded: true` to exclude their pages.

Markdown headings produce section pages using the existing section splitter; headings inside fenced code stay in their code block. YAML frontmatter is parsed by the central parser and is excluded from page content. Skill files use the same section indexing, retaining their source paths. `skills.enabled: false` excludes skill directories, including when changed during a session. Reading a skill excerpt does not automatically activate the skill or authorize its actions.

## Retrieval (lexical/graph; Laya removed)

1. Find the latest user message without replacing image or tool protocol blocks.
2. Refresh file metadata at most once every two seconds. Re-read/re-tokenize only changed files; deleted and superseded files leave the index. Concurrent refreshes share one operation.
3. Use inverted term postings to select matching pages, then the existing BM25 scorer to rank up to four pages.
4. Expand one outgoing `depends_on`/`relates_to` hop, up to eight total pages. Cycles cannot recurse.
5. Deterministic lexical/graph order is authoritative. The former Laya `brain_retrieval` rerank step is removed; the `brain.rerank` flag is deprecated and ignored. Cancellation returns no retrieved pages.
6. Add a cited, bounded reference block to the latest user message in outbound context only. The persisted transcript stays unchanged; normal provider secret obfuscation still applies.

The rendered block is capped at 8,000 characters, including guidance and citations. Indexing is bounded to 512 files, 256 KiB per file, 8 MiB aggregate file sizes, 128 sections per document, and 4,096 searchable pages. Read failures are logged and skipped. Directory/file symlinks are skipped and resolved paths must remain within their project or agent-directory jail. These limits make retrieval bounded, but they are not latency benchmarks.

SDK consumers can construct `MarkdownBrain` with explicit `BrainRoot` entries, call `refresh()` after writing files, and call `retrieve()` or `transform()`. `BrainRetrievalOptions` supports cancellation, scoring sanitization, and a smaller context character budget. The default index is in memory and rebuilds on session startup; it is not a persisted external PageIndex service.

## Agent-loop review

The runtime invokes `transformContext` before conversion to provider messages. In the main SDK path the sequence is extension context, steering, brain retrieval, provider conversion, and secret/image transforms (no pruning step). Auxiliary auto-learn capture agents retain their separate context transform.

Tool gating lives in `AgentSession`, checks arguments before execution, and checks again when extensions revise arguments. Unresolved gating requires human approval (fail closed). Invalid probability/confidence values now require approval as well; `NaN` comparisons previously could bypass that safeguard.

Completion classification is connected through `session/unexpected-stop-classifier.ts`, with fallback to its configured classifier. Classification uses heuristics and the main LLM only; there is no sidecar model-routing step. This change does not select a new model based on brain contents.

The existing `GlobalMemoryStore` remains a legacy project-scoped knowledge store. The new user brain supplies an explicit cross-project scope without migrating existing data. Retrieval does not automatically extract lessons, write preferences, resolve contradictions, or promote project knowledge into user storage. Those decisions need explicit authored Markdown or the existing memory workflows; retrieved data never grants tool permission.
