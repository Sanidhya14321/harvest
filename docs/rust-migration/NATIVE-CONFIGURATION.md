# Native configuration conversion checkpoint

Code is written and connected; behavioral verification and legacy settings
compatibility remain pending. This does not replace the full TS settings schema.

Native precedence, lowest first: defaults; selected user agent `config.toml`;
project `.omp/config.toml`; project `.harvest/config.toml`; ordered explicit
`--config` overlays; selected profile; environment; command-line flags.
`--profile` overrides `OMP_PROFILE`, which overrides the document's `profile`.
Profiles inherit with `extends`; unknown profiles and cycles are errors. Later
layers replace plugin arrays. Explicit missing overlays are errors.

```toml
model = "your-model"
base_url = "https://your-host/v1"
max_steps = 8
api = "openai-chat"
max_output_tokens = 4096
profile = "development"

[profiles.development]
max_steps = 16

[profiles.review]
extends = "development"
max_steps = 4
```

The same resolution serves sync/async `run`, `rpc`, SDK `config::resolve` and
`configuration::load`. `harvest config --profile review` prints redacted resolved
values without starting plugins or showing credentials, URL secrets or plugin
environment/arguments. Config reads are bounded and capability-relative; parser
errors omit source excerpts. Existing files are never rewritten automatically.

Native plugin configuration:

```toml
[[plugins]]
executable = "/absolute/path/to/native-plugin"
arguments = []
timeout_ms = 30000
```

Run/RPC require explicit `--enable-plugins` before starting these configured
programs. `--plugin PATH` selects a native executable directly. See the executable
protocol document. API/model/host selection is explicit; no provider policy is
inferred from a model name.

Pending conversion: full existing settings discovery/migration and schema, settings
schema and validation, external config-source precedence, profile interoperability,
settings UI/edit persistence, credential ownership/OAuth, and KDL catalog policy.
Retain TS settings sources and consumers until these contracts move.

Native MCP configuration and CLI now use this same resolver; `[[mcp]]` arrays
replace earlier layers, like plugins. Runtime startup requires `--enable-mcp`,
while explicit `--mcp-config` JSON files are host-selected inputs. Duplicate names
are errors. Inspection reports a server count and never connects. See
[NATIVE-MCP.md](NATIVE-MCP.md) for pending discovery/OAuth/legacy format behavior.

Native `[[lsp]]`/`[[dap]]` arrays now share these layers, replacing earlier arrays
and validating names/limits. They are exposed to model tools only when the host
selects --enable-lsp/--enable-debug; configuration inspection starts neither.
See [NATIVE-LSP-DAP.md](NATIVE-LSP-DAP.md) for lazy ownership and remaining work.

## JSONC and YAML overlay continuation

Base `7f06083`; code-written/connected, not behaviorally verified. The existing
`configuration::load` reader now delegates to `configuration/document.rs` for
explicit `--config file.json`, `.jsonc`, `.yaml` and `.yml` overlays. The same
native schema, profile merge, precedence and host startup selection still apply.
Automatic user/project discovery continues reading the existing TOML paths;
this does not reinterpret old settings files or migrate them automatically.
Other extensions retain the existing TOML decoding behavior.

```text
harvest config --config ./review.jsonc --profile review
harvest run --config ./native-services.yaml
```

JSON/JSONC accepts line/block comments and trailing commas outside strings,
while preserving byte positions/newlines for diagnostics. URLs, escaped quotes,
comment markers and commas inside strings stay intact. Unterminated comments,
invalid JSON and multiple values fail. YAML accepts one mapping document, with
bounded alias expansion. Complex keys, duplicate keys, merge keys, tagged values,
non-finite numbers and non-object roots fail rather than acquiring implicit
configuration meaning. JSONC/YAML decoding caps at 256 KiB source, 16,384 scalar/
container/key nodes, depth 64 and 1 MiB cumulative decoded string/key text. The
budget applies during deserialization, before an expanded tree is allocated.
Existing TOML parsing and source bounds remain as before.

Parsing errors expose configuration identity and location when available; schema
errors are generic. Raw parser messages, source excerpts, offending scalar values
and attached error chains are discarded to preserve credential redaction. No
write, cache, relocation or migration is added. `serde_yaml_ng` reuses the existing
locked workspace dependency; standard native loading needs no legacy runtime.

Stabilization must cover equivalent native documents through config/run/RPC/SDK,
ordered mixed-format overlays and profile inheritance, URL/string escapes,
comment/trailing-comma edge cases (including invalid empty-container commas),
malformed/multiple YAML documents, aliases/cycles/depth/node/text limits,
duplicate/complex/merge/tagged keys, non-finite values, missing/non-UTF8 input,
wrong types and credential-bearing parse failures. Preserve input bytes and
existing TOML behavior. Legacy JSON-to-YAML migration, source fallback selection,
full settings schemas, cache/invalidation/relocation and persistent editing remain
tracked prerequisites before deleting `config/config-file.ts`.
