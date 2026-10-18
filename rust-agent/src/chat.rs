use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionCall {
	pub name:      String,
	pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCall {
	pub id:       String,
	#[serde(rename = "type")]
	pub kind:     String,
	pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
	pub role:         String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub content:      Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tool_calls:   Option<Vec<ToolCall>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ToolSpec {
	#[serde(rename = "type")]
	kind:     &'static str,
	function: ToolFunction,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct ToolFunction {
	name:        &'static str,
	description: &'static str,
	parameters:  ToolParameters,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct ToolParameters {
	#[serde(rename = "type")]
	kind:       &'static str,
	properties: ToolProperties,
	required:   &'static [&'static str],
}

#[derive(Debug, Clone, Copy, Serialize)]
struct ToolProperties {
	#[serde(rename = "case", skip_serializing_if = "Option::is_none")]
	case_sensitive: Option<BooleanProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	gitignore:      Option<BooleanProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	path:           Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	old:            Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	new:            Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	contents:       Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	command:        Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pattern:        Option<StringProperty>,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct StringProperty {
	#[serde(rename = "type")]
	kind: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct BooleanProperty {
	#[serde(rename = "type")]
	kind: &'static str,
}
const BOOLEAN: BooleanProperty = BooleanProperty { kind: "boolean" };

const STRING: StringProperty = StringProperty { kind: "string" };

impl ToolSpec {
	pub const fn name(self) -> &'static str {
		self.function.name
	}
}

pub fn user(content: impl Into<String>) -> ChatMessage {
	ChatMessage {
		role:         "user".to_owned(),
		content:      Some(content.into()),
		tool_calls:   None,
		tool_call_id: None,
	}
}

pub fn system(content: &str) -> ChatMessage {
	ChatMessage {
		role:         "system".to_owned(),
		content:      Some(content.to_owned()),
		tool_calls:   None,
		tool_call_id: None,
	}
}

pub fn tool_result(id: &str, content: String) -> ChatMessage {
	ChatMessage {
		role:         "tool".to_owned(),
		content:      Some(content),
		tool_calls:   None,
		tool_call_id: Some(id.to_owned()),
	}
}

pub fn specs() -> Vec<ToolSpec> {
	vec![
		spec(
			"read",
			include_str!("prompts/tools/read.md").trim_end(),
			ToolProperties { path: Some(STRING), ..EMPTY },
			&["path"],
		),
		spec(
			"write",
			include_str!("prompts/tools/write.md").trim_end(),
			ToolProperties { path: Some(STRING), contents: Some(STRING), ..EMPTY },
			&["path", "contents"],
		),
		spec(
			"edit",
			include_str!("prompts/tools/edit.md").trim_end(),
			ToolProperties { path: Some(STRING), old: Some(STRING), new: Some(STRING), ..EMPTY },
			&["path", "old", "new"],
		),
		spec(
			"grep",
			include_str!("prompts/tools/grep.md").trim_end(),
			ToolProperties {
				pattern: Some(STRING),
				path: Some(STRING),
				case_sensitive: Some(BOOLEAN),
				gitignore: Some(BOOLEAN),
				..EMPTY
			},
			&["pattern"],
		),
		spec(
			"bash",
			include_str!("prompts/tools/bash.md").trim_end(),
			ToolProperties { command: Some(STRING), ..EMPTY },
			&["command"],
		),
	]
}

const EMPTY: ToolProperties = ToolProperties {
	case_sensitive: None,
	gitignore:      None,
	path:           None,
	old:            None,
	new:            None,
	contents:       None,
	command:        None,
	pattern:        None,
};

const fn spec(
	name: &'static str,
	description: &'static str,
	properties: ToolProperties,
	required: &'static [&'static str],
) -> ToolSpec {
	ToolSpec {
		kind:     "function",
		function: ToolFunction {
			name,
			description,
			parameters: ToolParameters { kind: "object", properties, required },
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
