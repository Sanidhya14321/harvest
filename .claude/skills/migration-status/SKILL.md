---
name: migration-status
description: Report the honest current state of the TypeScript-to-Rust migration from the executable gates (source ledger + capability inventory). Use when asked "how far is the migration", before choosing a next slice, or before claiming any progress.
---

# Migration status

Numbers come only from the executable gates — never estimate a percentage.

1. Run from repo root:
   ```sh
   cargo xtask migration
   cargo xtask capabilities
   ```
   `--require-complete` on either is the final-acceptance gate; it is *expected to fail* until the migration is done. Never weaken it.
2. Inspect states in `docs/rust-migration/source-ledger.json` (`entries[]`: `disposition`, `implementation`, `behavioral_verification`, `installed_verification`, `legacy_removal`, `contract`, `consumers`, `rust_destination`, ...) and `docs/rust-migration/capabilities.json` (`id`, `implementation.status`, `verification.status`). Group by top-level directory to find clusters:
   ```sh
   python3 -I -c "import json,collections;e=json.load(open('docs/rust-migration/source-ledger.json'))['entries'];c=collections.Counter((x['path'].split('/')[0]+'/'+x['path'].split('/')[1] if '/' in x['path'] else x['path'], x['implementation']) for x in e);[print(k,v) for k,v in c.most_common(40)]"
   ```
3. Cross-check with `docs/rust-migration/README.md` (implemented slices) and `HANDOFF.md` (next investigated slice, remaining checklist).
4. Report: migrated/verified counts, partial items, the recommended next slice and why (prefer leaf code with few consumers, or a shared dependency that unblocks many entries). State plainly that the shipped app still runs TypeScript unless the gates prove otherwise.
