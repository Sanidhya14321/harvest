# Provider accounting fixtures

`openai-usage-accounting.json` records outputs captured from
`calculateOpenAIUsageAccounting` in
`packages/ai/src/providers/openai-shared.ts` at commit `ac8a116`. Each case names
a distinct wire normalization boundary and supplies the expected token accounting
computed by the legacy helper. Cost calculation is excluded because it depends
on catalog policy that the native provider has not ported.

The Rust suite reads these JSON data fixtures directly. It does not run Bun or
load TypeScript to obtain expected results. Local installed-binary verification
also exercised all seven cases through a loopback SSE provider with Bun/Node
absent from PATH. This is fixture evidence, not a credentialed provider check.

# Session context fixtures

`session-context.json` captures the text model messages produced by
`buildSessionContext` followed by `defaultConvertToLlm` at commit `c671863`.
Eight distinct cases cover ordinary history, kept-tail compaction, a handoff
with no kept turns, reset/compaction precedence in both directions, sibling
branch selection, branch-return summaries and repeated compactions. The Rust
suite consumes the recorded JSON without executing TypeScript.

The native adapter retains summary whitespace rather than running the legacy
shared prompt formatter. Arbitrary Markdown/table/whitespace normalization,
custom messages, inferred model/thinking/mode settings, remote provider replay
and bitmap archives remain outside this text projection's parity evidence.
Active opaque compaction data is rejected; explicit later resets may discard
that context while preserving its on-disk history. Tool pairs must remain
complete after projection; native resume does not strip unmatched calls.
