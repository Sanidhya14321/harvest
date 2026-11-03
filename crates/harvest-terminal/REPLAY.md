# Rust terminal replay foundation

`frame` consumes application-owned history offers and a complete mutable
viewport. It serializes a replay into one packet, bottom-splits the history
suffix into unused viewport space, and diffs subsequent viewport updates.
The provider retains semantic transcript ownership and supplies fresh monotonic
IDs; the writer never infers finality from screen position.

`FrameSink::write_frame` must either enqueue the complete packet in order or
enqueue none. Backpressure and errors leave the history offer and writer state
unchanged. A successful queue acceptance updates writer state and acknowledges
the history ID immediately; physical output drain is a separate host concern.
A partial-write `std::io::Write` adapter cannot satisfy this interface directly.

The host invokes `settle_resize` after terminal geometry and cursor-anchor
recovery. Preserve leaves retained history alone. Append requests a current-width
replay only for a width change. Rebuild requests replay and clears with ED2 before
ED3, including height-only changes. Ordinary scrolling erases the live viewport
without a full-screen clear, avoiding tmux's preserved-screen scrollback path.

`replay` renders a portable `ReplayProjection` through the host's shared
`TextLayout`. Text, headings, tool arguments, summaries, and historical executions
all use that adapter. Images currently get a text fallback. Hidden context,
provider signatures, opaque provider payloads, and unknown bodies stay out of
the terminal view; original persisted records remain authoritative and unchanged.

`render_projection_bounded` preflights displayed text before layout, caps each
logical layout chunk, and limits both accumulated rows and output bytes. Any
overflow returns an error, never a partial successful transcript. The default
`render_projection` uses the same bounded behavior. A host layout adapter still
owns its per-call expansion; bounded chunks prevent a whole large session text
from reaching that adapter at once.

This slice does not replace application Composer/TranscriptStore, editor,
Markdown/math, image protocols, terminal probes/restoration, overlays, hardware
cursor behavior, resize bursts/anchor recovery, or scheduling. Those contracts
and the existing TypeScript implementations remain active until separately
ported and verified. No full TUI parity or TypeScript deletion is implied.
