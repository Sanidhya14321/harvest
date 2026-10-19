# Harvest offline tokenizers

`harvest-tokenizer` owns the existing offline counting engine and its embedded
vocabularies. Both the Rust SDK (`harvest_omp::tokenizer`) and the transitional
N-API adapter use this library. It has no N-API, Bun, Node, filesystem or network
runtime dependency; Zstandard decodes trusted embedded vocabulary bytes on first
use.

```rust
use harvest_tokenizer::Encoding;

let tokens = Encoding::O200kBase.count("hello world");
assert_eq!(tokens, 2);
```

`Encoding::count` accepts UTF-8 strings and UTF-16/UTF-32 slices directly.
`encode` returns token IDs for the BPE families; Claude reconstruction families
return `None`, since they reconstruct counts without token boundaries. Counts
cover ordinary content; callers own chat framing, persisted accounting identity,
limits and cancellation around their work. Encoding selection remains a catalog
policy responsibility, rather than a model-name heuristic in this crate.

The eight compressed vocabulary files moved without byte changes. Source,
format and regeneration instructions live in [data/README.md](data/README.md);
the existing regeneration tools remain under `../pi-natives/tools` until their
own Rust migration. Golden references and scanner/UTF differentials moved with
the engine. Five Claude corpus texts had been renamed without refreshing their
expected counts; all 1,479 family expectations were checked against pinned
Python `ctok==1.0.0`, correcting the 15 stale values without changing runtime
counting behavior. These offline references do not establish live-provider or
whole-agent token-budget parity.
