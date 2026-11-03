//! Credentials and untrusted provider text must never enter native diagnostics.
use anyhow::Result;
use harvest_omp::{
	chat::user,
	config::Settings,
	provider::{HttpModel, Model, ProviderRequestError},
};

mod common;

fn model(base_url: String) -> HttpModel {
	HttpModel::new(Settings {
		model: "fixture".into(),
		base_url,
		api_key: "fixture-api-key-private".into(),
		max_steps: 1,
	})
}

#[test]
fn http_status_errors_keep_numeric_categories_without_url_body_or_reason_secrets() -> Result<()> {
	for (streaming, status) in [(false, 401), (true, 429)] {
		let server = common::HttpFixture::spawn(
			status,
			"Denied-fixture-reason-private",
			"application/json",
			r#"{"error":{"message":"fixture-body-private"}}"#.into(),
		)?;
		let base_url =
			server
				.base_url()
				.replacen("http://", "http://fixture-user:fixture-url-private@", 1);
		let model = model(base_url);
		let result = if streaming {
			model.stream_response(&[user("hi")], &[], &Default::default(), &mut |_| Ok(()))
		} else {
			model.complete_response(&[user("hi")], &[])
		};
		let error = result.unwrap_err();
		assert!(
			matches!(error.downcast_ref::<ProviderRequestError>(), Some(ProviderRequestError::Status(code)) if *code == status)
		);
		let diagnostic = format!("{error:#}");
		assert!(diagnostic.contains(&status.to_string()));
		for secret in [
			"fixture-reason-private",
			"fixture-body-private",
			"fixture-url-private",
			"fixture-api-key-private",
			"fixture-user",
		] {
			assert!(!diagnostic.contains(secret));
		}
		assert_eq!(server.finish()?["stream"], streaming);
	}
	Ok(())
}

#[test]
fn invalid_transport_urls_and_debug_output_do_not_expose_configured_credentials() {
	let model = model(
		"invalid-scheme://fixture-user:fixture-password@localhost/?token=fixture-query-private"
			.into(),
	);
	let debug = format!("{model:?}");
	let error = model.complete_response(&[user("hi")], &[]).unwrap_err();
	assert!(matches!(
		error.downcast_ref::<ProviderRequestError>(),
		Some(ProviderRequestError::Transport(_))
	));
	let diagnostic = format!("{error:#}");
	assert!(diagnostic.contains("model transport failed"));
	for secret in
		["fixture-user", "fixture-password", "fixture-query-private", "fixture-api-key-private"]
	{
		assert!(!debug.contains(secret));
		assert!(!diagnostic.contains(secret));
	}
}

#[test]
fn unsuccessful_finish_reasons_are_rejected_without_echoing_untrusted_text() -> Result<()> {
	let marker = "fixture-provider-private";
	for (body, content_type) in [
		(
			format!(
				r#"{{"choices":[{{"message":{{"role":"assistant","content":"partial"}},"finish_reason":"{marker}"}}]}}"#
			),
			"application/json",
		),
		(
			format!(
				"data: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"{marker}\"\
				 }}]}}\n\ndata: [DONE]\n\n"
			),
			"text/event-stream",
		),
	] {
		let server = common::HttpFixture::spawn(200, "OK", content_type, body)?;
		let error = model(server.base_url())
			.stream_response(&[user("hi")], &[], &Default::default(), &mut |_| Ok(()))
			.unwrap_err();
		let diagnostic = format!("{error:#}");
		assert!(diagnostic.contains("unsuccessful finish reason"));
		assert!(!diagnostic.contains(marker));
		server.finish()?;
	}
	Ok(())
}
