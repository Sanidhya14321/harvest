# Harvest terminal protocols

`harvest-tui` owns reusable byte-level keyboard parsing, bounded terminal framing and paste normalization.
It has no JavaScript runtime, terminal ownership, process-global protocol switch,
or dependency on the agent execution loop. The Rust SDK exposes it as
`harvest_omp::tui`; the existing TUI reaches the same implementation through
thin N-API adapters.

Editor-normalized `matches_input_key` and `parse_input_key` take an explicit
`InputMode`, combining keypad text precedence and Backspace policy in one call.
The N-API bridge uses those entry points so key matching does not add a second
native round trip. Lower-level `matches_key` and `parse_key` retain protocol
semantics for consumers that manage their own input policy.

```rust
use harvest_tui::keys::{decode_printable_key, matches_key};

assert!(matches_key(b"\x1b[97;5u", "ctrl+a", true));
assert_eq!(decode_printable_key(b"\x1b[49:33;2u").as_deref(), Some("!"));
```

The key parser retains legacy, Kitty, xterm modifyOtherKeys, keypad, alternate
layout, lock-bit and press/repeat/release behavior. Text insertion rejects
bindings, release events, invalid scalars and incomplete modifyOtherKeys
sequences. Its explicit modifier-field grammar preserves the existing editor's
behavior; the lower-level Kitty parser also accepts empty modifier fields.
Paste normalization decodes tmux's two Ctrl+letter encodings before consumers
strip controls, preserving pasted line breaks and tabs without synthesizing
non-letter control bytes.

Run contracts with `cargo xtask test -p harvest-tui`, including the public API
doctest, or `bazel test //crates/harvest-tui:input_test` for the hermetic unit
harness. Existing TUI key/editor/paste tests continue exercising the N-API path.

`input::InputBuffer` frames split CSI/SS3/mouse/string sequences and bracketed
pastes, groups raw multiline input, and recovers from lost paste/string
terminators. Its limits count bytes. A host supplies a monotonic `Duration`,
calls `tick` at `next_delay`, and grants a second timer turn for Escape expiry so
already queued I/O can complete a sequence. Split reads above `MAX_INPUT_CHUNK`
with `end_of_read=false` except on the last chunk; this keeps same-read paste
and Enter delivery atomic. An oversized call fails before changing state.
`clear` releases buffered payloads and deadlines; diagnostics omit payloads.

The TypeScript `StdinBuffer` now only schedules host timers, converts its legacy
text input convention, and emits events from this engine. Existing framing,
watchdog, malformed-stream and editor contracts exercise that native bridge.
`output::ConptyChunks` yields borrowed UTF-8 writes under a byte limit,
preferring line boundaries. A scalar wider than a deliberately tiny limit is
emitted intact. `output::StallWatchdog` keeps watching an unhealthy backlog
below its original arm threshold and restarts its deadline only at new
low-water marks. Hosts supply monotonic elapsed time. The legacy terminal uses
the same policy and slices original JS strings at Rust-computed UTF-16 offsets,
so isolated surrogate code units survive until output encoding. Real Windows
ConPTY execution remains necessary in addition to platform-routing tests.

`writer::TtyWriter` owns an unbuffered `Write + Send` backend on a dedicated
thread and is usable without N-API. A default 256 MiB ceiling includes both
queued and in-flight bytes; hosts can lower it. Oversized frames fail as a
whole before publication. Bytes are accounted before queue publication; empty-queue notifications pair
with the waiter lock, preventing counter underflow and missed drain wakeups.
Each backend call is at most 16 KiB. Short/interrupted writes preserve their
unwritten tail; failures close admission and discard remaining output.

`stop` closes admission before its bounded flush. A blocked backend operation
can outlive a timed-out stop. Once it completes, the worker drops the remaining
output and releases its backend. The Unix
N-API adapter duplicates descriptors with close-on-exec and uses the same
queue with bounded scratch/spill UTF-16 conversion. It remains Unix-only;
Windows still uses the existing host output route. Real socket, controlled
blocked-write and legacy renderer contracts cover this path; actual Windows
terminal execution and full native lifecycle/restoration remain pending.

This library is an input foundation. The existing TypeScript terminal still
owns terminal lifecycle, renderer, focus and resize behavior. A native interactive terminal must verify those contracts
before the legacy implementation can be removed. No terminal backend has been
selected by extracting these protocol primitives.

A local warm microbenchmark (nine alternating samples after three warmups,
60,000 mixed printable/key/mouse/OSC/paste reads per sample, equal 440,000
payload bytes) measured the old Bun 1.3.10 framer at a median 8.44 ms and the
N-API adapter at 75.57 ms. The optimized pure Rust engine measured 8.93 ms
with a monotonic clock and deadline query on each feed. These isolate the
compatibility bridge cost: the legacy path is slower, and this is not an
end-to-end terminal performance improvement or a shipping performance gate.
The Rust application can use the engine directly without crossing N-API.

A separate warm output probe used 100 paints of 2,000 ANSI/CJK/emoji rows per
sample (17,800,800 UTF-16 units), three warmups and nine alternating samples.
The legacy chunker median was 31.71 ms; the native bridge median was 23.66 ms.
A deterministic legacy comparison covered 1,152 chunk cases, including isolated
surrogates, and 40,000 backlog state transitions without differences. This
isolates chunking and does not certify rendering, actual Windows execution, or
whole-application performance.

A local output-pump probe used 100 writes of a 2,000-row ANSI/CJK/emoji frame
(18,600,800 bytes per sample) to a real regular file, followed by bounded drain.
Separate fresh processes each ran three warmups and nine measured samples.
The old adapter median was 16.53 ms; the shared-pump median was 11.51 ms. This
measures local file output and transcoding, not a terminal emulator or Windows.
The legacy N-API adapter retains its permissive lone-surrogate byte encoding;
Rust `enqueue` callers provide explicit byte frames.
