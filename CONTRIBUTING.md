# Contributing to Harvest

Thank you for contributing to Harvest. Bug reports, documentation, tests, code,
and thoughtful design feedback all help improve the project. Contributions are
welcome from everyone; no invitation or vouch is required.

This guide explains how to prepare a focused contribution, verify it, and work
with maintainers through review.

## Before you start

- Read the [README](README.md), the relevant feature documentation under
  [docs](docs/), and the repository's [development rules](AGENTS.md).
- Search existing [issues](https://github.com/Sanidhya14321/harvest/issues) and
  [pull requests](https://github.com/Sanidhya14321/harvest/pulls) for related work.
- Small bug fixes, documentation updates, and narrowly scoped improvements can
  go directly to a pull request.
- Discuss major features, new dependencies, architectural changes, and changes
  spanning several packages with Harvest maintainers before implementation.
  Use the relevant existing discussion or a proposal pull request to agree on
  scope. Prior discussion does not guarantee acceptance.

If you are already implementing a change, do not open a separate actionable
issue for the same work: repository automation may pick it up independently.
Link an existing issue in your pull request when one applies.

## Reporting a problem

Include a concise description, steps to reproduce, expected and actual
behavior, Harvest version, operating system, terminal, and relevant provider or
configuration details. Provide a minimal example and sanitized logs or
screenshots when they help explain the problem. Remove credentials, tokens,
private repository content, and other sensitive data before posting.

For a suspected security vulnerability, use
[GitHub's private vulnerability reporting](https://github.com/Sanidhya14321/harvest/security/advisories/new)
if it is enabled. Otherwise, request a private reporting channel from a
maintainer before sharing exploit details publicly.

## Development setup

Install Git, [Bun](https://bun.sh/), the Rust toolchain specified in
[rust-toolchain.toml](rust-toolchain.toml), and the platform build tools required
for native compilation. On macOS, install Xcode Command Line Tools first.

Fork the repository, clone your fork, and create a branch for your change. From
the repository root, run:

```sh
bun setup
bun dev
```

`bun setup` installs workspace dependencies and builds the native addon. After
changing Rust crates or native bindings, run `bun run build:native` again.
For application structure and subsystem references, see the
[development guide](packages/coding-agent/DEVELOPMENT.md). The experimental Rust
runtime has its own [setup and compatibility notes](rust-agent/README.md).

## Preparing your change

Keep each pull request to one logical change. Follow existing conventions and
reuse shared utilities. Avoid unrelated cleanup, broad formatting changes,
dependency updates, and generated noise.

Add or update documentation when behavior, configuration, commands, or public
interfaces change. Add focused regression coverage for bug fixes and meaningful
behavioral changes. For user-visible package changes, add a concise entry under
`[Unreleased]` in the affected package's changelog; do not rewrite released
entries. Documentation-only changes generally do not need a package changelog.

Do not hand-edit generated catalog files. Update their sources and run the
appropriate generator, such as `bun run gen:compat` or `bun run gen:models`,
including the resulting outputs with the change.

## Verification

Run checks appropriate to the affected area. Common commands from the root are:

| Purpose                                 | Command                                           |
| --------------------------------------- | ------------------------------------------------- |
| Application lint, formatting, and types | `bun --cwd=packages/coding-agent run check`       |
| Application types only                  | `bun --cwd=packages/coding-agent run check:types` |
| Focused TypeScript test                 | `bun test <path-to-test>`                         |
| TypeScript CI suite                     | `bun scripts/ci-test-ts.ts <suite>`               |
| Rust tests and doctests                 | `bun run test:rs`                                 |
| Source CLI startup                      | `bun dev -- --version`                            |

Use package check commands for type checking; do not invoke the TypeScript
compiler directly. Use the CI runner for full TypeScript suites and the
repository runner for Rust tests. Check the relevant package's documentation
for additional requirements.

Verify the changed behavior as well as the automated checks. For a bug fix,
reproduce the original failure and repeat the same scenario after the fix.
For a feature, exercise the workflow end to end. For a UI change, interact with
the interface and inspect the result. For documentation, check commands,
relative links, image paths, and rendered formatting.

Report the exact scenario, command, and result in the pull request. State any
checks you could not run and any known failures; a passing type check alone
does not establish that a feature works.

## Pull request requirements

Use a descriptive title and include:

- The problem or motivation and the resulting behavior.
- At least one sentence written by you, in your own words, explaining what
  changed and why. A generated summary, transcript, or checklist alone does not
  satisfy this requirement.
- Relevant issue or proposal links, when available.
- Verification commands, observed results, and remaining limitations.
- Screenshots or terminal captures for visible UI changes, with sensitive
  information removed and fixture data identified.
- Migration or compatibility notes when existing users are affected.

Review every changed file before submitting. Respond to maintainer feedback
and apply suggestions only after checking them. Maintainers may request changes
or close submissions that lack verification, skip required discussion, contain
unreviewed output, or combine unrelated work.

## AI-assisted contributions

AI tools may assist your work. You remain responsible for its correctness,
scope, licensing, and maintenance. Give tools a clear scope, review the full
diff, reject unrelated changes, and understand the resulting behavior.

Run the relevant checks and exercise the change yourself before submission.
Submit only after that review; do not let an agent publish unreviewed work.
Write your own explanation and answer review questions yourself.

## Contribution licensing and attribution

By intentionally submitting a contribution for inclusion in Harvest, you agree
to license it under the [MIT License](LICENSE). You must have the right to
submit the material. No Contributor License Agreement (CLA) or Developer
Certificate of Origin (DCO) sign-off is required.

This policy does not relicense third-party or vendored material. Preserve all
applicable copyright, license, attribution, and notice files. Identify the
source and license of any third-party material you add, including assets and
documentation examples.

Contributors retain copyright in their original work. The root license names
Harvest's recorded human contributors individually: Sanidhya Vats, Naman, and
Pulkit. Inherited copyright and permission notices are preserved in
[third-party notices](THIRD-PARTY-NOTICES.txt). Attribution does not transfer
ownership or replace applicable notices. When adding contributor attribution,
use verified names from the contribution history and preserve existing rights.
Recorded code and documentation contributions appear in the
[contributor list](https://github.com/Sanidhya14321/harvest/graphs/contributors)
and repository history.

## Working together

Keep discussion respectful, specific, and focused on the work. Explain tradeoffs,
provide reproducible evidence, and allow time for review. Contributions of any
size are valuable when they improve Harvest and are ready for others to use
and maintain.
