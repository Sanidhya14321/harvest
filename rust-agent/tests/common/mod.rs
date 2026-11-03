//! Bounded loopback HTTP fixtures shared by native provider contract suites.
use std::{
	io::{BufRead, BufReader, Read, Write},
	net::{SocketAddr, TcpListener},
	thread::{self, JoinHandle},
	time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde_json::Value;

pub struct HttpFixture {
	address: SocketAddr,
	server:  JoinHandle<Result<Vec<Value>>>,
}
impl HttpFixture {
	pub fn spawn(
		status: u16,
		reason: &'static str,
		content_type: &'static str,
		body: String,
	) -> Result<Self> {
		Self::spawn_inspecting(status, reason, content_type, body, |_, _| Ok(()))
	}

	pub fn spawn_inspecting(
		status: u16,
		reason: &'static str,
		content_type: &'static str,
		body: String,
		inspect: impl FnOnce(&str, &Value) -> Result<()> + Send + 'static,
	) -> Result<Self> {
		let mut inspect = Some(inspect);
		Self::spawn_sequence(status, reason, content_type, vec![body], move |headers, value| {
			inspect
				.take()
				.context("fixture inspector already consumed")?(headers, value)
		})
	}

	pub fn spawn_sequence(
		status: u16,
		reason: &'static str,
		content_type: &'static str,
		bodies: Vec<String>,
		mut inspect: impl FnMut(&str, &Value) -> Result<()> + Send + 'static,
	) -> Result<Self> {
		let listener = TcpListener::bind("127.0.0.1:0")?;
		listener.set_nonblocking(true)?;
		let address = listener.local_addr()?;
		let server = thread::spawn(move || -> Result<Vec<Value>> {
			let mut values = Vec::new();
			for body in bodies {
				let deadline = Instant::now() + Duration::from_secs(10);
				let mut socket = loop {
					match listener.accept() {
						Ok((socket, _)) => break socket,
						Err(error)
							if error.kind() == std::io::ErrorKind::WouldBlock
								&& Instant::now() < deadline =>
						{
							thread::sleep(Duration::from_millis(5));
						},
						Err(error) => return Err(error.into()),
					}
				};
				// Winsock inherits the listener's nonblocking mode; Unix accept does
				// not. These bounded request reads require blocking sockets on both.
				socket.set_nonblocking(false)?;
				socket.set_read_timeout(Some(Duration::from_secs(5)))?;
				let mut reader = BufReader::new(&mut socket);
				let mut headers = Vec::new();
				let mut length = None;
				loop {
					let mut line = Vec::new();
					let read = (&mut reader)
						.take(16 * 1024 + 1)
						.read_until(b'\n', &mut line)?;
					headers.extend_from_slice(&line);
					if headers.len() > 16 * 1024 || read == 0 {
						bail!("invalid fixture HTTP headers");
					}
					if line == b"\r\n" {
						break;
					}
					let text = std::str::from_utf8(&line)?;
					if let Some((name, value)) = text.split_once(':')
						&& name.eq_ignore_ascii_case("content-length")
					{
						length = Some(value.trim().parse::<usize>()?);
					}
				}
				let length = length.context("missing request length")?;
				if length > 1024 * 1024 {
					bail!("fixture request exceeds limit");
				}
				let mut request = vec![0; length];
				reader.read_exact(&mut request)?;
				let value = serde_json::from_slice(&request)?;
				inspect(std::str::from_utf8(&headers)?, &value)?;
				drop(reader);
				write!(
					socket,
					"HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: \
					 {}\r\nConnection: close\r\n\r\n{body}",
					body.len()
				)?;
				values.push(value);
			}
			Ok(values)
		});
		Ok(Self { address, server })
	}

	pub fn base_url(&self) -> String {
		format!("http://{}/v1", self.address)
	}

	pub fn finish(self) -> Result<Value> {
		self
			.finish_sequence()?
			.pop()
			.context("fixture served no requests")
	}

	pub fn finish_sequence(self) -> Result<Vec<Value>> {
		self
			.server
			.join()
			.map_err(|_| anyhow::anyhow!("fixture server panicked"))?
	}
}
