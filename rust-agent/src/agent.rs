use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::chat::{self, ChatMessage, ToolCall};
use crate::provider::Model;
use crate::{session, tools};

const SYSTEM: &str =
    "You are omp, a coding agent. Use tools to inspect the workspace, then answer.";

pub(crate) fn run(
    workspace: &Path,
    prompt: &str,
    max_steps: u32,
    model: &dyn Model,
) -> Result<String> {
    if prompt.is_empty() {
        bail!("prompt is empty");
    }
    if max_steps == 0 {
        bail!("max steps must be at least 1");
    }
    let root = workspace
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", workspace.display()))?;
    let system_message = chat::system(SYSTEM);
    let user_message = chat::user(prompt);
    session::append(&root, &system_message)?;
    session::append(&root, &user_message)?;
    let mut messages = vec![system_message, user_message];
    for _step in 1..=max_steps {
        let response = model.complete(&messages, &chat::specs())?;
        session::append(&root, &response)?;
        let calls = response.tool_calls.clone().unwrap_or_default();
        let answer = response.content.clone().unwrap_or_default();
        messages.push(response);
        if calls.is_empty() {
            return Ok(answer);
        }
        record_tools(&root, &calls, &mut messages)?;
    }
    bail!("stopped after {max_steps} steps")
}

fn record_tools(root: &Path, calls: &[ToolCall], messages: &mut Vec<ChatMessage>) -> Result<()> {
    for call in calls {
        let output = match tools::invoke(root, call) {
            Ok(text) => text,
            Err(error) => format!("error: {error:#}"),
        };
        let message = chat::tool_result(&call.id, output);
        session::append(root, &message)?;
        messages.push(message);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use anyhow::Result;

    use super::run;
    use crate::chat::{ChatMessage, FunctionCall, ToolCall, specs};
    use crate::provider::Model;

    struct Script {
        replies: RefCell<VecDeque<ChatMessage>>,
    }

    impl Model for Script {
        fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[crate::chat::ToolSpec],
        ) -> Result<ChatMessage> {
            self.replies
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("the scripted model ran out of replies"))
        }
    }

    fn tool_call(path: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_owned(),
            content: None,
            tool_calls: Some(vec![ToolCall {
                id: "call-1".to_owned(),
                kind: "function".to_owned(),
                function: FunctionCall {
                    name: "read".to_owned(),
                    arguments: format!(r#"{{"path":"{path}"}}"#),
                },
            }]),
            tool_call_id: None,
        }
    }

    #[test]
    fn agent_reads_a_file_and_returns_the_final_answer() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("note.txt"), "hello from disk").unwrap();
        let model = Script {
            replies: RefCell::new(VecDeque::from([
                tool_call("note.txt"),
                ChatMessage {
                    role: "assistant".to_owned(),
                    content: Some("the note says hello".to_owned()),
                    tool_calls: None,
                    tool_call_id: None,
                },
            ])),
        };
        let answer = run(root.path(), "read the note", 4, &model).unwrap();
        assert_eq!(answer, "the note says hello");
        let session = std::fs::read_to_string(root.path().join(".omp/session.jsonl")).unwrap();
        assert!(session.contains("hello from disk"));
        assert!(!specs().is_empty());
    }

    #[test]
    fn agent_stops_when_the_model_keeps_calling_tools() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("note.txt"), "x").unwrap();
        let model = Script {
            replies: RefCell::new(VecDeque::from([
                tool_call("note.txt"),
                tool_call("note.txt"),
            ])),
        };
        let error = run(root.path(), "loop", 1, &model).unwrap_err();
        assert!(error.to_string().contains("stopped after 1"));
    }
}
