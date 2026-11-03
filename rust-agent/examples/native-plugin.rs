//! Native plugin example. Build with Cargo, then explicitly select its binary
//! using harvest run --plugin PATH. No owned interpreter or browser glue.
use anyhow::Result;
use harvest_omp::{
	chat::{FunctionCall, ToolCall, ToolSpec},
	plugins::sdk::{self, Context, Handler},
};
use serde_json::{Value, json};

struct WorkspaceRead;
impl Handler for WorkspaceRead {
	fn name(&self) -> &str {
		"workspace-read"
	}

	fn tools(&self) -> Vec<ToolSpec> {
		vec![ToolSpec::from_schema("plugin_read".into(),include_str!("../src/prompts/tools/read.md").trim().into(),json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false})).expect("static object schema")]
	}

	fn call(&mut self, _name: &str, arguments: Value, context: &mut Context<'_>) -> Result<Value> {
		context.cancellation.heartbeat()?;
		let result = context.host_call(ToolCall {
			id:       "read".into(),
			kind:     "function".into(),
			function: FunctionCall {
				name:      "read".into(),
				arguments: serde_json::to_string(&arguments)?,
			},
		})?;
		Ok(json!({"text":result}))
	}
}
fn main() -> Result<()> {
	sdk::serve_stdio(&mut WorkspaceRead)
}
