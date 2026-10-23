# RPC pagination baseline

`rpc-pages.json` is inert input/output data captured by invoking
`pageRpcMessages` in the unchanged legacy implementation at commit `25314f6`.
Source: `packages/coding-agent/src/modes/rpc/rpc-messages.ts`, Git blob
`9c0b8bad0c3667aa12904f7f060738c26e2d8046`.

Each case records messages, snapshot, options and the returned JSON or error.
The cursor strings are compared exactly, including the base64url JSON layout.
Large-message and copy-candidate tests are separate behavioral contracts; this
fixture does not establish transport v2 or whole RPC parity.
