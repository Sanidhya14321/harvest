# RPC pagination baseline

`rpc-pages.json` is inert input/output data captured by invoking
`pageRpcMessages` in the unchanged legacy implementation at commit `25314f6`.
Source: `packages/coding-agent/src/modes/rpc/rpc-messages.ts`, Git blob
`9c0b8bad0c3667aa12904f7f060738c26e2d8046`.

Each case records messages, snapshot, options and the returned JSON or error.
The cursor strings are compared exactly, including the base64url JSON layout.
Large-message and copy-candidate tests are separate behavioral contracts; this
fixture does not establish transport v2 or whole RPC parity.

`rpc-transport.json` records actual v2 encoder output hashes from legacy
`rpc-frame.ts` at `81de404` (blob
`6a5e0f217e5f3a442e7fd91311966af7cd466247`). Each case constructs a response
object in alphabetical key order: command `get_messages`, data containing
`payload` (the recorded glyph repeated the recorded number of times), id
`oracle`, success `true`, type `response`. The logical JSON hash and every
physical line's byte count/hash include the exact original encoding; physical
hashes include the newline. Chunk ID is `rpc-1`, from a fresh v2 encoder.
The Rust tests reconstruct these large inert inputs from the compact descriptors.
