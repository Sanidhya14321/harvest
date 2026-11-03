Run: {{run}}
Trace: {{trace}}
{{metadata}}

{{#if focus}}Reviewer focus notes:
{{focus}}

{{/if}}{{turn_log}}

Final assistant message (verbatim, may be truncated):
"""
{{final_prose}}
"""

{{#if failed}}Run status is "fail". Write the Story Arc now; the LAST bullet MUST be **Failure analysis** per the rules.{{else}}Write the Story Arc now.{{/if}}
