# media:docx — shared native DOCX conversion

Prerequisite: `control:playbook`. Capability `surface:media-local-inference`
stays partial. Sources: `packages/utils/src/docx/converter.ts` and `docx/xml.ts`.
The original sources and four behavioral fixtures are captured from `043709e`.

Contract: preserve headings, run formatting and inheritance, nested numbering,
hyperlinks, merged tables, footnotes, style maps, diagnostics and inline/custom
images. No external relationship is fetched and archive contents never extract
to disk. Bound ZIP input, declared and actual expansion, entry count, XML nodes,
XML/render depth, table dimensions, callback payloads and output. Reject malformed
archives/XML, resource exhaustion and cancellation rather than returning partial
success. Preserve diagnostic order and callback image order, including repeated
references. Native reads remain jailed and cancellation-aware.

Destination: `harvest_media::docx`; direct native application read/document APIs,
Rust SDK, and thin N-API prepare/finish adapters. The existing `utils/src/docx.ts`
becomes a temporary callback/file-loading boundary; it contains no XML, archive,
style, table or HTML rendering engine. Its retirement stays with `migrate:primitives`
until the final TS Markit consumer moves. Generated JS bindings remain temporary.

Steps: capture baseline fixtures/results through the actual legacy API; port the
engine to Rust; verify exact normal outputs plus failure/resource contracts;
connect native jailed reads and standalone conversion; connect the existing
Markit import through the N-API bridge; rebuild bindings and run DOCX/Markit host
checks; delete only the converter and its private XML helper after all their
consumers move. Keep independent XML, archive, DOM and Markdown engines while
other consumers need them. The callback may perform its original caller-owned
work; Rust owns image attribute escaping and final rendering.

Verification:

```text
cargo fmt -p harvest-media -p pi-natives -p harvest-omp -- --check
cargo clippy --locked -p harvest-media -p harvest-omp --all-targets --all-features -- -D warnings
cargo xtask test -p harvest-media
cargo xtask test -p harvest-omp --no-default-features --features core
bun run build:native
bun test packages/utils/test/docx.test.ts packages/coding-agent/test/utils/markit-cache.test.ts
bun run --cwd packages/coding-agent check:types
cargo xtask migration
cargo xtask capabilities
cargo xtask playbook check
cargo xtask source-audit verify --scope affected --base 043709e --plan
```

Also verify the copied native application with an empty PATH against captured
DOCX fixtures: HTML/Markdown output, jailed read, malformed input refusal and
unchanged source bytes. The source-bound full transition at `043709e` passed
471 Rust contracts, five doctests, the native rebuild and all 49 workspace groups;
its 22 singleton failures exactly match the recorded baseline and remain failed.
Do not weaken those unrelated contracts or claim complete media parity.

Recovery: revert the coherent engine/consumer/ledger slice, restoring sources
from the preceding commit. Keep all original DOCX input bytes and fixtures.
Completion requires baseline parity, native SDK/application and rebuilt Markit
consumer evidence, eligible staged removals, and honest task/capability records.

Custom image attributes use a deterministic byte-sorted order in Rust and values
are HTML-escaped. Attribute names must be ASCII letters/digits or `_:-`;
malformed names now fail rather than being inserted verbatim into HTML. These
explicit bounds/rejections are deliberate contract tightening, not claims that
legacy malformed-input behavior is preserved. The captured normal documents
retain exact HTML and diagnostic outputs.

## Verified checkpoint and remaining gates

After integrating `6eefffc`, 136 media/core application tests plus one doctest,
eight copied-binary CLI contracts, 11 DOCX/Markit host tests, native/coding-agent
typechecks and all 49 workspace groups pass. Strict media/application Clippy and
formatting pass. The corrected Rust host detector rebuilt the addon with 139
exports. Both private legacy sources (847 lines) are removed; temporary public
callback transport and Markit remain pending. `tooling:host-detect` is the verified
build prerequisite. Normal baseline fixtures are captured from `043709e`.

The broader native suite passes three groups but fails six TUI contracts; isolated
partial-OSC input failure reproduces while isolated ConPTY disambiguation passes,
indicating cleanup cascades must be investigated. Whole-addon strict Clippy has
14 diagnostics outside the DOCX adapter. The inherited deletion gate also fails.
These failures stay recorded, not waived; see LEGACY-TERMINAL-CONTRACTS.md and
CONCURRENT-REMOVALS.md. Focused verification does not certify full product parity.
