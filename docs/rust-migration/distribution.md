# Native distribution contracts

`cargo xtask package matrix` emits the GitHub Actions `include` object used by
native release and cross-check jobs. `xtask/src/artifacts.rs` owns the target,
runner, executable and asset identities. The matrix covers GNU and musl Linux
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

The local host-linked GNU build references GLIBC_2.39. The existing native build
contract has a GLIBC_2.17 portability floor, so that host build is not portability
proof. The hermetic GNU toolchain, full platform execution, registry publication
dependency closure and final legacy release-owner removal must be resolved before
cutover. The source and capability gates still block native publication until the
full migration is verified; this asset contract does not bypass them.
