//! Mobile memory is deferred unless the user chooses local processing or an explicit API destination.
use crate::{
    Error, Result,
    local::LocalModels,
    providers::{Provider, RouteTarget, RuntimeProviders},
    rules,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryMode {
    #[default]
    Deferred,
    Local,
    Api,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryApi {
    pub provider: String,
    pub model: String,
    /// Consent covers history processing for turn/cast/episode/curation/review jobs.
    pub history_consent: bool,
    #[serde(default = "output_budget")]
    pub max_tokens: u64,
    #[serde(default = "input_budget")]
    pub max_input_tokens: usize,
    #[serde(default = "timeout")]
    pub timeout_seconds: u64,
}
fn output_budget() -> u64 {
    1200
}
fn input_budget() -> usize {
    6000
}
fn timeout() -> u64 {
    60
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryPolicy {
    #[serde(default)]
    pub mode: MemoryMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<MemoryApi>,
}
pub type PolicySnapshot = Arc<RwLock<MemoryPolicy>>;

impl MemoryPolicy {
    pub fn parse(value: &Value) -> Result<Self> {
        let policy: Self = serde_json::from_value(value.clone())
            .map_err(|_| Error::new(400, "Política de memória inválida."))?;
        if policy.mode == MemoryMode::Api && policy.api.is_none()
            || policy.api.as_ref().is_some_and(|a| {
                !a.history_consent
                    || a.provider.is_empty()
                    || a.model.is_empty()
                    || !(256..=2048).contains(&a.max_tokens)
                    || !(1024..=16384).contains(&a.max_input_tokens)
                    || !(5..=120).contains(&a.timeout_seconds)
            })
        {
            return Err(Error::new(
                400,
                "API de memória exige destino explícito, historyConsent=true e limites válidos.",
            ));
        }
        Ok(policy)
    }
    pub fn check_destination(&self, runtime: &RuntimeProviders) -> Result<()> {
        if self.mode == MemoryMode::Api {
            let api = self.api.as_ref().ok_or_else(unavailable)?;
            if !api.history_consent {
                return Err(unavailable());
            }
            runtime
                .snapshot()
                .ok_or_else(unavailable)?
                .target(&RouteTarget {
                    provider: api.provider.clone(),
                    model: api.model.clone(),
                })?;
        }
        Ok(())
    }
}
fn unavailable() -> Error {
    Error::new(503, "Backend de memória indisponível; tarefa adiada.")
        .extra("code", "MEMORY_BACKEND_UNAVAILABLE")
}

pub enum MemoryBackend {
    Local {
        local: LocalModels,
        compact: bool,
    },
    Api {
        client: reqwest::Client,
        provider: Provider,
        api: MemoryApi,
    },
}
impl MemoryBackend {
    pub async fn resolve(
        local: &LocalModels,
        runtime: &RuntimeProviders,
        policy: &MemoryPolicy,
        client: &reqwest::Client,
    ) -> Result<Option<Self>> {
        if local.is_mobile() {
            Self::mobile(local, runtime, policy, client).await
        } else {
            Self::desktop(local, runtime, policy, client).await
        }
    }

    pub async fn desktop(
        local: &LocalModels,
        runtime: &RuntimeProviders,
        policy: &MemoryPolicy,
        client: &reqwest::Client,
    ) -> Result<Option<Self>> {
        if policy.mode == MemoryMode::Deferred {
            return Ok(None);
        }
        if policy.mode == MemoryMode::Local && local.chat_available() {
            return Ok(Some(Self::Local {
                local: local.clone(),
                compact: false,
            }));
        }
        if let Ok(val) = std::env::var("MC_TEST_LOCAL_URL") {
            if val.trim().is_empty() {
                return Ok(None);
            }
        }
        Self::desktop_upstream(runtime, policy, client).await
    }

    pub fn desktop_upstream_model(
        runtime: &RuntimeProviders,
        policy: &MemoryPolicy,
    ) -> Option<String> {
        if policy.mode == MemoryMode::Deferred {
            return None;
        }
        if let Ok(val) = std::env::var("MC_TEST_LOCAL_URL") {
            if val.trim().is_empty() {
                return None;
            }
        }
        let snapshot = runtime.snapshot()?;
        if let Some(api) = &policy.api {
            Some(api.model.clone())
        } else {
            let provider = snapshot.default_provider();
            let model = std::env::var("MC_MEMORY_MODEL")
                .or_else(|_| std::env::var("MEMORY_MODEL"))
                .unwrap_or_else(|_| {
                    if let Some(m) = provider.models.first() {
                        m.clone()
                    } else {
                        "oc/deepseek-v4.1-flash-free".to_string()
                    }
                });
            Some(model)
        }
    }

    async fn desktop_upstream(
        runtime: &RuntimeProviders,
        policy: &MemoryPolicy,
        client: &reqwest::Client,
    ) -> Result<Option<Self>> {
        let Some(snapshot) = runtime.snapshot() else {
            return Ok(None);
        };
        let (provider, model) = if let Some(api) = &policy.api {
            match snapshot.target(&RouteTarget {
                provider: api.provider.clone(),
                model: api.model.clone(),
            }) {
                Ok((p, m)) => (p, m),
                Err(_) => (snapshot.default_provider().clone(), api.model.clone()),
            }
        } else {
            let provider = snapshot.default_provider().clone();
            let model = std::env::var("MC_MEMORY_MODEL")
                .or_else(|_| std::env::var("MEMORY_MODEL"))
                .unwrap_or_else(|_| {
                    if let Some(m) = provider.models.first() {
                        m.clone()
                    } else {
                        "oc/deepseek-v4.1-flash-free".to_string()
                    }
                });
            (provider, model)
        };
        let mut model = model;
        if provider.is_loopback() && !provider.preflight(&model).await {
            if model != "Deepseek" && provider.preflight("Deepseek").await {
                model = "Deepseek".to_string();
            } else {
                return Ok(None);
            }
        }
        let api = MemoryApi {
            provider: provider.id.clone(),
            model,
            history_consent: true,
            max_tokens: 3000,
            max_input_tokens: 32000,
            timeout_seconds: 90,
        };
        Ok(Some(Self::Api {
            client: client.clone(),
            provider,
            api,
        }))
    }

    pub async fn mobile(
        local: &LocalModels,
        runtime: &RuntimeProviders,
        policy: &MemoryPolicy,
        client: &reqwest::Client,
    ) -> Result<Option<Self>> {
        // Standby pauses claims and embeddings for ALL policies, including API.
        if !local.mobile_activated() || policy.mode == MemoryMode::Deferred {
            return Ok(None);
        }
        match policy.mode {
            MemoryMode::Deferred => Ok(None),
            MemoryMode::Local if local.chat_available() => Ok(Some(Self::Local {
                local: local.clone(),
                compact: true,
            })),
            MemoryMode::Local => Ok(None),
            MemoryMode::Api => {
                let api = policy.api.clone().ok_or_else(unavailable)?;
                if !api.history_consent {
                    return Err(unavailable());
                }
                let (provider, _) =
                    runtime
                        .snapshot()
                        .ok_or_else(unavailable)?
                        .target(&RouteTarget {
                            provider: api.provider.clone(),
                            model: api.model.clone(),
                        })?;
                if provider.is_loopback() && !provider.preflight(&api.model).await {
                    return Err(unavailable());
                }
                Ok(Some(Self::Api {
                    client: client.clone(),
                    provider,
                    api,
                }))
            }
        }
    }
    pub fn compact(&self) -> bool {
        matches!(self, Self::Local { compact: true, .. })
    }
    pub fn available(&self) -> bool {
        match self {
            Self::Local { local, .. } => local.chat_available(),
            Self::Api { .. } => true,
        }
    }
    pub async fn chat_json(
        &self,
        messages: Vec<Value>,
        schema: Value,
        max_tokens: u64,
        temperature: f64,
    ) -> Result<Value> {
        let result = match self {
            Self::Local { local, .. } => {
                local
                    .chat_json(messages, schema.clone(), max_tokens, temperature)
                    .await?
            }
            Self::Api {
                client,
                provider,
                api,
            } => {
                let reserve = max_tokens.min(api.max_tokens).min(provider.max_output);
                let input = rules::estimate_messages(&messages);
                if input > api.max_input_tokens
                    || input as u64 + reserve + 64 > provider.context_window
                {
                    return Err(Error::new(
                        502,
                        "Entrada de memória excedeu o orçamento configurado; retry reduzido necessário.",
                    ));
                }
                let adapted_messages: Vec<Value> = messages
                    .into_iter()
                    .flat_map(|m| {
                        if m["role"] == "system" {
                            vec![
                                json!({"role":"user","content":format!("[INSTRUÇÕES DO SISTEMA]\n{}", m["content"].as_str().unwrap_or_default())}),
                                json!({"role":"assistant","content":"Entendido. Seguirei estritamente as instruções fornecidas."}),
                            ]
                        } else {
                            vec![m]
                        }
                    })
                    .collect();
                let body = json!({
                    "messages": adapted_messages,
                    "stream": false,
                    "temperature": temperature,
                    "max_tokens": reserve,
                    "chat_template_kwargs": { "enable_thinking": false },
                    "thinking": { "type": "disabled" },
                    "response_format": {
                        "type": "json_schema",
                        "json_schema": {
                            "name": "memory",
                            "strict": true,
                            "schema": schema
                        }
                    }
                });
                // No retry or route fallback here: this POST may have been accepted.
                let mut response = provider
                    .chat_request(
                        client,
                        &api.model,
                        &body,
                        Duration::from_secs(api.timeout_seconds),
                    )
                    .send()
                    .await
                    .map_err(|_| unavailable())?;
                let mut status = response.status();
                if status.as_u16() == 400 {
                    let err_text = response.text().await.unwrap_or_default();
                    if err_text.contains("response_format") || err_text.contains("json_schema") || err_text.contains("unavailable") {
                        tracing::warn!("Upstream rejected json_schema; retrying with json_object fallback: {}", err_text);
                        let mut fallback_body = body.clone();
                        fallback_body["response_format"] = json!({ "type": "json_object" });
                        let fb_response = provider
                            .chat_request(
                                client,
                                &api.model,
                                &fallback_body,
                                Duration::from_secs(api.timeout_seconds),
                            )
                            .send()
                            .await
                            .map_err(|_| unavailable())?;
                        if fb_response.status().as_u16() == 400 {
                            let fb_err = fb_response.text().await.unwrap_or_default();
                            tracing::warn!("Upstream also rejected json_object; retrying without response_format: {}", fb_err);
                            let mut unconstrained_body = fallback_body.clone();
                            unconstrained_body.as_object_mut().unwrap().remove("response_format");
                            response = provider
                                .chat_request(
                                    client,
                                    &api.model,
                                    &unconstrained_body,
                                    Duration::from_secs(api.timeout_seconds),
                                )
                                .send()
                                .await
                                .map_err(|_| unavailable())?;
                            status = response.status();
                        } else {
                            response = fb_response;
                            status = response.status();
                        }
                    } else {
                        tracing::warn!(status = status.as_u16(), body = %err_text, "API de memória recusou requisição");
                        return Err(Error::new(
                            502,
                            format!("API de memória recusou o schema ou a entrada (HTTP {status}): {err_text}"),
                        ));
                    }
                }
                if matches!(status.as_u16(), 401 | 403 | 404 | 408 | 429)
                    || status.is_server_error()
                    || status.is_redirection()
                {
                    return Err(unavailable());
                }
                if !status.is_success() {
                    let err_text = response.text().await.unwrap_or_default();
                    tracing::warn!(status = status.as_u16(), body = %err_text, "API de memória recusou requisição");
                    return Err(Error::new(
                        502,
                        format!("API de memória recusou o schema ou a entrada (HTTP {status}): {err_text}"),
                    ));
                }
                let mut stream = response.bytes_stream();
                let mut bytes = Vec::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|_| unavailable())?;
                    if bytes.len() + chunk.len() > 1024 * 1024 {
                        return Err(Error::new(502, "Resposta de memória excedeu o limite."));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                parse_model_content(&bytes)?
            }
        };
        validate_schema(&result, &schema)?;
        Ok(result)
    }
}

/// Validates the subset used by the bundled extraction/cast/review/curation schemas before sanitization.
/// A parseable but missing field is a normal job retry, never success or backend unavailability.
pub fn validate_schema(value: &Value, schema: &Value) -> Result<()> {
    fn valid(v: &Value, s: &Value, depth: usize) -> bool {
        if depth > 32 {
            return false;
        }
        let matches_type = |t: &str| match t {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "integer" => v.is_i64() || v.is_u64(),
            "number" => v.is_number(),
            "boolean" => v.is_boolean(),
            "null" => v.is_null(),
            _ => false,
        };
        if let Some(t) = s["type"].as_str()
            && !matches_type(t)
        {
            return false;
        }
        if let Some(types) = s["type"].as_array()
            && !types.iter().any(|t| t.as_str().is_some_and(matches_type))
        {
            return false;
        }
        if let Some(values) = s["enum"].as_array()
            && !values.contains(v)
        {
            return false;
        }
        if let Some(any) = s["anyOf"].as_array()
            && !any.iter().any(|s| valid(v, s, depth + 1))
        {
            return false;
        }
        if let Some(object) = v.as_object() {
            if s["required"].as_array().is_some_and(|keys| {
                keys.iter()
                    .any(|k| !k.as_str().is_some_and(|k| object.contains_key(k)))
            }) {
                return false;
            }
            for (key, field) in object {
                if let Some(field_schema) = s["properties"].get(key) {
                    if !valid(field, field_schema, depth + 1) {
                        return false;
                    }
                } else if s["additionalProperties"] == false {
                    return false;
                }
            }
        }
        if let Some(items) = v.as_array()
            && !s["items"].is_null()
            && items
                .iter()
                .any(|item| !valid(item, &s["items"], depth + 1))
        {
            return false;
        }
        true
    }
    if valid(value, schema, 0) {
        Ok(())
    } else {
        Err(Error::new(
            502,
            "Saída de memória incompleta ou incompatível com o schema; retry necessário.",
        ))
    }
}

fn parse_model_content(bytes: &[u8]) -> Result<Value> {
    if bytes.starts_with(b"data: ") || bytes.windows(7).any(|w| w == b"\ndata: ") {
        let text = std::str::from_utf8(bytes).map_err(|_| Error::new(502, "Resposta de memória não é UTF-8."))?;
        let mut combined_content = String::new();
        for line in text.lines() {
            let line = line.trim();
            if let Some(data) = line.strip_prefix("data: ") {
                let data = data.trim();
                if data == "[DONE]" {
                    break;
                }
                if let Ok(chunk) = serde_json::from_str::<Value>(data) {
                    if let Some(content) = chunk["choices"][0]["delta"]["content"].as_str() {
                        combined_content.push_str(content);
                    } else if let Some(content) = chunk["choices"][0]["message"]["content"].as_str() {
                        combined_content.push_str(content);
                    }
                }
            }
        }
        if !combined_content.is_empty() {
            return parse_json_content(&combined_content);
        }
    }

    let response: Value = serde_json::from_slice(bytes)
        .map_err(|_| Error::new(502, "Envelope JSON de memória inválido."))?;
    if matches!(
        response["choices"][0]["finish_reason"].as_str(),
        Some("length" | "max_tokens")
    ) {
        return Err(Error::new(
            502,
            "JSON de memória ficou incompleto; retry necessário.",
        ));
    }
    let content = response["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default();
    parse_json_content(content)
}

fn parse_json_content(raw: &str) -> Result<Value> {
    let mut text = raw.trim();
    if let Some(pos) = text.find("</think>") {
        text = text[pos + "</think>".len()..].trim();
    }
    if let Some(stripped) = text.strip_prefix("```json") {
        text = stripped;
    } else if let Some(stripped) = text.strip_prefix("```") {
        text = stripped;
    }
    if let Some(pos) = text.rfind("```") {
        text = text[..pos].trim();
    }
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        return Ok(v);
    }
    if let Some(start) = text.find('{')
        && let Some(end) = text.rfind('}')
        && end > start
    {
        if let Ok(v) = serde_json::from_str::<Value>(&text[start..=end]) {
            return Ok(v);
        }
    }
    Err(Error::new(502, "API de memória não devolveu JSON completo."))
}
