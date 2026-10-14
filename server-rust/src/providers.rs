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
//!       "models": ["<model id configured by the user>"] }
//!   ]
//! }
//! ```
//!
//! A desktop route is `"<provider id>:<model>"`. A route
//! without a known provider prefix goes to the default provider, so existing routes such as
//! `cbai/deepseek-v4.1-flash` keep working. Without a file, a single provider is built from the environment
//! (`MC_UPSTREAM_URL`/`NINEROUTER_URL` and `MC_UPSTREAM_API_KEY`/`NINEROUTER_API_KEY`).
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::Path,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};

#[derive(Clone)]
pub struct Provider {
    pub id: String,
    pub base_url: String,
    pub api_key: Option<String>,
    /// Fixed model list. When empty, the list comes from the provider's `/models` endpoint.
    pub models: Vec<String>,
    /// Optional read-only request log (9Router's SQLite database) used to confirm the reasoning effort the
    /// upstream model actually received.
    pub request_log: Option<PathBuf>,
    pub reasoning_format: ReasoningFormat,
    pub context_window: u64,
    pub max_output: u64,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("id", &self.id)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("models", &self.models)
            .finish_non_exhaustive()
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        if let Some(key) = &mut self.api_key {
            key.zeroize();
        }
    }
}

/// The serialized representation is used only by the encrypted Store worker.
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Secret(pub String);
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.0.zeroize();
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningFormat {
    #[default]
    Omit,
    Openai,
    Openrouter,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteTarget {
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MobileProvider {
    pub id: String,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<Secret>,
    pub models: Vec<String>,
    #[serde(default)]
    pub allow_loopback: bool,
    #[serde(default)]
    pub reasoning_format: ReasoningFormat,
    #[serde(default = "default_context")]
    pub context_window: u64,
    #[serde(default = "default_output")]
    pub max_output: u64,
}
fn default_context() -> u64 {
    8192
}
fn default_output() -> u64 {
    2048
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MobileProviders {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub providers: Vec<MobileProvider>,
    #[serde(default)]
    pub aliases: BTreeMap<String, RouteTarget>,
    /// Key is the original, stored route. Used only when its loopback preflight fails.
    #[serde(default)]
    pub fallbacks: BTreeMap<String, RouteTarget>,
}

fn config_error() -> Error {
    Error::new(400, "Configuração de provedores mobile inválida.")
}
fn loopback_url(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(|h| {
        h == "localhost"
            || h.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    })
}
fn mobile_url(raw: &str, allow_loopback: bool) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw).map_err(|_| config_error())?;
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some()
        || url.query().is_some() || url.fragment().is_some()
        || !(url.scheme() == "https" || url.scheme() == "http" && allow_loopback && loopback_url(&url))
        || loopback_url(&url) && !allow_loopback
        // Anthropic's native Messages API is not an OpenAI chat-completions API.
        || url.host_str() == Some("api.anthropic.com") || url.path().trim_end_matches('/').ends_with("/messages")
        || url.path().trim_end_matches('/').ends_with("/chat/completions") || raw.contains('\\')
    {
        return Err(config_error());
    }
    Ok(url)
}

impl MobileProviders {
    pub fn parse(value: &Value) -> Result<Self> {
        if serde_json::to_vec(value).map_err(|_| config_error())?.len() > 128 * 1024 {
            return Err(config_error());
        }
        let config: Self = serde_json::from_value(value.clone()).map_err(|_| config_error())?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if self.providers.len() > 16 || self.aliases.len() > 256 || self.fallbacks.len() > 256 {
            return Err(config_error());
        }
        let mut ids = std::collections::HashSet::new();
        for p in &self.providers {
            if p.id.is_empty()
                || p.id.len() > 64
                || !p
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
                || !ids.insert(&p.id)
                || p.models.is_empty()
                || p.models.len() > 128
                || p.models.iter().any(|m| {
                    m.trim().is_empty() || m.len() > 256 || m.chars().any(char::is_control)
                })
                || !(1024..=262144).contains(&p.context_window)
                || !(256..=8192).contains(&p.max_output)
                || p.max_output >= p.context_window
            {
                return Err(config_error());
            }
            mobile_url(&p.base_url, p.allow_loopback)?;
            if let Some(key) = &p.api_key {
                if key.0.is_empty()
                    || key.0.len() > 4096
                    || key.0.chars().any(char::is_control)
                    || reqwest::header::HeaderValue::from_str(&format!("Bearer {}", key.0)).is_err()
                {
                    return Err(config_error());
                }
            }
        }
        if self.default.as_ref().is_some_and(|id| !ids.contains(id)) {
            return Err(config_error());
        }
        for (route, target) in self.aliases.iter().chain(self.fallbacks.iter()) {
            if route.is_empty()
                || route.len() > 512
                || route == "mobile-local"
                || route.starts_with("mobile-api:")
                || !self
                    .providers
                    .iter()
                    .any(|p| p.id == target.provider && p.models.contains(&target.model))
            {
                return Err(config_error());
            }
        }
        // A fallback cannot make an unmapped legacy alias eligible for cloud use.
        if !self.fallbacks.is_empty() {
            let providers = self.runtime_unchecked();
            if self.fallbacks.keys().any(|r| {
                providers
                    .mobile_resolve(r)
                    .is_none_or(|(p, _)| !p.is_loopback())
            }) {
                return Err(config_error());
            }
        }
        Ok(())
    }
    fn runtime_unchecked(&self) -> Providers {
        Providers {
            list: self
                .providers
                .iter()
                .map(|p| Provider {
                    id: p.id.clone(),
                    base_url: p.base_url.trim_end_matches('/').to_owned(),
                    api_key: p.api_key.as_ref().map(|k| k.0.clone()),
                    models: p.models.clone(),
                    request_log: None,
                    reasoning_format: p.reasoning_format,
                    context_window: p.context_window,
                    max_output: p.max_output,
                })
                .collect(),
            default: self
                .default
                .as_ref()
                .and_then(|id| self.providers.iter().position(|p| &p.id == id))
                .unwrap_or(0),
            aliases: self.aliases.clone(),
            fallbacks: self.fallbacks.clone(),
            mobile: true,
        }
    }
    pub fn runtime(&self) -> Result<Option<Arc<Providers>>> {
        self.validate()?;
        Ok((!self.providers.is_empty()).then(|| Arc::new(self.runtime_unchecked())))
    }
    pub fn redacted(&self) -> Value {
        serde_json::json!({"default":self.default,"providers":self.providers.iter().map(|p| serde_json::json!({
            "id":p.id,"baseUrl":p.base_url,"models":p.models,"hasApiKey":p.api_key.is_some(),
            "allowLoopback":p.allow_loopback,"protocol":"openai-compatible","reasoningFormat":p.reasoning_format,
            "contextWindow":p.context_window,"maxOutput":p.max_output
        })).collect::<Vec<_>>(),"aliases":self.aliases,"fallbacks":self.fallbacks})
    }
}

/// Every operation holds its own Arc snapshot; changes never reroute an accepted POST.
#[derive(Clone, Default)]
pub struct RuntimeProviders(Arc<RwLock<Option<Arc<Providers>>>>);
impl RuntimeProviders {
    pub fn new(providers: Option<Arc<Providers>>) -> Self {
        Self(Arc::new(RwLock::new(providers)))
    }
    pub fn snapshot(&self) -> Option<Arc<Providers>> {
        self.0.read().unwrap().clone()
    }
    pub fn set(&self, providers: Option<Arc<Providers>>) {
        *self.0.write().unwrap() = providers;
    }
}

#[derive(Clone, Debug)]
pub enum SelectedBackend {
    Cloud {
        provider: Provider,
        model: String,
        preflight_fallback: bool,
    },
    Local,
}
impl SelectedBackend {
    pub fn diagnostic(&self, requested: &str) -> Value {
        match self {
            Self::Cloud {
                provider,
                model,
                preflight_fallback,
            } => {
                serde_json::json!({"requestedRoute":requested,"effectiveRoute":format!("{}:{model}", provider.id),"backend":"api","provider":provider.id,"model":model,"preflightFallback":preflight_fallback})
            }
            Self::Local => {
                serde_json::json!({"requestedRoute":requested,"effectiveRoute":"mobile-local","backend":"local","model":"Qwen2.5-1.5B-Instruct","originalRoutePreserved":true})
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Providers {
    list: Vec<Provider>,
    default: usize,
    aliases: BTreeMap<String, RouteTarget>,
    fallbacks: BTreeMap<String, RouteTarget>,
    mobile: bool,
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
                Error::new(
                    400,
                    format!("Cannot read providers file {}: {e}", file.display()),
                )
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
                reasoning_format: ReasoningFormat::Omit,
                context_window: 128000,
                max_output: 8192,
            }],
            default: 0,
            aliases: BTreeMap::new(),
            fallbacks: BTreeMap::new(),
            mobile: false,
        }))
    }

    pub fn from_json(value: &Value) -> Result<Self> {
        let mut list = Vec::new();
        for p in value["providers"].as_array().into_iter().flatten() {
            let id = p["id"].as_str().unwrap_or_default().trim();
            if id.is_empty() || id.contains(':') || list.iter().any(|x: &Provider| x.id == id) {
                return Err(Error::new(
                    400,
                    format!("Invalid or duplicated provider id: {id:?}"),
                ));
            }
            let base_url = p["baseUrl"]
                .as_str()
                .unwrap_or_default()
                .trim_end_matches('/');
            if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
                return Err(Error::new(
                    400,
                    format!("Provider {id}: baseUrl must be http(s)"),
                ));
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
                reasoning_format: ReasoningFormat::Omit,
                context_window: 128000,
                max_output: 8192,
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
        let aliases = serde_json::from_value(
            value
                .get("aliases")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        )
        .map_err(|_| config_error())?;
        let fallbacks = serde_json::from_value(
            value
                .get("fallbacks")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        )
        .map_err(|_| config_error())?;
        let providers = Self {
            list,
            default,
            aliases,
            fallbacks,
            mobile: false,
        };
        if providers
            .aliases
            .values()
            .chain(providers.fallbacks.values())
            .any(|t| !providers.list.iter().any(|p| p.id == t.provider) || t.model.is_empty())
        {
            return Err(config_error());
        }
        Ok(providers)
    }

    pub fn default_provider(&self) -> &Provider {
        &self.list[self.default]
    }

    /// Splits a route into its provider and the model name sent upstream.
    pub fn resolve<'a>(&'a self, route: &str) -> (&'a Provider, String) {
        if let Some(t) = self.aliases.get(route) {
            return (
                self.list.iter().find(|p| p.id == t.provider).unwrap(),
                t.model.clone(),
            );
        }
        if let Some((id, model)) = route.split_once(':')
            && let Some(p) = self.list.iter().find(|p| p.id == id)
        {
            return (p, model.to_owned());
        }
        (self.default_provider(), route.to_owned())
    }

    /// Route id as the app shows and stores it: the default provider's models keep their bare name.
    fn route_id(&self, index: usize, model: &str) -> String {
        if self.mobile {
            format!("mobile-api:{}:{model}", self.list[index].id)
        } else if index == self.default {
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
        out.extend(self.aliases.keys().cloned());
        out.sort();
        out.dedup();
        match (out.is_empty(), last_error) {
            (true, Some(e)) => Err(e),
            _ => Ok(out),
        }
    }

    pub fn target(&self, target: &RouteTarget) -> Result<(Provider, String)> {
        let provider = self
            .list
            .iter()
            .find(|p| p.id == target.provider && (!self.mobile || p.models.contains(&target.model)))
            .ok_or_else(|| {
                Error::new(503, "Destino explícito indisponível.")
                    .extra("code", "PROVIDER_UNAVAILABLE")
            })?;
        Ok((provider.clone(), target.model.clone()))
    }
    fn mobile_resolve(&self, route: &str) -> Option<(Provider, String)> {
        if let Some(target) = self.aliases.get(route) {
            return self.target(target).ok();
        }
        let explicit = route.strip_prefix("mobile-api:").unwrap_or(route);
        if let Some((provider, model)) = explicit.split_once(':') {
            return self
                .target(&RouteTarget {
                    provider: provider.into(),
                    model: model.into(),
                })
                .ok();
        }
        if self.list.is_empty() {
            return None;
        }
        let provider = self.default_provider();
        provider
            .models
            .iter()
            .any(|m| m == route)
            .then(|| (provider.clone(), route.to_owned()))
    }
    pub async fn select(&self, route: &str) -> SelectedBackend {
        let target = if self.mobile {
            self.mobile_resolve(route)
        } else {
            let (p, m) = self.resolve(route);
            Some((p.clone(), m))
        };
        let Some((provider, model)) = target else {
            return SelectedBackend::Local;
        };
        if provider.is_loopback()
            && (self.mobile || self.fallbacks.contains_key(route))
            && !provider.preflight(&model).await
        {
            if let Some(target) = self.fallbacks.get(route)
                && let Ok((provider, model)) = self.target(target)
            {
                return SelectedBackend::Cloud {
                    provider,
                    model,
                    preflight_fallback: true,
                };
            }
            return SelectedBackend::Local;
        }
        SelectedBackend::Cloud {
            provider,
            model,
            preflight_fallback: false,
        }
    }
}

impl Provider {
    pub fn is_loopback(&self) -> bool {
        reqwest::Url::parse(&self.base_url).is_ok_and(|u| loopback_url(&u))
    }
    pub async fn preflight(&self, model: &str) -> bool {
        // A listening port is not proof of an OpenAI-compatible service. This GET is
        // read-only, never follows redirects, and includes body validation in its deadline.
        static CLIENT: std::sync::LazyLock<Option<reqwest::Client>> =
            std::sync::LazyLock::new(|| {
                reqwest::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .ok()
            });
        let Some(client) = CLIENT.as_ref() else {
            return false;
        };
        tokio::time::timeout(Duration::from_millis(400), async {
            let mut request = client.get(format!("{}/models", self.base_url.trim_end_matches('/')));
            if let Some(key) = &self.api_key {
                request = request.bearer_auth(key);
            }
            let Ok(response) = request.send().await else {
                return false;
            };
            if !response.status().is_success() {
                return false;
            }
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
                let Ok(chunk) = chunk else {
                    return false;
                };
                if bytes.len() + chunk.len() > 128 * 1024 {
                    return false;
                }
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice::<Value>(&bytes)
                .ok()
                .is_some_and(|body| {
                    body["data"].as_array().is_some_and(|models| {
                        models.iter().any(|m| m["id"].as_str() == Some(model))
                    })
                })
        })
        .await
        .unwrap_or(false)
    }
    pub fn chat_request(
        &self,
        client: &reqwest::Client,
        model: &str,
        body: &Value,
        timeout: Duration,
    ) -> reqwest::RequestBuilder {
        let mut body = body.clone();
        body["model"] = Value::String(model.to_owned());
        let mut request = client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body)
            .timeout(timeout);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        request
    }
    pub fn prepare_mobile_plan(&self, plan: &mut crate::presets::Plan) {
        let body = plan.body.as_object_mut().unwrap();
        for name in ["reasoning_effort", "reasoning", "output_config", "thinking"] {
            body.remove(name);
        }
        let enabled = plan.controls["thinkingEnabled"] != false;
        let effort = plan.controls["requestedEffort"].as_str();
        match self.reasoning_format {
            ReasoningFormat::Openai => {
                if let Some(e) = effort {
                    body.insert(
                        "reasoning_effort".into(),
                        serde_json::json!(if enabled { e } else { "none" }),
                    );
                }
            }
            ReasoningFormat::Openrouter => {
                body.insert(
                    "reasoning".into(),
                    if enabled {
                        serde_json::json!({"effort":effort,"enabled":true})
                    } else {
                        serde_json::json!({"enabled":false})
                    },
                );
            }
            ReasoningFormat::Omit => {}
        }
        plan.capacity = serde_json::json!({"contextWindow":self.context_window,"maxOutput":self.max_output,"source":"configured"});
        plan.controls["sentParams"] = Value::Object(
            body.iter()
                .filter(|(k, _)| {
                    matches!(k.as_str(), "reasoning_effort" | "reasoning" | "max_tokens")
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        );
        plan.controls["confirmation"] = serde_json::json!("unverified");
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
        return Err(Error::new(
            502,
            format!("Provider {} HTTP {}", p.id, response.status()),
        ));
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
        assert_eq!(
            (b.id.as_str(), m.as_str(), b.base_url.as_str()),
            ("main", "cbai/glm-5.2", "http://127.0.0.1:20128/v1")
        );
        // an unknown prefix is part of the model name of the default provider
        assert_eq!(p.resolve("other:model").1, "other:model");
        assert_eq!(p.route_id(1, "deepseek/x"), "openrouter:deepseek/x");
    }

    #[test]
    fn invalid_files_are_rejected() {
        assert!(Providers::from_json(&json!({"providers":[]})).is_err());
        assert!(
            Providers::from_json(&json!({"providers":[{"id":"a:b","baseUrl":"http://x"}]}))
                .is_err()
        );
        assert!(
            Providers::from_json(&json!({"providers":[{"id":"a","baseUrl":"ftp://x"}]})).is_err()
        );
        assert!(
            Providers::from_json(
                &json!({"default":"zz","providers":[{"id":"a","baseUrl":"http://x"}]})
            )
            .is_err()
        );
    }
}
