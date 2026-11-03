//! The `web_search` tool: the legacy provider fallback chain, failure summary
//! and LLM formatting, with Exa, Tavily and Brave (API keys from the
//! environment) and keyless `DuckDuckGo` (the no-JS HTML frontend).
use std::{sync::LazyLock, time::Duration};

use anyhow::Result;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};

/// `DEFAULT_WEB_SEARCH_TIMEOUT_SECONDS`.
const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub(crate) struct Args {
	query:              String,
	recency:            Option<String>,
	limit:              Option<f64>,
	num_search_results: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Source {
	pub title:          String,
	pub url:            String,
	pub snippet:        Option<String>,
	pub published_date: Option<String>,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Response {
	pub answer:  Option<String>,
	pub sources: Vec<Source>,
}

/// One HTTP exchange: `(status, body)`.
pub(crate) trait Http {
	fn send(
		&self,
		method: &str,
		url: &str,
		headers: &[(&str, String)],
		body: Option<String>,
	) -> Result<(u16, String)>;
}

struct Ureq;

impl Http for Ureq {
	fn send(
		&self,
		method: &str,
		url: &str,
		headers: &[(&str, String)],
		body: Option<String>,
	) -> Result<(u16, String)> {
		let agent = ureq::AgentBuilder::new().timeout(TIMEOUT).build();
		let mut request = agent.request(method, url);
		for (name, value) in headers {
			request = request.set(name, value);
		}
		let result = match body {
			Some(body) => request.send_string(&body),
			None => request.call(),
		};
		let response = match result {
			Ok(response) => response,
			Err(ureq::Error::Status(_, response)) => response,
			Err(error) => return Err(error.into()),
		};
		let status = response.status();
		let mut text = String::new();
		std::io::Read::read_to_string(
			&mut std::io::Read::take(response.into_reader(), MAX_RESPONSE_BYTES),
			&mut text,
		)?;
		Ok((status, text))
	}
}

/// `SearchProviderError`.
#[derive(Debug)]
struct ProviderError {
	message: String,
	status:  Option<u16>,
}

fn provider_error(message: impl Into<String>, status: Option<u16>) -> ProviderError {
	ProviderError { message: message.into(), status }
}

static CREDIT_BODY: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"(?i)credits?\s*(?:exhausted|exceeded)|quota|insufficient").expect("regex")
});

/// `classifyProviderHttpError`.
fn classify(provider: &str, status: u16, body: &str) -> Option<ProviderError> {
	let message = if CREDIT_BODY.is_match(body) {
		format!("{provider}: credits exhausted")
	} else {
		match status {
			402 => format!("{provider}: 402 credits exhausted"),
			401 => format!("{provider}: 401 unauthorized"),
			403 => format!("{provider}: 403 forbidden"),
			_ => return None,
		}
	};
	Some(provider_error(message, Some(status)))
}

/// `clampNumResults`.
fn clamp(value: Option<f64>, default: usize, max: usize) -> usize {
	match value {
		Some(value) if value.is_finite() && value != 0.0 => (value.max(1.0) as usize).min(max),
		_ => default,
	}
}

struct Provider {
	id:    &'static str,
	label: &'static str,
	env:   Option<&'static str>,
}

/// The implemented subset of `SEARCH_PROVIDER_ORDER`, in order.
const PROVIDERS: [Provider; 4] = [
	Provider { id: "exa", label: "Exa", env: Some("EXA_API_KEY") },
	Provider { id: "tavily", label: "Tavily", env: Some("TAVILY_API_KEY") },
	Provider { id: "brave", label: "Brave", env: Some("BRAVE_API_KEY") },
	Provider { id: "duckduckgo", label: "DuckDuckGo", env: None },
];

fn json_body(provider: &str, label: &str, status: u16, body: &str) -> Result<Value, ProviderError> {
	if !(200..300).contains(&status) {
		if let Some(classified) = classify(provider, status, body) {
			return Err(classified);
		}
		return Err(provider_error(format!("{label} API error ({status}): {body}"), Some(status)));
	}
	serde_json::from_str(body)
		.map_err(|_| provider_error(format!("{label} API returned invalid JSON"), Some(500)))
}

fn text(value: &Value) -> Option<String> {
	value
		.as_str()
		.filter(|text| !text.is_empty())
		.map(str::to_owned)
}

fn search_exa(http: &dyn Http, key: &str, args: &Args) -> Result<Response, ProviderError> {
	let num = args.num_search_results.or(args.limit).map(|n| n as usize);
	let body = json!({
		"query": args.query,
		"numResults": num.unwrap_or(10),
		"type": "auto",
		"contents": {"summary": {"query": args.query}},
	});
	let (status, raw) = http
		.send(
			"POST",
			"https://api.exa.ai/search",
			&[("Content-Type", "application/json".into()), ("x-api-key", key.into())],
			Some(body.to_string()),
		)
		.map_err(|error| provider_error(error.to_string(), None))?;
	let data = json_body("exa", "Exa", status, &raw)?;
	let mut sources: Vec<Source> = data["results"]
		.as_array()
		.into_iter()
		.flatten()
		.filter_map(|result| {
			let url = text(&result["url"])?;
			let snippet = text(&result["summary"])
				.or_else(|| text(&result["text"]))
				.or_else(|| {
					result["highlights"].as_array().map(|parts| {
						parts
							.iter()
							.filter_map(Value::as_str)
							.collect::<Vec<_>>()
							.join(" ")
					})
				});
			Some(Source {
				title: text(&result["title"]).unwrap_or_else(|| url.clone()),
				snippet: snippet
					.filter(|s| !s.is_empty())
					.map(|s| s.chars().take(1000).collect()),
				published_date: text(&result["publishedDate"]),
				url,
			})
		})
		.collect();
	if let Some(num) = num {
		sources.truncate(num);
	}
	Ok(Response { answer: None, sources })
}

fn search_tavily(http: &dyn Http, key: &str, args: &Args) -> Result<Response, ProviderError> {
	let num = clamp(args.num_search_results.or(args.limit), 5, 20);
	let mut body = json!({
		"query": args.query,
		"search_depth": "basic",
		"max_results": num,
		"include_answer": "advanced",
		"include_raw_content": false,
	});
	if let Some(recency) = &args.recency {
		body["time_range"] = json!(recency);
	}
	let (status, raw) = http
		.send(
			"POST",
			"https://api.tavily.com/search",
			&[("Content-Type", "application/json".into()), ("Authorization", format!("Bearer {key}"))],
			Some(body.to_string()),
		)
		.map_err(|error| provider_error(error.to_string(), None))?;
	if !(200..300).contains(&status) {
		if let Some(classified) = classify("tavily", status, &raw) {
			return Err(classified);
		}
		let parsed: Option<Value> = serde_json::from_str(&raw).ok();
		let message = parsed
			.as_ref()
			.and_then(|v| {
				v["detail"]["error"]
					.as_str()
					.or_else(|| v["error"].as_str())
					.or_else(|| v["message"].as_str())
			})
			.map_or_else(|| raw.trim().to_owned(), str::to_owned);
		return Err(provider_error(format!("Tavily API error ({status}): {message}"), Some(status)));
	}
	let data: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
	let sources = data["results"]
		.as_array()
		.into_iter()
		.flatten()
		.filter_map(|result| {
			let url = text(&result["url"])?;
			Some(Source {
				title: text(&result["title"]).unwrap_or_else(|| url.clone()),
				snippet: result["content"].as_str().map(str::to_owned),
				published_date: text(&result["published_date"]),
				url,
			})
		})
		.take(num)
		.collect();
	let answer = data["answer"]
		.as_str()
		.map(str::trim)
		.filter(|a| !a.is_empty())
		.map(str::to_owned);
	Ok(Response { answer, sources })
}

static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").expect("regex"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("regex"));

fn normalize_text(value: &Value, max: usize) -> Option<String> {
	let text = value.as_str()?;
	let text = SPACES
		.replace_all(&TAGS.replace_all(text, " "), " ")
		.trim()
		.to_owned();
	if text.is_empty() {
		return None;
	}
	Some(if text.chars().count() <= max {
		text
	} else {
		format!("{}…", text.chars().take(max - 1).collect::<String>())
	})
}

fn search_brave(http: &dyn Http, key: &str, args: &Args) -> Result<Response, ProviderError> {
	let num = clamp(args.num_search_results.or(args.limit), 10, 20);
	if args.query.chars().count() > 500 {
		return Err(provider_error("Brave search queries cannot exceed 500 characters", Some(400)));
	}
	let mut url = format!(
		"https://api.search.brave.com/res/v1/web/search?q={}&count={num}&extra_snippets=true&text_decorations=false&safesearch=moderate",
		form_urlencoded::byte_serialize(args.query.as_bytes()).collect::<String>()
	);
	if let Some(freshness) = args.recency.as_deref().and_then(|r| match r {
		"day" => Some("pd"),
		"week" => Some("pw"),
		"month" => Some("pm"),
		"year" => Some("py"),
		_ => None,
	}) {
		url.push_str("&freshness=");
		url.push_str(freshness);
	}
	let (status, raw) = http
		.send(
			"GET",
			&url,
			&[("Accept", "application/json".into()), ("X-Subscription-Token", key.into())],
			None,
		)
		.map_err(|error| provider_error(error.to_string(), None))?;
	let data = json_body("brave", "Brave", status, &raw)?;
	let sources = data["web"]["results"]
		.as_array()
		.into_iter()
		.flatten()
		.filter_map(|result| {
			let url = result["url"].as_str().filter(|u| {
				u.len() <= 2048 && (u.starts_with("http://") || u.starts_with("https://"))
			})?;
			let mut snippets: Vec<String> = Vec::new();
			for value in std::iter::once(&result["description"])
				.chain(result["extra_snippets"].as_array().into_iter().flatten())
			{
				if let Some(snippet) = normalize_text(value, 8000)
					&& !snippets.contains(&snippet)
				{
					snippets.push(snippet);
				}
			}
			let combined = snippets.join("\n");
			Some(Source {
				title:          normalize_text(&result["title"], 300).unwrap_or_else(|| url.to_owned()),
				url:            url.to_owned(),
				snippet:        (!combined.is_empty()).then_some(combined),
				published_date: normalize_text(&result["age"], 100),
			})
		})
		.take(num)
		.collect();
	Ok(Response { answer: None, sources })
}

static DDG_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r#"<div\b[^>]*\bclass="[^"]*\bresult\b[^"]*"[^>]*>"#).expect("regex")
});
static DDG_TITLE: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(
		r#"<a\b[^>]*\bclass="[^"]*\bresult__a\b[^"]*"[^>]*\bhref="([^"]+)"[^>]*>([\s\S]*?)</a>"#,
	)
	.expect("regex")
});
static DDG_SNIPPET: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r#"<(?:a|div|span)\b[^>]*\bclass="[^"]*\bresult__snippet\b[^"]*"[^>]*>([\s\S]*?)</(?:a|div|span)>"#)
		.expect("regex")
});
static DDG_UDDG: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"[?&]uddg=([^&]+)").expect("regex"));

/// `decodeHtmlText`.
fn decode_html(value: &str) -> String {
	static NUMERIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#(\d+);").expect("regex"));
	static HEX: LazyLock<Regex> =
		LazyLock::new(|| Regex::new(r"(?i)&#x([0-9a-f]+);").expect("regex"));
	let text = TAGS.replace_all(value, " ");
	let text = NUMERIC.replace_all(&text, |c: &regex::Captures<'_>| {
		c[1]
			.parse::<u32>()
			.ok()
			.and_then(char::from_u32)
			.map(String::from)
			.unwrap_or_default()
	});
	let text = HEX.replace_all(&text, |c: &regex::Captures<'_>| {
		u32::from_str_radix(&c[1], 16)
			.ok()
			.and_then(char::from_u32)
			.map(String::from)
			.unwrap_or_default()
	});
	let mut text = text.into_owned();
	for (entity, replacement) in [
		("&nbsp;", " "),
		("&amp;", "&"),
		("&lt;", "<"),
		("&gt;", ">"),
		("&quot;", "\""),
		("&#39;", "'"),
		("&apos;", "'"),
	] {
		text = text.replace(entity, replacement);
	}
	SPACES.replace_all(&text, " ").trim().to_owned()
}

/// `unwrapResultUrl`.
fn unwrap_ddg_url(href: &str) -> Option<String> {
	let decoded = href.replace("&amp;", "&");
	if let Some(wrapped) = DDG_UDDG.captures(&decoded) {
		return percent_decode(&wrapped[1]);
	}
	if let Some(rest) = decoded.strip_prefix("//") {
		return Some(format!("https://{rest}"));
	}
	(decoded.starts_with("http://") || decoded.starts_with("https://")).then_some(decoded)
}

fn percent_decode(text: &str) -> Option<String> {
	let bytes = text.as_bytes();
	let mut out = Vec::with_capacity(bytes.len());
	let mut index = 0;
	while index < bytes.len() {
		if bytes[index] == b'%' {
			let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
			out.push(u8::from_str_radix(hex, 16).ok()?);
			index += 3;
		} else {
			out.push(bytes[index]);
			index += 1;
		}
	}
	String::from_utf8(out).ok()
}

/// `parseHtmlResults`.
fn parse_ddg(html: &str) -> Vec<Source> {
	let starts: Vec<usize> = DDG_BLOCK.find_iter(html).map(|m| m.end()).collect();
	let mut out = Vec::new();
	for (index, start) in starts.iter().enumerate() {
		let end = starts
			.get(index + 1)
			.map_or(html.len(), |next| html[..*next].rfind("<div").unwrap_or(*next));
		let block = &html[*start..end.max(*start)];
		let Some(title) = DDG_TITLE.captures(block) else {
			continue;
		};
		let Some(url) = unwrap_ddg_url(&title[1]) else {
			continue;
		};
		let title_text = decode_html(&title[2]);
		if title_text.is_empty() {
			continue;
		}
		let snippet = DDG_SNIPPET
			.captures(block)
			.map(|c| decode_html(&c[1]))
			.filter(|s| !s.is_empty());
		out.push(Source { title: title_text, url, snippet, published_date: None });
	}
	out
}

fn search_duckduckgo(http: &dyn Http, args: &Args) -> Result<Response, ProviderError> {
	let num = clamp(args.num_search_results.or(args.limit), 10, 20);
	let mut form = form_urlencoded::Serializer::new(String::new());
	form
		.append_pair("q", &args.query)
		.append_pair("kl", "us-en");
	if let Some(df) = args
		.recency
		.as_deref()
		.and_then(|r| r.chars().next())
		.filter(|c| "dwmy".contains(*c))
	{
		form.append_pair("df", &df.to_string());
	}
	form.append_pair("b", "");
	let (status, html) = http
		.send(
			"POST",
			"https://html.duckduckgo.com/html/",
			&[
				("Content-Type", "application/x-www-form-urlencoded".into()),
				("Referer", "https://html.duckduckgo.com/".into()),
				(
					"User-Agent",
					"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like \
					 Gecko) Chrome/126.0 Safari/537.36"
						.into(),
				),
			],
			Some(form.finish()),
		)
		.map_err(|error| provider_error(error.to_string(), None))?;
	if !(200..300).contains(&status) {
		return Err(classify("duckduckgo", status, &html).unwrap_or_else(|| {
			provider_error(format!("DuckDuckGo HTML error ({status})"), Some(status))
		}));
	}
	if html.contains("anomaly-modal") || html.contains("anomaly.js") {
		return Err(provider_error(
			"DuckDuckGo blocked the request with a bot-detection challenge. DuckDuckGo throttles \
			 automated HTML searches from datacenter/shared-egress IPs; configure a credentialed \
			 provider such as Brave, Tavily, Exa, or Kagi for reliable web search.",
			Some(429),
		));
	}
	let mut seen = Vec::new();
	let sources = parse_ddg(&html)
		.into_iter()
		.filter(|s| {
			let fresh = !seen.contains(&s.url);
			seen.push(s.url.clone());
			fresh
		})
		.take(num)
		.collect();
	Ok(Response { answer: None, sources })
}

fn truncate_text(text: &str, max: usize) -> String {
	if text.encode_utf16().count() <= max {
		return text.to_owned();
	}
	format!("{}…", text.chars().take(max.saturating_sub(1)).collect::<String>())
}

fn age_label(published: Option<&str>) -> Option<String> {
	let published = published?;
	let seconds = chrono::DateTime::parse_from_rfc3339(published)
		.ok()
		.map(|date| (chrono::Utc::now() - date.with_timezone(&chrono::Utc)).num_seconds())
		.or_else(|| {
			chrono::NaiveDate::parse_from_str(published, "%Y-%m-%d")
				.ok()
				.map(|date| {
					(chrono::Utc::now().naive_utc() - date.and_hms_opt(0, 0, 0).expect("midnight"))
						.num_seconds()
				})
		});
	match seconds {
		Some(seconds) if seconds > 0 => {
			let label = crate::tree::format_age_label(seconds as u64);
			if label.is_empty() {
				Some(published.to_owned())
			} else {
				Some(label)
			}
		},
		_ => Some(published.to_owned()),
	}
}

/// `formatForLLM`.
fn format_for_llm(response: &Response) -> String {
	let count = |label: &str, n: usize| format!("{n} {label}{}", if n == 1 { "" } else { "s" });
	let mut parts = Vec::new();
	if let Some(answer) = &response.answer {
		parts.push(answer.clone());
		if !response.sources.is_empty() {
			parts.push("\n## Sources".to_owned());
			parts.push(count("source", response.sources.len()));
		}
	}
	for (index, source) in response.sources.iter().enumerate() {
		let age = age_label(source.published_date.as_deref())
			.map_or_else(String::new, |age| format!(" ({age})"));
		parts.push(format!("[{}] {}{age}\n    {}", index + 1, source.title, source.url));
		if let Some(snippet) = source.snippet.as_deref().filter(|s| !s.is_empty()) {
			parts.push(format!("    {}", truncate_text(snippet, 240)));
		}
	}
	parts.join("\n")
}

/// `executeSearch` over `http`, with keys from `env`.
pub(crate) fn search_with(
	http: &dyn Http,
	args: &Args,
	env: &dyn Fn(&str) -> Option<String>,
) -> String {
	let mut failures: Vec<(&Provider, ProviderError)> = Vec::new();
	let mut available = 0;
	for provider in &PROVIDERS {
		let key = match provider.env {
			Some(name) => match env(name).filter(|key| !key.is_empty()) {
				Some(key) => key,
				None => continue,
			},
			None => String::new(),
		};
		available += 1;
		let result = match provider.id {
			"exa" => search_exa(http, &key, args),
			"tavily" => search_tavily(http, &key, args),
			"brave" => search_brave(http, &key, args),
			_ => search_duckduckgo(http, args),
		};
		match result {
			Ok(response) if response.answer.is_some() || !response.sources.is_empty() => {
				return format_for_llm(&response);
			},
			Ok(_) => failures.push((
				provider,
				provider_error(
					format!("{} returned no renderable search content.", provider.label),
					Some(204),
				),
			)),
			Err(error) => failures.push((provider, error)),
		}
	}
	if available == 0 {
		return "Error: No web search provider configured.".to_owned();
	}
	let describe = |provider: &Provider, error: &ProviderError| match error.status {
		Some(401 | 403) => format!(
			"{} authorization failed ({}). Check API key or base URL.",
			provider.label,
			error.status.unwrap_or_default()
		),
		_ => error.message.clone(),
	};
	let message = if failures.len() > 1 {
		format!(
			"All web search providers failed: {}",
			failures
				.iter()
				.map(|(p, e)| format!("{}: {}", p.id, describe(p, e)))
				.collect::<Vec<_>>()
				.join("; ")
		)
	} else {
		failures
			.last()
			.map_or_else(String::new, |(p, e)| describe(p, e))
	};
	format!("Error: {message}")
}

pub(crate) fn web_search(args: &Args) -> String {
	search_with(&Ureq, args, &|name| std::env::var(name).ok())
}

#[cfg(test)]
mod tests {
	use std::cell::RefCell;

	use super::*;

	struct Fake(RefCell<Vec<(u16, String)>>, RefCell<Vec<String>>);

	impl Http for Fake {
		fn send(
			&self,
			method: &str,
			url: &str,
			_: &[(&str, String)],
			_: Option<String>,
		) -> Result<(u16, String)> {
			self.1.borrow_mut().push(format!("{method} {url}"));
			Ok(self.0.borrow_mut().remove(0))
		}
	}

	fn args(query: &str) -> Args {
		Args {
			query:              query.into(),
			recency:            None,
			limit:              None,
			num_search_results: None,
		}
	}

	#[test]
	fn falls_back_through_keyed_providers_and_formats_sources() {
		let fake = Fake(
			RefCell::new(vec![
				(401, "nope".into()),
				(200, r#"{"answer":"Rust 1.90","results":[{"title":"Blog","url":"https://r.rs","content":"release notes"}]}"#.into()),
			]),
			RefCell::new(Vec::new()),
		);
		let env =
			|name: &str| matches!(name, "EXA_API_KEY" | "TAVILY_API_KEY").then(|| "k".to_owned());
		let out = search_with(&fake, &args("rust"), &env);
		assert_eq!(
			out,
			"Rust 1.90\n\n## Sources\n1 source\n[1] Blog\n    https://r.rs\n    release notes"
		);
		assert_eq!(fake.1.borrow()[0], "POST https://api.exa.ai/search");
	}

	#[test]
	fn duckduckgo_parses_results_and_failures_are_summarized() {
		let html = r#"<div class="result results_links"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fex.com%2Fa&amp;rut=1">Ex <b>A</b></a><a class="result__snippet">snip &amp; more</a></div><div class="nav-link"></div>"#;
		let fake = Fake(RefCell::new(vec![(200, html.into())]), RefCell::new(Vec::new()));
		assert_eq!(
			search_with(&fake, &args("a"), &|_| None),
			"[1] Ex A\n    https://ex.com/a\n    snip & more"
		);
		let fake = Fake(
			RefCell::new(vec![(500, "boom".into()), (200, "<html>anomaly-modal</html>".into())]),
			RefCell::new(Vec::new()),
		);
		let out =
			search_with(&fake, &args("a"), &|name| (name == "BRAVE_API_KEY").then(|| "k".into()));
		assert!(
			out.starts_with(
				"Error: All web search providers failed: brave: Brave API error (500): boom; \
				 duckduckgo: DuckDuckGo blocked"
			),
			"{out}"
		);
	}
}
