use std::time::Duration;

use harvest_runtime::cancel::{AbortReason, CancelToken};

#[tokio::test]
async fn cancellation_wakes_every_consumer_and_preserves_the_first_reason() {
	let mut cancellation = CancelToken::default();
	let abort = cancellation.emplace_abort_token();
	let consumers: Vec<_> = (0..8)
		.map(|_| {
			let token = cancellation.clone();
			tokio::spawn(async move { token.wait().await })
		})
		.collect();
	abort.abort(AbortReason::User);
	abort.abort(AbortReason::Signal);
	for consumer in consumers {
		assert!(matches!(
			tokio::time::timeout(Duration::from_secs(1), consumer)
				.await
				.unwrap()
				.unwrap(),
			AbortReason::User
		));
	}
	assert!(cancellation.heartbeat().is_err());
}

#[tokio::test]
async fn deadline_interrupts_a_wait_without_an_external_abort() {
	let token = CancelToken::with_timeout(Some(Duration::from_millis(5)));
	assert!(matches!(
		tokio::time::timeout(Duration::from_secs(1), token.wait())
			.await
			.unwrap(),
		AbortReason::Timeout
	));
	assert!(token.heartbeat().is_err());
}
