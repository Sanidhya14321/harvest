Submit subagent output: `{ data: <your output> }` for success, `{ error: "message" }` for failure. Never both; never a bare payload outside `data`.

When `data` is omitted, your last assistant turn becomes the raw final result.
