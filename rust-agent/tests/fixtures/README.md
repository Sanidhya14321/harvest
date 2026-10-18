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
