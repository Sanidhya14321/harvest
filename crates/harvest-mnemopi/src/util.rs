//! ids, timestamps, row conversion.

use std::{
	collections::hash_map::RandomState,
	hash::{BuildHasher, Hasher},
	sync::{
		OnceLock,
		atomic::{AtomicU32, Ordering},
	},
};

use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
use rusqlite::types::ValueRef;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub type Row = Map<String, Value>;

pub fn sha256_hex16(value: &str) -> String {
	let digest = Sha256::digest(value.as_bytes());
	let mut out = String::with_capacity(16);
	for byte in digest.iter().take(8) {
		out.push_str(&format!("{byte:02x}"));
	}
	out
}

fn id_seed() -> (u64, u64) {
	static SEED: OnceLock<(u64, u64)> = OnceLock::new();
	*SEED.get_or_init(|| {
		let mut a = RandomState::new().build_hasher();
		a.write_u32(std::process::id());
		let mut b = RandomState::new().build_hasher();
		b.write_u128(Utc::now().timestamp_nanos_opt().unwrap_or_default() as u128);
		(a.finish(), b.finish())
	})
}

static ID_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Mirrors TS `generateId`: sha256(content \0 iso \0 nonce)[..16].
pub fn generate_id(content: &str, now: DateTime<Utc>) -> String {
	let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
	let (a, b) = id_seed();
	sha256_hex16(&format!("{content}\0{}\0{a:x}:{b:x}:{counter:x}", iso(now)))
}

/// Mirrors TS `stableMemoryId`.
pub fn stable_memory_id(content: &str, source: &str) -> String {
	if source.is_empty() {
		sha256_hex16(content)
	} else {
		sha256_hex16(&format!("{content}\0{source}"))
	}
}

/// JS `Date#toISOString` formatting.
pub fn iso(dt: DateTime<Utc>) -> String {
	dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn now_iso() -> String {
	iso(Utc::now())
}

pub fn iso_ago_ms(ms: i64) -> String {
	iso(Utc::now() - chrono::Duration::milliseconds(ms))
}

/// Lenient timestamp parse (RFC3339, naive `YYYY-MM-DD[ T]HH:MM:SS`, or date).
pub fn parse_ts(raw: &str) -> Option<DateTime<Utc>> {
	let raw = raw.trim();
	if raw.is_empty() {
		return None;
	}
	if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
		return Some(dt.with_timezone(&Utc));
	}
	for fmt in
		["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"]
	{
		if let Ok(dt) = NaiveDateTime::parse_from_str(raw, fmt) {
			return Some(dt.and_utc());
		}
	}
	NaiveDate::parse_from_str(raw, "%Y-%m-%d")
		.ok()
		.and_then(|d| d.and_hms_opt(0, 0, 0))
		.map(|d| d.and_utc())
}

pub fn value_ref_to_json(v: ValueRef<'_>) -> Value {
	match v {
		ValueRef::Null => Value::Null,
		ValueRef::Integer(i) => Value::from(i),
		ValueRef::Real(f) => serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number),
		ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
		ValueRef::Blob(b) => Value::Array(b.iter().map(|x| Value::from(*x)).collect()),
	}
}

pub fn row_to_map(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
	let stmt = row.as_ref();
	let mut out = Map::new();
	for i in 0..stmt.column_count() {
		let name = stmt.column_name(i)?.to_string();
		out.insert(name, value_ref_to_json(row.get_ref(i)?));
	}
	Ok(out)
}

pub fn rstr(row: &Row, key: &str) -> Option<String> {
	match row.get(key) {
		Some(Value::String(s)) => Some(s.clone()),
		Some(Value::Null) | None => None,
		Some(other) => Some(other.to_string()),
	}
}

pub fn rnum(row: &Row, key: &str, fallback: f64) -> f64 {
	match row.get(key) {
		Some(Value::Number(n)) => n.as_f64().unwrap_or(fallback),
		Some(Value::String(s)) => s.parse().unwrap_or(fallback),
		_ => fallback,
	}
}

pub fn placeholders(n: usize) -> String {
	vec!["?"; n].join(",")
}

pub fn round4(v: f64) -> f64 {
	(v * 10000.0).round() / 10000.0
}

pub fn clamp01(v: f64) -> f64 {
	v.clamp(0.0, 1.0)
}

/// JSON value -> SQL binding, falling back when the value isn't scalar.
pub fn json_to_sql(
	value: Option<&Value>,
	fallback: rusqlite::types::Value,
) -> rusqlite::types::Value {
	use rusqlite::types::Value as V;
	match value {
		Some(Value::String(s)) => V::Text(s.clone()),
		Some(Value::Number(n)) => n
			.as_i64()
			.map(V::Integer)
			.or_else(|| n.as_f64().map(V::Real))
			.unwrap_or(fallback),
		Some(Value::Bool(b)) => V::Integer(i64::from(*b)),
		Some(Value::Null) => V::Null,
		_ => fallback,
	}
}

/// Char-safe prefix of at most `n` chars.
pub fn prefix_chars(s: &str, n: usize) -> String {
	s.chars().take(n).collect()
}
