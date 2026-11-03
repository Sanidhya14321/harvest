use std::{cell::RefCell, collections::VecDeque, io, num::NonZeroU64, rc::Rc};

use harvest_terminal::frame::{
	FrameError, FrameOutcome, FramePlan, FrameProvider, FrameSink, FrameWriter, HistoryBatch,
	HistoryKind, OutputAcceptance, ResizeMode, TextLayout, ViewportSize,
};

type Events = Rc<RefCell<Vec<String>>>;

const fn size(columns: usize, rows: usize) -> ViewportSize {
	ViewportSize { columns, rows }
}

fn batch(id: u64, rows: &[&str], kind: HistoryKind) -> HistoryBatch {
	HistoryBatch::new(NonZeroU64::new(id).expect("nonzero fixture ID"), strings(rows), kind)
}

fn strings(rows: &[&str]) -> Vec<String> {
	rows.iter().map(|row| (*row).to_owned()).collect()
}

#[derive(Default)]
struct Provider {
	plan:     FramePlan,
	acks:     Vec<u64>,
	replays:  usize,
	ledger:   Vec<String>,
	latest:   u64,
	deferred: bool,
	events:   Events,
}

impl FrameProvider for Provider {
	fn render_frame(&mut self, _size: ViewportSize) -> FramePlan {
		self.plan.clone()
	}

	fn acknowledge_history(&mut self, id: NonZeroU64) {
		self.events.borrow_mut().push(format!("ack:{id}"));
		self.acks.push(id.get());
		if self
			.plan
			.history
			.as_ref()
			.is_some_and(|history| history.id() == id)
		{
			self.latest = self.latest.max(id.get());
			self.plan.history = None;
		}
		if self.deferred {
			self.deferred = false;
			self.begin_history_replay();
		}
	}

	fn begin_history_replay(&mut self) {
		if self.plan.history.is_some() {
			self.deferred = true;
			return;
		}
		self.replays += 1;
		self.latest += 1;
		self.plan.history = Some(HistoryBatch::new(
			NonZeroU64::new(self.latest).expect("next history ID"),
			self.ledger.clone(),
			HistoryKind::Replay,
		));
	}
}

#[derive(Default)]
struct Sink {
	answers:  VecDeque<Result<OutputAcceptance, io::ErrorKind>>,
	attempts: Vec<String>,
	accepted: Vec<String>,
	events:   Events,
}

impl FrameSink for Sink {
	fn write_frame(&mut self, packet: &[u8]) -> io::Result<OutputAcceptance> {
		let packet = String::from_utf8(packet.to_vec()).expect("UTF-8 frame");
		self.attempts.push(packet.clone());
		let answer = self
			.answers
			.pop_front()
			.unwrap_or(Ok(OutputAcceptance::Accepted));
		match answer {
			Ok(OutputAcceptance::Accepted) => {
				self.accepted.push(packet);
				self.events.borrow_mut().push("accepted".to_owned());
				Ok(OutputAcceptance::Accepted)
			},
			Ok(OutputAcceptance::Backpressure) => Ok(OutputAcceptance::Backpressure),
			Err(kind) => Err(io::Error::from(kind)),
		}
	}
}

/// Only ASCII fixtures exercise geometry here. Unicode/ANSI measurement is a
/// host adapter contract, not a second implementation inside the writer.
struct AsciiLayout;

impl TextLayout for AsciiLayout {
	fn fit_row(&mut self, row: &str, columns: usize) -> String {
		assert!(row.is_ascii());
		row.chars().take(columns).collect()
	}

	fn wrap_text(&mut self, text: &str, columns: usize) -> Vec<String> {
		assert!(text.is_ascii());
		text
			.lines()
			.flat_map(|line| {
				line
					.as_bytes()
					.chunks(columns)
					.map(|row| String::from_utf8(row.to_vec()).expect("ASCII fixture row"))
			})
			.collect()
	}
}

#[test]
fn rejected_or_backpressured_writes_keep_the_offer_and_retry_identical_bytes_before_ack() {
	let events = Events::default();
	let mut provider = Provider {
		plan: FramePlan {
			history:  Some(batch(1, &["history"], HistoryKind::Append)),
			viewport: strings(&["editor"]),
		},
		events: events.clone(),
		..Provider::default()
	};
	let mut sink = Sink {
		answers: VecDeque::from([Err(io::ErrorKind::BrokenPipe), Ok(OutputAcceptance::Backpressure)]),
		events: events.clone(),
		..Sink::default()
	};
	let mut writer = FrameWriter::new(true);
	assert!(matches!(
		writer.render(size(20, 3), &mut provider, &mut sink, &mut AsciiLayout),
		Err(FrameError::Output(_))
	));
	assert_eq!(
		writer
			.render(size(20, 3), &mut provider, &mut sink, &mut AsciiLayout)
			.expect("backpressure"),
		FrameOutcome::Deferred
	);
	assert!(events.borrow().is_empty());
	assert_eq!(writer.accepted_history_id(), 0);
	assert!(provider.plan.history.is_some());
	let result = writer
		.render(size(20, 3), &mut provider, &mut sink, &mut AsciiLayout)
		.expect("retry accepted");
	assert_eq!(result, FrameOutcome::Accepted {
		wrote:        true,
		history_ack:  NonZeroU64::new(1),
		request_next: true,
	});
	assert_eq!(*events.borrow(), ["accepted", "ack:1"]);
	assert!(
		sink
			.attempts
			.iter()
			.all(|packet| packet == &sink.accepted[0])
	);
	assert_eq!(writer.accepted_history_id(), 1);
	// Accepted means queued, so no physical-drain callback is needed for ACK.
	assert_eq!(provider.acks, [1]);
}

#[test]
fn one_replay_packet_bottom_splits_history_then_diffs_only_the_mutable_tail() {
	let mut provider = Provider {
		plan: FramePlan {
			history:  Some(batch(1, &["hist-1", "hist-2", "hist-3", "hist-4"], HistoryKind::Replay)),
			viewport: strings(&["live", "editor"]),
		},
		..Provider::default()
	};
	let mut writer = FrameWriter::new(true);
	let mut sink = Sink::default();
	let outcome = writer
		.render(size(20, 4), &mut provider, &mut sink, &mut AsciiLayout)
		.expect("atomic replay");
	assert!(matches!(outcome, FrameOutcome::Accepted { request_next: false, .. }));
	assert_eq!(sink.accepted.len(), 1);
	let packet = &sink.accepted[0];
	let mut last = 0;
	for row in ["hist-1", "hist-2", "hist-3", "hist-4", "live", "editor"] {
		let position = packet
			.find(row)
			.expect("every replay row reached one output packet");
		assert!(position > last);
		last = position;
	}
	assert!(packet.ends_with("\x1b[3;1H\x1b[?7h\x1b[?2026l"));
	provider.plan.viewport = strings(&["live-updated", "editor"]);
	writer
		.render(size(20, 4), &mut provider, &mut sink, &mut AsciiLayout)
		.expect("incremental viewport");
	assert_eq!(sink.accepted.len(), 2);
	let update = &sink.accepted[1];
	assert!(update.contains("\x1b[3;1H\x1b[2Klive-updated"));
	assert!(!update.contains("hist-"));
	assert!(!update.contains("editor"));
	assert_eq!(provider.acks, [1]);
}

#[test]
fn stale_accepted_offers_are_acknowledged_without_reappending_history() {
	let mut provider = Provider {
		plan: FramePlan {
			history:  Some(batch(7, &["committed"], HistoryKind::Append)),
			viewport: strings(&["editor"]),
		},
		..Provider::default()
	};
	let mut sink = Sink::default();
	let mut writer = FrameWriter::default();
	writer
		.render(size(20, 4), &mut provider, &mut sink, &mut AsciiLayout)
		.expect("first acceptance");
	provider.plan.history = Some(batch(7, &["must-not-append"], HistoryKind::Append));
	let duplicate = writer
		.render(size(20, 4), &mut provider, &mut sink, &mut AsciiLayout)
		.expect("duplicate offer");
	assert_eq!(duplicate, FrameOutcome::Accepted {
		wrote:        false,
		history_ack:  NonZeroU64::new(7),
		request_next: false,
	});
	assert_eq!(sink.accepted.len(), 1);
	assert_eq!(provider.acks, [7, 7]);
	provider.plan.history = Some(batch(8, &["next-row"], HistoryKind::Append));
	writer
		.render(size(20, 4), &mut provider, &mut sink, &mut AsciiLayout)
		.expect("next acceptance");
	assert_eq!(sink.accepted[1].matches("next-row").count(), 1);
	assert!(!sink.accepted[1].contains("committed"));
	assert!(!sink.accepted[1].contains("must-not-append"));
}

#[test]
fn viewport_scroll_protection_never_issues_a_full_screen_clear_that_tmux_archives() {
	let mut provider = Provider {
		plan: FramePlan {
			history:  None,
			viewport: strings(&["old-live-1", "old-live-2", "old-live-3", "editor"]),
		},
		..Provider::default()
	};
	let mut sink = Sink::default();
	let mut writer = FrameWriter::default();
	writer
		.render(size(20, 4), &mut provider, &mut sink, &mut AsciiLayout)
		.expect("live viewport");
	provider.plan.history = Some(batch(1, &["hist-1", "hist-2"], HistoryKind::Append));
	provider.plan.viewport = strings(&["new-live-1", "new-live-2", "new-live-3", "editor"]);
	writer
		.render(size(20, 4), &mut provider, &mut sink, &mut AsciiLayout)
		.expect("scrolling append");
	let packet = &sink.accepted[1];
	// EL2 row one, then ED0 from row two: tmux cannot archive unfinished rows.
	assert!(packet.contains("\x1b[1;1H\x1b[2K\x1b[2;1H\x1b[J\x1b[1;1H"));
	assert!(!packet.contains("\x1b[2J"));
	assert!(!packet.contains("\x1b[3J"));
	assert!(!packet.contains("old-live"));
	assert_eq!(provider.acks, [1]);
}

#[test]
fn resize_modes_replay_only_when_requested_and_clear_in_tmux_safe_order() {
	for mode in [ResizeMode::Preserve, ResizeMode::Append, ResizeMode::Rebuild] {
		let mut provider = Provider {
			plan: FramePlan {
				history:  Some(batch(1, &["history"], HistoryKind::Append)),
				viewport: strings(&["editor"]),
			},
			ledger: strings(&["history"]),
			..Provider::default()
		};
		let mut writer = FrameWriter::default();
		let mut sink = Sink::default();
		writer
			.render(size(20, 4), &mut provider, &mut sink, &mut AsciiLayout)
			.expect("initial frame");
		writer.settle_resize(size(30, 5), mode, &mut provider);
		writer.settle_resize(size(30, 5), mode, &mut provider);
		writer
			.render(size(30, 5), &mut provider, &mut sink, &mut AsciiLayout)
			.expect("settled resize");
		let packet = &sink.accepted[1];
		assert_eq!(provider.replays, usize::from(mode != ResizeMode::Preserve));
		assert_eq!(packet.contains("history"), mode != ResizeMode::Preserve);
		assert_eq!(packet.contains("\x1b[2J\x1b[3J"), mode == ResizeMode::Rebuild);
		assert_eq!(
			provider.acks,
			if mode == ResizeMode::Preserve {
				vec![1]
			} else {
				vec![1, 2]
			}
		);
	}
}

#[test]
fn append_height_only_resize_does_not_duplicate_history_and_failed_rebuild_keeps_clear_pending() {
	let mut provider = Provider {
		plan: FramePlan {
			history:  Some(batch(1, &["history"], HistoryKind::Append)),
			viewport: strings(&["editor"]),
		},
		ledger: strings(&["history"]),
		..Provider::default()
	};
	let mut sink = Sink::default();
	let mut writer = FrameWriter::default();
	writer
		.render(size(20, 3), &mut provider, &mut sink, &mut AsciiLayout)
		.expect("initial frame");
	writer.settle_resize(size(20, 5), ResizeMode::Append, &mut provider);
	writer
		.render(size(20, 5), &mut provider, &mut sink, &mut AsciiLayout)
		.expect("height-only grow");
	assert_eq!(provider.replays, 0);
	assert!(!sink.accepted[1].contains("history"));
	writer.settle_resize(size(30, 3), ResizeMode::Rebuild, &mut provider);
	sink.answers.push_back(Err(io::ErrorKind::WouldBlock));
	assert!(
		writer
			.render(size(30, 3), &mut provider, &mut sink, &mut AsciiLayout)
			.is_err()
	);
	assert_eq!(provider.acks, [1]);
	assert_eq!(writer.accepted_history_id(), 1);
	writer
		.render(size(30, 3), &mut provider, &mut sink, &mut AsciiLayout)
		.expect("resize retry");
	assert_eq!(sink.attempts[2], sink.attempts[3]);
	assert!(sink.accepted[2].contains("\x1b[2J\x1b[3J"));
	assert_eq!(provider.acks, [1, 2]);
}

#[test]
fn invalid_geometry_oversized_output_and_bad_layout_never_ack_or_enqueue() {
	let mut provider = Provider {
		plan: FramePlan {
			history:  Some(batch(1, &["history"], HistoryKind::Append)),
			viewport: strings(&["editor"]),
		},
		..Provider::default()
	};
	let mut writer = FrameWriter::default();
	let mut sink = Sink::default();
	assert!(matches!(
		writer.render(size(0, 3), &mut provider, &mut sink, &mut AsciiLayout),
		Err(FrameError::InvalidSize)
	));
	provider.plan.viewport = vec!["x".repeat(1024 * 1024 + 1)];
	assert!(matches!(
		writer.render(size(20, 3), &mut provider, &mut sink, &mut AsciiLayout),
		Err(FrameError::FrameTooLarge)
	));
	provider.plan.viewport = strings(&["two\nrows"]);
	assert!(matches!(
		writer.render(size(20, 3), &mut provider, &mut sink, &mut AsciiLayout),
		Err(FrameError::InvalidLayoutRow)
	));
	assert!(sink.attempts.is_empty());
	assert!(provider.acks.is_empty());
	assert_eq!(writer.accepted_history_id(), 0);
}

#[test]
fn reset_waits_for_an_outstanding_append_before_clearing_and_replaying_the_complete_ledger() {
	let mut provider = Provider {
		plan: FramePlan {
			history:  Some(batch(1, &["previous-history"], HistoryKind::Append)),
			viewport: strings(&["editor"]),
		},
		ledger: strings(&["previous-history", "pending-history"]),
		..Provider::default()
	};
	let mut writer = FrameWriter::default();
	let mut sink = Sink::default();
	writer
		.render(size(30, 4), &mut provider, &mut sink, &mut AsciiLayout)
		.expect("initial frame");
	provider.plan.history = Some(batch(2, &["pending-history"], HistoryKind::Append));
	writer.reset_display(&mut provider);
	writer
		.render(size(30, 4), &mut provider, &mut sink, &mut AsciiLayout)
		.expect("accept pending append");
	assert!(!sink.accepted[1].contains("\x1b[3J"));
	assert_eq!(provider.acks, [1, 2]);
	assert_eq!(provider.replays, 1);
	writer
		.render(size(30, 4), &mut provider, &mut sink, &mut AsciiLayout)
		.expect("complete reset replay");
	assert!(sink.accepted[2].contains("\x1b[2J\x1b[3J"));
	assert!(sink.accepted[2].contains("previous-history"));
	assert!(sink.accepted[2].contains("pending-history"));
	assert_eq!(provider.acks, [1, 2, 3]);
}
