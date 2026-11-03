use serde::Serialize;

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
	case_sensitive:     Option<BooleanProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	gitignore:          Option<BooleanProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	path:               Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	old:                Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	new:                Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	content:            Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	command:            Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pattern:            Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	input:              Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	hidden:             Option<BooleanProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	limit:              Option<NumberProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	skip:               Option<NumberProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pat:                Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	query:              Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	recency:            Option<RawSchema>,
	#[serde(skip_serializing_if = "Option::is_none")]
	num_search_results: Option<NumberProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	thoughts:           Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	goal:               Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	report:             Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	ops:                Option<RawSchema>,
	#[serde(skip_serializing_if = "Option::is_none")]
	paths:              Option<RawSchema>,
	#[serde(skip_serializing_if = "Option::is_none")]
	op:                 Option<RawSchema>,
	#[serde(skip_serializing_if = "Option::is_none")]
	list:               Option<RawSchema>,
	#[serde(skip_serializing_if = "Option::is_none")]
	task:               Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	phase:              Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	items:              Option<RawSchema>,
	#[serde(skip_serializing_if = "Option::is_none")]
	reason:             Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	timeout:            Option<NumberProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	cwd:                Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	agent: Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	name: Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	context: Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	error: Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	objective: Option<StringProperty>,
	#[serde(skip_serializing_if = "Option::is_none")]
	questions: Option<RawSchema>,
	#[serde(skip_serializing_if = "Option::is_none")]
	data: Option<RawSchema>,
	#[serde(skip_serializing_if = "Option::is_none")]
	token_budget: Option<NumberProperty>,
}

/// A literal JSON Schema fragment for shapes the typed properties cannot
/// express (enums, arrays of objects).
#[derive(Debug, Clone, Copy)]
struct RawSchema(&'static str);

impl Serialize for RawSchema {
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serde_json::value::RawValue::from_string(self.0.to_owned())
			.map_err(serde::ser::Error::custom)?
			.serialize(serializer)
	}
}

const STRING_ARRAY: RawSchema = RawSchema(r#"{"type":"array","items":{"type":"string"}}"#);

#[derive(Debug, Clone, Copy, Serialize)]
struct NumberProperty {
	#[serde(rename = "type")]
	kind: &'static str,
}
const NUMBER: NumberProperty = NumberProperty { kind: "number" };

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

/// Whether `name` is one of the tools offered to the model.
pub fn is_tool(name: &str) -> bool {
	core_specs().iter().any(|spec| spec.function.name == name)
}

pub(crate) fn core_specs() -> Vec<ToolSpec> {
	vec![
		spec(
			"read",
			include_str!("prompts/tools/read.md").trim_end(),
			&ToolProperties { path: Some(STRING), ..EMPTY },
			&["path"],
		),
		spec(
			"write",
			include_str!("prompts/tools/write.md").trim_end(),
			&ToolProperties { path: Some(STRING), content: Some(STRING), ..EMPTY },
			&["path", "content"],
		),
		spec(
			"edit",
			pi_edit::description(pi_edit::EditMode::Hashline).trim_end(),
			&ToolProperties { input: Some(STRING), ..EMPTY },
			&["input"],
		),
		spec(
			"grep",
			include_str!("prompts/tools/grep.md").trim_end(),
			&ToolProperties {
				pattern: Some(STRING),
				path: Some(STRING),
				case_sensitive: Some(BOOLEAN),
				gitignore: Some(BOOLEAN),
				skip: Some(NUMBER),
				..EMPTY
			},
			&["pattern"],
		),
		spec(
			"glob",
			include_str!("prompts/tools/glob.md").trim_end(),
			&ToolProperties {
				path: Some(STRING),
				hidden: Some(BOOLEAN),
				gitignore: Some(BOOLEAN),
				limit: Some(NUMBER),
				..EMPTY
			},
			&[],
		),
		spec(
			"bash",
			include_str!("prompts/tools/bash.md").trim_end(),
			&ToolProperties {
				command: Some(STRING),
				timeout: Some(NUMBER),
				cwd: Some(STRING),
				..EMPTY
			},
			&["command"],
		),
		spec(
			"todo",
			include_str!("prompts/tools/todo.md").trim_end(),
			&ToolProperties {
				op: Some(RawSchema(
					r#"{"type":"string","enum":["init","start","done","rm","drop","block","unblock","append","view"]}"#,
				)),
				list: Some(RawSchema(
					r#"{"type":"array","items":{"type":"object","properties":{"phase":{"type":"string"},"items":{"type":"array","items":{"type":"string"},"minItems":1}},"required":["phase","items"]}}"#,
				)),
				task: Some(STRING),
				phase: Some(STRING),
				items: Some(STRING_ARRAY),
				reason: Some(STRING),
				..EMPTY
			},
			&["op"],
		),
		spec(
			"ast_grep",
			include_str!("prompts/tools/ast-grep.md").trim_end(),
			&ToolProperties { pat: Some(STRING), path: Some(STRING), skip: Some(NUMBER), ..EMPTY },
			&["pat"],
		),
		spec(
			"ast_edit",
			include_str!("prompts/tools/ast-edit.md").trim_end(),
			&ToolProperties {
				ops: Some(RawSchema(
					r#"{"type":"array","minItems":1,"items":{"type":"object","properties":{"pat":{"type":"string"},"out":{"type":"string"}},"required":["pat","out"]}}"#,
				)),
				paths: Some(RawSchema(r#"{"type":"array","minItems":1,"items":{"type":"string"}}"#)),
				..EMPTY
			},
			&["ops", "paths"],
		),
		spec(
			"web_search",
			include_str!("prompts/tools/web-search.md").trim_end(),
			&ToolProperties {
				query: Some(STRING),
				recency: Some(RawSchema(r#"{"type":"string","enum":["day","week","month","year"]}"#)),
				limit: Some(NUMBER),
				num_search_results: Some(NUMBER),
				..EMPTY
			},
			&["query"],
		),
		spec(
			"think",
			"private scratchpad; not shown to user",
			&ToolProperties { thoughts: Some(STRING), ..EMPTY },
			&["thoughts"],
		),
		spec(
			"checkpoint",
			include_str!("prompts/tools/checkpoint.md").trim_end(),
			&ToolProperties { goal: Some(STRING), ..EMPTY },
			&["goal"],
		),
		spec(
			"rewind",
			include_str!("prompts/tools/rewind.md").trim_end(),
			&ToolProperties { report: Some(STRING), ..EMPTY },
			&["report"],
		),
		spec(
			"task",
			include_str!("prompts/tools/task.md").trim_end(),
			&ToolProperties {
				agent: Some(STRING),
				name: Some(STRING),
				context: Some(STRING),
				task: Some(STRING),
				..EMPTY
			},
			&["task"],
		),
		spec(
			"ask",
			include_str!("prompts/tools/ask.md").trim_end(),
			&ToolProperties {
				questions: Some(RawSchema(
					r#"{"type":"array","minItems":1,"items":{"type":"object","properties":{"id":{"type":"string"},"question":{"type":"string"},"options":{"type":"array","items":{"type":"object","properties":{"label":{"type":"string"},"description":{"type":"string"}},"required":["label"]}},"multi":{"type":"boolean"},"recommended":{"type":"number"}},"required":["id","question","options"]}}"#,
				)),
				..EMPTY
			},
			&["questions"],
		),
		spec(
			"yield",
			include_str!("prompts/tools/yield.md").trim_end(),
			&ToolProperties { data: Some(RawSchema("{}")), error: Some(STRING), ..EMPTY },
			&[],
		),
		spec(
			"goal",
			include_str!("prompts/tools/goal.md").trim_end(),
			&ToolProperties {
				op: Some(RawSchema(
					r#"{"type":"string","enum":["create","get","resume","complete","drop"]}"#,
				)),
				objective: Some(STRING),
				token_budget: Some(NUMBER),
				..EMPTY
			},
			&["op"],
		),
	]
}

const EMPTY: ToolProperties = ToolProperties {
	case_sensitive:     None,
	gitignore:          None,
	path:               None,
	old:                None,
	new:                None,
	content:            None,
	command:            None,
	pattern:            None,
	input:              None,
	hidden:             None,
	limit:              None,
	skip:               None,
	pat:                None,
	query:              None,
	recency:            None,
	num_search_results: None,
	thoughts:           None,
	goal:               None,
	report:             None,
	ops:                None,
	paths:              None,
	op:                 None,
	list:               None,
	task:               None,
	phase:              None,
	items:              None,
	reason:             None,
	timeout:            None,
	cwd:                None,
	agent: None,
	name: None,
	context: None,
	error: None,
	objective: None,
	questions: None,
	data: None,
	token_budget: None,
};

const fn spec(
	name: &'static str,
	description: &'static str,
	properties: &ToolProperties,
	required: &'static [&'static str],
) -> ToolSpec {
	ToolSpec {
		kind:     "function",
		function: ToolFunction {
			name,
			description,
			parameters: ToolParameters { kind: "object", properties: *properties, required },
		},
	}
}

/// Main's tool schemas as `(name, description, parameters)` for the runtime
/// spec list.
pub(crate) fn schemas() -> Vec<(String, String, serde_json::Value)> {
	core_specs()
		.into_iter()
		.map(|spec| {
			let value = serde_json::to_value(spec).expect("static tool schema serializes");
			let function = &value["function"];
			(
				function["name"].as_str().unwrap_or_default().to_owned(),
				function["description"]
					.as_str()
					.unwrap_or_default()
					.to_owned(),
				function["parameters"].clone(),
			)
		})
		.collect()
}
