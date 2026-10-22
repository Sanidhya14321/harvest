Control the host's dedicated persistent Chrome profile using native CDP commands.
Requires human approval. Use action `tabs`, `open` (url), `navigate` (tab, url),
`document` (tab), `query`/`click` (tab, selector), `type` (tab, selector, text),
`screenshot` (tab), or `close` (tab). Use the returned tab ID in later calls.
Navigation waits for load. URLs use HTTP/HTTPS or about:blank. Selectors are CSS.
JavaScript run/call, injected scripts and the browser extension relay are retired.
Screenshots return a PNG file path in the workspace. Authentication is separate
in this dedicated profile. Browser options/profile paths are configured by the host.
