Agent model: {{model}}

Assistant prose: {{prose}}

{{#if results}}{{#each results}}Tool call {{number}}: {{tool}} → {{status}}
{{body}}

{{/each}}{{else}}Tool calls: none.

{{/if}}{{#if todo_only}}NOTE: this turn only updated the todo checklist. The checklist items are PLANS, not events; your sentence must summarize only the checklist state (item counts, statuses, the in-progress item).

{{/if}}One sentence:
