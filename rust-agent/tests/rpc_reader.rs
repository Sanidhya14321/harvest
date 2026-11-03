use std::{sync::Arc, time::Duration};

use anyhow::Result;
use harvest_omp::{
	protocol::rpc_frame::{self, MAX_FRAME_BYTES, Version},
	rpc::reader::Reader,
};
use serde_json::json;
use tokio::{
	io::AsyncWriteExt,
	sync::{Notify, Semaphore},
};

#[tokio::test]
async fn cancelled_reads_retain_split_unicode_and_eof_accepts_one_final_json_object() -> Result<()>
{
	let (mut input, output) = tokio::io::duplex(64);
	let mut reader = Reader::new(output);
	let bytes = serde_json::to_vec(&json!({"type":"response","text":"🦀"}))?;
	let split = bytes.iter().position(|byte| *byte == 0xf0).unwrap() + 2;
	input.write_all(&bytes[..split]).await?;
	assert!(
		tokio::time::timeout(Duration::from_millis(20), reader.next())
			.await
			.is_err()
	);
	assert!(reader.set_version(Version::V2).is_err());
	input.write_all(&bytes[split..]).await?;
	input.shutdown().await?;
	assert_eq!(reader.next().await?, Some(json!({"type":"response","text":"🦀"})));
	assert!(reader.next().await?.is_none());
	assert!(reader.next().await?.is_none());
	Ok(())
}

#[tokio::test]
async fn cancelled_chunk_read_resumes_without_loss_and_rejects_truncated_process_exit() -> Result<()>
{
	for truncate in [false, true] {
		let (mut input, output) = tokio::io::duplex(1024);
		let mut reader = Reader::new(output);
		reader.set_version(Version::V2)?;
		let value = json!({"type":"response","payload":"🦀".repeat(300_000)});
		let mut frames = rpc_frame::encode(&value, Version::V2, "sdk-read".into())?;
		let entered = Arc::new(Notify::new());
		let release = Arc::new(Semaphore::new(0));
		let signal = entered.clone();
		let gate = release.clone();
		let producer = tokio::spawn(async move {
			input.write_all(&frames.next().unwrap()?).await?;
			signal.notify_one();
			gate.acquire().await?.forget();
			if !truncate {
				for line in frames {
					input.write_all(&line?).await?;
				}
			}
			input.shutdown().await?;
			Ok::<(), anyhow::Error>(())
		});
		tokio::select! {
			result=reader.next()=>panic!("first chunk cannot finish logical frame: {result:?}"),
			()=entered.notified()=>{},
		}
		release.add_permits(1);
		if truncate {
			assert!(reader.next().await.is_err());
			assert!(reader.next().await.is_err());
		} else {
			assert_eq!(reader.next().await?, Some(value));
			assert!(reader.next().await?.is_none());
		}
		producer.await??;
	}
	Ok(())
}

#[tokio::test]
async fn reader_rejects_oversized_physical_lines_without_retaining_unbounded_input() -> Result<()> {
	let (mut input, output) = tokio::io::duplex(1024);
	let producer = tokio::spawn(async move {
		for _ in 0..=MAX_FRAME_BYTES / 1024 {
			input.write_all(&[b'x'; 1024]).await?;
		}
		input.write_all(b"\n").await?;
		Ok::<(), anyhow::Error>(())
	});
	let mut reader = Reader::new(output);
	assert!(
		reader
			.next()
			.await
			.unwrap_err()
			.to_string()
			.contains("exceeds frame limit")
	);
	assert!(reader.next().await.is_err());
	producer.await??;
	Ok(())
}

#[tokio::test]
async fn chunks_before_negotiation_and_nonobjects_fail_the_native_reader() {
	for value in [json!([]), json!({"type":"rpc_chunk"})] {
		let bytes = serde_json::to_vec(&value).unwrap();
		let mut reader = Reader::new(std::io::Cursor::new(bytes));
		assert!(reader.next().await.is_err());
		assert!(reader.next().await.is_err());
	}
}
