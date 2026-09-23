import urllib.request
import json

criteria = {
    "scout": "Codebase navigation and discovery. Select when the user wants to understand or find code without modifying it. Example tasks: 'Where is the auth handler defined?', 'Find all references to AgentMessage', 'Trace the request lifecycle'.",
    "reviewer": "Code review and pull request inspection. Select when the user wants qualitative feedback or bug inspection on changes. Example tasks: 'Review this diff for bugs', 'Check this PR for edge cases', 'Verify correctness of this commit'.",
    "security-reviewer": "Security and vulnerability assessment. Select when the task is specifically focused on security threats or sensitive data leaks. Example tasks: 'Check for SQL injection or path traversal', 'Scan for hardcoded API keys', 'Audit permissions'.",
    "sonic": "Mechanical and repetitive tasks. Select for non-creative, simple bulk edits that don't need deep reasoning. Example tasks: 'Replace tab indents with spaces across all files', 'Convert CRLF to LF', 'Rename imports'.",
    "task": "Implementation and active software engineering. Select when the agent must write code, create files, implement algorithms, or execute end-to-end tasks. Example tasks: 'Build a rate limiter', 'Fix this bug and write tests', 'Implement feature X'."
}

task = "Search through the codebase to find where the tool definitions are registered and trace the imports."
payload = {
    "state": {"task_assignment": task},
    "questions": {
        "subagent_choice": {
            "type": "choice",
            "instructions": "Which specialized subagent is best suited to execute this assigned task?",
            "criteria": criteria
        }
    }
}
req = urllib.request.Request("http://127.0.0.1:8177/v1/decide", data=json.dumps(payload).encode("utf-8"), headers={"Content-Type": "application/json"})
resp = json.loads(urllib.request.urlopen(req).read().decode("utf-8"))
ans = resp["answers"]["subagent_choice"]
print("Pick:", ans["choice"])
print("Confidence:", ans["confidence"])
print("Probabilities:", ans["probabilities"])
