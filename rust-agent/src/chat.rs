use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FunctionCall {
    pub(crate) name: String,
    pub(crate) arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ToolCall {
    pub(crate) id: String,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ChatMessage {
    pub(crate) role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct ToolSpec {
    #[serde(rename = "type")]
    kind: &'static str,
    function: ToolFunction,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct ToolFunction {
    name: &'static str,
    description: &'static str,
    parameters: ToolParameters,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct ToolParameters {
    #[serde(rename = "type")]
    kind: &'static str,
    properties: ToolProperties,
    required: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, Serialize)]
struct ToolProperties {
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<StringProperty>,
    #[serde(skip_serializing_if = "Option::is_none")]
    old: Option<StringProperty>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new: Option<StringProperty>,
    #[serde(skip_serializing_if = "Option::is_none")]
    contents: Option<StringProperty>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<StringProperty>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pattern: Option<StringProperty>,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct StringProperty {
    #[serde(rename = "type")]
    kind: &'static str,
}

const STRING: StringProperty = StringProperty { kind: "string" };

impl ToolSpec {
    pub(crate) const fn name(self) -> &'static str {
        self.function.name
    }
}

pub(crate) fn user(content: impl Into<String>) -> ChatMessage {
    ChatMessage {
        role: "user".to_owned(),
        content: Some(content.into()),
        tool_calls: None,
        tool_call_id: None,
    }
}

pub(crate) fn system(content: &str) -> ChatMessage {
    ChatMessage {
        role: "system".to_owned(),
        content: Some(content.to_owned()),
        tool_calls: None,
        tool_call_id: None,
    }
}

pub(crate) fn tool_result(id: &str, content: String) -> ChatMessage {
    ChatMessage {
        role: "tool".to_owned(),
        content: Some(content),
        tool_calls: None,
        tool_call_id: Some(id.to_owned()),
    }
}

pub(crate) fn specs() -> Vec<ToolSpec> {
    vec![
        spec(
            "read",
            "Read a UTF-8 file inside the workspace.",
            ToolProperties {
                path: Some(STRING),
                ..EMPTY
            },
            &["path"],
        ),
        spec(
            "write",
            "Create or replace a UTF-8 file inside the workspace.",
            ToolProperties {
                path: Some(STRING),
                contents: Some(STRING),
                ..EMPTY
            },
            &["path", "contents"],
        ),
        spec(
            "edit",
            "Replace the one occurrence of old with new.",
            ToolProperties {
                path: Some(STRING),
                old: Some(STRING),
                new: Some(STRING),
                ..EMPTY
            },
            &["path", "old", "new"],
        ),
        spec(
            "grep",
            "Search workspace files with a Rust regex.",
            ToolProperties {
                pattern: Some(STRING),
                path: Some(STRING),
                ..EMPTY
            },
            &["pattern"],
        ),
        spec(
            "bash",
            "Run a command in the workspace and return its output.",
            ToolProperties {
                command: Some(STRING),
                ..EMPTY
            },
            &["command"],
        ),
    ]
}

const EMPTY: ToolProperties = ToolProperties {
    path: None,
    old: None,
    new: None,
    contents: None,
    command: None,
    pattern: None,
};

const fn spec(
    name: &'static str,
    description: &'static str,
    properties: ToolProperties,
    required: &'static [&'static str],
) -> ToolSpec {
    ToolSpec {
        kind: "function",
        function: ToolFunction {
            name,
            description,
            parameters: ToolParameters {
                kind: "object",
                properties,
                required,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{specs, user};

    #[test]
    fn tool_catalog_names_the_workspace_tools() {
        let names: Vec<&str> = specs().iter().map(|spec| spec.function.name).collect();
        assert_eq!(names, ["read", "write", "edit", "grep", "bash"]);
    }

    #[test]
    fn user_message_has_no_tool_payload() {
        let message = user("hello");
        assert_eq!(message.role, "user");
        assert_eq!(message.content.as_deref(), Some("hello"));
        assert!(message.tool_calls.is_none());
    }
}
