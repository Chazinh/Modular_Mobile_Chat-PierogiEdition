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
//!
//! Desktop HTTP is accepted only for literal loopback IPs or exactly `localhost`.
//! Existing non-loopback HTTP configurations must use HTTPS; configurations are never rewritten.
//! Redirects are disabled, so the configured base must be the final OpenAI-compatible API base.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::Path,
    path::PathBuf,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

fn canonical_base_url(raw: &str, unify_loopback: bool) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    let Ok(mut url) = reqwest::Url::parse(trimmed) else {
        return trimmed.to_ascii_lowercase();
    };
    if unify_loopback && let Some(host) = url.host_str() {
        let is_loopback_alias = host.eq_ignore_ascii_case("localhost")
            || host == "127.0.0.1"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| match ip {
                    std::net::IpAddr::V4(v4) => v4 == std::net::Ipv4Addr::LOCALHOST,
                    std::net::IpAddr::V6(v6) => {
                        v6.is_loopback()
                            || v6
                                .to_ipv4_mapped()
                                .is_some_and(|v4| v4 == std::net::Ipv4Addr::LOCALHOST)
                    }
                });
        if is_loopback_alias && host != "127.0.0.1" {
            let _ = url.set_host(Some("127.0.0.1"));
        }
    }
    let clean_path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&clean_path);
    url.as_str().trim_end_matches('/').to_owned()
}

pub fn normalize_base_url(raw: &str) -> String {
    canonical_base_url(raw, true)
}

fn normalize_preflight_base_url(raw: &str) -> String {
    canonical_base_url(raw, false)
}

pub fn upstream_max_concurrent_from_env() -> usize {
    match std::env::var("MC_UPSTREAM_MAX_CONCURRENT")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
    {
        Some(0) => 0,
        Some(n) => n.clamp(1, 32),
        None => 1,
    }
}

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
    pub max_concurrent: Option<usize>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<usize>,
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
                .is_ok_and(|ip| match ip {
                    std::net::IpAddr::V4(ip) => ip.is_loopback(),
                    std::net::IpAddr::V6(ip) => ip.is_loopback()
                        || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
                })
    })
}
fn desktop_url(raw: &str) -> Result<reqwest::Url> {
    // Check the original spelling too: URL parsing normalizes backslashes and strips controls.
    let invalid = || {
        Error::new(
            400,
            "baseUrl inválida: use HTTPS ou HTTP somente em loopback, sem credenciais, query ou fragmento.",
        )
    };
    let url = reqwest::Url::parse(raw).map_err(|_| invalid())?;
    let authority = raw
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or_default());
    let path: Vec<_> = url.path().split('/').collect();
    if raw.trim() != raw
        || raw.contains('\\')
        || raw.chars().any(char::is_control)
        || authority.is_none_or(|host| host.is_empty() || host.contains('@'))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || url.scheme() == "http" && loopback_url(&url))
        || path.windows(2).any(|pair| pair == ["chat", "completions"])
    {
        return Err(invalid());
    }
    Ok(url)
}

fn mobile_url(raw: &str, allow_loopback: bool) -> Result<reqwest::Url> {
    // Reuse the raw checks before URL normalization, including duplicate chat/completions paths.
    let url = desktop_url(raw).map_err(|_| config_error())?;
    if loopback_url(&url) && !allow_loopback
        // Anthropic's native Messages API is not an OpenAI chat-completions API.
        || url.host_str() == Some("api.anthropic.com")
        || url.path().split('/').any(|part| part == "messages")
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
                || p.max_concurrent.is_some_and(|n| n > 32)
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
                    max_concurrent: p.max_concurrent,
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
            discovered: None,
            preflight_cache: PreflightCache::default(),
        }
    }
    pub fn runtime(&self) -> Result<Option<Arc<Providers>>> {
        self.runtime_with_discovered(None)
    }
    /// Ephemeral discovery never changes saved defaults, aliases, keys or providers.
    pub(crate) fn runtime_with_discovered(&self, discovered: Option<Provider>) -> Result<Option<Arc<Providers>>> {
        self.validate()?;
        let mut runtime = self.runtime_unchecked();
        if let Some(provider) = discovered
            && !runtime.list.iter().any(|p| p.id == provider.id)
        {
            runtime.discovered = Some(provider.id.clone());
            runtime.list.push(provider);
        }
        Ok((!runtime.list.is_empty()).then(|| Arc::new(runtime)))
    }
    pub fn redacted(&self) -> Value {
        serde_json::json!({"default":self.default,"providers":self.providers.iter().map(|p| {
            let mut item = serde_json::json!({
                "id":p.id,"baseUrl":p.base_url,"models":p.models,"hasApiKey":p.api_key.is_some(),
                "allowLoopback":p.allow_loopback,"protocol":"openai-compatible","reasoningFormat":p.reasoning_format,
                "contextWindow":p.context_window,"maxOutput":p.max_output
            });
            if let Some(mc) = p.max_concurrent {
                item["maxConcurrent"] = serde_json::json!(mc);
            }
            item
        }).collect::<Vec<_>>(),"aliases":self.aliases,"fallbacks":self.fallbacks})
    }
}

const PREFLIGHT_TTL: Duration = Duration::from_secs(5);
const PREFLIGHT_CACHE_MAX: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PreflightKey {
    base_url: String,
    key_hash: Option<String>,
    model: String,
}

#[derive(Clone, Copy, Debug)]
struct PreflightEntry {
    at: Instant,
}

#[derive(Clone, Default, Debug)]
pub struct PreflightCache {
    entries: Arc<RwLock<HashMap<PreflightKey, PreflightEntry>>>,
    epoch: Arc<AtomicU64>,
}

impl PreflightCache {
    pub fn clear(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut guard) = self.entries.write() {
            guard.clear();
        }
    }

    fn key_for(provider: &Provider, model: &str) -> PreflightKey {
        PreflightKey {
            base_url: normalize_preflight_base_url(&provider.base_url),
            key_hash: provider.api_key.as_deref().map(crate::rules::hash),
            model: model.to_ascii_lowercase(),
        }
    }

    fn is_hit(&self, key: &PreflightKey) -> bool {
        let Ok(guard) = self.entries.read() else {
            return false;
        };
        guard
            .get(key)
            .is_some_and(|entry| entry.at.elapsed() < PREFLIGHT_TTL)
    }

    fn record_hit(&self, key: PreflightKey, observed_epoch: u64) {
        if self.epoch.load(Ordering::Acquire) != observed_epoch {
            return;
        }
        let Ok(mut guard) = self.entries.write() else {
            return;
        };
        if self.epoch.load(Ordering::Acquire) != observed_epoch {
            return;
        }
        guard.retain(|_, v| v.at.elapsed() < PREFLIGHT_TTL);
        if guard.len() >= PREFLIGHT_CACHE_MAX
            && let Some(oldest_key) = guard
                .iter()
                .min_by_key(|(_, v)| v.at)
                .map(|(k, _)| k.clone())
        {
            guard.remove(&oldest_key);
        }
        guard.insert(key, PreflightEntry { at: Instant::now() });
    }
}

fn clamp_concurrency_limit(limit: usize) -> usize {
    if limit == 0 {
        0
    } else {
        limit.clamp(1, 32)
    }
}

pub struct OutboundPermit {
    gate: Arc<OutboundGate>,
}

impl Drop for OutboundPermit {
    fn drop(&mut self) {
        {
            let mut state = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
            state.in_flight = state.in_flight.saturating_sub(1);
        }
        self.gate.notify.notify_waiters();
    }
}

struct QueueGuard {
    gate: Arc<OutboundGate>,
    ticket: Option<u64>,
}

impl QueueGuard {
    fn disarm(&mut self) {
        self.ticket = None;
    }
}

impl Drop for QueueGuard {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            {
                let mut state = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
                state.queue.retain(|&t| t != ticket);
            }
            self.gate.notify.notify_waiters();
        }
    }
}

struct OutboundGateState {
    in_flight: usize,
    limit: usize,
    next_ticket: u64,
    queue: VecDeque<u64>,
}

struct OutboundGate {
    state: Mutex<OutboundGateState>,
    notify: tokio::sync::Notify,
}

impl OutboundGate {
    fn new(limit: usize) -> Self {
        Self {
            state: Mutex::new(OutboundGateState {
                in_flight: 0,
                limit,
                next_ticket: 0,
                queue: VecDeque::new(),
            }),
            notify: tokio::sync::Notify::new(),
        }
    }

    fn set_limit(&self, new_limit: usize) {
        let should_notify = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let old_limit = state.limit;
            if old_limit == new_limit {
                false
            } else {
                state.limit = new_limit;
                !state.queue.is_empty() && (new_limit == 0 || new_limit > old_limit)
            }
        };
        if should_notify {
            self.notify.notify_waiters();
        }
    }

    async fn acquire(self: &Arc<Self>, wait_timeout: Duration) -> Result<OutboundPermit> {
        let mut guard = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.limit == 0 || (state.queue.is_empty() && state.in_flight < state.limit) {
                state.in_flight += 1;
                return Ok(OutboundPermit { gate: self.clone() });
            }
            let ticket = state.next_ticket;
            state.next_ticket += 1;
            state.queue.push_back(ticket);
            QueueGuard {
                gate: self.clone(),
                ticket: Some(ticket),
            }
        };
        let ticket = guard.ticket.expect("ticket assigned");
        tokio::time::timeout(wait_timeout, async move {
            loop {
                let notified = self.notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                {
                    let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    let can_enter = if state.limit == 0 {
                        state.queue.retain(|&t| t != ticket);
                        true
                    } else if state.queue.front() == Some(&ticket) && state.in_flight < state.limit {
                        state.queue.pop_front();
                        true
                    } else {
                        false
                    };
                    if can_enter {
                        state.in_flight += 1;
                        let wake_next = !state.queue.is_empty()
                            && (state.limit == 0 || state.in_flight < state.limit);
                        guard.disarm();
                        drop(state);
                        if wake_next {
                            self.notify.notify_waiters();
                        }
                        return OutboundPermit { gate: self.clone() };
                    }
                }
                notified.as_mut().await;
            }
        })
        .await
        .map_err(|_| {
            Error::new(504, "Fila do provedor excedeu o tempo limite.")
                .extra("code", "UPSTREAM_QUEUE_TIMEOUT")
        })
    }
}

/// Every operation holds its own Arc snapshot; changes never reroute an accepted POST.
#[derive(Clone)]
pub struct RuntimeProviders {
    providers: Arc<RwLock<Option<Arc<Providers>>>>,
    format_cache: Arc<RwLock<HashMap<String, u8>>>,
    outbound_gates: Arc<RwLock<HashMap<String, Arc<OutboundGate>>>>,
    preflight_cache: PreflightCache,
    default_max_concurrent: usize,
}
impl Default for RuntimeProviders {
    fn default() -> Self {
        Self::with_default_concurrency(None, 1)
    }
}
impl RuntimeProviders {
    pub fn new(providers: Option<Arc<Providers>>) -> Self {
        Self::with_default_concurrency(providers, upstream_max_concurrent_from_env())
    }
    pub fn new_for_android(providers: Option<Arc<Providers>>) -> Self {
        Self::with_default_concurrency(providers, 1)
    }
    pub fn with_default_concurrency(
        providers: Option<Arc<Providers>>,
        default_max_concurrent: usize,
    ) -> Self {
        let rt = Self {
            providers: Arc::new(RwLock::new(providers)),
            format_cache: Arc::new(RwLock::new(HashMap::new())),
            outbound_gates: Arc::new(RwLock::new(HashMap::new())),
            preflight_cache: PreflightCache::default(),
            default_max_concurrent: clamp_concurrency_limit(default_max_concurrent),
        };
        let guard = rt.providers.read().unwrap();
        rt.reconcile_gates_locked(guard.as_deref());
        drop(guard);
        rt
    }
    pub fn snapshot(&self) -> Option<Arc<Providers>> {
        self.providers.read().unwrap().clone()
    }
    pub fn set(&self, providers: Option<Arc<Providers>>) {
        let mut guard = self.providers.write().unwrap();
        *guard = providers;
        self.preflight_cache.clear();
        self.reconcile_gates_locked(guard.as_deref());
    }
    fn effective_limit_from_snapshot(
        snapshot: Option<&Providers>,
        default_max_concurrent: usize,
        normalized_key: &str,
        provider_limit: Option<usize>,
    ) -> (usize, bool) {
        let mut min_positive: Option<usize> = None;
        let mut found_in_snapshot = false;
        if let Some(snap) = snapshot {
            for p in &snap.list {
                if normalize_base_url(&p.base_url) == normalized_key {
                    found_in_snapshot = true;
                    let resolved = clamp_concurrency_limit(
                        p.max_concurrent.unwrap_or(default_max_concurrent),
                    );
                    if resolved > 0 {
                        min_positive = Some(min_positive.map_or(resolved, |m| m.min(resolved)));
                    }
                }
            }
        }
        if found_in_snapshot {
            (min_positive.unwrap_or(0), true)
        } else {
            (
                clamp_concurrency_limit(provider_limit.unwrap_or(default_max_concurrent)),
                false,
            )
        }
    }
    fn reconcile_gates_locked(&self, snapshot: Option<&Providers>) {
        let Some(snapshot) = snapshot else {
            return;
        };
        let mut new_limits = HashMap::new();
        for p in &snapshot.list {
            let key = normalize_base_url(&p.base_url);
            new_limits.entry(key.clone()).or_insert_with(|| {
                Self::effective_limit_from_snapshot(
                    Some(snapshot),
                    self.default_max_concurrent,
                    &key,
                    None,
                )
                .0
            });
        }
        let mut gates = self.outbound_gates.write().unwrap();
        for (key, limit_new) in new_limits {
            if let Some(existing) = gates.get(&key) {
                existing.set_limit(limit_new);
            } else {
                gates.insert(key, Arc::new(OutboundGate::new(limit_new)));
            }
        }
    }
    pub async fn acquire_outbound(
        &self,
        provider: &Provider,
        wait_timeout: Duration,
    ) -> Result<Option<OutboundPermit>> {
        let key = normalize_base_url(&provider.base_url);
        let gate = {
            let providers_guard = self.providers.read().unwrap();
            let (limit, in_snapshot) = Self::effective_limit_from_snapshot(
                providers_guard.as_deref(),
                self.default_max_concurrent,
                &key,
                provider.max_concurrent,
            );
            let read_gates = self.outbound_gates.read().unwrap();
            if let Some(existing) = read_gates.get(&key) {
                let gate = existing.clone();
                drop(read_gates);
                if !in_snapshot {
                    gate.set_limit(limit);
                }
                gate
            } else {
                drop(read_gates);
                let mut write_gates = self.outbound_gates.write().unwrap();
                let gate = write_gates
                    .entry(key)
                    .or_insert_with(|| Arc::new(OutboundGate::new(limit)))
                    .clone();
                gate.set_limit(limit);
                gate
            }
        };
        let permit = gate.acquire(wait_timeout).await?;
        Ok(Some(permit))
    }
    pub async fn select(&self, snapshot: &Providers, route: &str) -> SelectedBackend {
        snapshot
            .select_with_cache(route, Some(&self.preflight_cache))
            .await
    }
    pub async fn preflight(&self, provider: &Provider, model: &str) -> bool {
        provider
            .preflight_cached(&self.preflight_cache, model, Duration::from_millis(3000))
            .await
    }
    pub fn get_format(&self, provider: &str, model: &str) -> Option<u8> {
        self.format_cache.read().ok()?.get(&format!("{provider}:{model}")).copied()
    }
    pub fn set_format(&self, provider: &str, model: &str, format: u8) {
        if let Ok(mut cache) = self.format_cache.write() {
            cache.insert(format!("{provider}:{model}"), format);
        }
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
                serde_json::json!({"requestedRoute":requested,"effectiveRoute":"mobile-local","backend":"local","model":"Nemotron-3.5-Bolt-E4B-4GB","originalRoutePreserved":true})
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
    /// This provider requires a full mobile-api route; legacy names never opt in.
    discovered: Option<String>,
    preflight_cache: PreflightCache,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn read_local_9router_key() -> Option<String> {
    let appdata = std::env::var_os("APPDATA")?;
    let db_path = PathBuf::from(appdata).join("9router/db/data.sqlite");
    if !db_path.is_file() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    conn.query_row(
        "SELECT key FROM apiKeys WHERE isActive=1 LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
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
        desktop_url(&url)?;
        Ok(Some(Self {
            list: vec![Provider {
                id: "default".into(),
                base_url: url.trim_end_matches('/').to_owned(),
                api_key: env("MC_UPSTREAM_API_KEY")
                    .or_else(|| env("NINEROUTER_API_KEY"))
                    .or_else(|| env("NINEROUTER_KEY"))
                    .or_else(read_local_9router_key),
                models: Vec::new(),
                request_log: env("NINEROUTER_DB").map(PathBuf::from),
                reasoning_format: ReasoningFormat::Omit,
                context_window: 128000,
                max_output: 8192,
                max_concurrent: None,
            }],
            default: 0,
            aliases: BTreeMap::new(),
            fallbacks: BTreeMap::new(),
            mobile: false,
            discovered: None,
            preflight_cache: PreflightCache::default(),
        }))
    }

    pub fn from_json(value: &Value) -> Result<Self> {
        let mut list = Vec::new();
        for p in value["providers"].as_array().into_iter().flatten() {
            let id = p["id"].as_str().unwrap_or_default().trim();
            if id.is_empty() || id.contains(':') || list.iter().any(|x: &Provider| x.id == id) {
                return Err(Error::new(
                    400,
                    "Invalid or duplicated provider id.",
                ));
            }
            let base_url = p["baseUrl"]
                .as_str()
                .unwrap_or_default()
                .trim_end_matches('/');
            desktop_url(base_url)?;
            let api_key = p["apiKey"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| p["apiKeyEnv"].as_str().and_then(env));
            let max_concurrent = match p.get("maxConcurrent") {
                None | Some(Value::Null) => None,
                Some(Value::Number(num)) => {
                    let Some(n) = num.as_u64() else {
                        return Err(Error::new(400, "Invalid maxConcurrent in providers file."));
                    };
                    if n == 0 {
                        Some(0)
                    } else {
                        Some(n.clamp(1, 32) as usize)
                    }
                }
                Some(_) => {
                    return Err(Error::new(400, "Invalid maxConcurrent in providers file."));
                }
            };
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
                max_concurrent,
            });
        }
        if list.is_empty() {
            return Err(Error::new(400, "The providers file lists no provider."));
        }
        let default = match value["default"].as_str() {
            Some(id) => list
                .iter()
                .position(|p| p.id == id)
                .ok_or_else(|| Error::new(400, "Unknown default provider."))?,
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
            discovered: None,
            preflight_cache: PreflightCache::default(),
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
        let explicit = route.strip_prefix("mobile-api:");
        if let Some((provider, model)) = explicit.unwrap_or(route).split_once(':') {
            if explicit.is_none() && self.discovered.as_deref() == Some(provider) {
                return None;
            }
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
        if self.discovered.as_deref() == Some(provider.id.as_str()) {
            return None;
        }
        provider
            .models
            .iter()
            .any(|m| m == route)
            .then(|| (provider.clone(), route.to_owned()))
    }
    pub fn model_ids(&self) -> Vec<String> {
        self.list.iter().flat_map(|p| p.models.iter().cloned()).collect()
    }
    pub async fn select(&self, route: &str) -> SelectedBackend {
        self.select_with_cache(route, None).await
    }
    pub async fn select_with_cache(
        &self,
        route: &str,
        cache: Option<&PreflightCache>,
    ) -> SelectedBackend {
        let target = if self.mobile {
            self.mobile_resolve(route)
        } else {
            let (p, m) = self.resolve(route);
            Some((p.clone(), m))
        };
        let Some((provider, model)) = target else {
            return SelectedBackend::Local;
        };
        let timeout = if self.mobile {
            Duration::from_millis(400)
        } else {
            Duration::from_millis(3000)
        };
        let cache = cache.unwrap_or(&self.preflight_cache);
        if provider.is_loopback()
            && (self.mobile || self.fallbacks.contains_key(route))
            && !provider.preflight_cached(cache, &model, timeout).await
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
        self.preflight_with_timeout(model, Duration::from_millis(3000))
            .await
    }
    pub async fn preflight_cached(
        &self,
        cache: &PreflightCache,
        model: &str,
        timeout: Duration,
    ) -> bool {
        if desktop_url(&self.base_url).is_err() {
            return false;
        }
        let key = PreflightCache::key_for(self, model);
        if cache.is_hit(&key) {
            return true;
        }
        let epoch = cache.epoch.load(Ordering::Acquire);
        let ok = self.preflight_with_timeout(model, timeout).await;
        if ok {
            cache.record_hit(key, epoch);
        }
        ok
    }
    pub async fn preflight_with_timeout(&self, model: &str, timeout: Duration) -> bool {
        if desktop_url(&self.base_url).is_err() {
            return false;
        }
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
        tokio::time::timeout(timeout, async {
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
                if bytes.len() + chunk.len() > 1024 * 1024 {
                    return false;
                }
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice::<Value>(&bytes)
                .ok()
                .is_some_and(|body| {
                    body["data"].as_array().is_some_and(|models| {
                        models.iter().any(|m| {
                            m["id"]
                                .as_str()
                                .is_some_and(|id| id.eq_ignore_ascii_case(model))
                        })
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
        let base = desktop_url(&self.base_url).ok();
        let mut body = body.clone();
        body["model"] = Value::String(model.to_owned());
        // Explicit breakpoints are an OpenRouter extension. Other compatible
        // endpoints (including 9Router) retain their existing wire contract.
        if base.as_ref().is_some_and(|url| url.host_str() == Some("openrouter.ai"))
            && let Some(system) = body["messages"].as_array_mut().and_then(|messages| messages.first_mut())
            && system["role"] == "system"
            && let Some(text) = system["content"].as_str()
        {
            system["content"] = serde_json::json!([{"type":"text","text":text,"cache_control":{"type":"ephemeral"}}]);
        }
        if let Some(obj) = body.as_object_mut() {
            // Strip safetySettings, safety_settings, and unrestrictedSafety so upstream OpenAI-compatible endpoints never fail with invalid_request_error
            obj.remove("unrestrictedSafety");
            obj.remove("safetySettings");
            obj.remove("safety_settings");
        }
        // Preserve the RequestBuilder API: an invalid base produces a sanitized build
        // error (empty URL) before send, including for directly constructed Providers.
        let endpoint = base.map(|_| format!("{}/chat/completions", self.base_url.trim_end_matches('/'))).unwrap_or_default();
        let mut request = client
            .post(endpoint)
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
    desktop_url(&p.base_url)?;
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
    let data = crate::bounded_http::json(response, crate::bounded_http::JSON_LIMIT)
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
    provider.chat_request(client, &model, body, timeout)
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
                &json!({"default":"zz","providers":[{"id":"a","baseUrl":"https://x"}]})
            )
            .is_err()
        );
    }

    #[test]
    fn mobile_urls_reject_raw_normalization_credentials_spoofs_and_native_endpoints() {
        for raw in [
            " https://provider.example/v1",
            "https://provider.example/v1 ",
            "https://provider.example/\nSECRET",
            "https://provider.\texample/v1",
            "https://provider.example/\0v1",
            "https:provider.example/v1",
            "https:///v1",
            "https://",
            "https://@provider.example/v1",
            "https://user@provider.example/v1",
            "https://user:SECRET@provider.example/v1",
            "https://provider.example\\@evil.example/v1",
            "https://provider.example/v1?",
            "https://provider.example/v1#",
            "https://provider.example/v1?key=SECRET",
            "https://provider.example/v1#SECRET",
            "http://localhost.evil/v1",
            "http://127.0.0.1.evil/v1",
            "http://[::ffff:192.168.1.2]/v1",
            "http://[::ffff:8.8.8.8]/v1",
            "http://192.168.1.2/v1",
            "http://provider.example/v1",
            "ftp://provider.example/v1",
            "https://provider.example/v1/chat/completions",
            "https://provider.example/chat/completions/v1/",
            "https://api.anthropic.com/v1",
            "https://provider.example/v1/messages/",
            "https://provider.example/v1/messages/v1/",
        ] {
            let error = MobileProviders::parse(&json!({"providers":[{
                "id":"fixture", "baseUrl":raw, "apiKey":"KEY_SECRET",
                "models":["org/model"], "allowLoopback":true
            }]}))
            .unwrap_err();
            assert_eq!(error.status, 400, "{raw:?}");
            assert!(!format!("{error:?}").contains("SECRET"));
        }
    }

    #[test]
    fn mobile_urls_keep_https_and_require_a_boolean_loopback_opt_in() {
        for raw in [
            "https://openrouter.ai/api/v1",
            "https://api.openai.com/v1",
            "https://provider.example/api/v1/",
            "https://localhost.evil/v1",
            "https://192.168.1.2/v1",
            "https://[2001:db8::1]/v1",
            "http://localhost:12345/v1",
            "http://127.12.34.56/v1/",
            "http://[::1]:12345/v1",
            "http://[::ffff:127.0.0.1]:12345/v1",
            "https://localhost/v1",
            "https://[::ffff:127.0.0.1]/v1",
        ] {
            let mut value = json!({"providers":[{
                "id":"fixture", "baseUrl":raw, "models":["org/model"], "allowLoopback":true
            }]});
            assert!(MobileProviders::parse(&value).is_ok(), "{raw}");
            let is_loopback = loopback_url(&reqwest::Url::parse(raw).unwrap());
            value["providers"][0]["allowLoopback"] = json!(false);
            assert_eq!(
                MobileProviders::parse(&value).is_ok(),
                !is_loopback,
                "{raw}"
            );
            value["providers"][0]
                .as_object_mut()
                .unwrap()
                .remove("allowLoopback");
            assert_eq!(
                MobileProviders::parse(&value).is_ok(),
                !is_loopback,
                "{raw}"
            );
        }
        for flag in [
            json!("true"),
            json!("false"),
            json!(1),
            json!(0),
            Value::Null,
            json!([]),
        ] {
            let value = json!({"providers":[{
                "id":"fixture", "baseUrl":"http://127.0.0.1/v1", "models":["org/model"],
                "allowLoopback":flag
            }]});
            assert!(MobileProviders::parse(&value).is_err());
        }
    }

    #[test]
    fn desktop_urls_reject_credentials_remote_http_spoofs_and_duplicate_endpoints() {
        for raw in [
            "http://example.com/v1",
            "http://localhost.evil/v1",
            "http://127.0.0.1.evil/v1",
            "http://[::ffff:192.168.1.2]/v1",
            "http://[::ffff:8.8.8.8]/v1",
            "https://user:SECRET@host.example/v1",
            "https://user@host.example/v1",
            "https://@host.example/v1",
            "https://host.example/v1?key=SECRET",
            "https://host.example/v1#SECRET",
            "https://host.example\\@evil/v1",
            "https://host.example/\nSECRET",
            " https://host.example/v1",
            "https:host.example/v1",
            "https:///v1",
            "https://",
            "ftp://host.example/v1",
            "https://host.example/v1/chat/completions",
            "https://host.example/chat/completions/v1/",
        ] {
            let error = Providers::from_json(
                &json!({"providers":[{"id":"fixture","baseUrl":raw,"apiKey":"KEY_SECRET"}]}),
            )
            .unwrap_err();
            assert_eq!(error.status, 400, "{raw}");
            assert!(!format!("{error:?}").contains("SECRET"));
        }
        for raw in [
            "http://localhost:12345/v1",
            "http://127.0.0.1/v1/",
            "http://127.12.34.56/v1",
            "http://[::1]:12345/v1",
            "http://[::ffff:127.0.0.1]:12345/v1",
            "https://provider.example/api/v1",
            "https://192.168.1.2/v1",
            "https://[2001:db8::1]/v1",
        ] {
            let providers =
                Providers::from_json(&json!({"providers":[{"id":"fixture","baseUrl":raw}]})).unwrap();
            assert_eq!(
                providers.resolve("org/model").0.base_url,
                raw.trim_end_matches('/')
            );
        }
    }

    #[tokio::test]
    async fn invalid_direct_provider_never_builds_a_request_with_credentials() {
        let providers = Providers::from_json(
            &json!({"providers":[{"id":"fixture","baseUrl":"https://safe.example/v1"}]}),
        )
        .unwrap();
        let mut provider = providers.resolve("model").0.clone();
        provider.base_url = "https://user:SECRET@127.0.0.1:1/v1".into();
        let error = provider
            .chat_request(
                &reqwest::Client::new(),
                "model",
                &json!({}),
                Duration::from_secs(1),
            )
            .send()
            .await
            .unwrap_err();
        assert!(error.is_builder());
        assert!(!error.to_string().contains("SECRET"));
        assert!(!provider.preflight("model").await);
    }

    #[tokio::test]
    async fn model_list_accepts_json_and_bounds_declared_and_chunked_bodies() {
        use crate::bounded_http::{
            JSON_LIMIT,
            tests::{chunked_oversize, serve},
        };
        let normal = br#"{"data":[{"id":"org/model"}]}"#.to_vec();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for (headers, body, ok) in [
            (
                format!("Content-Length: {}\r\n", normal.len()),
                normal,
                true,
            ),
            (
                format!("Content-Length: {}\r\n", JSON_LIMIT + 1),
                vec![],
                false,
            ),
            (
                "Transfer-Encoding: chunked\r\n".into(),
                chunked_oversize(JSON_LIMIT),
                false,
            ),
        ] {
            let fixture = serve(&headers, body).await;
            let providers =
                Providers::from_json(&json!({"providers":[{"id":"fixture","baseUrl":fixture.url}]}))
                    .unwrap();
            let (provider, _) = providers.resolve("org/model");
            let result = tokio::time::timeout(Duration::from_secs(2), fetch_models(&client, &provider))
                .await
                .unwrap();
            if ok {
                assert_eq!(result.unwrap(), ["org/model"]);
            } else {
                assert_eq!(result.unwrap_err().status, 502);
            }
        }
    }

    #[test]
    fn environment_loading_validates_urls_in_an_isolated_process() {
        let dir = tempfile::tempdir().unwrap();
        for (raw, expected) in [
            ("http://remote.example/v1", "invalid"),
            ("https://u:SECRET@remote.example/v1", "invalid"),
            ("http://[::ffff:127.0.0.1]:12345/v1", "valid"),
        ] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "providers::tests::environment_url_fixture",
                    "--ignored",
                ])
                .env_clear()
                .env("MC_UPSTREAM_URL", raw)
                .env("MC_UPSTREAM_API_KEY", "KEY_SECRET")
                .env("MC_TEST_DATA_DIR", dir.path())
                .env("MC_TEST_EXPECTED", expected);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                child.creation_flags(0x08000000);
            }
            let output = child.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }

    #[test]
    #[ignore = "isolated environment subprocess fixture only"]
    fn environment_url_fixture() {
        let dir = std::env::var_os("MC_TEST_DATA_DIR").unwrap();
        let loaded = Providers::load(Path::new(&dir), None);
        if std::env::var("MC_TEST_EXPECTED").unwrap() == "valid" {
            assert!(loaded.unwrap().is_some());
        } else {
            let error = loaded.unwrap_err();
            assert_eq!(error.status, 400);
            assert!(!format!("{error:?}").contains("SECRET"));
        }
    }

    #[test]
    fn effective_limit_from_snapshot_resolves_omitted_and_zero_correctly() {
        let p_zero_and_omitted = Providers::from_json(&json!({
            "providers": [
                { "id": "p1", "baseUrl": "http://127.0.0.1:20128/v1", "maxConcurrent": 0 },
                { "id": "p2", "baseUrl": "http://127.0.0.1:20128/v1/" }
            ]
        }))
        .unwrap();
        assert_eq!(
            RuntimeProviders::effective_limit_from_snapshot(
                Some(&p_zero_and_omitted),
                1,
                "http://127.0.0.1:20128/v1",
                None
            ),
            (1, true)
        );
        assert_eq!(
            RuntimeProviders::effective_limit_from_snapshot(
                Some(&p_zero_and_omitted),
                4,
                "http://127.0.0.1:20128/v1",
                None
            ),
            (4, true)
        );

        let p_one_zero_omitted = Providers::from_json(&json!({
            "providers": [
                { "id": "p1", "baseUrl": "http://127.0.0.1:20128/v1", "maxConcurrent": 1 },
                { "id": "p2", "baseUrl": "http://127.0.0.1:20128/v1", "maxConcurrent": 0 },
                { "id": "p3", "baseUrl": "http://127.0.0.1:20128/v1" }
            ]
        }))
        .unwrap();
        assert_eq!(
            RuntimeProviders::effective_limit_from_snapshot(
                Some(&p_one_zero_omitted),
                4,
                "http://127.0.0.1:20128/v1",
                None
            ),
            (1, true)
        );

        let p_zero_only = Providers::from_json(&json!({
            "providers": [
                { "id": "p1", "baseUrl": "http://127.0.0.1:20128/v1", "maxConcurrent": 0 }
            ]
        }))
        .unwrap();
        assert_eq!(
            RuntimeProviders::effective_limit_from_snapshot(
                Some(&p_zero_only),
                1,
                "http://127.0.0.1:20128/v1",
                None
            ),
            (0, true)
        );
    }

    #[test]
    fn normalize_base_url_unifies_loopback_aliases_without_mutating_provider_base_url() {
        for raw in [
            "http://127.0.0.1:20128/v1/",
            "HTTP://LOCALHOST:20128/v1",
            "http://localhost:20128/v1/",
            "http://[::1]:20128/v1/",
            "http://[::ffff:127.0.0.1]:20128/v1",
        ] {
            assert_eq!(
                normalize_base_url(raw),
                "http://127.0.0.1:20128/v1",
                "failed for {raw}"
            );
        }
        assert_eq!(
            normalize_base_url("http://127.0.0.2:20128/v1/"),
            "http://127.0.0.2:20128/v1"
        );
        assert_eq!(
            normalize_base_url("https://localhost.evil/v1/"),
            "https://localhost.evil/v1"
        );
        let p = Providers::from_json(&json!({
            "providers": [{ "id": "local", "baseUrl": "http://localhost:20128/v1/" }]
        }))
        .unwrap();
        assert_eq!(p.default_provider().base_url, "http://localhost:20128/v1");
    }

    #[test]
    fn normalize_preflight_base_url_preserves_loopback_host_literals_and_canonicalizes_casing_ports() {
        let v4 = normalize_preflight_base_url("http://127.0.0.1:20128/v1/");
        let host = normalize_preflight_base_url("HTTP://LOCALHOST:20128/v1/");
        let v6 = normalize_preflight_base_url("http://[::1]:20128/v1/");
        let v4_mapped = normalize_preflight_base_url("http://[::ffff:127.0.0.1]:20128/v1/");

        assert_eq!(v4, "http://127.0.0.1:20128/v1");
        assert_eq!(host, "http://localhost:20128/v1");
        assert_eq!(v6, "http://[::1]:20128/v1");
        assert_eq!(v4_mapped, "http://[::ffff:7f00:1]:20128/v1");
        let unique: std::collections::HashSet<&str> =
            [v4.as_str(), host.as_str(), v6.as_str(), v4_mapped.as_str()]
                .into_iter()
                .collect();
        assert_eq!(unique.len(), 4);

        assert_eq!(
            normalize_preflight_base_url("HTTP://LOCALHOST:80/v1/"),
            "http://localhost/v1"
        );
        assert_eq!(
            normalize_preflight_base_url("https://OpenRouter.AI:443/api/v1/"),
            "https://openrouter.ai/api/v1"
        );
        assert_eq!(
            normalize_preflight_base_url("http://[0:0:0:0:0:0:0:1]:20128/v1/"),
            "http://[::1]:20128/v1"
        );
        assert_eq!(
            normalize_preflight_base_url("http://[::FFFF:127.0.0.1]:20128/v1/"),
            v4_mapped
        );
        assert_ne!(
            normalize_preflight_base_url("http://127.0.0.1:20128/V1"),
            "http://127.0.0.1:20128/v1"
        );

        let make_provider = |id: &str, base_url: &str| Provider {
            id: id.into(),
            base_url: base_url.into(),
            api_key: Some("k".into()),
            models: vec![],
            request_log: None,
            reasoning_format: ReasoningFormat::Omit,
            context_window: 128000,
            max_output: 8192,
            max_concurrent: Some(1),
        };
        let keys: std::collections::HashSet<PreflightKey> = [
            make_provider("v4", "http://127.0.0.1:20128/v1"),
            make_provider("host", "http://localhost:20128/v1"),
            make_provider("v6", "http://[::1]:20128/v1"),
            make_provider("v4m", "http://[::ffff:127.0.0.1]:20128/v1"),
        ]
        .iter()
        .map(|p| PreflightCache::key_for(p, "custom/model"))
        .collect();
        assert_eq!(keys.len(), 4);
    }

}
