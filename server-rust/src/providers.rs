//! Model providers. Every provider is an OpenAI-compatible HTTP API (`/models` and `/chat/completions`):
//! 9Router, OpenRouter, a local llama.cpp/vLLM/Ollama server, or anything else that speaks that protocol.
//!
//! Providers come from a JSON file (`MC_PROVIDERS_FILE`, or `providers.json` in the data directory):
//!
//! ```json
//! {
//!   "default": "9router",
//!   "providers": [
//!     { "id": "9router", "baseUrl": "http://127.0.0.1:20128/v1", "apiKeyEnv": "NINEROUTER_API_KEY",
//!       "requestLog": "C:/Users/me/AppData/Roaming/9router/db/data.sqlite" },
//!     { "id": "openrouter", "baseUrl": "https://openrouter.ai/api/v1", "apiKeyEnv": "OPENROUTER_API_KEY",
//!       "models": ["deepseek/deepseek-v4.1-flash"] }
//!   ]
//! }
//! ```
//!
//! A route is `"<provider id>:<model>"` (for example `openrouter:deepseek/deepseek-v4.1-flash`). A route
//! without a known provider prefix goes to the default provider, so existing routes such as
//! `cbai/deepseek-v4.1-flash` keep working. Without a file, a single provider is built from the environment
//! (`MC_UPSTREAM_URL`/`NINEROUTER_URL` and `MC_UPSTREAM_API_KEY`/`NINEROUTER_API_KEY`).
use crate::{Error, Result};
use serde_json::Value;
use std::{path::Path, path::PathBuf, time::Duration};

#[derive(Clone, Debug)]
pub struct Provider {
    pub id: String,
    pub base_url: String,
    pub api_key: Option<String>,
    /// Fixed model list. When empty, the list comes from the provider's `/models` endpoint.
    pub models: Vec<String>,
    /// Optional read-only request log (9Router's SQLite database) used to confirm the reasoning effort the
    /// upstream model actually received.
    pub request_log: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct Providers {
    list: Vec<Provider>,
    default: usize,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

impl Providers {
    /// Loads the provider list. `fallback_url` is the URL used when neither a file nor an URL variable
    /// exists (production keeps the historical local 9Router address).
    pub fn load(data_dir: &Path, fallback_url: Option<&str>) -> Result<Option<Self>> {
        let file = env("MC_PROVIDERS_FILE")
            .map(PathBuf::from)
            .or_else(|| Some(data_dir.join("providers.json")).filter(|p| p.exists()));
        if let Some(file) = file {
            let text = std::fs::read_to_string(&file).map_err(|e| {
                Error::new(400, format!("Cannot read providers file {}: {e}", file.display()))
            })?;
            let value: Value = serde_json::from_str(&text)
                .map_err(|e| Error::new(400, format!("Invalid providers file: {e}")))?;
            return Self::from_json(&value).map(Some);
        }
        let Some(url) = env("MC_UPSTREAM_URL")
            .or_else(|| env("NINEROUTER_URL"))
            .or_else(|| fallback_url.map(str::to_owned))
        else {
            return Ok(None);
        };
        Ok(Some(Self {
            list: vec![Provider {
                id: "default".into(),
                base_url: url.trim_end_matches('/').to_owned(),
                api_key: env("MC_UPSTREAM_API_KEY")
                    .or_else(|| env("NINEROUTER_API_KEY"))
                    .or_else(|| env("NINEROUTER_KEY")),
                models: Vec::new(),
                request_log: env("NINEROUTER_DB").map(PathBuf::from),
            }],
            default: 0,
        }))
    }

    pub fn from_json(value: &Value) -> Result<Self> {
        let mut list = Vec::new();
        for p in value["providers"].as_array().into_iter().flatten() {
            let id = p["id"].as_str().unwrap_or_default().trim();
            if id.is_empty() || id.contains(':') || list.iter().any(|x: &Provider| x.id == id) {
                return Err(Error::new(400, format!("Invalid or duplicated provider id: {id:?}")));
            }
            let base_url = p["baseUrl"].as_str().unwrap_or_default().trim_end_matches('/');
            if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
                return Err(Error::new(400, format!("Provider {id}: baseUrl must be http(s)")));
            }
            let api_key = p["apiKey"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| p["apiKeyEnv"].as_str().and_then(env));
            list.push(Provider {
                id: id.to_owned(),
                base_url: base_url.to_owned(),
                api_key: api_key.filter(|k| !k.is_empty()),
                models: p["models"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| m.as_str().map(str::to_owned))
                    .collect(),
                request_log: p["requestLog"].as_str().map(PathBuf::from),
            });
        }
        if list.is_empty() {
            return Err(Error::new(400, "The providers file lists no provider."));
        }
        let default = match value["default"].as_str() {
            Some(id) => list
                .iter()
                .position(|p| p.id == id)
                .ok_or_else(|| Error::new(400, format!("Unknown default provider: {id}")))?,
            None => 0,
        };
        Ok(Self { list, default })
    }

    pub fn default_provider(&self) -> &Provider {
        &self.list[self.default]
    }

    /// Splits a route into its provider and the model name sent upstream.
    pub fn resolve<'a>(&'a self, route: &str) -> (&'a Provider, String) {
        if let Some((id, model)) = route.split_once(':')
            && let Some(p) = self.list.iter().find(|p| p.id == id)
        {
            return (p, model.to_owned());
        }
        (self.default_provider(), route.to_owned())
    }

    /// Route id as the app shows and stores it: the default provider's models keep their bare name.
    fn route_id(&self, index: usize, model: &str) -> String {
        if index == self.default {
            model.to_owned()
        } else {
            format!("{}:{model}", self.list[index].id)
        }
    }

    /// Every model of every provider, as route ids. A provider that fails to answer is skipped (its
    /// error is returned only when no provider answered at all).
    pub async fn models(&self, client: &reqwest::Client) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let mut last_error = None;
        for (index, p) in self.list.iter().enumerate() {
            if !p.models.is_empty() {
                out.extend(p.models.iter().map(|m| self.route_id(index, m)));
                continue;
            }
            match fetch_models(client, p).await {
                Ok(models) => out.extend(models.iter().map(|m| self.route_id(index, m))),
                Err(e) => last_error = Some(e),
            }
        }
        match (out.is_empty(), last_error) {
            (true, Some(e)) => Err(e),
            _ => Ok(out),
        }
    }
}

async fn fetch_models(client: &reqwest::Client, p: &Provider) -> Result<Vec<String>> {
    let mut request = client
        .get(format!("{}/models", p.base_url))
        .timeout(Duration::from_secs(10));
    if let Some(key) = &p.api_key {
        request = request.bearer_auth(key);
    }
    let response = request
        .send()
        .await
        .map_err(|_| Error::new(502, format!("Provider {} unavailable.", p.id)))?;
    if !response.status().is_success() {
        return Err(Error::new(502, format!("Provider {} HTTP {}", p.id, response.status())));
    }
    let data: Value = response
        .json()
        .await
        .map_err(|_| Error::new(502, format!("Provider {}: invalid model list.", p.id)))?;
    Ok(data["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str().map(str::to_owned))
        .collect())
}

/// POST `/chat/completions` on the route's provider, with the provider prefix removed from `model`.
pub fn chat_request(
    client: &reqwest::Client,
    providers: &Providers,
    body: &Value,
    timeout: Duration,
) -> reqwest::RequestBuilder {
    let (provider, model) = providers.resolve(body["model"].as_str().unwrap_or_default());
    let mut body = body.clone();
    body["model"] = Value::String(model);
    let mut request = client
        .post(format!("{}/chat/completions", provider.base_url))
        .json(&body)
        .timeout(timeout);
    if let Some(key) = &provider.api_key {
        request = request.bearer_auth(key);
    }
    request
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn routes_pick_the_provider_by_prefix_and_fall_back_to_the_default() {
        let p = Providers::from_json(&json!({"default":"main","providers":[
            {"id":"main","baseUrl":"http://127.0.0.1:20128/v1/"},
            {"id":"openrouter","baseUrl":"https://openrouter.ai/api/v1","models":["deepseek/x"]}
        ]}))
        .unwrap();
        let (a, m) = p.resolve("openrouter:deepseek/x");
        assert_eq!((a.id.as_str(), m.as_str()), ("openrouter", "deepseek/x"));
        let (b, m) = p.resolve("cbai/glm-5.2");
        assert_eq!((b.id.as_str(), m.as_str(), b.base_url.as_str()), ("main", "cbai/glm-5.2", "http://127.0.0.1:20128/v1"));
        // an unknown prefix is part of the model name of the default provider
        assert_eq!(p.resolve("other:model").1, "other:model");
        assert_eq!(p.route_id(1, "deepseek/x"), "openrouter:deepseek/x");
    }

    #[test]
    fn invalid_files_are_rejected() {
        assert!(Providers::from_json(&json!({"providers":[]})).is_err());
        assert!(Providers::from_json(&json!({"providers":[{"id":"a:b","baseUrl":"http://x"}]})).is_err());
        assert!(Providers::from_json(&json!({"providers":[{"id":"a","baseUrl":"ftp://x"}]})).is_err());
        assert!(Providers::from_json(&json!({"default":"zz","providers":[{"id":"a","baseUrl":"http://x"}]})).is_err());
    }
}
