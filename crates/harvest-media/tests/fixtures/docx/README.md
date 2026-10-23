# DOCX legacy behavior fixtures

Captured through `convertToHtml` at commit `043709e577114bcb3c7f1a8d328eaa3ad5d88500`
before replacing the TypeScript engine. Inputs are the four ZIP documents built
by the existing test, retained as inert document data. JSON files are actual
legacy HTML/diagnostic results, not expectations derived from the Rust renderer.

- `formatting`: headings, run formatting, unknown styles, breaks and footnotes.
- `lists`: nested ordered and unordered numbering.
- `tables-links`: hyperlinks and horizontal/vertical merged cells.
- `images`: inline PNG output; `image-callback.json` records custom attributes.

The original suite also converted the formatting document by filesystem path.
The duplicate result is intentionally omitted. Malformed-input and resource-limit
contracts are separate Rust tests and do not claim legacy failure equivalence.

Source blob identities:

- `packages/utils/src/docx/converter.ts`: `2c14d00e9f100e1ea224e47aee8082a5b7ef5c6a`
- `packages/utils/src/docx/xml.ts`: `b1b667b152c5e83ac8e2e967ad136173ec34fd69`
- `packages/utils/test/docx.test.ts`: `8fdb112f2d72d1b7a63a7fc76cd8d0264c1da99a`
