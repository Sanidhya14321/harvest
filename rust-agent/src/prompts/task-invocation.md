<agent name="{{agent}}">
{{{body}}}
</agent>

{{#if context}}
<context>
{{{context}}}
</context>

{{/if}}
{{{task}}}

When finished, call `yield` with your result.
