//! Read-only, bounded evidence from the router's own request log.
use crate::{rules, s};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
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
pub fn find(file: &Path, since: &str, last_user: &str, family: &str) -> Value {
    let read = || -> std::result::Result<Value, Box<dyn std::error::Error>> {
        let db = Connection::open_with_flags(
            file,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_millis(100))?;
        // Rows above 512 KiB are discarded in the query itself: they never pass through process memory.
        let mut query=db.prepare("SELECT timestamp,provider,model,data FROM requestDetails WHERE timestamp>=? AND length(CAST(data AS BLOB))<=524288 ORDER BY timestamp DESC LIMIT 60")?;
        let mut rows = query.query([since])?;
        while let Some(row) = rows.next()? {
            let raw: String = row.get(3)?;
            if raw.len() > 512 * 1024 {
                continue;
            }
            let Ok(data) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            let last = data["request"]["messages"]
                .as_array()
                .and_then(|a| a.iter().rev().find(|m| m["role"] == "user"))
                .map(|m| s(m, "content"));
            if last.as_deref() != Some(last_user) {
                continue;
            }
            let request = &data["providerRequest"];
            let tokens = &data["tokens"];
            let usage = &data["providerResponse"]["usage"];
            return Ok(
                json!({"found":true,"provider":row.get::<_,String>(1)?,"model":row.get::<_,String>(2)?,
                "effort":provider_effort(family,request),"maxTokens":request.get("max_tokens").or(request.pointer("/request/generationConfig/maxOutputTokens")),
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
