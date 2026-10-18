# Harvest terminal protocols

`harvest-tui` owns reusable byte-level keyboard parsing and paste normalization.
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

This library is an input foundation. The existing TypeScript terminal still
owns fragmented-input timers, paste framing, lifecycle, renderer, focus and
resize behavior. A native interactive terminal must verify those contracts
before the legacy implementation can be removed. No terminal backend has been
selected by extracting these protocol primitives.
