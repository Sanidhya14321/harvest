# Python evaluation migration

Harvest retires its Python evaluation kernel in the Rust-only release. The
`eval` Python backend, direct session Python execution, `user_python` execution
hooks, managed kernel setup and injected Python browser/computer helpers no
longer execute code. Old execution requests return an actionable migration error;
settings and environment flags cannot re-enable a kernel.

Use the normal shell tool when explicitly running user-requested external
programs. Use the Rust SDK and executable plugin protocol for integrations.
Historical Python transcript records and display output remain readable. No
kernel state is carried forward or silently reinterpreted as JavaScript.

JavaScript evaluation remains a separately tracked migration prerequisite while
retained browser/computer consumers still depend on its shared engine. Removing
Python execution does not complete the full language-eval retirement capability.
