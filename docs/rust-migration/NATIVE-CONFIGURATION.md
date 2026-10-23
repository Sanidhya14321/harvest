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

Pending conversion: full existing YAML/JSONC settings loading/migration, settings
schema and validation, external config-source precedence, profile interoperability,
settings UI/edit persistence, credential ownership/OAuth, and KDL catalog policy.
Retain TS settings sources and consumers until these contracts move.
