# Native Messages wire fixtures

`text-usage.sse` is synthetic inert response data, not executable code or a live
credentialed capture. Its event/usage fields follow the retained wire definitions
in `packages/ai/src/providers/anthropic-wire.ts` and split usage handling in
`anthropic.ts` at baseline `3e38027`. `future: 1e400` intentionally exercises
lossless unknown-number retention. It does not establish whole-provider parity.
