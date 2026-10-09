# omp

`omp` is the Rust coding agent for Harvest. It talks to an
OpenAI-compatible chat completions endpoint, then reads, writes, edits,
searches, and runs commands in a workspace.

## Build

```sh
cargo build --locked --manifest-path rust-agent/Cargo.toml
cargo test --locked --manifest-path rust-agent/Cargo.toml
```

## Run

```sh
export OMP_API_KEY=sk-example
omp run --prompt "Summarize README.md" --workspace .
omp tools
omp --version
```

Configuration is read from flags, then `OMP_MODEL`, `OMP_BASE_URL`, `OMP_API_KEY`
or `OPENAI_API_KEY`, then `.omp/config.toml` in the workspace.

## Checks

GitHub Actions for this crate follow the Quinjet set: format, clippy, tests on
four operating systems, rustdoc, MSRV 1.88, feature powerset, cross checks,
coverage, packaging, installers, Homebrew formula rendering, security audits,
and a tagged release build.
