Apply the following exact changes to the workspace files. Preserve every other
file and byte. Each old block must match the input exactly. A block marked with
an explicit starting line must be applied at that line. Do not execute fixture
source code or change the expected output data.
{{#each files}}
## File: {{{path}}}
{{#each hunks}}
{{#if unique}}
Replace this uniquely occurring old block:
{{else}}
Replace this old block starting at line {{start_line}}:
{{/if}}
{{{../fence}}}
{{{old}}}
{{{../fence}}}

With this new block:
{{{../fence}}}
{{{new}}}
{{{../fence}}}
{{/each}}
{{/each}}
