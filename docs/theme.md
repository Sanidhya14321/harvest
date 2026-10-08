# Theming Reference

This document describes how theming works in the coding-agent today: schema, loading, runtime behavior, and failure modes.

## What the theme system controls

The theme system drives:

- foreground/background color tokens used across the TUI
- markdown styling adapters (`getMarkdownTheme()`)
- selector/editor/settings list adapters (`getSelectListTheme()`, `getEditorTheme()`, `getSettingsListTheme()`)
- symbol preset + symbol overrides (`unicode`, `nerd`, `ascii`)
- syntax highlighting colors used by native highlighter (`@harvest/pi-natives`)
- status line segment colors

Primary implementation: `src/modes/theme/theme.ts`.

## Theme JSON shape

Theme files are JSON objects validated against the runtime schema in `theme.ts` (`themeJsonSchema`) and mirrored by `src/modes/theme/theme-schema.json`.

Top-level fields:

- `name` (required)
- `colors` (required; legacy tokens required, semantic surfaces optional)
- `vars` (optional; reusable color variables)
- `export` (optional; HTML export colors)
- `symbols` (optional)
  - `preset` (optional: `unicode | nerd | ascii`)
  - `overrides` (optional: key/value overrides for `SymbolKey`)

Color values accept:

- hex string (`"#RRGGBB"`)
- 256-color index (`0..255`)
- variable reference string (resolved through `vars`)
- empty string (`""`) meaning terminal default (`\x1b[39m` fg, `\x1b[49m` bg)

## Required and optional color tokens

Legacy tokens below are required in `colors` except `thinkingMax`, which falls back to `thinkingXhigh`. The semantic surface tokens are optional so existing custom themes remain loadable.

### Core text and borders (11)

`accent`, `border`, `borderAccent`, `borderMuted`, `success`, `error`, `warning`, `muted`, `dim`, `text`, `thinkingText`

### Background blocks (7)

`selectedBg`, `userMessageBg`, `customMessageBg`, `toolPendingBg`, `toolSuccessBg`, `toolErrorBg`, `statusLineBg`

### Semantic surfaces (optional)

`screenBg` paints the fullscreen workspace, `panelBg` paints sidebars and output panels, `raisedBg` paints hovered/raised rows, `composerBg` paints the prompt editor, and `modalBg` paints dialogs and managers. Selected rows use `selectedBg` across their allocation.

Legacy fallback chains resolve in `src/modes/theme/loader.ts`: screen to terminal default; panel to tool success/status; raised to selection/pending; composer to raised/pending; modal to panel/success. An explicitly empty token retains terminal default and is not treated as missing. Surface fill restores the enclosing background after nested full/background resets, while retaining explicit nested backgrounds, OSC hyperlinks, and cursor/image protocols.

### Message/tool text (5)

`userMessageText`, `customMessageText`, `customMessageLabel`, `toolTitle`, `toolOutput`

### Markdown (10)

`mdHeading`, `mdLink`, `mdLinkUrl`, `mdCode`, `mdCodeBlock`, `mdCodeBlockBorder`, `mdQuote`, `mdQuoteBorder`, `mdHr`, `mdListBullet`

### Tool diff + syntax highlighting (12)

`toolDiffAdded`, `toolDiffRemoved`, `toolDiffContext`,
`syntaxComment`, `syntaxKeyword`, `syntaxFunction`, `syntaxVariable`, `syntaxString`, `syntaxNumber`, `syntaxType`, `syntaxOperator`, `syntaxPunctuation`

### Mode/thinking borders (8 required, 1 optional)

`thinkingOff`, `thinkingMinimal`, `thinkingLow`, `thinkingMedium`, `thinkingHigh`, `thinkingXhigh`, optional `thinkingMax`, `bashMode`, `pythonMode`

### Status line segment colors (13)

`statusLineSep`, `statusLineModel`, `statusLinePath`, `statusLineGitClean`, `statusLineGitDirty`, `statusLineContext`, `statusLineSpend`, `statusLineStaged`, `statusLineDirty`, `statusLineUntracked`, `statusLineOutput`, `statusLineCost`, `statusLineSubagents`

## Optional tokens

### `export` section (optional)

Used for HTML export theming helpers:

- `export.pageBg`
- `export.cardBg`
- `export.infoBg`

If omitted, export code derives defaults from resolved theme colors.

### `symbols` section (optional)

- `symbols.preset` sets a theme-level default symbol set.
- `symbols.overrides` can override individual `SymbolKey` values.
- `symbols.spinnerFrames` overrides the loading spinner frames. Accepts either a flat `string[]` (applied to both spinner types) or an object `{ "status"?: string[], "activity"?: string[] }` to override each type independently. Any type not specified falls back to the symbol preset's default frames. `status` drives the ~12.5fps spinner used by loaders and tool-execution indicators; `activity` drives the ~30fps spinner used by markdown progress bars and similar high-frequency UI.

Runtime precedence:

1. settings `symbolPreset` override (if set)
2. theme JSON `symbols.preset`
3. fallback `"unicode"`

Invalid override keys are ignored and logged (`logger.debug`).

#### Box-drawing borders

Outlined detail frames use `boxRound.*` tokens, with junctions from `boxSharp.*`; Markdown tables retain sharp borders. Dialogs use filled surfaces and routine activity uses compact rails. Composer shapes and scrollbars receive the same symbol preset through the TUI adapters. ASCII changes application chrome rather than transliterating message/file content.

Override behavior follows from that split:

- `boxRound.{topLeft,topRight,bottomLeft,bottomRight,horizontal,vertical}` restyle rounded detail frames.
- `boxSharp.{cross,teeDown,teeUp,teeRight,teeLeft}` restyle dividers/junctions everywhere (rounded frames and tables alike).
- `boxSharp.{topLeft,topRight,bottomLeft,bottomRight}` now affect markdown table corners only.

## Built-in vs custom theme sources

Theme lookup order (`loadThemeJson`):

1. built-in embedded themes (`dark.json`, `light.json`, and all `defaults/*.json` compiled into `defaultThemes`)
2. custom theme file: `<customThemesDir>/<name>.json`

Custom themes directory comes from `getCustomThemesDir()`:

- default: `~/.harvest/agent/themes`
- overridden by `PI_CODING_AGENT_DIR` (`$PI_CODING_AGENT_DIR/themes`)

`getAvailableThemes()` returns merged built-in + custom names, sorted, with built-ins taking precedence on name collision.

## Loading, validation, and resolution

For custom theme files:

1. read JSON
2. parse JSON
3. validate against `themeJsonSchema`
4. resolve `vars` references recursively
5. convert resolved values to ANSI by terminal capability mode

Validation behavior:

- missing required color tokens: explicit grouped error message
- bad token types/values: validation errors with JSON path
- unknown theme file: `Theme not found: <name>`

Var reference behavior:

- supports nested references
- throws on missing variable reference
- throws on circular references

## Terminal color mode behavior

Color mode detection (`detectColorMode`) reuses the central terminal capability model and `detectColorLevel` policy. Explicit `FORCE_COLOR` takes precedence; `FORCE_COLOR=0`, the presence of `NO_COLOR`, or `TERM=dumb` disables color. Otherwise terminal capabilities choose truecolor or 256 color. Theme callbacks remain usable in color-free mode, with selection/focus/status conveyed through text and symbols. Markdown color swatches and syntax/Mermaid coloring also follow this policy.

Conversion behavior:

- hex -> `Bun.color(..., "ansi-16m" | "ansi-256")`
- numeric -> `38;5` / `48;5` ANSI
- `""` -> default fg/bg reset
- color-free -> no foreground/background SGR from theme styling

## Runtime switching behavior

### Initial theme (`initTheme`)

`main.ts` initializes theme with settings:

- `symbolPreset`
- `colorBlindMode`
- `theme.dark`
- `theme.light`

Auto theme slot selection uses terminal appearance in this order:

1. terminal-reported OSC 11 background luminance, unless the macOS/Zellij fallback path is active
2. `COLORFGBG` background index (`< 8` => dark, `>= 8` => light)
3. macOS appearance fallback only for the known-broken macOS/Zellij OSC 11 path
4. dark slot fallback

Current defaults from settings schema:

- `theme.dark = "harvest"`
- `theme.light = "harvest-light"`
- `symbolPreset = "unicode"`
- `colorBlindMode = false`

Explicit older/custom theme choices remain selected. Interactive standalone entrypoints use `initConfiguredTheme()` to resolve effective preferences; its read-only settings path does not open the agent database or persist a migration. Startup prepaint uses cached preferences when available and the same fresh default slots otherwise.

### Explicit switching (`setTheme`)

- loads selected theme
- updates global `theme` singleton
- optionally starts watcher
- triggers `onThemeChange` callback

On failure:

- falls back to built-in `dark`
- returns `{ success: false, error }`

### Preview switching (`previewTheme`)

- applies temporary preview theme to global `theme`
- does **not** change persisted settings by itself
- returns success/error without fallback replacement

Settings UI uses this for live preview and restores prior theme on cancel.

## Watchers and live reload

When watcher is enabled (`setTheme(..., true)` / interactive init):

- watches `<customThemesDir>/<currentTheme>.json` only when that file exists
- built-ins are effectively not watched; built-in theme lookup also takes precedence over same-name custom files
- matching file changes schedule a debounced reload; reload errors or temporary file absence keep the last successfully loaded theme
- the watcher does not perform a delete/rename fallback; it waits for a future successful reload or explicit theme switch

Auto mode also reevaluates dark/light slot mapping from terminal appearance changes, `SIGWINCH`, and the macOS fallback observer when active.

## Color-blind mode behavior

`colorBlindMode` changes only one token at runtime:

- `toolDiffAdded` is HSV-adjusted (green shifted toward blue)
- adjustment is applied only when resolved value is a hex string

Other tokens are unchanged.

## Where theme settings are persisted

Theme-related settings are persisted by `Settings` to global config YAML:

- path: `<agentDir>/config.yml`
- default agent dir: `~/.harvest/agent`
- effective default file: `~/.harvest/agent/config.yml`

Persisted keys:

- `theme.dark`
- `theme.light`
- `symbolPreset`
- `colorBlindMode`

Legacy migration exists: old flat `theme: "name"` is migrated to nested `theme.dark` or `theme.light` based on luminance detection.

## Creating a custom theme (practical)

1. Create file in custom themes dir, e.g. `~/.harvest/agent/themes/my-theme.json`.
2. Include `name`, optional `vars`, and **all required** `colors` tokens.
3. Optionally include `symbols` and `export`.
4. Select the theme in Settings (`Appearance -> Dark Theme` or `Appearance -> Light Theme`) depending on which auto slot you want.

Minimal skeleton:

```json
{
  "name": "my-theme",
  "vars": {
    "accent": "#7aa2f7",
    "muted": 244
  },
  "colors": {
    "accent": "accent",
    "border": "#4c566a",
    "borderAccent": "accent",
    "borderMuted": "muted",
    "success": "#9ece6a",
    "error": "#f7768e",
    "warning": "#e0af68",
    "muted": "muted",
    "dim": 240,
    "text": "",
    "thinkingText": "muted",

    "selectedBg": "#2a2f45",
    "userMessageBg": "#1f2335",
    "userMessageText": "",
    "customMessageBg": "#24283b",
    "customMessageText": "",
    "customMessageLabel": "accent",
    "toolPendingBg": "#1f2335",
    "toolSuccessBg": "#1f2d2a",
    "toolErrorBg": "#2d1f2a",
    "toolTitle": "",
    "toolOutput": "muted",

    "mdHeading": "accent",
    "mdLink": "accent",
    "mdLinkUrl": "muted",
    "mdCode": "#c0caf5",
    "mdCodeBlock": "#c0caf5",
    "mdCodeBlockBorder": "muted",
    "mdQuote": "muted",
    "mdQuoteBorder": "muted",
    "mdHr": "muted",
    "mdListBullet": "accent",

    "toolDiffAdded": "#9ece6a",
    "toolDiffRemoved": "#f7768e",
    "toolDiffContext": "muted",

    "syntaxComment": "#565f89",
    "syntaxKeyword": "#bb9af7",
    "syntaxFunction": "#7aa2f7",
    "syntaxVariable": "#c0caf5",
    "syntaxString": "#9ece6a",
    "syntaxNumber": "#ff9e64",
    "syntaxType": "#2ac3de",
    "syntaxOperator": "#89ddff",
    "syntaxPunctuation": "#9aa5ce",

    "thinkingOff": 240,
    "thinkingMinimal": 244,
    "thinkingLow": "#7aa2f7",
    "thinkingMedium": "#2ac3de",
    "thinkingHigh": "#bb9af7",
    "thinkingXhigh": "#f7768e",
    "thinkingMax": "#ff007c",

    "bashMode": "#2ac3de",
    "pythonMode": "#bb9af7",

    "statusLineBg": "#16161e",
    "statusLineSep": 240,
    "statusLineModel": "#bb9af7",
    "statusLinePath": "#7aa2f7",
    "statusLineGitClean": "#9ece6a",
    "statusLineGitDirty": "#e0af68",
    "statusLineContext": "#2ac3de",
    "statusLineSpend": "#7dcfff",
    "statusLineStaged": "#9ece6a",
    "statusLineDirty": "#e0af68",
    "statusLineUntracked": "#f7768e",
    "statusLineOutput": "#c0caf5",
    "statusLineCost": "#ff9e64",
    "statusLineSubagents": "#bb9af7"
  }
}
```

## Testing custom themes

Use this workflow:

1. Start interactive mode (watcher enabled from startup).
2. Open settings and preview theme values (live `previewTheme`).
3. For custom theme files, edit the JSON while running and confirm auto-reload on save.
4. Exercise critical surfaces:
   - markdown rendering
   - tool blocks (pending/success/error)
   - diff rendering (added/removed/context)
   - status line readability
   - thinking level border changes
   - bash/python mode border colors
5. Validate both symbol presets if your theme depends on glyph width/appearance.

## Real constraints and caveats

- All `colors` tokens are required for custom themes except optional `thinkingMax`, which falls back to `thinkingXhigh`.
- `export` and `symbols` are optional.
- `$schema` in theme JSON is informational; runtime validation is enforced by the ArkType-compatible schema in code (`themeJsonSchema` in `src/modes/theme/schema.ts`).
- `setTheme` failure falls back to `dark`; `previewTheme` failure does not replace current theme.
- File watcher reload errors or temporary missing files keep the current loaded theme until a successful reload or explicit theme switch.

Defaults are harvest (dark) and harvest-light (light) with restrained surfaces: screen #0a0a0a, panel #141414, input #1e1e1e, accent #fab283. Optional surface tokens screenBg/panelBg/raisedBg/composerBg/modalBg fall back centrally so legacy custom themes still load.
