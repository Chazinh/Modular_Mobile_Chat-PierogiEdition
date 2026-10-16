//! Read-only, bounded evidence from the router's own request log.
use crate::rules;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};
use std::{borrow::Cow, collections::HashMap, path::Path, time::Duration};
pub fn provider_effort(family: &str, request: &Value) -> Value {
    match family {
        "claude" => request["output_config"]["effort"].clone(),
        "gemini" => request
            .pointer("/request/generationConfig/thinkingConfig/thinkingLevel")
            .or(request.pointer("/generationConfig/thinkingConfig/thinkingLevel"))
            .cloned()
            .unwrap_or(Value::Null),
        "deepseek" => request
            .get("reasoning_effort")
            .or(request.pointer("/reasoning/effort"))
            .cloned()
            .unwrap_or(Value::Null),
        _ => request
            .get("reasoning_effort")
            .or(request.pointer("/output_config/effort"))
            .cloned()
            .unwrap_or(Value::Null),
    }
}
type Fields<'a> = HashMap<String, &'a RawValue>;
/// Top-level fields of a JSON object, each left as unparsed text (nothing below the first level is built).
fn fields(raw: &str) -> Option<Fields<'_>> {
    serde_json::from_str(raw).ok()
}
fn parse(raw: Option<&&RawValue>) -> Value {
    raw.and_then(|r| serde_json::from_str(r.get()).ok())
        .unwrap_or(Value::Null)
}
#[derive(Deserialize)]
struct Message<'a> {
    #[serde(borrow, default)]
    role: Option<&'a RawValue>,
    #[serde(borrow, default)]
    content: Option<&'a RawValue>,
}
fn is_user(role: &RawValue) -> bool {
    let text = role.get();
    text == "\"user\""
        || (text.contains('\\')
            && serde_json::from_str::<Cow<str>>(text).is_ok_and(|r| r == "user"))
}
/// Text of the last `user` message of `request.messages` (empty when its content is not a string), parsed only for
/// that one message; the others stay as unparsed slices.
fn last_user_text(request: &RawValue) -> Option<String> {
    let messages = fields(request.get())?;
    let messages: Vec<Message> = serde_json::from_str(messages.get("messages")?.get()).ok()?;
    let message = messages
        .iter()
        .rev()
        .find(|m| m.role.is_some_and(is_user))?;
    Some(
        message
            .content
            .and_then(|c| serde_json::from_str::<Cow<str>>(c.get()).ok())
            .map(Cow::into_owned)
            .unwrap_or_default(),
    )
}
/// The parts of `providerRequest` this reader looks at, in their original places, so `provider_effort` and the
/// lookups below see the same paths as in the full document. The rest (the messages the provider received, Gemini's
/// `contents`) is never built.
fn provider_request(raw: Option<&&RawValue>) -> Value {
    let Some(top) = raw.and_then(|r| fields(r.get())) else {
        return Value::Null;
    };
    let mut out = serde_json::Map::new();
    for key in [
        "reasoning_effort",
        "max_tokens",
        "thinking",
        "output_config",
        "reasoning",
        "generationConfig",
    ] {
        if let Some(raw) = top.get(key) {
            out.insert(key.into(), parse(Some(raw)));
        }
    }
    if let Some(config) = top
        .get("request")
        .and_then(|r| fields(r.get()))
        .and_then(|inner| inner.get("generationConfig").map(|g| parse(Some(g))))
    {
        out.insert("request".into(), json!({"generationConfig":config}));
    }
    Value::Object(out)
}
/// A distinctive fragment of `text` that JSON encoders write verbatim (ASCII letters, digits, space and a few
/// plain punctuation marks; no quotes, backslashes, `/` or `<>&`, which some encoders escape): the longest run of
/// them, at most 64 chars. SQLite uses it (`instr`) to skip rows that cannot hold the text; the exact comparison
/// still happens on the parsed message. `None` when no run is long enough to be worth it.
fn needle(text: &str) -> Option<&str> {
    let run = text
        .split(|c: char| {
            !(c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    ' ' | '.' | ',' | ':' | ';' | '-' | '_' | '(' | ')' | '=' | '+'
                ))
        })
        .fold(
            "",
            |best, run| if run.len() > best.len() { run } else { best },
        );
    (run.len() >= 24).then(|| &run[..run.len().min(64)])
}
pub fn find(file: &Path, since: &str, last_user: &str, family: &str) -> Value {
    let read = || -> std::result::Result<Value, Box<dyn std::error::Error>> {
        let db = Connection::open_with_flags(
            file,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_millis(100))?;
        // Subtract up to 30s from since if it's an RFC3339 timestamp to accommodate clock skew or logging latency
        let query_since = chrono::DateTime::parse_from_rfc3339(since)
            .map(|dt| {
                (dt - chrono::Duration::seconds(30))
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            })
            .unwrap_or_else(|_| since.to_string());
        // Rows above 512 KiB are discarded in the query itself: they never pass through process memory, and neither
        // do the rows that do not even contain a fragment of the text (`?2`, when there is one).
        let mut query=db.prepare("SELECT timestamp,provider,model,data FROM requestDetails WHERE timestamp>=?1 AND length(CAST(data AS BLOB))<=524288 AND (?2 IS NULL OR instr(data,?2)>0) ORDER BY timestamp DESC LIMIT 20")?;
        let mut rows = query.query(rusqlite::params![query_since, needle(last_user)])?;
        while let Some(row) = rows.next()? {
            let raw: String = row.get(3)?;
            if raw.len() > 512 * 1024 {
                continue;
            }
            // Only the pieces that are read get parsed; the request/response bodies stay unparsed text.
            let Some(data) = fields(&raw) else {
                continue;
            };
            let last = data.get("request").and_then(|r| last_user_text(r));
            if last.as_deref() != Some(last_user) {
                continue;
            }
            let request = provider_request(data.get("providerRequest"));
            let tokens = parse(data.get("tokens"));
            let usage = data
                .get("providerResponse")
                .and_then(|r| fields(r.get()))
                .map(|response| parse(response.get("usage")))
                .unwrap_or(Value::Null);
            return Ok(
                json!({"found":true,"provider":row.get::<_,String>(1)?,"model":row.get::<_,String>(2)?,
                "effort":provider_effort(family,&request),"maxTokens":request.get("max_tokens").or(request.pointer("/request/generationConfig/maxOutputTokens")),
                "thinking":request.get("thinking").or(request.pointer("/request/generationConfig/thinkingConfig")),
                "usage":{"input":tokens["prompt_tokens"],"output":tokens["completion_tokens"],"reasoning":tokens.get("reasoning_tokens").or(tokens.pointer("/completion_tokens_details/reasoning_tokens")),
                    "cacheRead":tokens.get("cache_read_input_tokens").or(tokens.get("cached_tokens")).or(tokens.pointer("/prompt_tokens_details/cached_tokens")).or(usage.get("cache_read_input_tokens")),
                    "cacheWrite":tokens.get("cache_creation_input_tokens").or(usage.get("cache_creation_input_tokens"))}}),
            );
        }
        Ok(json!({"found":false,"reason":"requisição correspondente não registrada"}))
    };
    if !file.is_file() {
        return json!({"found":false,"reason":"registro indisponível","verification":"unverified"});
    }
    read().unwrap_or_else(|e|json!({"found":false,"reason":rules::clip(&format!("registro incompatível/ilegível: {e}"),240),"verification":"unverified"}))
}
