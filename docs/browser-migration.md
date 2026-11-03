# Native browser control

The Chrome extension relay and `harvest browser-relay` have been removed on
`ts-to-rust`. Use the native Rust candidate's controller:

```text
harvest browser --chrome /absolute/path/to/chrome --profile /new/dedicated/directory
```

Use the native `harvest`/`omp` binary; the transitional Bun CLI does not implement
this controller. Build it with `cargo build --locked -p harvest-omp --bins` while
native release installation remains pending. Add `--headless` for automation.
The new directory must be empty; Harvest marks and locks it. Existing ordinary
Chrome profiles are refused. Sign in separately in the visible dedicated browser.
Profile data survives shutdown. EOF or Ctrl+C closes the owned browser process.

Send one structured JSON object per stdin line, for example:

```json
{"action":"open","url":"https://example.org"}
```

Use the returned `tab` ID for document/query/click/type/screenshot/close actions.
See [the native API](../crates/harvest-browser/README.md) for fields and bounds.
The Rust SDK and native sessions share this engine and host approval boundary.
Native `run` enables the browser tool with `--browser-chrome PATH` and
`--browser-profile DIR`. JavaScript `run`/`call` are not native browser actions.

Remove `browser.relay` and `browser.relayUrl` from configuration, unset
`PI_BROWSER_RELAY`, and remove `app.relay` from old calls. Enabled relay requests
return this migration guidance; they never install or auto-start a relay.
An existing extension can be removed manually through Chrome's extension manager.
Harvest does not uninstall user extensions or delete old browser profiles.

Other transitional Puppeteer/search/PDF/computer consumers remain tracked migration
work. This retirement does not certify complete browser or whole-product parity.
