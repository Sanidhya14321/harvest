# Harvest native browser control

`harvest-browser` is the ordinary Rust CDP engine shared by the native application
and Rust SDK. Chrome/Chromium is an external dependency. It uses a dedicated
persistent profile, native CDP DOM/input/navigation/capture commands and a single
writer. It does not inject JavaScript or use the Chrome relay extension.

Start a native controller with an explicitly chosen executable and new profile:

```text
harvest browser --chrome /usr/bin/chromium --profile /path/to/harvest-browser
```

The controller accepts one structured JSON action per stdin line and writes one
`{"ok":true,"result":...}` or `{"ok":false,"error":...}` response per line.
For example, send `{"action":"open","url":"https://example.org"}`, then use
the returned `tab` ID in `{"action":"query","tab":"...","selector":"h1"}`.

| Action | Fields |
| --- | --- |
| tabs | none |
| open | url |
| navigate | tab, url |
| document | tab |
| query, click | tab, selector |
| type | tab, selector, text |
| screenshot | tab |
| close | tab |

Navigation waits for the matching document load. Screenshots contain PNG base64.
The native session tool stores captures in the jailed workspace and returns a
file path because its current conversation codec is text-only. Large document
results use the session tool's existing bounded text output.

Enable the approved tool in native `run` with `--browser-chrome PATH` and
`--browser-profile DIR`; add `--browser-headless` when needed. The default tool
catalog does not advertise an unconfigured browser. Every model browser call
passes through host approval with the final arguments before launch or action.
Rust embedding uses `AgentSession::configure_browser` or
`SessionEngineOptions::browser`, both sharing the same owner and engine.

The directory must be empty or carry Harvest's ownership marker. Concurrent
owners and unmarked existing profiles are refused. Sign in separately in the
visible dedicated browser. EOF or Ctrl+C closes/reaps owned Chrome while keeping
profile data; SDK owners must be dropped or shut down. Interrupted CDP operations
poison the connection and are never replayed automatically; restart/reconfigure
that owner before proceeding. HTTP(S) and about:blank navigation are supported;
JavaScript `run`/`call` and ordinary-profile attachment are not native actions.

Operations inherit caller cancellation and have a 30-second upper budget;
startup has a 15-second budget. CDP frames are capped at 16 MiB, requests at
128 KiB, screenshots at 12 MiB base64 and tabs at 128. Event retention and input
queues are bounded. Chrome sandboxing is enabled by default. `--no-sandbox` is an
explicit CLI/test option for constrained environments.

The real-browser workflow is Rust and static HTML, with no fixture scripts:

```text
cargo xtask test -p harvest-browser -p harvest-runtime
cargo run --locked -p harvest-browser --example verify_browser -- /path/to/chrome
```

This engine does not certify migration of the remaining legacy browser,
search/PDF, computer, eval or browser-product consumers. Their retirement and
replacement records remain in the migration playbook.
