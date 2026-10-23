Language-server navigation, diagnostics and text refactoring for host-configured servers.
Use status/capabilities to inspect servers; file queries use a workspace path.
Positions use a 1-based line and a 0-based UTF-16 character, or an unambiguous symbol substring (#N selects an occurrence).
Actions: hover, definition, type_definition, implementation, references, symbols, diagnostics, rename, code_actions, format, request, reload, status, capabilities.
Rename applies by default; apply:false previews. Formatting and code actions preview by default. Applied edits require host approval of the returned changes.
Raw request takes method plus object payload. Resource create/rename/delete operations are not available in this candidate.

Use language-server rename/reference actions for symbol-aware cross-file changes. Do not substitute text replacement for a cross-file symbol rename when a matching server is available.
