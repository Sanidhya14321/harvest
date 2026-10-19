//! Bounded off-thread output for an unbuffered terminal writer.
use std::{
	io::{self, Write},
	num::NonZeroUsize,
	sync::{
		Arc, Condvar, Mutex, MutexGuard, PoisonError,
		atomic::{AtomicBool, AtomicUsize, Ordering},
	},
	thread::JoinHandle,
	time::{Duration, Instant},
};

pub const MAX_QUEUED_BYTES: usize = 256 * 1024 * 1024;
const WRITE_BYTES: usize = 16 * 1024;
const RETAIN_BYTES: usize = 16 * 1024 * 1024;
struct Queue {
	bytes:  Vec<u8>,
	closed: bool,
}
struct Inner {
	queue:   Mutex<Queue>,
	cv:      Condvar,
	pending: AtomicUsize,
	abort:   AtomicBool,
	dead:    AtomicBool,
	limit:   usize,
}
impl Inner {
	fn queue(&self) -> MutexGuard<'_, Queue> {
		self.queue.lock().unwrap_or_else(PoisonError::into_inner)
	}

	fn progress(&self, count: usize) {
		let previous = self.pending.fetch_sub(count, Ordering::AcqRel);
		if previous == count {
			// Flush waiters need the empty-queue transition, not each partial
			// write. Pair notification with their predicate lock so the final
			// progress cannot land between a predicate check and parking.
			let _queue = self.queue();
			self.cv.notify_all();
		}
	}

	fn fail(&self, remaining: usize) {
		let mut queue = self.queue();
		let dropped = queue.bytes.len() + remaining;
		queue.bytes.clear();
		queue.closed = true;
		self.pending.fetch_sub(dropped, Ordering::AcqRel);
		self.dead.store(true, Ordering::Release);
		self.cv.notify_all();
	}
}
struct WorkerExit<'a>(&'a Inner);
impl Drop for WorkerExit<'_> {
	fn drop(&mut self) {
		if std::thread::panicking() {
			let mut queue = self.0.queue();
			queue.bytes.clear();
			queue.closed = true;
			self.0.pending.store(0, Ordering::Release);
			self.0.dead.store(true, Ordering::Release);
			self.0.cv.notify_all();
		}
	}
}

struct Reservation<'a> {
	bytes:     &'a mut Vec<u8>,
	start:     usize,
	committed: bool,
}
impl Drop for Reservation<'_> {
	fn drop(&mut self) {
		if !self.committed {
			self.bytes.truncate(self.start);
		}
	}
}

/// Enqueue work without waiting for the terminal to drain.
///
/// The backend must perform unbuffered writes; pending counts bytes accepted
/// by that backend.
/// A timed-out stop detaches the blocked operation. The worker owns the backend
/// and releases it when that operation returns, without closing a reused fd.
pub struct TtyWriter {
	inner:  Arc<Inner>,
	thread: Option<JoinHandle<()>>,
}
impl TtyWriter {
	pub fn new(writer: impl Write + Send + 'static) -> io::Result<Self> {
		Self::with_limit(writer, NonZeroUsize::new(MAX_QUEUED_BYTES).unwrap())
	}

	pub fn with_limit(writer: impl Write + Send + 'static, limit: NonZeroUsize) -> io::Result<Self> {
		if limit.get() > MAX_QUEUED_BYTES {
			return Err(io::Error::new(
				io::ErrorKind::InvalidInput,
				"terminal queue exceeds the supported limit",
			));
		}
		let inner = Arc::new(Inner {
			queue:   Mutex::new(Queue { bytes: Vec::new(), closed: false }),
			cv:      Condvar::new(),
			pending: AtomicUsize::new(0),
			abort:   AtomicBool::new(false),
			dead:    AtomicBool::new(false),
			limit:   limit.get(),
		});
		let state = Arc::clone(&inner);
		let thread = std::thread::Builder::new()
			.name("tty-writer".into())
			.spawn(move || {
				let _exit = WorkerExit(&state);
				pump(writer, &state);
			})?;
		Ok(Self { inner, thread: Some(thread) })
	}

	/// Accept an entire frame and return the current pending-byte count.
	pub fn enqueue(&self, bytes: &[u8]) -> io::Result<usize> {
		self.enqueue_with(bytes.len(), |output| {
			output.copy_from_slice(bytes);
			Ok(bytes.len())
		})
	}

	/// Reserve bounded output and fill it under the queue lock. On error or
	/// panic the reservation is rolled back; no partial frame is published.
	pub fn enqueue_with(
		&self,
		maximum: usize,
		fill: impl FnOnce(&mut [u8]) -> io::Result<usize>,
	) -> io::Result<usize> {
		let mut queue = self.inner.queue();
		if queue.closed || self.dead() {
			return Err(io::ErrorKind::BrokenPipe.into());
		}
		if maximum > self.inner.limit.saturating_sub(self.pending()) {
			return Err(io::Error::new(
				io::ErrorKind::WouldBlock,
				"terminal output queue limit exceeded",
			));
		}
		if maximum == 0 {
			return Ok(self.pending());
		}
		queue
			.bytes
			.try_reserve(maximum)
			.map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
		let start = queue.bytes.len();
		let reservation = Reservation { bytes: &mut queue.bytes, start, committed: false };
		reservation.bytes.resize(start + maximum, 0);
		let mut reservation = reservation;
		let count = fill(&mut reservation.bytes[start..])?;
		if count > maximum {
			return Err(io::ErrorKind::InvalidData.into());
		}
		reservation.bytes.truncate(start + count);
		// Account before unlocking; the worker can never drain uncounted bytes.
		self.inner.pending.fetch_add(count, Ordering::AcqRel);
		reservation.committed = true;
		drop(reservation);
		drop(queue);
		self.inner.cv.notify_all();
		Ok(self.pending())
	}

	/// Bytes accepted but not yet written or discarded after backend failure.
	pub fn pending(&self) -> usize {
		self.inner.pending.load(Ordering::Acquire)
	}

	/// Whether the backend failed and its remaining output was discarded.
	pub fn dead(&self) -> bool {
		self.inner.dead.load(Ordering::Acquire)
	}

	/// Wait for an empty queue. `dead` distinguishes discarded output from
	/// delivery.
	pub fn flush_sync(&self, timeout: Duration) -> bool {
		let start = Instant::now();
		let mut queue = self.inner.queue();
		while self.pending() > 0 && !self.dead() {
			let Some(remaining) = timeout.checked_sub(start.elapsed()) else {
				return false;
			};
			if remaining.is_zero() {
				return false;
			}
			let (guard, _) = self
				.inner
				.cv
				.wait_timeout(queue, remaining)
				.unwrap_or_else(PoisonError::into_inner);
			queue = guard;
		}
		self.pending() == 0
	}

	pub fn stop(&mut self, timeout: Duration) -> bool {
		let Some(thread) = self.thread.take() else {
			return self.pending() == 0;
		};
		{
			self.inner.queue().closed = true;
		}
		self.inner.cv.notify_all();
		let drained = self.flush_sync(timeout);
		if drained || self.dead() {
			let _ = thread.join();
		} else {
			self.inner.abort.store(true, Ordering::Release);
			let mut queue = self.inner.queue();
			let dropped = queue.bytes.len();
			queue.bytes.clear();
			self.inner.pending.fetch_sub(dropped, Ordering::AcqRel);
			drop(queue);
			self.inner.cv.notify_all();
		}
		drained
	}
}
impl Drop for TtyWriter {
	fn drop(&mut self) {
		self.stop(Duration::ZERO);
	}
}
fn pump(mut writer: impl Write, inner: &Inner) {
	let mut front = Vec::new();
	loop {
		{
			let mut queue = inner.queue();
			while queue.bytes.is_empty() {
				if queue.closed {
					return;
				}
				queue = inner.cv.wait(queue).unwrap_or_else(PoisonError::into_inner);
			}
			std::mem::swap(&mut queue.bytes, &mut front);
		}
		let mut offset = 0;
		while offset < front.len() {
			if inner.abort.load(Ordering::Acquire) {
				inner.fail(front.len() - offset);
				return;
			}
			let end = (offset + WRITE_BYTES).min(front.len());
			match writer.write(&front[offset..end]) {
				Ok(0) => {
					inner.fail(front.len() - offset);
					return;
				},
				Ok(count) if count <= end - offset => {
					offset += count;
					inner.progress(count);
				},
				Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
				Ok(_) | Err(_) => {
					inner.fail(front.len() - offset);
					return;
				},
			}
		}
		front.clear();
		if front.capacity() > RETAIN_BYTES {
			front.shrink_to_fit();
		}
	}
}

#[cfg(test)]
mod tests {
	use std::sync::mpsc::{self, Receiver, Sender};

	use super::*;
	struct GateWriter {
		start:   Option<Sender<()>>,
		release: Receiver<()>,
		output:  Arc<Mutex<Vec<u8>>>,
		closed:  Option<Sender<()>>,
	}
	impl Write for GateWriter {
		fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
			if let Some(start) = self.start.take() {
				let _ = start.send(());
				self
					.release
					.recv_timeout(Duration::from_secs(5))
					.map_err(|_| io::ErrorKind::TimedOut)?;
			}
			self.output.lock().unwrap().extend_from_slice(bytes);
			Ok(bytes.len())
		}

		fn flush(&mut self) -> io::Result<()> {
			Ok(())
		}
	}
	impl Drop for GateWriter {
		fn drop(&mut self) {
			if let Some(closed) = self.closed.take() {
				let _ = closed.send(());
			}
		}
	}
	#[test]
	fn a_blocked_inflight_write_counts_against_the_queue_cap_without_losing_accepted_bytes() {
		let (started, begin) = mpsc::channel();
		let (release, gate) = mpsc::channel();
		let output = Arc::new(Mutex::new(Vec::new()));
		let sink = GateWriter {
			start:   Some(started),
			release: gate,
			output:  Arc::clone(&output),
			closed:  None,
		};
		let mut writer = TtyWriter::with_limit(sink, NonZeroUsize::new(65536).unwrap()).unwrap();
		let frame = vec![b'a'; 50000];
		writer.enqueue(&frame).unwrap();
		begin.recv_timeout(Duration::from_secs(2)).unwrap();
		assert_eq!(writer.pending(), 50000);
		assert_eq!(writer.enqueue(&vec![b'b'; 20000]).unwrap_err().kind(), io::ErrorKind::WouldBlock);
		assert_eq!(writer.pending(), 50000);
		release.send(()).unwrap();
		assert!(writer.flush_sync(Duration::from_secs(2)));
		writer.enqueue(b"tail").unwrap();
		assert!(writer.stop(Duration::from_secs(2)));
		let mut expected = frame;
		expected.extend_from_slice(b"tail");
		assert_eq!(*output.lock().unwrap(), expected);
		assert_eq!(writer.enqueue(b"late").unwrap_err().kind(), io::ErrorKind::BrokenPipe);
	}
	#[test]
	fn timed_out_stop_returns_while_io_is_blocked_and_releases_the_backend_after_io_resumes() {
		let (started, begin) = mpsc::channel();
		let (release, gate) = mpsc::channel();
		let (closed, dropped) = mpsc::channel();
		let output = Arc::new(Mutex::new(Vec::new()));
		let sink = GateWriter {
			start:   Some(started),
			release: gate,
			output:  Arc::clone(&output),
			closed:  Some(closed),
		};
		let mut writer = TtyWriter::new(sink).unwrap();
		writer.enqueue(&vec![b'x'; WRITE_BYTES * 4]).unwrap();
		begin.recv_timeout(Duration::from_secs(2)).unwrap();
		let start = Instant::now();
		assert!(!writer.stop(Duration::from_millis(10)));
		assert!(start.elapsed() < Duration::from_secs(1));
		assert_eq!(writer.enqueue(b"late").unwrap_err().kind(), io::ErrorKind::BrokenPipe);
		release.send(()).unwrap();
		dropped.recv_timeout(Duration::from_secs(2)).unwrap();
		assert_eq!(*output.lock().unwrap(), vec![b'x'; WRITE_BYTES]);
		assert_eq!(writer.pending(), 0);
	}
	#[cfg(unix)]
	#[test]
	fn real_socket_output_preserves_frame_order_and_failed_reservations_publish_no_partial_frame() {
		use std::{io::Read, os::unix::net::UnixStream};
		let (sink, mut reader) = UnixStream::pair().unwrap();
		reader
			.set_read_timeout(Some(Duration::from_secs(3)))
			.unwrap();
		let consumer = std::thread::spawn(move || {
			let mut bytes = Vec::new();
			reader.read_to_end(&mut bytes).unwrap();
			bytes
		});
		let mut writer = TtyWriter::new(sink).unwrap();
		writer.enqueue(b"prefix\n").unwrap();
		let error = writer
			.enqueue_with(7, |bytes| {
				bytes.copy_from_slice(b"CORRUPT");
				Err(io::ErrorKind::InvalidInput.into())
			})
			.unwrap_err();
		assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
		let frame = "字🙂\n".repeat(10000);
		writer.enqueue(frame.as_bytes()).unwrap();
		assert!(writer.stop(Duration::from_secs(2)));
		assert_eq!(consumer.join().unwrap(), [b"prefix\n".as_slice(), frame.as_bytes()].concat());
		assert_eq!(writer.pending(), 0);
	}
	#[test]
	fn a_failed_backend_drops_queued_output_and_refuses_future_frames() {
		struct Dead;
		impl Write for Dead {
			fn write(&mut self, _: &[u8]) -> io::Result<usize> {
				Err(io::ErrorKind::BrokenPipe.into())
			}

			fn flush(&mut self) -> io::Result<()> {
				Ok(())
			}
		}
		let mut writer = TtyWriter::new(Dead).unwrap();
		writer.enqueue(b"frame").unwrap();
		assert!(writer.flush_sync(Duration::from_secs(2)));
		assert!(writer.dead());
		assert_eq!(writer.pending(), 0);
		assert_eq!(writer.enqueue(b"later").unwrap_err().kind(), io::ErrorKind::BrokenPipe);
		writer.stop(Duration::ZERO);
	}
	#[test]
	fn interrupted_short_writes_preserve_the_unwritten_tail() {
		struct Short {
			attempt: usize,
			output:  Arc<Mutex<Vec<u8>>>,
		}
		impl Write for Short {
			fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
				self.attempt += 1;
				if self.attempt == 1 {
					return Err(io::ErrorKind::Interrupted.into());
				}
				let count = bytes.len().min(3);
				self
					.output
					.lock()
					.unwrap()
					.extend_from_slice(&bytes[..count]);
				Ok(count)
			}

			fn flush(&mut self) -> io::Result<()> {
				Ok(())
			}
		}
		let output = Arc::new(Mutex::new(Vec::new()));
		let mut writer = TtyWriter::new(Short { attempt: 0, output: Arc::clone(&output) }).unwrap();
		writer.enqueue(b"short writes").unwrap();
		assert!(writer.stop(Duration::from_secs(2)));
		assert_eq!(*output.lock().unwrap(), b"short writes");
	}
	#[test]
	fn backend_panics_release_waiters_and_close_admission() {
		struct Panics;
		impl Write for Panics {
			fn write(&mut self, _: &[u8]) -> io::Result<usize> {
				panic!("backend failed");
			}

			fn flush(&mut self) -> io::Result<()> {
				Ok(())
			}
		}
		let mut writer = TtyWriter::new(Panics).unwrap();
		writer.enqueue(b"frame").unwrap();
		assert!(writer.flush_sync(Duration::from_secs(2)));
		assert!(writer.dead());
		assert_eq!(writer.enqueue(b"after panic").unwrap_err().kind(), io::ErrorKind::BrokenPipe);
		writer.stop(Duration::ZERO);
	}

	#[test]
	fn oversized_queue_configuration_is_rejected_before_admitting_frames() {
		let result =
			TtyWriter::with_limit(io::sink(), NonZeroUsize::new(MAX_QUEUED_BYTES + 1).unwrap());
		assert_eq!(result.err().unwrap().kind(), io::ErrorKind::InvalidInput);
	}

	#[test]
	fn each_backend_write_stays_under_the_terminal_syscall_cap() {
		struct Sizes(Arc<Mutex<Vec<usize>>>);
		impl Write for Sizes {
			fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
				self.0.lock().unwrap().push(bytes.len());
				Ok(bytes.len())
			}

			fn flush(&mut self) -> io::Result<()> {
				Ok(())
			}
		}
		let sizes = Arc::new(Mutex::new(Vec::new()));
		let mut writer = TtyWriter::new(Sizes(Arc::clone(&sizes))).unwrap();
		writer.enqueue(&vec![b'x'; WRITE_BYTES * 2 + 7]).unwrap();
		assert!(writer.stop(Duration::from_secs(2)));
		assert_eq!(*sizes.lock().unwrap(), [WRITE_BYTES, WRITE_BYTES, 7]);
	}

	#[test]
	fn zero_length_backend_writes_fail_closed_instead_of_spinning_forever() {
		struct Zero;
		impl Write for Zero {
			fn write(&mut self, _: &[u8]) -> io::Result<usize> {
				Ok(0)
			}

			fn flush(&mut self) -> io::Result<()> {
				Ok(())
			}
		}
		let mut writer = TtyWriter::new(Zero).unwrap();
		writer.enqueue(b"frame").unwrap();
		assert!(writer.flush_sync(Duration::from_secs(2)));
		assert!(writer.dead());
		assert_eq!(writer.enqueue(b"later").unwrap_err().kind(), io::ErrorKind::BrokenPipe);
		writer.stop(Duration::ZERO);
	}
}
