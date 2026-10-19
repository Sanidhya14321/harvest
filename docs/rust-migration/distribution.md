# Native distribution contracts

`cargo xtask package matrix` emits the GitHub Actions `include` object used by
native release and cross-check jobs. `xtask/src/artifacts.rs` owns the target,
runner, executable, GNU Bazel target and asset identities. The matrix covers GNU and musl Linux
on x64/ARM64, macOS on Intel/Apple Silicon, and native Windows on x64/ARM64.
The macOS ARM runner uses GitHub's documented `macos-15` label; runner labels
were checked against the [official runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).

Raw assets retain existing installer names: `omp-linux-{x64,arm64}`,
`omp-linux-musl-{x64,arm64}`, `omp-darwin-{x64,arm64}`, and
`omp-windows-{x64,arm64}.exe`. A same-stem `.zip` accompanies each raw asset.
Each ZIP carries `omp`/`harvest` (or their `.exe` counterparts), `LICENSE` and
`THIRD-PARTY-NOTICES.txt`. Entries are regular files with fixed timestamps and
explicit permissions. Both alias payloads must match the raw binary; local and
central ZIP headers must agree. The bounded validator rejects missing targets,
changed aliases, ambiguous headers and different legal notices.

```sh
cargo xtask package archive --target x86_64-unknown-linux-gnu --binary target/release/omp --output-dir dist
cargo xtask package verify-assets --directory dist
cargo xtask checksums dist/SHA256SUMS.txt dist/omp-*
cargo xtask package homebrew --version 1.2.3 --checksums dist/SHA256SUMS.txt --output omp.rb
cargo xtask package winget --version 1.2.3 --release-date 2026-10-19 --checksums dist/SHA256SUMS.txt --archive-output dist/omp-winget-manifests.zip
```

Regenerate checksums after adding the manifest ZIP. The WinGet submission helper
calls `package verify-winget` before any submission operation. This verifies the
actual manifest ZIP and both Windows ZIP hashes, parses the known YAML files,
and requires data equivalence with native-generated manifests for the version,
release date, aliases, architecture-specific URLs and checksums. Extra fields
are rejected. Only the three known, validated manifest names are extracted.
The helper was not used to submit or message any remote repository during local
verification.

Both native bootstrap installers use `SHA256SUMS.txt`. Linux selection detects
musl; macOS detects Rosetta; Windows ARM64 selects its native artifact. Aliases
are installed independently of completion registration. An existing unrelated
`harvest` command causes refusal before replacing either installed command.
Completion setup accepts matching real binaries, hardlinks and relative package
manager symlinks. Per-target cargo-binstall ZIP overrides follow its
[documented metadata contract](https://github.com/cargo-bins/cargo-binstall/blob/main/SUPPORT.md).

## Evidence and remaining cutover constraints

GNU shipping binaries use `bazel build //:app-linux-x64` or
`//:app-linux-arm64`. These application targets select the existing Zig-based
GLIBC 2.17 platform and Cargo's shipping codegen profile (fat LTO, one codegen
unit, stripped symbols). Addon builds retain their separate CI profile. Bazel
and Cargo use the workspace's pinned nightly; CLI version metadata comes from
the inherited workspace version in either build system.

After staging the artifact inside the workspace, run:

```sh
cargo xtask package verify-glibc --binary portable/omp --max-glibc 2.17
HARVEST_TEST_BINARY="$PWD/portable/omp" cargo xtask test -p harvest-omp --test cli --all-features
sh rust-agent/tests/portable-gnu.sh portable/omp
```

The ABI checker passes an authorized read-only descriptor to Linux `readelf`,
compares numeric symbol versions, and rejects unsupported or newer requirements.
The release job enforces this ceiling before uploading either GNU artifact.
CI also executes the provider, file-read, session, streamed-resume and brain
contracts against the actual shipping binary, with no helpers on its PATH.
The last command repeats those contracts in a digest-pinned CentOS 7 container
with GLIBC 2.17, a read-only root, no capabilities, and only disposable test
fixtures mounted. Host networking reaches the fixtures' loopback-only model.
The helper selects a pinned architecture manifest. For an emulated ARM64 run,
set `HARVEST_PORTABLE_PLATFORM=linux/arm64` and
`HARVEST_PORTABLE_EMULATOR=/absolute/path/to/qemu-aarch64-static`; the emulator
is copied into the disposable fixture tree. No host kernel registration or
privileged container is needed.

The pinned compiler requests the Cortex-A53 843419 workaround on AArch64.
Zig 0.14's driver does not accept that flag, so the shared driver stages its
link privately and uses the same embedded LLD with its lossless reproducer to
perform the final protected link. Extraction bounds bytes and entries and
rejects traversal, links and duplicate files. The completed binary replaces the
requested output atomically. A self-checking AArch64 assembly fixture, built
from a path containing spaces, exits successfully only when the vulnerable
load has been replaced by the workaround thunk; ARM CI and release jobs run it
alongside an unprotected negative control that must exit with status 1.

Local Linux verification used the default `full` release profile, including
shell execution. The release build took 4m06s in this workspace and produced
120,816,344-byte aliases. These are measurements of this build, not a comparison
with a captured legacy performance baseline. Real cargo-binstall 1.26.0 was
verified against its official release-asset SHA-256 and consumed the per-target
metadata through a loopback server. Both installed aliases executed and installed
working completions with Bun/Node absent from PATH. Installed xtask exercised
all eight fixture asset identities, ZIP interoperability, checksums, Homebrew,
both WinGet architectures and failure preservation without JS runtimes.

Shell bootstrap fixtures exercised GNU/musl routing, ARM64, Rosetta, aliases,
upgrades, unrelated-command refusal and checksum failures. The PowerShell parser
accepted both scripts, and the actual PowerShell runtime rejected a non-Windows
host before downloads or installation mutation. Native Windows execution is
reserved for its Windows/ARM64 CI jobs; these local checks do not stand in for it.

The host-linked GNU build references GLIBC_2.39 and is rejected by the new gate.
The hermetic x64 shipping build produced a 120,963,496-byte executable with a
maximum GLIBC requirement of 2.17. Six CLI contracts passed both on the local
host and in the GLIBC 2.17 container; cargo-binstall also installed and executed
both aliases from that hermetic build. These are Linux x64 execution results.
Other platform execution, registry publication dependency closure and final
legacy release-owner removal must still be resolved before cutover. The source
and capability gates block native publication until the full migration is
verified; this artifact verification does not bypass them.

The final shared-toolchain build also produced a 118,083,056-byte ARM64 GNU
binary with a maximum GLIBC requirement of 2.17. The same six CLI contracts
passed on the final x64 build in the x64 container and on the ARM64 build under
QEMU 7.2 in the ARM64 GLIBC 2.17 container. The A53 fixture passed under QEMU
and its unprotected control exited with status 1. ARM64 results here are
emulated execution; they do not establish native ARM hardware, macOS or Windows
coverage. Direct driver probes also checked relative/whitespace paths,
repeatable output, caller-owned reproducers, prior-output preservation, and
rejection of traversal, absolute, symlink, duplicate and oversized tar entries.
