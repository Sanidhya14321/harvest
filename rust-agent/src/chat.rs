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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
	#[serde(rename = "type")]
	kind:     String,
	function: ToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolFunction {
	name:        String,
	description: String,
	parameters:  serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct ToolProperties {
	#[serde(rename = "case", skip_serializing_if = "Option::is_none")]
	case_sensitive: Option<BooleanProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	gitignore:      Option<BooleanProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	path:           Option<&'static StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	old:            Option<&'static StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	new:            Option<&'static StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	contents:       Option<&'static StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	command:        Option<&'static StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pattern:        Option<&'static StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	action:         Option<&'static StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	tab:            Option<&'static StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	url:            Option<&'static StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	selector:       Option<&'static StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	text:           Option<&'static StringProperty>,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct StringProperty {
	#[serde(rename = "type")]
	kind:        &'static str,
	#[serde(rename = "enum", skip_serializing_if = "Option::is_none")]
	enum_values: Option<&'static [&'static str]>,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct BooleanProperty {
	#[serde(rename = "type")]
	kind: &'static str,
}
const BOOLEAN: BooleanProperty = BooleanProperty { kind: "boolean" };

const STRING: &StringProperty = &StringProperty { kind: "string", enum_values: None };

impl ToolSpec {
	pub fn name(&self) -> &str {
		&self.function.name
	}

	pub fn validate(&self) -> anyhow::Result<()> {
		anyhow::ensure!(self.kind == "function", "unsupported tool schema kind");
		Self::from_schema(
			self.function.name.clone(),
			self.function.description.clone(),
			self.function.parameters.clone(),
		)
		.map(|_| ())
	}

	pub fn from_schema(
		name: String,
		description: String,
		parameters: serde_json::Value,
	) -> anyhow::Result<Self> {
		anyhow::ensure!(
			!name.is_empty()
				&& name.len() <= 128
				&& name
					.bytes()
					.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')),
			"invalid tool name"
		);
		anyhow::ensure!(
			description.len() <= 16 * 1024 && parameters.is_object() && parameters["type"] == "object",
			"invalid tool schema"
		);
		Ok(Self {
			kind:     "function".into(),
			function: ToolFunction { name, description, parameters },
		})
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

pub(crate) fn browser_spec() -> ToolSpec {
	spec(
		"browser",
		include_str!("prompts/tools/browser.md").trim_end(),
		ToolProperties {
			action: Some(&StringProperty {
				kind:        "string",
				enum_values: Some(&[
					"tabs",
					"open",
					"navigate",
					"document",
					"query",
					"click",
					"type",
					"screenshot",
					"close",
				]),
			}),
			tab: Some(STRING),
			url: Some(STRING),
			selector: Some(STRING),
			text: Some(STRING),
			..EMPTY
		},
		&["action"],
	)
}

const EMPTY: ToolProperties = ToolProperties {
	action:         None,
	tab:            None,
	url:            None,
	selector:       None,
	text:           None,
	case_sensitive: None,
	gitignore:      None,
	path:           None,
	old:            None,
	new:            None,
	contents:       None,
	command:        None,
	pattern:        None,
};

fn spec(
	name: &'static str,
	description: &'static str,
	properties: ToolProperties,
	required: &'static [&'static str],
) -> ToolSpec {
	ToolSpec {
		kind:     "function".into(),
		function: ToolFunction {
			name:        name.into(),
			description: description.into(),
			parameters:  serde_json::json!({"type":"object", "properties":properties, "required":required}),
		},
	}
}

#[cfg(test)]
mod tests {
	use super::{ToolSpec, specs, user};

	#[test]
	fn tool_catalog_names_the_workspace_tools() {
		let specifications = specs();
		let names: Vec<&str> = specifications.iter().map(ToolSpec::name).collect();
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
