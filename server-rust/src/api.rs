use crate::{
    Error, Result,
    auth::{self, Auth},
    config::Config,
    generation::{Locks, Manager},
    jobs::Runner,
    local::LocalModels,
    presets, rules, s,
    store::{Request, Store, TurnRequest},
};
use axum::{
    Router,
    body::Bytes,
    extract::{ConnectInfo, DefaultBodyLimit, OriginalUri, State},
    http::{HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{any, post},
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::{Notify, broadcast};
use tower_http::services::{ServeDir, ServeFile};

/// Route ids and when they were fetched.
type ModelsCache = Arc<std::sync::Mutex<Option<(std::time::Instant, Vec<String>)>>>;

async fn bootstrap_validation(path: &std::path::Path) -> Option<Value> {
    use tokio::io::AsyncReadExt;
    const LIMIT: usize = 1024 * 1024;
    let file = tokio::fs::File::open(path).await.ok()?;
    if file.metadata().await.ok()?.len() >= LIMIT as u64 {
        return None;
    }
    // Bound the read too: the file may grow after metadata, and invalid files stay optional.
    let mut reader = file.take(LIMIT as u64);
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).await.ok()?;
        if n == 0 {
            return serde_json::from_slice(&bytes).ok();
        }
        if bytes.len() + n >= LIMIT {
            return None;
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
}

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub config: Config,
    pub local: LocalModels,
    pub generation: Manager,
    pub runner: Runner,
    pub auth: Auth,
    pub events: broadcast::Sender<Value>,
    pub locks: Locks,
    pub validation: Value,
    pub providers: crate::providers::RuntimeProviders,
    pub policy: crate::memory_backend::PolicySnapshot,
    mobile_providers: Arc<tokio::sync::Mutex<crate::providers::MobileProviders>>,
    configuration_gate: Arc<tokio::sync::Mutex<()>>,
    pub(crate) discovery: Option<Arc<crate::local_discovery::Discovery>>,
    /// Route list as last fetched from the providers (see `models`).
    models_cache: ModelsCache,
    client: reqwest::Client,
    worker: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}
impl AppState {
    pub async fn new(config: Config) -> Result<Self> {
        let store = Store::open_for_config(&config)?;
        store.call(Request::RecoverInterrupted).await?;
        store.call(Request::RequeueJobs).await?;
        if !config.review {
            // No automatic review: releases turns blocked by old flags and the memories that
            // were waiting for review (nothing is deleted; the flags become "dismissed").
            store.call(Request::ReleaseLegacyReview).await?;
        }
        store.call(Request::ImportLegacy).await?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::new(500, "Cliente HTTP não iniciou."))?;
        let local = LocalModels::new(&config, client.clone())?;
        let mobile_providers = if config.is_android() {
            let value = store.call(Request::LoadMobileProviders).await?;
            if value.is_null() {
                crate::providers::MobileProviders::default()
            } else {
                crate::providers::MobileProviders::parse(&value)?
            }
        } else {
            crate::providers::MobileProviders::default()
        };
        let providers = crate::providers::RuntimeProviders::new(if config.is_android() {
            mobile_providers.runtime()?
        } else {
            config.providers.clone()
        });
        let memory_policy = if store.encrypted {
            match store.call(Request::LoadMobilePolicy).await {
                Ok(value) if !value.is_null() => {
                    crate::memory_backend::MemoryPolicy::parse(&value).unwrap_or_else(|_| {
                        crate::memory_backend::MemoryPolicy {
                            mode: crate::memory_backend::MemoryMode::Local,
                            api: None,
                        }
                    })
                }
                _ => {
                    let policy = if config.is_android() {
                        crate::memory_backend::MemoryPolicy::default()
                    } else {
                        crate::memory_backend::MemoryPolicy {
                            mode: crate::memory_backend::MemoryMode::Local,
                            api: None,
                        }
                    };
                    let _ = store.call(Request::SaveMobilePolicy(policy.clone())).await;
                    policy
                }
            }
        } else {
            crate::memory_backend::MemoryPolicy {
                mode: crate::memory_backend::MemoryMode::Local,
                api: None,
            }
        };
        let policy = Arc::new(std::sync::RwLock::new(memory_policy));
        let (events, _) = broadcast::channel(64);
        let jobs = Arc::new(Notify::new());
        let locks = Locks::default();
        let validation = bootstrap_validation(&config.data_dir.join("route-validation.json"))
            .await
            .unwrap_or(json!({"routes":{}}));
        let mut generation = Manager::new(
            store.clone(),
            local.clone(),
            client.clone(),
            config.clone(),
            events.clone(),
            jobs.clone(),
        );
        generation.locks = locks.clone();
        generation.providers = providers.clone();
        let mut runner = Runner::new(
            store.clone(),
            local.clone(),
            jobs,
            events.clone(),
            config.review,
        );
        runner.set_backends(providers.clone(), policy.clone(), client.clone());
        let auth = Auth::new(store.clone(), config.require_pairing || store.encrypted);
        let worker = Arc::new(tokio::sync::Mutex::new(Some(runner.start())));
        let discovery = config.is_android().then(|| Arc::new(crate::local_discovery::Discovery::new()));
        let state = Self {
            store,
            config,
            local,
            generation,
            runner,
            auth,
            events,
            locks,
            validation,
            providers,
            policy,
            mobile_providers: Arc::new(tokio::sync::Mutex::new(mobile_providers)),
            configuration_gate: Arc::new(tokio::sync::Mutex::new(())),
            discovery,
            models_cache: Arc::default(),
            client,
            worker,
        };
        state.runner.kick();
        Ok(state)
    }
    pub async fn shutdown(&self) {
        self.generation.shutdown().await;
        self.runner.stop();
        if let Some(mut worker) = self.worker.lock().await.take()
            && tokio::time::timeout(Duration::from_secs(10), &mut worker)
                .await
                .is_err()
        {
            worker.abort();
            let _ = worker.await;
        }
        let _ = self.local.stop().await;
        let _ = self.store.call(Request::Close).await;
    }
    /// Routes for `/presets` and `/models`: the providers are asked at most once per minute.
    async fn models(&self) -> Result<Vec<String>> {
        self.refresh_discovery().await?;
        const TTL: Duration = Duration::from_secs(60);
        if let Some((at, ids)) = self.models_cache.lock().unwrap().as_ref()
            && at.elapsed() < TTL
        {
            return Ok(ids.clone());
        }
        self.fresh_models().await
    }
    /// Always asks the providers (`/status` reports whether they answer right now) and refreshes the cache.
    async fn fresh_models(&self) -> Result<Vec<String>> {
        self.refresh_discovery().await?;
        let snapshot = self.providers.snapshot();
        let mut ids: Vec<String> = match snapshot {
            Some(providers) => match providers.models(&self.client).await {
                Ok(ids) => ids,
                Err(_) if self.config.is_android() => Vec::new(),
                Err(e) => return Err(e),
            },
            None => Vec::new(),
        }
        .into_iter()
        .filter(|id| presets::selectable_route(id))
        .collect();
        if self.config.is_android() {
            ids.push("mobile-local".into());
        }
        *self.models_cache.lock().unwrap() = Some((std::time::Instant::now(), ids.clone()));
        Ok(ids)
    }
    async fn local_status(&self) -> Result<Value> {
        let discovery = self.refresh_discovery().await?;
        let mut status = self.local.status().await;
        if let Some(discovery) = discovery {
            status["discovery"] = json!(discovery);
        }
        if !self.config.is_android() && !self.local.chat_available() {
            if let Some(model) = crate::memory_backend::MemoryBackend::desktop_upstream_model(
                &self.providers,
                &self.policy.read().unwrap(),
            ) {
                status["chat"]["ok"] = json!(true);
                status["chat"]["model"] = json!(format!("API ({})", model));
                status["chat"]["backend"] = json!("upstream");
                status["chat"]["reason"] =
                    json!("Usando modelo remoto/upstream configurado no provedor.");
            }
        }
        status["memoryPolicy"] = json!(self.policy.read().unwrap().clone());
        status["memoryConcurrency"] = json!(1);
        status["localRoute"] = json!("mobile-local");
        status["providerCount"] = json!(self.mobile_providers.lock().await.providers.len());
        Ok(status)
    }
    /// Demand-driven only. Serialize publication with saved edits so neither can overwrite the other.
    async fn refresh_discovery(&self) -> Result<Option<crate::local_discovery::Snapshot>> {
        if !self.config.is_android() {
            return Ok(None);
        }
        let Some(discovery) = &self.discovery else {
            return Ok(None);
        };
        let _gate = self.configuration_gate.lock().await;
        let (snapshot, changed) = discovery.snapshot().await;
        if changed {
            let saved = self.mobile_providers.lock().await;
            self.providers.set(saved.runtime_with_discovered(snapshot.provider())?);
            *self.models_cache.lock().unwrap() = None;
        }
        Ok(Some(snapshot))
    }
    async fn default_mobile_model(&self) -> String {
        let saved = self.mobile_providers.lock().await;
        // Discovery is never the default or consent for an old/bare route.
        saved.providers.iter()
            .find(|p| saved.default.as_ref() == Some(&p.id))
            .or_else(|| saved.providers.first())
            .and_then(|p| p.models.first().map(|m| format!("mobile-api:{}:{m}", p.id)))
            .unwrap_or_else(|| "mobile-local".into())
    }
    fn require_mobile(&self) -> Result<()> {
        if self.config.is_android() {
            Ok(())
        } else {
            Err(Error::new(
                409,
                "Configuração disponível apenas no host mobile.",
            ))
        }
    }
    async fn resolve_route(&self, body: &Value, current: Option<&Value>) -> Result<Value> {
        match presets::resolve(body, current) {
            Ok(route) => Ok(route),
            Err(original) => {
                let route = body["routeId"].as_str().or(body["model"].as_str()).unwrap_or_default();
                if presets::for_route(route)["id"] != "generic" {
                    return Err(original);
                }
                presets::resolve_available(body, current, &self.models().await?)
            }
        }
    }
    async fn update_providers(&self, body: &Value) -> Result<Value> {
        self.require_mobile()?;
        let _gate = self.configuration_gate.lock().await;
        let mut next = crate::providers::MobileProviders::parse(body)?;
        let mut old = self.mobile_providers.lock().await;
        // Omission preserves a key only for the SAME endpoint. Null explicitly removes it.
        for p in &mut next.providers {
            let key_omitted = body["providers"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|v| v["id"] == p.id)
                .is_some_and(|v| v.get("apiKey").is_none());
            if key_omitted
                && let Some(previous) = old.providers.iter().find(|v| {
                    v.id == p.id
                        && v.base_url.trim_end_matches('/') == p.base_url.trim_end_matches('/')
                })
            {
                p.api_key = previous.api_key.clone();
            }
        }
        let discovered = match &self.discovery {
            Some(discovery) => discovery.cached_provider().await,
            None => None,
        };
        let runtime = next.runtime_with_discovered(discovered)?;
        self.store
            .call(Request::SaveMobileProviders(next.clone()))
            .await?;
        self.providers.set(runtime);
        *old = next;
        *self.models_cache.lock().unwrap() = None;
        self.runner.kick();
        Ok(old.redacted())
    }
    async fn update_policy(&self, body: &Value) -> Result<Value> {
        let _gate = self.configuration_gate.lock().await;
        let policy = crate::memory_backend::MemoryPolicy::parse(body)?;
        policy.check_destination(&self.providers)?;
        if self.store.encrypted {
            self.store
                .call(Request::SaveMobilePolicy(policy.clone()))
                .await?;
        }
        *self.policy.write().unwrap() = policy.clone();
        self.runner.kick();
        Ok(json!({"memoryPolicy":policy,"concurrency":1}))
    }
    fn changed(&self, c: &str, kind: &str, value: &Value) {
        let _ = self
            .events
            .send(json!({"convId":c,"type":kind,"revision":value["revision"]}));
        self.runner.kick();
    }
}
pub fn router(state: AppState) -> Router {
    let static_service = ServeDir::new(&state.config.dist)
        .not_found_service(ServeFile::new(state.config.dist.join("index.html")));
    Router::new()
        .route(
            "/api/import",
            post(dispatch).layer(DefaultBodyLimit::max(crate::store::IMPORT_LIMIT)),
        )
        .route(
            "/api/sync/delta",
            any(dispatch).layer(DefaultBodyLimit::max(crate::store::sync::MAX_BYTES)),
        )
        .route("/api/{*path}", any(dispatch))
        .fallback_service(static_service)
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}
async fn guard(
    State(state): State<AppState>,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|v| v.0);
    let Some(peer) = peer else {
        return Error::new(403, "Endereço do cliente indisponível.").into_response();
    };
    if !auth::private(peer.ip()) {
        return Error::new(403, "Acesso permitido apenas pela rede local.").into_response();
    }
    let origin = request
        .headers()
        .get("origin")
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned);
    if let Some(origin) = &origin {
        let host = request
            .headers()
            .get("host")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default();
        let parsed = reqwest::Url::parse(origin);
        let allowed = matches!(
            origin.as_str(),
            "capacitor://localhost" | "http://localhost" | "https://localhost"
        ) || parsed.is_ok_and(|u| {
            u.host_str().is_some_and(|h| {
                format!(
                    "{h}{}",
                    u.port().map(|p| format!(":{p}")).unwrap_or_default()
                ) == host
                    || auth::loopback(peer.ip()) && matches!(h, "localhost" | "127.0.0.1" | "[::1]")
            })
        });
        if !allowed {
            return Error::new(403, "Origem não permitida.").into_response();
        }
        if request.method() == Method::OPTIONS {
            return cors(StatusCode::NO_CONTENT.into_response(), origin);
        }
    }
    let path = request.uri().path();
    let public = matches!(path, "/api/health" | "/api/pair" | "/api/pair/info")
        || (path == "/api/pair/start" && auth::loopback(peer.ip()))
        || !path.starts_with("/api/");
    let local_control = path.starts_with("/api/local/");
    let sync_control = path.starts_with("/api/sync/");
    if !public
        && (local_control || sync_control
            || state.auth.required && (state.store.encrypted || !auth::loopback(peer.ip())))
    {
        let token = request
            .headers()
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|t| t.strip_prefix("Bearer "))
            .or_else(|| {
                request
                    .headers()
                    .get("cookie")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| {
                        v.split(';')
                            .find_map(|item| item.trim().strip_prefix("mc_session="))
                    })
            })
            .unwrap_or_default();
        match state.auth.verify(token).await {
            Ok(device) if !device.is_null() => {
                request.extensions_mut().insert(device);
            }
            Ok(_) => {
                let r = Error::new(401, "Aparelho não pareado.")
                    .extra("code", "UNPAIRED")
                    .into_response();
                return if let Some(origin) = origin {
                    cors(r, &origin)
                } else {
                    r
                };
            }
            Err(e) => return e.into_response(),
        }
    }
    let response = next.run(request).await;
    if let Some(origin) = origin {
        cors(response, &origin)
    } else {
        response
    }
}
fn cors(mut r: Response, origin: &str) -> Response {
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(origin) {
        h.insert("access-control-allow-origin", v);
    }
    for (k, v) in [
        ("vary", "Origin"),
        (
            "access-control-allow-headers",
            "Authorization, Content-Type",
        ),
        (
            "access-control-allow-methods",
            "GET,POST,PUT,PATCH,DELETE,OPTIONS",
        ),
        ("access-control-max-age", "600"),
        ("access-control-allow-credentials", "true"),
    ] {
        h.insert(k, HeaderValue::from_static(v));
    }
    r
}
fn response(value: Value, status: u16) -> Response {
    let mut result = (StatusCode::from_u16(status).unwrap(), axum::Json(value)).into_response();
    result
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    result
}
fn checked_id(id: &str) -> Result<String> {
    if id.len() < 20 || id.len() > 40 || !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(Error::new(400, "ID de chat inválido."));
    }
    Ok(id.into())
}
pub async fn write_character(store: &Store, config: &Config, c: &str, text: &str) -> Result<()> {
    checked_id(c)?;
    if store.encrypted {
        store
            .call(Request::WritePrivateCharacter {
                conv_id: c.into(),
                text: text.into(),
            })
            .await?;
        return Ok(());
    }
    let dir = config.data_dir.join("chats").join(c);
    tokio::fs::create_dir_all(&dir).await?;
    let resolved = dir.canonicalize()?;
    if !resolved.starts_with(config.data_dir.canonicalize()?) {
        return Err(Error::new(400, "Pasta da ficha fora do DATA_DIR."));
    }
    let file = dir.join("character.txt");
    if file.exists()
        && (file.symlink_metadata()?.file_type().is_symlink()
            || crate::config::hard_links(&file)? > 1)
    {
        return Err(Error::new(400, "Ficha não pode ser um link."));
    }
    let tmp = dir.join(format!(".character-{}.tmp", crate::id()));
    tokio::fs::write(&tmp, text).await?;
    if let Err(e) = tokio::fs::rename(&tmp, &file).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e.into());
    }
    Ok(())
}
pub async fn sync_character(store: &Store, config: &Config, c: &str) -> Result<bool> {
    checked_id(c)?;
    // Mirrored cards are canonical in the transactional Store. An older protected/plain
    // sidecar must never restore the pre-sync revision on the next generation.
    if !store.call(Request::SyncCharacterSource(c.into())).await?.is_null() {
        return Ok(false);
    }
    let conv = store.call(Request::GetMeta(c.to_owned())).await?;
    if store.encrypted {
        let value = store.call(Request::ReadPrivateCharacter(c.into())).await?;
        if value.is_null() {
            write_character(store, config, c, &s(&conv, "prompt")).await?;
            return Ok(false);
        }
        let text = value
            .as_str()
            .ok_or_else(|| Error::new(409, "Ficha protegida inválida."))?;
        if text == s(&conv, "prompt") {
            return Ok(false);
        }
        store
            .call(Request::SetCharacter {
                conv_id: c.into(),
                text: text.into(),
                source: "external".into(),
            })
            .await?;
        return Ok(true);
    }
    let file = config.data_dir.join("chats").join(c).join("character.txt");
    if !file.exists() {
        write_character(store, config, c, &s(&conv, "prompt")).await?;
        return Ok(false);
    }
    if file.metadata()?.len() > 800000 {
        return Err(Error::new(422, "Personalização grande demais."));
    }
    let text = tokio::fs::read_to_string(file).await?;
    if text == s(&conv, "prompt") {
        return Ok(false);
    }
    store
        .call(Request::SetCharacter {
            conv_id: c.to_owned(),
            text,
            source: "file".into(),
        })
        .await?;
    store
        .call(Request::EnqueueJob(json!({"convId":c,"kind":"cast"})))
        .await?;
    Ok(true)
}
async fn persona(state: &AppState, body: &Value) -> Result<Value> {
    let list = state.store.call(Request::Personas).await?;
    if let Some(id) = body["personaId"].as_str().filter(|s| !s.is_empty()) {
        return list
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["id"] == id)
            .cloned()
            .ok_or_else(|| Error::new(404, "Perfil de persona não encontrado."));
    }
    if let Some(name) = body["persona"].as_str().filter(|s| !s.trim().is_empty()) {
        return Ok(json!({"name":rules::clip(name.trim(),60),"description":""}));
    }
    Ok(list
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(json!({"name":"Alex","description":""})))
}
async fn dispatch(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response> {
    let path = uri.path().trim_start_matches("/api/");
    let seg = path.split('/').collect::<Vec<_>>();
    let mut b = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice::<Value>(&body).map_err(|_| Error::new(400, "JSON inválido."))?
    };
    if !b.is_object() {
        return Err(Error::new(400, "Objeto JSON esperado."));
    }
    let mut query = uri
        .query()
        .map(|q| {
            reqwest::Url::parse(&format!("http://f/?{q}"))
                .map(|u| {
                    u.query_pairs()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect::<std::collections::HashMap<_, _>>()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let get = method == Method::GET;
    let post = method == Method::POST;
    let put = method == Method::PUT;
    let delete = method == Method::DELETE;
    let mut status = 200;
    let out = match seg.as_slice() {
        ["sync", "status"] if get => {
            let observe = query.remove("observe").is_some_and(|value| value == "1");
            let _memory_pause = if observe { None } else { Some(state.runner.pause_for_sync().await) };
            let mut out=state.store.call(Request::SyncStatus(crate::store::sync::query(&query, false)?)).await?;
            out["embeddingFingerprint"]=json!(state.local.fingerprint().await);
            out
        }
        ["sync", "delta"] if get || post => {
            let _memory_pause = state.runner.pause_for_sync().await;
            let value = if get { crate::store::sync::query(&query, true)? } else { b };
            let out = state.store.call(Request::SyncDelta {
                value,
                embedding_model: state.local.fingerprint().await,
            }).await?;
            if out["code"] == "CONFLICT" {
                status = 409;
            }
            for c in out["accepted"].as_array().into_iter().flatten().filter_map(|v| v["convId"].as_str()) {
                state.changed(c, "synced", &json!({"id":c}));
            }
            out
        }
        ["health"] if get => {
            json!({"ok":true,"name":"modularchat","version":2,"release":"2.7.2","runtime":"rust","encrypted":state.store.encrypted})
        }
        ["status"] if get => {
            let models = state.fresh_models().await;
            json!({"db":state.store.call(Request::Status).await?,"local":state.local_status().await?,"jobs":state.runner.state().await,"router":match models{Ok(m)=>json!({"ok":true,"models":m.len()}),Err(e)=>json!({"ok":false,"error":e.message})},"generating":state.generation.busy_ids().await,"time":crate::now()})
        }
        ["local", "status"] if get => state.local_status().await?,
        ["local", "discovery"] if get => {
            state.require_mobile()?;
            json!(state.refresh_discovery().await?)
        }
        ["local", "providers"] if get => {
            state.require_mobile()?;
            let discovery = state.refresh_discovery().await?;
            let mut out = state.mobile_providers.lock().await.redacted();
            out["discovery"] = json!(discovery);
            out
        }
        ["local", "providers"] if put => state.update_providers(&b).await?,
        ["local", "policy"] if get => {
            json!({"memoryPolicy":state.policy.read().unwrap().clone(),"concurrency":1})
        }
        ["local", "policy"] if put || post => state.update_policy(&b).await?,
        ["local", "activate"] if post => {
            status = 202;
            let out = state.local.activate().await?;
            state.runner.kick();
            out
        }
        ["local", "standby"] if post => state.local.standby().await?,
        ["pair", "info"] if get => {
            json!({"required":state.auth.required,"loopback":auth::loopback(peer.ip())})
        }
        ["pair", "start"] if post => state.auth.start().await,
        ["pair"] if post => {
            status = 201;
            let paired = state.auth.pair(&b, peer.ip()).await?;
            let _ = state.events.send(json!({"type":"devices"}));
            paired
        }
        ["privacy", "cache-key"] if get => state.store.call(Request::PrivacyKey).await?,
        ["devices"] if get => json!({"devices":state.store.call(Request::Devices).await?}),
        ["devices", id] if delete => {
            state
                .store
                .call(Request::RevokeDevice((*id).into()))
                .await?;
            let devices = state.store.call(Request::Devices).await?;
            let _ = state.events.send(json!({"type":"devices"}));
            json!({"ok":true,"devices":devices})
        }
        ["presets"] if get => {
            let ids = state.models().await.unwrap_or_default();
            json!({"presets":presets::public(&ids,&state.validation),"validation":state.validation})
        }
        ["models"] if get => {
            let ids = state.models().await?;
            let default = if state.config.is_android() { state.default_mobile_model().await } else { "cbai/deepseek-v4.1-flash".into() };
            json!({"models":ids,"default":default,"presets":presets::public(&ids,&state.validation),"routes":ids.iter().map(|id|json!({"id":id,"presetId":presets::for_route(id)["id"]})).collect::<Vec<_>>()})
        }
        ["validation"] if get => state.validation.clone(),
        ["events"] if get => {
            let stream = tokio_stream::wrappers::BroadcastStream::new(state.events.subscribe())
                .map(|event| {
                    Ok::<_, std::convert::Infallible>(
                        Event::default().data(
                            event
                                .unwrap_or_else(|_| json!({"type":"resync"}))
                                .to_string(),
                        ),
                    )
                });
            return Ok(Sse::new(stream)
                .keep_alive(KeepAlive::new().interval(Duration::from_secs(20)))
                .into_response());
        }
        ["personas"] if get => json!({"personas":state.store.call(Request::Personas).await?}),
        ["personas"] if post => {
            b.as_object_mut()
                .ok_or_else(|| Error::new(400, "Objeto esperado."))?
                .remove("id");
            status = 201;
            state.store.call(Request::SavePersona(b)).await?
        }
        ["personas", id] if put => {
            b["id"] = json!(id);
            state.store.call(Request::SavePersona(b)).await?
        }
        ["personas", id] if delete => {
            state
                .store
                .call(Request::DeletePersona((*id).into()))
                .await?
        }
        ["chats"] if get => {
            json!({"chats":state.store.call(Request::ListConversations{archived:query.get("archived").is_some_and(|v|v=="1"),query:query.get("q").cloned().unwrap_or_default()}).await?})
        }
        ["chats"] if post => {
            if state.config.is_android()
                && ["routeId", "model", "presetId"]
                    .iter()
                    .all(|k| b.get(*k).is_none())
            {
                b["routeId"] = json!("mobile-local");
                b["presetId"] = json!("mobile-local");
            }
            let route = state.resolve_route(&b, None).await?;
            let persona = persona(&state, &b).await?;
            b["persona"] = persona["name"].clone();
            if !b["settings"].is_object() {
                b["settings"] = json!({});
            }
            if persona["id"].is_string() {
                b["settings"]["personaId"] = persona["id"].clone();
            }
            for k in ["routeId", "presetId", "effort"] {
                b[k] = route[k].clone();
            }
            b.as_object_mut().unwrap().remove("id");
            b["prompt"] = json!(rules::clip(
                b["prompt"].as_str().unwrap_or_default(),
                200000
            ));
            let mut chat = state.store.call(Request::CreateConversation(b)).await?;
            let c = s(&chat, "id");
            write_character(&state.store, &state.config, &c, &s(&chat, "prompt")).await?;
            state.store.call(Request::SaveCast{conv_id:c.clone(),value:json!({"name":chat["persona"],"kind":"user","source":"manual","description":persona["description"]})}).await?;
            state.store.call(Request::ReanalyzeCast(c.clone())).await?;
            state
                .store
                .call(Request::EnqueueJob(json!({"convId":c,"kind":"cast"})))
                .await?;
            state.changed(&c, "created", &chat);
            chat["history"] = json!([]);
            status = 201;
            chat
        }
        ["history"] if delete => {
            let mut busy = Vec::new();
            for c in state.generation.busy_ids().await {
                let conv = state.store.call(Request::GetMeta(c.clone())).await?;
                busy.push(json!({"id":c,"name":conv["name"]}));
            }
            if !busy.is_empty() {
                return Err(Error::new(
                    409,
                    format!(
                        "Aguarde terminar: {}",
                        busy.iter()
                            .map(|v| s(v, "name"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                )
                .extra("busy", json!(busy)));
            }
            let out = state.store.call(Request::ClearAll).await?;
            let _ = state.events.send(json!({"type":"cleared"}));
            out
        }
        ["trash"] if get => json!({"items":state.store.call(Request::Trash).await?,"days":30}),
        ["settings", "global"] if get => {
            json!({"settings":state.store.call(Request::GlobalSettings).await?,"limits":{"botCharsMin":500,"botCharsMax":8000,"monologueCharsMin":100,"monologueCharsMax":2000,"contentMax":1500,"tempMin":0.2,"tempMax":1.4}})
        }
        ["settings", "global"] if put => {
            let settings = state
                .store
                .call(Request::SetGlobalSettings(b.clone()))
                .await?;
            let _ = state.events.send(json!({"type":"settings"}));
            json!({"settings":settings})
        }
        ["trash", "purge"] if post => {
            let busy = state.generation.busy().await;
            if b["all"] == true && !busy.is_empty() {
                return Err(Error::new(
                    409,
                    "Há resposta em andamento; tente de novo quando terminar.",
                ));
            }
            let out = state.store.call(Request::PurgeTrash(b.clone())).await?;
            let mut done = out.clone();
            done["ok"] = json!(true);
            done
        }
        ["trash", "restore"] if post => {
            let c = checked_id(&s(
                &b,
                if b["type"] == "conversation" {
                    "id"
                } else {
                    "convId"
                },
            ))?;
            let out = if b["type"] == "conversation" {
                state
                    .store
                    .call(Request::RestoreConversation(c.clone()))
                    .await?
            } else {
                state
                    .store
                    .call(Request::RestoreBatch {
                        conv_id: c.clone(),
                        batch: s(&b, "batch"),
                    })
                    .await?
            };
            state.changed(&c, "restored", &out);
            out
        }
        ["search"] if get => {
            state
                .store
                .call(Request::Search {
                    query: query.get("q").cloned().unwrap_or_default(),
                    conv_id: query.get("chat").cloned(),
                    limit: 40,
                })
                .await?
        }
        ["export"] if get => {
            let data = state.store.call(Request::Export(None)).await?;
            if state.store.encrypted {
                state.store.call(Request::ProtectExport(data)).await?
            } else {
                data
            }
        }
        ["import"] if post => {
            if b["format"] == "modularchat-encrypted-export" {
                b = state.store.call(Request::DecryptImport(b)).await?;
            }
            let out = state.store.call(Request::Import(b)).await?;
            for c in out["imported"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let chat = state.store.call(Request::GetConversation(c.into())).await?;
                write_character(&state.store, &state.config, c, &s(&chat, "prompt")).await?;
                state.changed(c, "created", &chat);
            }
            status = 201;
            out
        }
        ["backup"] if post => state.store.call(Request::Backup).await?,
        ["chats", id, rest @ ..] => {
            let c = checked_id(id)?;
            if rest == ["message"] && post {
                b["convId"] = json!(c);
                if b["mode"] == "edit" && b["targetId"].is_null() {
                    b["targetId"] = b["messageId"].clone();
                }
                let req: TurnRequest =
                    serde_json::from_value(b).map_err(|_| Error::new(400, "Modo inválido."))?;
                let (generation, replay) = state.generation.start(req).await?;
                return Ok(state
                    .generation
                    .stream(generation, 0, replay)
                    .await?
                    .into_response());
            }
            if let ["generations", gid, "stream"] = rest
                && get
            {
                let generation = if let Some(g) = state.generation.get(&c, Some(gid)).await {
                    g
                } else {
                    state.generation.persisted(&c, gid).await?
                };
                let from = query
                    .get("from")
                    .map(|s| s.parse::<usize>())
                    .transpose()
                    .map_err(|_| Error::new(400, "Offset inválido."))?
                    .unwrap_or(0);
                return Ok(state
                    .generation
                    .stream(generation, from, false)
                    .await?
                    .into_response());
            }
            if let ["generations", gid, "stop"] = rest
                && post
            {
                return Ok(response(state.generation.stop(&c, gid).await?, 200));
            }
            let lock = state.locks.get(&c).await;
            let _guard = lock.lock().await;
            let out = match rest {
                [] if get => {
                    sync_character(&state.store, &state.config, &c).await?;
                    let mut chat = state
                        .store
                        .call(Request::GetConversation(c.clone()))
                        .await?;
                    chat["memorySummary"] =
                        state.store.call(Request::Memories(c.clone())).await?["sync"].clone();
                    chat["generation"] = if let Some(g) = state.generation.get(&c, None).await {
                        g.info()
                    } else {
                        Value::Null
                    };
                    chat
                }
                [] if put => {
                    let current = state
                        .store
                        .call(Request::GetConversation(c.clone()))
                        .await?;
                    if ["routeId", "model", "presetId", "effort"]
                        .iter()
                        .any(|k| b.get(*k).is_some())
                    {
                        let route = state.resolve_route(&b, Some(&current)).await?;
                        for k in ["routeId", "presetId", "effort"] {
                            b[k] = route[k].clone();
                        }
                    }
                    if b["personaId"].is_string() {
                        let persona = persona(&state, &b).await?;
                        b["persona"] = persona["name"].clone();
                        if !b["settings"].is_object() {
                            b["settings"] = json!({});
                        }
                        b["settings"]["personaId"] = persona["id"].clone();
                    }
                    let out = state
                        .store
                        .call(Request::PatchConversation {
                            conv_id: c.clone(),
                            base_character_rev: b["baseCharacterRev"].as_i64(),
                            patch: b,
                        })
                        .await?;
                    if out["characterChanged"] == true {
                        write_character(&state.store, &state.config, &c, &s(&out, "prompt"))
                            .await?;
                        state
                            .store
                            .call(Request::EnqueueJob(json!({"convId":c,"kind":"cast"})))
                            .await?;
                    }
                    out
                }
                [] if delete => {
                    state
                        .store
                        .call(Request::DeleteConversation(c.clone()))
                        .await?
                }
                ["restore"] if post => {
                    state
                        .store
                        .call(Request::RestoreConversation(c.clone()))
                        .await?
                }
                ["duplicate"] if post => {
                    let chat = state
                        .store
                        .call(Request::DuplicateConversation(c.clone()))
                        .await?;
                    write_character(
                        &state.store,
                        &state.config,
                        &s(&chat, "id"),
                        &s(&chat, "prompt"),
                    )
                    .await?;
                    status = 201;
                    chat
                }
                ["history"] if delete => state.store.call(Request::ClearHistory(c.clone())).await?,
                ["messages", mid] if put => {
                    let out = state
                        .store
                        .call(Request::EditMessage {
                            conv_id: c.clone(),
                            part_id: (*mid).into(),
                            content: s(&b, "content"),
                            op_id: b["opId"].as_str().map(str::to_owned),
                        })
                        .await?;
                    message_result(out)
                }
                ["messages", mid] if delete => {
                    state
                        .store
                        .call(Request::DeleteMessage {
                            conv_id: c.clone(),
                            message_id: (*mid).into(),
                        })
                        .await?
                }
                ["messages", mid, "versions"] if get => {
                    json!({"versions":state.store.call(Request::Versions{conv_id:c.clone(),part_id:(*mid).into()}).await?})
                }
                ["messages", mid, "versions", vid, "select"] if post => message_result(
                    state
                        .store
                        .call(Request::SelectVersion {
                            conv_id: c.clone(),
                            part_id: (*mid).into(),
                            version_id: (*vid).into(),
                        })
                        .await?,
                ),
                ["branches"] if get => {
                    json!({"branches":state.store.call(Request::Branches(c.clone())).await?})
                }
                ["branches", "activate"] if post => {
                    state
                        .store
                        .call(Request::ActivateBranch {
                            conv_id: c.clone(),
                            interaction_id: s(&b, "interactionId"),
                        })
                        .await?;
                    state
                        .store
                        .call(Request::GetConversation(c.clone()))
                        .await?
                }
                ["cast"] if get => {
                    json!({"cast":state.store.call(Request::Cast(c.clone())).await?})
                }
                ["cast"] if post => {
                    status = 201;
                    b["source"] = json!("manual");
                    b["confirmed"] = json!(true);
                    state
                        .store
                        .call(Request::SaveCast {
                            conv_id: c.clone(),
                            value: b,
                        })
                        .await?;
                    json!({"cast":state.store.call(Request::Cast(c.clone())).await?})
                }
                ["cast", "reanalyze"] if post => {
                    sync_character(&state.store, &state.config, &c).await?;
                    state.store.call(Request::ReanalyzeCast(c.clone())).await?;
                    state
                        .store
                        .call(Request::EnqueueJob(json!({"convId":c,"kind":"cast"})))
                        .await?;
                    json!({"ok":true})
                }
                ["cast", cid] if put => {
                    b["id"] = json!(cid);
                    state
                        .store
                        .call(Request::SaveCast {
                            conv_id: c.clone(),
                            value: b,
                        })
                        .await?
                }
                ["cast", cid] if delete => {
                    state
                        .store
                        .call(Request::DeleteCast {
                            conv_id: c.clone(),
                            cast_id: (*cid).into(),
                        })
                        .await?
                }
                ["cast", cid, "merge"] if post => {
                    state
                        .store
                        .call(Request::MergeCast {
                            conv_id: c.clone(),
                            from_id: (*cid).into(),
                            into_id: s(&b, "intoId"),
                        })
                        .await?
                }
                ["interactions", iid, "source"] if get => {
                    state
                        .store
                        .call(Request::Source {
                            conv_id: c.clone(),
                            interaction_id: (*iid).into(),
                        })
                        .await?
                }
                ["memory"] if get => state.store.call(Request::Memories(c.clone())).await?,
                ["memory"] if post => {
                    status = 201;
                    state
                        .store
                        .call(Request::AddMemory {
                            conv_id: c.clone(),
                            value: b,
                        })
                        .await?
                }
                ["memory", mid] if put => {
                    state
                        .store
                        .call(Request::PatchMemory {
                            conv_id: c.clone(),
                            memory_id: (*mid).into(),
                            value: b,
                        })
                        .await?
                }
                ["memory", mid] if delete => {
                    state
                        .store
                        .call(Request::DeleteMemory {
                            conv_id: c.clone(),
                            memory_id: (*mid).into(),
                        })
                        .await?
                }
                ["flags", fid, "resolve"] if post => {
                    state
                        .store
                        .call(Request::ResolveFlag {
                            conv_id: c.clone(),
                            flag_id: (*fid).into(),
                            action: s(&b, "action"),
                        })
                        .await?
                }
                ["jobs", "retry"] if post => {
                    state.store.call(Request::RetryJobs(c.clone())).await?
                }
                ["export"] if get => {
                    let data = state.store.call(Request::Export(Some(c.clone()))).await?;
                    if state.store.encrypted {
                        state.store.call(Request::ProtectExport(data)).await?
                    } else {
                        data
                    }
                }
                _ => return Err(Error::new(404, "Endpoint não encontrado.")),
            };
            if !get {
                state.changed(&c, "updated", &out);
            }
            out
        }
        _ => return Err(Error::new(404, "Endpoint não encontrado.")),
    };
    let cookie = if seg == ["pair"] && state.store.encrypted && auth::loopback(peer.ip()) {
        Some(format!(
            "mc_session={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000",
            s(&out, "token")
        ))
    } else {
        None
    };
    let mut result = response(out, status);
    if let Some(cookie) = cookie {
        result.headers_mut().insert(
            "set-cookie",
            HeaderValue::from_str(&cookie).map_err(|_| Error::new(500, "Cookie inválido."))?,
        );
    }
    Ok(result)
}
fn message_result(mut out: Value) -> Value {
    if out["message"].is_object() {
        let revision = out["revision"].clone();
        out = out["message"].clone();
        out["revision"] = revision;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request as HttpRequest};
    use http_body_util::BodyExt;
    use std::{io::Write, path::Path};
    use tower::ServiceExt;

    fn isolated_config(dir: &Path) -> Config {
        Config {
            data_dir: dir.into(),
            dist: dir.join("absent"),
            address: "127.0.0.1:0".parse().unwrap(),
            require_pairing: false,
            providers: None,
            max_generations: 1,
            production: false,
            native_lib_dir: None,
            review: false,
        }
    }

    async fn boot_validation(dir: &Path) -> Value {
        let state = AppState::new(isolated_config(dir)).await.unwrap();
        let validation = state.validation.clone();
        state.shutdown().await;
        validation
    }

    fn sparse_fixture(path: &Path, len: u64) {
        let file = std::fs::File::create(path).unwrap();
        #[cfg(windows)]
        {
            use std::{ffi::c_void, os::windows::io::AsRawHandle};
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn DeviceIoControl(
                    file: *mut c_void,
                    code: u32,
                    input: *mut c_void,
                    input_len: u32,
                    output: *mut c_void,
                    output_len: u32,
                    returned: *mut u32,
                    overlapped: *mut c_void,
                ) -> i32;
            }
            let mut returned = 0;
            // SAFETY: the file owns the live handle, FSCTL_SET_SPARSE needs no buffers,
            // and this synchronous call only writes the live DWORD `returned`.
            let ok = unsafe {
                DeviceIoControl(
                    file.as_raw_handle(),
                    0x0009_00c4,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    0,
                    &mut returned,
                    std::ptr::null_mut(),
                )
            };
            assert_ne!(ok, 0, "{}", std::io::Error::last_os_error());
        }
        file.set_len(len).unwrap();
    }

    #[tokio::test]
    async fn bootstrap_route_validation_preserves_the_strict_limit_and_ignores_sparse_files() {
        const LIMIT: usize = 1024 * 1024;
        let routes = json!({"routes":{"fixture":{"maxOutput":8192}}});
        for len in [64, LIMIT - 1, LIMIT] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("route-validation.json");
            let mut file = std::fs::File::create(path).unwrap();
            let content = routes.to_string();
            file.write_all(content.as_bytes()).unwrap();
            file.write_all(&vec![b' '; len - content.len()]).unwrap();
            drop(file);
            assert_eq!(
                boot_validation(dir.path()).await,
                if len < LIMIT {
                    routes.clone()
                } else {
                    json!({"routes":{}})
                },
                "file length: {len}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("route-validation.json");
        sparse_fixture(&path, 128 * 1024 * 1024);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 128 * 1024 * 1024);
        assert_eq!(boot_validation(dir.path()).await, json!({"routes":{}}));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(boot_validation(dir.path()).await, json!({"routes":{}}));
        std::fs::write(&path, b"{invalid JSON").unwrap();
        assert_eq!(boot_validation(dir.path()).await, json!({"routes":{}}));
    }

    const TOKEN: &str = "lote7_fixture_0123456789abcdef0123456789abcdef0123456789abcdef";
    #[tokio::test]
    async fn device_pair_and_revocation_publish_updates_and_return_the_updated_list() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(isolated_config(dir.path())).await.unwrap();
        state.runner.stop();
        let mut events = state.events.subscribe();
        let code = state.auth.start().await;
        let request = HttpRequest::builder().method("POST").uri("/api/pair")
            .extension(ConnectInfo("127.0.0.1:0".parse::<SocketAddr>().unwrap()))
            .header("content-type", "application/json")
            .body(Body::from(json!({"code":code["code"],"deviceName":"Pairing fixture"}).to_string())).unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let auth: Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv()).await.unwrap().unwrap();
        assert_eq!(event, json!({"type":"devices"}), "the update contains no credentials");
        let request = HttpRequest::builder().method("DELETE").uri(format!("/api/devices/{}", s(&auth,"deviceId")))
            .extension(ConnectInfo("127.0.0.1:0".parse::<SocketAddr>().unwrap()))
            .header("authorization", format!("Bearer {}", s(&auth,"token")))
            .body(Body::empty()).unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result: Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(result["ok"], true);
        let device = result["devices"].as_array().unwrap().iter().find(|row| row["id"] == auth["deviceId"]).unwrap();
        assert!(!device["revoked_at"].is_null());
        assert!(state.auth.verify(&s(&auth,"token")).await.unwrap().is_null());
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv()).await.unwrap().unwrap();
        assert_eq!(event, json!({"type":"devices"}));
        state.shutdown().await;
    }

    fn mobile_input(raw: &str, flag: Value) -> Value {
        json!({"default":"fixture","providers":[{
            "id":"fixture", "baseUrl":raw, "apiKey":"KEY_SECRET",
            "models":["org/model"], "allowLoopback":flag
        }]})
    }
    async fn put_providers(state: &AppState, body: Value) -> (u16, Value) {
        let request = HttpRequest::builder()
            .method("PUT")
            .uri("/api/local/providers")
            .extension(ConnectInfo("127.0.0.1:0".parse::<SocketAddr>().unwrap()))
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        let code = response.status().as_u16();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (code, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn mobile_provider_put_validates_raw_urls_and_boolean_flags_before_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let native = tempfile::tempdir().unwrap();
        let mut config = Config::for_android(dir.path(), native.path()).unwrap();
        config.address = "127.0.0.1:0".parse().unwrap();
        let mut state = AppState::new(config).await.unwrap();
        state.runner.stop();
        // This test sends only PUTs. Disable discovery even if a later handler starts refreshing it.
        state.discovery = None;
        state
            .store
            .call(Request::AddDevice {
                name: "Lote7 isolated fixture".into(),
                token_hash: rules::hash(TOKEN),
            })
            .await
            .unwrap();
        let (code, public) = put_providers(
            &state,
            mobile_input("https://openrouter.ai/api/v1", json!(false)),
        )
        .await;
        assert_eq!(code, 200);
        assert_eq!(public["providers"][0]["hasApiKey"], true);
        assert!(!public.to_string().contains("SECRET"));
        let saved = state
            .store
            .call(Request::LoadMobileProviders)
            .await
            .unwrap();
        for raw in [
            " https://provider.example/v1",
            "https://provider.example/v1 ",
            "https://provider.\texample/v1",
            "https://provider.example/\nSECRET",
            "https:provider.example/v1",
            "https://@provider.example/v1",
            "https://user:SECRET@provider.example/v1",
            "https://provider.example\\@evil/v1",
            "https://provider.example/v1?",
            "https://provider.example/v1#",
            "http://localhost.evil/v1",
            "http://127.0.0.1.evil/v1",
            "http://[::ffff:8.8.8.8]/v1",
            "http://192.168.1.2/v1",
            "https://provider.example/chat/completions/v1/",
            "https://provider.example/v1/messages/v1/",
            "https://api.anthropic.com/v1",
        ] {
            let (code, error) = put_providers(&state, mobile_input(raw, json!(true))).await;
            assert_eq!(code, 400, "{raw:?}");
            assert!(!error.to_string().contains("SECRET"));
        }
        for raw in [
            "http://localhost/v1",
            "https://127.0.0.1/v1",
            "http://[::ffff:127.0.0.1]/v1",
        ] {
            for flag in [
                json!(false),
                json!("true"),
                json!("false"),
                json!(1),
                json!(0),
                Value::Null,
            ] {
                let (code, _) = put_providers(&state, mobile_input(raw, flag)).await;
                assert_eq!(code, 400, "{raw}");
            }
            let mut omitted = mobile_input(raw, json!(true));
            omitted["providers"][0]
                .as_object_mut()
                .unwrap()
                .remove("allowLoopback");
            assert_eq!(put_providers(&state, omitted).await.0, 400);
        }
        assert_eq!(
            state
                .store
                .call(Request::LoadMobileProviders)
                .await
                .unwrap(),
            saved
        );
        assert_eq!(state.mobile_providers.lock().await.redacted(), public);
        for raw in [
            "http://localhost/v1",
            "http://[::1]/v1",
            "http://[::ffff:127.0.0.1]/v1",
        ] {
            assert_eq!(
                put_providers(&state, mobile_input(raw, json!(true)))
                    .await
                    .0,
                200
            );
        }
        state.shutdown().await;
    }
}
