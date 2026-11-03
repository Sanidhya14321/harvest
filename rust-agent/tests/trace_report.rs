use std::sync::{
	Mutex,
	atomic::{AtomicUsize, Ordering},
};

use anyhow::{Result, bail};
use harvest_omp::{
	chat::{ChatMessage, ToolSpec},
	provider::Model,
	trace_report::{self, Entry, Item, Options, Trace},
};
use harvest_runtime::cancel::CancelToken;
#[derive(Default)]
struct Recorder {
	prompts: Mutex<Vec<String>>,
	calls:   AtomicUsize,
	reject:  bool,
}
impl Model for Recorder {
	fn model_id(&self) -> &str {
		"scripted"
	}

	fn complete(&self, messages: &[ChatMessage], tools: &[ToolSpec]) -> Result<ChatMessage> {
		assert!(tools.is_empty());
		self.calls.fetch_add(1, Ordering::Relaxed);
		let prompt = messages[1].content.as_deref().unwrap();
		self.prompts.lock().unwrap().push(prompt.into());
		if self.reject {
			return Ok(ChatMessage {
				role:         "assistant".into(),
				content:      Some("never".into()),
				tool_calls:   Some(vec![harvest_omp::chat::ToolCall {
					id:       "x".into(),
					kind:     "function".into(),
					function: harvest_omp::chat::FunctionCall {
						name:      "shell".into(),
						arguments: "{}".into(),
					},
				}]),
				tool_call_id: None,
			});
		}
		Ok(ChatMessage {
			role:         "assistant".into(),
			content:      Some(
				if prompt.contains("One sentence:") {
					if prompt.contains("first prose") {
						"The agent inspected first.rs."
					} else {
						"The agent inspected second.rs."
					}
				} else {
					"### Story Arc\n- **Turns 1–3 (3 turns): inspection**: Results show inspection."
				}
				.into(),
			),
			tool_calls:   None,
			tool_call_id: None,
		})
	}
}
fn sample() -> Trace {
	Trace {
		job_name:     "run".into(),
		trace:        "trace".into(),
		total_events: 10,
		entries:      vec![
			Entry::Assistant {
				model: "a".into(),
				text:  "first prose".into(),
				tools: vec!["read".into(), "read".into()],
			},
			Entry::Notice { text: "checkpoint".into() },
			Entry::ToolResult {
				tool:     "read".into(),
				is_error: true,
				text:     "first.rs missing".into(),
			},
			Entry::Assistant { model: "b".into(), text: "second prose".into(), tools: vec![] },
		],
	}
}
fn options() -> Options {
	Options {
		run:         "run".into(),
		trace:       "trace".into(),
		focus:       Some("known correct fix".into()),
		concurrency: 4,
	}
}
#[test]
fn grouping_keeps_notice_order_and_results_on_their_turn() -> Result<()> {
	let items = trace_report::group(&sample())?;
	assert!(matches!(items[1], Item::Notice { .. }));
	assert!(matches!(&items[0],Item::Turn{results,..} if results.len()==1 && results[0].is_error));
	let mut orphan = sample();
	orphan.entries.remove(0);
	assert!(trace_report::group(&orphan).is_err());
	Ok(())
}
#[test]
fn map_reduce_keeps_order_usage_unknown_and_failure_focus() -> Result<()> {
	let tiny = Recorder::default();
	let synth = Recorder::default();
	let run = serde_json::from_value(
		serde_json::json!({"run":{"jobName":"run","benchmark":"edit","dataset":"inert","models":"source-model"},"traces":[{"name":"trace","task":"task","status":"fail","reward":0,"costUsd":null,"durationMs":1000}]}),
	)?;
	let report = trace_report::generate(
		&sample(),
		Some(&run),
		&options(),
		&tiny,
		&synth,
		&CancelToken::default(),
	)?;
	assert_eq!((report.items, report.turns), (3, 2));
	assert_eq!((report.tiny_usage.calls, report.tiny_usage.unavailable_calls), (2, 2));
	assert!(report.markdown.contains("`read` ×2 (errored: `read`)"));
	assert!(report.markdown.find("first.rs").unwrap() < report.markdown.find("second.rs").unwrap());
	let prompts = synth.prompts.lock().unwrap();
	assert!(prompts[0].contains("LAST bullet MUST be **Failure analysis**"));
	assert!(prompts[0].contains("known correct fix"));
	assert!(report.markdown.contains("cost unavailable"));
	Ok(())
}
#[test]
fn invalid_identity_metadata_and_cancellation_cost_no_requests() -> Result<()> {
	let model = Recorder::default();
	let mut opts = options();
	opts.trace = "different".into();
	assert!(
		trace_report::generate(&sample(), None, &opts, &model, &model, &CancelToken::default())
			.is_err()
	);
	let mut token = CancelToken::default();
	let abort = token.emplace_abort_token();
	abort.abort(harvest_runtime::cancel::AbortReason::User);
	assert!(trace_report::generate(&sample(), None, &options(), &model, &model, &token).is_err());
	assert_eq!(model.calls.load(Ordering::Relaxed), 0);
	Ok(())
}
#[test]
fn model_tools_cannot_execute_and_reduce_never_starts() -> Result<()> {
	let tiny = Recorder { reject: true, ..Default::default() };
	let synth = Recorder::default();
	assert!(
		trace_report::generate(&sample(), None, &options(), &tiny, &synth, &CancelToken::default())
			.is_err()
	);
	assert_eq!(synth.calls.load(Ordering::Relaxed), 0);
	Ok(())
}
#[test]
fn unsupported_and_oversized_source_are_errors() -> Result<()> {
	assert!(trace_report::decode(br#"{"jobName":"run","trace":"trace","entries":[{"kind":"conversation","text":"unsupported"}],"totalEvents":1}"#).is_err());
	assert!(trace_report::decode(&vec![b' '; 8 * 1024 * 1024 + 1]).is_err());
	if trace_report::fetch("http://user:secret@localhost/", "run", "trace", &CancelToken::default())
		.is_ok()
	{
		bail!("credential URL admitted");
	}
	Ok(())
}
#[test]
fn native_branch_projection_omits_private_fields_and_never_changes_source() -> Result<()> {
	let bytes = br#"{"type":"session","version":3,"id":"source","cwd":"/tmp"}
{"type":"message","id":"a","parentId":null,"message":{"role":"assistant","model":"m","content":[{"type":"text","text":"intent"},{"type":"thinking","thinking":"private thought"},{"type":"toolCall","id":"c","name":"read","arguments":{"path":"file"}}]}}
{"type":"message","id":"b","parentId":"a","message":{"role":"toolResult","toolCallId":"c","toolName":"read","isError":false,"content":[{"type":"text","text":"file result"}]}}
"#;

	let original = bytes.to_vec();
	let trace = trace_report::from_session(bytes, "b", "run", "trace", &CancelToken::default())?;
	assert_eq!(bytes.as_slice(), original.as_slice());
	assert_eq!(trace.entries.len(), 2);
	let serialized = serde_json::to_string(&trace)?;
	assert!(!serialized.contains("private thought"));
	assert!(!serialized.contains("arguments"));
	assert!(serialized.contains("file result"));
	let mut malformed = bytes.to_vec();
	malformed.extend_from_slice(b"broken\n");
	assert!(
		trace_report::from_session(&malformed, "b", "run", "trace", &CancelToken::default()).is_err()
	);
	Ok(())
}
