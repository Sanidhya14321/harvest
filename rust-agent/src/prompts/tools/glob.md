Globs files and directories with fast pattern matching.

<instruction>
- `path`: glob, file, or directory; separate targets with `;` (`src/**/*.ts; test/**/*.ts`). Omitted → the workspace root.
- `ssh://` has no local path; use `read`. Internal URLs are not supported by this host.
- `gitignore` defaults `true`. Set `false` for ignored files such as `.env*`, logs, or build output.
- `hidden` defaults `true`; pair it with `gitignore: false` for ignored dotfiles.
- `limit` defaults to 200 (the maximum).
</instruction>

<output>
Matches are newest-first and grouped by directory; directories end in `/`.
</output>
