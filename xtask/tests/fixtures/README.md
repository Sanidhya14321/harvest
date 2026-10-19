# Release-note oracle

`release-notes.json` captures `mergePackageSection` from the legacy
`scripts/ci-release-notes.ts` at commit `153bc7c`, before its deletion. Seven
cases cover range boundaries, unpublished tags, newest-first deduplication,
category ordering, continuation lines, absent targets, single-version behavior
and non-version heading boundaries. Rust tests consume the JSON directly; they
do not execute Bun or load the deleted TypeScript implementation.

The replacement additionally includes native Cargo package changelogs, validates
version inputs, sorts unknown categories by deterministic UTF-8 order and rejects
source/output collisions and escaping paths. GitHub selection is tested with a
bounded published-release snapshot and a local substitute for the gh executable;
those checks do not publish or inspect an authenticated remote release.
