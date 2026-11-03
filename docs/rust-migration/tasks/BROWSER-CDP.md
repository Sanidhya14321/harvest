# browser:cdp-core — dedicated-profile Rust browser control

Prerequisite: `control:playbook`. Capability: native browser replacement;
`migrate:browser` remains incomplete until its consumers and legacy sources move.
This leaf completes the ordinary Rust engine and its native CLI/session/SDK
consumers only; it owns no legacy source-removal claim.
Inputs: existing browser prelude/registry/tab supervisors and Rust CLI/SDK.

Destination: ordinary `harvest-browser` crate and `harvest browser` CLI. Chrome is
an allowed external native program. Use CDP DOM, Page, Target and Input commands;
never Runtime.evaluate, injected JavaScript, Playwright or the extension relay.

Contract: own a marked, dedicated persistent profile and cross-process writer
lock. Never attach to a user's ordinary browser profile. Bound startup, frames,
responses, event draining, tabs and operation time. Preserve cancellation and
cleanup; shutting down kills/reaps the owned Chrome process, while profile data
persists. Implement tabs/open/navigate/document/query/click/type/screenshot/close.
Errors must not publish successful results. Use literal argument arrays and
jailed profile metadata. Chrome sandbox disabling is an explicit test option,
never a default. Authentication happens separately in the dedicated profile.

Steps: implement the shared engine; connect CLI and SDK; verify real headless
Chrome against local static HTML, including DOM/input/navigation/capture, profile
reopening, ownership refusal and shutdown; add malformed/frame/cancellation
contracts with a Rust CDP peer. Follow-on migration tasks replace legacy browser prelude consumers
with the structured native API, retire JS run/call with migration guidance, and
remove each orphaned browser worker, relay, injected helper and dependency only
after its last consumer moves. That follow-on work is not certified by this
engine leaf. Shared eval/computer helpers stay pending until
all consumers migrate. Do not delete unrelated browser products or tests.

Check `cargo fmt -p harvest-browser -p harvest-omp -- --check`, strict Clippy,
`cargo xtask test -p harvest-browser`, affected Rust CLI/SDK tests and a copied
native application binary with Bun/Node absent. Rebuild N-API and run affected
Bun suites if a transitional binding is added. Finalize ledger and evidence;
stage eligible deletions, run migration/capability/playbook gates and a
source-bound verification plan before each coherent commit.

Recovery: revert one bounded engine/connection slice, preserving the profile and
existing user sessions. No publishing or default launcher cutover is authorized
by this task. Missing consumer parity becomes an explicit prerequisite; never
claim source-removal completion merely from the new engine compiling.
