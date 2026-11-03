//! Bounded configuration decoding. Parser errors never retain source excerpts,
//! scalar values or third-party error chains that may disclose credentials.
use std::{fmt, path::Path};

use anyhow::{Result, ensure};
use serde::de::{self, DeserializeOwned, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

struct Budget {
	nodes: usize,
	text:  usize,
}
struct Seed<'a> {
	budget: &'a mut Budget,
	depth:  usize,
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
	type Value = Value;

	fn deserialize<D: de::Deserializer<'de>>(
		self,
		deserializer: D,
	) -> std::result::Result<Value, D::Error> {
		if self.depth > 64 || self.budget.nodes == 0 {
			return Err(de::Error::custom("configuration structure limit"));
		}
		self.budget.nodes -= 1;
		deserializer.deserialize_any(self)
	}
}
impl<'de> Visitor<'de> for Seed<'_> {
	type Value = Value;

	fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("bounded configuration data")
	}

	fn visit_unit<E: de::Error>(self) -> std::result::Result<Value, E> {
		Ok(Value::Null)
	}

	fn visit_none<E: de::Error>(self) -> std::result::Result<Value, E> {
		Ok(Value::Null)
	}

	fn visit_bool<E: de::Error>(self, value: bool) -> std::result::Result<Value, E> {
		Ok(Value::Bool(value))
	}

	fn visit_i64<E: de::Error>(self, value: i64) -> std::result::Result<Value, E> {
		Ok(Value::Number(value.into()))
	}

	fn visit_u64<E: de::Error>(self, value: u64) -> std::result::Result<Value, E> {
		Ok(Value::Number(value.into()))
	}

	fn visit_f64<E: de::Error>(self, value: f64) -> std::result::Result<Value, E> {
		Number::from_f64(value)
			.map(Value::Number)
			.ok_or_else(|| E::custom("non-finite configuration number"))
	}

	fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Value, E> {
		self.visit_string(value.to_owned())
	}

	fn visit_string<E: de::Error>(self, value: String) -> std::result::Result<Value, E> {
		self.budget.text = self
			.budget
			.text
			.checked_sub(value.len())
			.ok_or_else(|| E::custom("configuration text limit"))?;
		Ok(Value::String(value))
	}

	fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> std::result::Result<Value, A::Error> {
		let mut values = Vec::new();
		while let Some(value) =
			sequence.next_element_seed(Seed { budget: self.budget, depth: self.depth + 1 })?
		{
			values.push(value);
		}
		Ok(Value::Array(values))
	}

	fn visit_map<A: MapAccess<'de>>(self, mut mapping: A) -> std::result::Result<Value, A::Error> {
		let mut values = Map::new();
		while let Some(key) =
			mapping.next_key_seed(Seed { budget: self.budget, depth: self.depth + 1 })?
		{
			let Value::String(key) = key else {
				return Err(de::Error::custom("configuration keys must be strings"));
			};
			if key == "<<" {
				return Err(de::Error::custom("configuration merge keys are unsupported"));
			}
			if values.contains_key(&key) {
				return Err(de::Error::custom("duplicate configuration key"));
			}
			let value =
				mapping.next_value_seed(Seed { budget: self.budget, depth: self.depth + 1 })?;
			values.insert(key, value);
		}
		Ok(Value::Object(values))
	}
}

/// Blank comments and trailing commas outside JSON strings. Replacing bytes
/// with spaces preserves line/column locations and leaves string escapes
/// intact.
fn jsonc(source: &str) -> Result<Vec<u8>> {
	let mut bytes = source.as_bytes().to_vec();
	let mut index = 0;
	let mut string = false;
	while index < bytes.len() {
		match bytes[index] {
			b'\\' if string => {
				index += 2;
				continue;
			},
			b'"' => string = !string,
			b'/' if !string && bytes.get(index + 1) == Some(&b'/') => {
				while index < bytes.len() && !matches!(bytes[index], b'\r' | b'\n') {
					bytes[index] = b' ';
					index += 1;
				}
				continue;
			},
			b'/' if !string && bytes.get(index + 1) == Some(&b'*') => {
				bytes[index..index + 2].fill(b' ');
				index += 2;
				while index + 1 < bytes.len() && &bytes[index..index + 2] != b"*/" {
					if !matches!(bytes[index], b'\r' | b'\n') {
						bytes[index] = b' ';
					}
					index += 1;
				}
				ensure!(index + 1 < bytes.len(), "unterminated configuration comment");
				bytes[index..index + 2].fill(b' ');
				index += 2;
				continue;
			},
			_ => {},
		}
		index += 1;
	}
	// Second pass after comments are removed, so commas followed by comments
	// and a closing bracket have the same meaning as whitespace-separated ones.
	index = 0;
	string = false;
	while index < bytes.len() {
		match bytes[index] {
			b'\\' if string => {
				index += 2;
				continue;
			},
			b'"' => string = !string,
			b',' if !string => {
				let next = bytes[index + 1..].iter().find(|b| !b.is_ascii_whitespace());
				let previous = bytes[..index].iter().rfind(|b| !b.is_ascii_whitespace());
				if matches!(next, Some(b']' | b'}'))
					&& previous.is_some_and(|b| !matches!(b, b'[' | b'{' | b':' | b','))
				{
					bytes[index] = b' ';
				}
			},
			_ => {},
		}
		index += 1;
	}
	Ok(bytes)
}

pub(super) fn parse<T: DeserializeOwned>(source: &str, path: &Path, label: &str) -> Result<T> {
	ensure!(source.len() <= 256 * 1024, "configuration exceeds 256 KiB");
	let mut budget = Budget { nodes: 16_384, text: 1024 * 1024 };
	let seed = Seed { budget: &mut budget, depth: 0 };
	let value = match path.extension().and_then(|s| s.to_str()) {
		Some("json" | "jsonc") => {
			let bytes = jsonc(source)?;
			let mut deserializer = serde_json::Deserializer::from_slice(&bytes);
			let value = seed.deserialize(&mut deserializer).map_err(|error| {
				anyhow::anyhow!(
					"failed to parse {label} at line {} column {}",
					error.line(),
					error.column()
				)
			})?;
			deserializer.end().map_err(|error| {
				anyhow::anyhow!(
					"failed to parse {label} at line {} column {}",
					error.line(),
					error.column()
				)
			})?;
			value
		},
		Some("yaml" | "yml") => {
			let mut documents = serde_yaml_ng::Deserializer::from_str(source);
			let document = documents
				.next()
				.ok_or_else(|| anyhow::anyhow!("empty {label}"))?;
			let value = seed.deserialize(document).map_err(|error| {
				if let Some(location) = error.location() {
					anyhow::anyhow!(
						"failed to parse {label} at line {} column {}",
						location.line(),
						location.column()
					)
				} else {
					anyhow::anyhow!("failed to parse {label}")
				}
			})?;
			ensure!(documents.next().is_none(), "{label} must contain one YAML document");
			value
		},
		_ => {
			return toml::from_str(source).map_err(|error: toml::de::Error| {
				if let Some(span) = error.span() {
					anyhow::anyhow!("failed to parse {label} at byte {}", span.start)
				} else {
					anyhow::anyhow!("failed to parse {label}")
				}
			});
		},
	};
	ensure!(value.is_object(), "{label} must be a configuration object");
	T::deserialize(value).map_err(|_| anyhow::anyhow!("invalid {label} schema"))
}
