use std::{
	io::{Read, Write},
	net::{TcpListener, TcpStream},
	sync::mpsc,
	thread,
	time::{Duration, Instant},
};

use harvest_omp::{
	chat::user,
	config::Settings,
	provider::{HttpModel, Model},
};
use harvest_runtime::cancel::{AbortReason, CancelToken};

fn read_request(socket: &mut TcpStream) {
	socket
		.set_read_timeout(Some(Duration::from_secs(3)))
		.unwrap();
	let mut bytes = Vec::new();
	let mut buffer = [0_u8; 1024];
	loop {
		let count = socket.read(&mut buffer).unwrap();
		assert_ne!(count, 0, "client must send a complete request");
		bytes.extend_from_slice(&buffer[..count]);
		assert!(bytes.len() < 64 * 1024, "fixture request must stay bounded");
		if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
			let header = String::from_utf8_lossy(&bytes[..end]);
			let length: usize = header
				.lines()
				.find_map(|line| {
					line
						.to_ascii_lowercase()
						.strip_prefix("content-length:")
						.map(|value| value.trim().parse().unwrap())
				})
				.unwrap();
			if bytes.len() >= end + 4 + length {
				return;
			}
		}
	}
}

fn model(listener: &TcpListener) -> HttpModel {
	HttpModel::new(Settings {
		model:     "fixture".into(),
		base_url:  format!("http://{}/v1", listener.local_addr().unwrap()),
		api_key:   String::new(),
		max_steps: 1,
	})
}

fn stalled_request_cancels(partial_body: Option<&'static str>) {
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	let model = model(&listener);
	let (ready, waiting) = mpsc::sync_channel(1);
	let server = thread::spawn(move || {
		let (mut socket, _) = listener.accept().unwrap();
		read_request(&mut socket);
		if let Some(body) = partial_body {
			write!(
				socket,
				"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: \
				 100000\r\n\r\n{body}"
			)
			.unwrap();
			socket.flush().unwrap();
		}
		ready.send(()).unwrap();
		let mut byte = [0_u8; 1];
		assert_eq!(
			socket.read(&mut byte).unwrap(),
			0,
			"cancellation must close the pending HTTP connection"
		);
	});
	let mut token = CancelToken::default();
	let abort = token.emplace_abort_token();
	let canceller = thread::spawn(move || {
		waiting.recv_timeout(Duration::from_secs(3)).unwrap();
		if partial_body.is_some() {
			thread::sleep(Duration::from_millis(50));
		}
		abort.abort(AbortReason::User);
	});
	let started = Instant::now();
	let mut deltas = Vec::new();
	let error = model
		.stream(&[user("probe")], &[], &token, &mut |text| {
			deltas.push(text.to_owned());
			Ok(())
		})
		.unwrap_err();
	assert!(error.to_string().contains("Aborted: User"), "{error:#}");
	assert!(
		started.elapsed() < Duration::from_secs(1),
		"cancellation cannot wait for the 60-second network timeout"
	);
	assert!(deltas.is_empty(), "a partial frame cannot become a text delta");
	canceller.join().unwrap();
	server.join().unwrap();
}

#[test]
fn cancellation_interrupts_pending_response_headers_and_closes_connection() {
	stalled_request_cancels(None);
}

#[test]
fn cancellation_interrupts_a_pending_read_in_a_fragmented_sse_event() {
	stalled_request_cancels(Some("data: {\"choices\":"));
}

#[test]
fn streamed_callbacks_stay_on_the_host_thread_and_can_reenter_the_provider() {
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	let model = model(&listener);
	let server = thread::spawn(move || {
		let (mut outer, _) = listener.accept().unwrap();
		read_request(&mut outer);
		let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"outer\"},\"\
		            finish_reason\":null}]}\n\ndata: \
		            {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: \
		            [DONE]\n\n";
		write!(
			outer,
			"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
			body.len()
		)
		.unwrap();
		outer.flush().unwrap();
		let (mut inner, _) = listener.accept().unwrap();
		read_request(&mut inner);
		let body = "{\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"inner\"}}]}";
		write!(inner, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
	});
	let host = thread::current().id();
	let mut nested = None;
	let outer = model
		.stream(&[user("outer")], &[], &CancelToken::default(), &mut |text| {
			assert_eq!(
				thread::current().id(),
				host,
				"host callbacks cannot move to a transport worker"
			);
			assert_eq!(text, "outer");
			nested = model.complete(&[user("inner")], &[])?.content;
			Ok(())
		})
		.unwrap();
	assert_eq!(outer.content.as_deref(), Some("outer"));
	assert_eq!(nested.as_deref(), Some("inner"));
	server.join().unwrap();
}
