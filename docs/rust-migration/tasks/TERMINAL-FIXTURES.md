# control:terminal-fixtures — explicit color capabilities in renderer contracts

Prerequisite: `control:playbook`. This is a fixture repair; the legacy test source
remains owned by `migrate:application` and no source-removal progress is claimed.

Inputs: `packages/coding-agent/test/tools/bash-sixel-render.test.ts`, theme
loader/color mode and the existing SIXEL/terminal-graphics host suites. Two style
assertions fail headlessly because the automatically detected theme has no color:
the warning/error prefix comparison degenerates to an empty string and no SGR
runs can be emitted. Forcing color globally breaks unrelated silent-stderr tests.

Contract/destination: construct this test's existing dark theme with the loader's
explicit `truecolor` mode. Retain all warning-versus-error, multiline SGR, SIXEL
bypass, text and cache assertions. Do not change production color detection,
global environment, application entrypoints or persisted formats. This simulates
the colored terminal whose behavior the assertions already specify.

Steps: reproduce without FORCE_COLOR; use the existing loader option; format and
type-check; run the three affected host files without a global color override.
No deletions or new runtime API are needed. Completion requires all 38 existing
contracts, including both formerly failing style checks, to pass headlessly.
Recovery: revert only this fixture's explicit mode and its task evidence.

```sh
bun test packages/coding-agent/test/utils/terminal-graphics.test.ts packages/coding-agent/test/bash-execution-sixel.test.ts packages/coding-agent/test/tools/bash-sixel-render.test.ts
```

The broad singleton/transition failures are separate; this fixture repair does
not certify their behavior or any complete native UI capability.
