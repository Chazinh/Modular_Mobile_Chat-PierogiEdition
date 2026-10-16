use super::*;
use crate::{
    api::{self, AppState},
    config::Config,
    providers::{MobileProviders, Providers, SelectedBackend},
    store::{Request, TurnRequest},
};
use axum::{
    Json, Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request as HttpRequest, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
};
use http_body_util::BodyExt;
use serde_json::json;
use std::sync::{Arc, Mutex as StdMutex};
use tower::ServiceExt;

const TOKEN: &str = "lote4a_fixture_0123456789abcdef0123456789abcdef0123456789abcdef";
const SECRET: &str = "lote4a-synthetic-secret-never-import";
const MODEL: &str = "org/exact-model:revision";
const ROUTE: &str = "mobile-api:9router-local:org/exact-model:revision";

#[derive(Clone)]
enum Reply {
    Models,
    Native,
    Json(Value),
    Status(u16),
    Html,
    Oversize,
    ChunkedOversize,
    ChunkedModels,
    ExactLimit,
    SlowHeaders,
    SlowBody,
    Redirect(String),
    PostError,
}
#[derive(Clone)]
struct Call {
    method: String,
    path: String,
    headers: axum::http::HeaderMap,
    body: Value,
}
struct Fixture {
    url: String,
    reply: Arc<StdMutex<Reply>>,
    calls: Arc<StdMutex<Vec<Call>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    fn posts(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.method == "POST")
            .count()
    }
    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
    fn anonymous_gets(&self) {
        for call in self
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.method == "GET")
        {
            for name in ["authorization", "proxy-authorization", "cookie"] {
                assert!(call.headers.get(name).is_none());
            }
            assert!(call.body.is_null());
            assert!(matches!(
                call.path.as_str(),
                "/v1/models" | "/global/health"
            ));
        }
    }
}
async fn fixture(reply: Reply) -> Fixture {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let shared_reply = Arc::new(StdMutex::new(reply));
    let seen = calls.clone();
    let mode = shared_reply.clone();
    let app = Router::new().fallback(any(move |request: HttpRequest<Body>| {
        let seen = seen.clone();
        let mode = mode.clone();
        async move {
            let (parts, body) = request.into_parts();
            let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
            let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let is_post = parts.method == "POST";
            seen.lock().unwrap().push(Call {
                method: parts.method.to_string(), path: parts.uri.path().into(), headers: parts.headers, body,
            });
            let reply = mode.lock().unwrap().clone();
            if is_post {
                if matches!(reply, Reply::PostError) {
                    return (StatusCode::BAD_GATEWAY, SECRET).into_response();
                }
                let event = json!({"choices":[{"delta":{"content":"Lia: Vamos examinar essa carta com você."},"finish_reason":null}]});
                let end = json!({"choices":[{"delta":{},"finish_reason":"stop"}]});
                return ([("content-type", "text/event-stream")], format!("data: {event}\n\ndata: {end}\n\ndata: [DONE]\n\n")).into_response();
            }
            match reply {
                Reply::Models | Reply::PostError => Json(json!({"data":[{"id":MODEL},{"id":MODEL}],"private":SECRET})).into_response(),
                Reply::Native => Json(json!({"healthy":true,"version":"1.2.3","private":SECRET})).into_response(),
                Reply::Json(value) => Json(value).into_response(),
                Reply::Status(code) => (StatusCode::from_u16(code).unwrap(), SECRET).into_response(),
                Reply::Html => format!("<html>{SECRET}</html>").into_response(),
                Reply::Oversize => Response::builder().header("content-length", MODELS_LIMIT + 1)
                    .body(Body::from_stream(futures_util::stream::pending::<std::result::Result<String, std::io::Error>>())).unwrap(),
                Reply::ChunkedOversize => {
                    let chunks = (0..3).map(|_| Ok::<_, std::io::Error>(vec![b' '; 64 * 1024]));
                    Response::new(Body::from_stream(futures_util::stream::iter(chunks)))
                }
                Reply::ChunkedModels => {
                    let chunks = [Ok::<_, std::io::Error>("{\"data\":[".to_owned()), Ok(format!("{{\"id\":\"{MODEL}\"}}]}}"))];
                    Response::new(Body::from_stream(futures_util::stream::iter(chunks)))
                }
                Reply::ExactLimit => {
                    let mut bytes = json!({"data":[{"id":MODEL}]}).to_string().into_bytes();
                    bytes.resize(MODELS_LIMIT,b' ');
                    Response::new(Body::from(bytes))
                }
                Reply::SlowHeaders => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    Json(json!({"data":[{"id":MODEL}]})).into_response()
                }
                Reply::SlowBody => Response::new(Body::from_stream(futures_util::stream::once(async {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    Ok::<_, std::io::Error>("{}")
                }))),
                Reply::Redirect(location) => (StatusCode::FOUND, [("location", location)]).into_response(),
            }
        }
    }));
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Fixture {
        url,
        reply: shared_reply,
        calls,
        task,
    }
}
fn discovery(nine: &Fixture, code: &Fixture) -> Arc<Discovery> {
    let mut discovery = Discovery::new();
    // Endpoint injection exists ONLY in this cfg(test) module. No production setter or env override.
    discovery.nine_router = format!("{}/v1/models", nine.url);
    discovery.open_code = format!("{}/global/health", code.url);
    Arc::new(discovery)
}
async fn expire(discovery: &Discovery) {
    if let Some((at, _)) = discovery.cache.lock().await.as_mut() {
        *at = Instant::now() - TTL;
    }
}
async fn state(
    dir: &std::path::Path,
    native: &std::path::Path,
    discovery: Arc<Discovery>,
) -> AppState {
    let mut state = AppState::new(Config::for_android(dir, native).unwrap())
        .await
        .unwrap();
    state.discovery = Some(discovery);
    state
        .store
        .call(Request::AddDevice {
            name: "Lote4A isolated fixture".into(),
            token_hash: crate::rules::hash(TOKEN),
        })
        .await
        .unwrap();
    state
}
async fn call(
    state: &AppState,
    method: &str,
    path: &str,
    body: Option<Value>,
    auth: bool,
) -> (u16, Value) {
    let mut request = HttpRequest::builder()
        .uri(path)
        .method(method)
        .extension(ConnectInfo(
            "127.0.0.1:43210".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .header("content-type", "application/json");
    if auth {
        request = request.header("authorization", format!("Bearer {TOKEN}"));
    }
    let response = api::router(state.clone())
        .oneshot(
            request
                .body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[test]
fn model_validation_is_bounded_strict_and_deduplicated() {
    assert_eq!(
        model_ids(&json!({"data":[{"id":MODEL},{"id":MODEL}]})).unwrap(),
        [MODEL]
    );
    let max = "a".repeat(256);
    assert_eq!(
        model_ids(&json!({"data":[{"id":max}]})).unwrap()[0].len(),
        256
    );
    assert_eq!(
        model_ids(
            &json!({"data":(0..128).map(|i|json!({"id":format!("m{i}")})).collect::<Vec<_>>()})
        )
        .unwrap()
        .len(),
        128
    );
    for value in [
        json!({}),
        json!({"data":{}}),
        json!({"data":[{}]}),
        json!({"data":[{"id":3}]}),
        json!({"data":[{"id":" \t"}]}),
        json!({"data":[{"id":"bad\nmodel"}]}),
        json!({"data":[{"id":"x".repeat(257)}]}),
        json!({"data":vec![json!({"id":"x"});129]}),
    ] {
        assert!(model_ids(&value).is_none());
    }
}

#[tokio::test]
async fn discovery_probes_classify_auth_protocol_size_redirects_and_deadlines() {
    struct Capture(Arc<StdMutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let logs = Arc::new(StdMutex::new(Vec::new()));
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || Capture(writer.clone()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let target = fixture(Reply::Models).await;
    let code = fixture(Reply::Native).await;
    for (reply, expected) in [
        (Reply::Models, Some(Status::Available)),
        (Reply::ChunkedModels, Some(Status::Available)),
        (Reply::ExactLimit, Some(Status::Available)),
        (Reply::Status(401), Some(Status::AuthRequired)),
        (Reply::Status(403), Some(Status::AuthRequired)),
        (Reply::Status(404), Some(Status::Incompatible)),
        (Reply::Html, Some(Status::Incompatible)),
        (Reply::Oversize, Some(Status::Incompatible)),
        (Reply::ChunkedOversize, Some(Status::Incompatible)),
        (
            Reply::Redirect(format!("{}/v1/models", target.url)),
            Some(Status::Incompatible),
        ),
        (Reply::SlowHeaders, None),
        (Reply::SlowBody, None),
    ] {
        let nine = fixture(reply).await;
        let discovery = discovery(&nine, &code);
        let start = Instant::now();
        let (snapshot, _) = discovery.snapshot().await;
        assert!(
            start.elapsed() < Duration::from_millis(950),
            "both probes must share a subsecond deadline"
        );
        if let Some(expected) = expected {
            assert_eq!(snapshot.nine_router.status, expected);
        } else {
            assert_ne!(snapshot.nine_router.status, Status::Available);
        }
        assert_eq!(snapshot.open_code.status, Status::DetectedNativeApi);
        assert_eq!(snapshot.open_code.version.as_deref(), Some("1.2.3"));
        assert!(!serde_json::to_string(&snapshot).unwrap().contains(SECRET));
        if let Some(provider) = snapshot.provider() {
            assert_eq!(provider.id, PROVIDER_ID);
            assert_eq!(provider.models, [MODEL]);
            assert!(provider.api_key.is_none() && provider.request_log.is_none());
        }
        nine.anonymous_gets();
        assert_eq!(nine.posts(), 0);
    }
    assert_eq!(target.count(), 0, "redirect target must never be contacted");
    code.anonymous_gets();
    let slow_nine = fixture(Reply::SlowHeaders).await;
    let slow_code = fixture(Reply::SlowBody).await;
    let start = Instant::now();
    discovery(&slow_nine, &slow_code).snapshot().await;
    assert!(
        start.elapsed() < Duration::from_millis(950),
        "slow probes must run concurrently"
    );
    assert!(!String::from_utf8_lossy(&logs.lock().unwrap()).contains(SECRET));
}

#[tokio::test]
async fn native_health_requires_healthy_and_a_bounded_version_and_never_registers_a_provider() {
    let nine = fixture(Reply::Status(401)).await;
    for reply in [
        Reply::Json(json!({"healthy":false,"version":"1"})),
        Reply::Json(json!({"healthy":true})),
        Reply::Json(json!({"healthy":true,"version":"bad\nversion"})),
        Reply::Json(json!({"healthy":true,"version":"v".repeat(129)})),
        Reply::Native,
        Reply::Status(401),
        Reply::Oversize,
        Reply::ChunkedOversize,
    ] {
        let valid = matches!(reply, Reply::Native);
        let auth = matches!(reply, Reply::Status(401));
        let code = fixture(reply).await;
        let (snapshot, _) = discovery(&nine, &code).snapshot().await;
        assert_eq!(
            snapshot.open_code.status,
            if valid {
                Status::DetectedNativeApi
            } else if auth {
                Status::AuthRequired
            } else {
                Status::Incompatible
            }
        );
        assert!(snapshot.provider().is_none());
        assert!(snapshot.open_code.models.is_empty());
    }
}

#[tokio::test]
async fn ttl_and_singleflight_cover_success_and_failure_without_background_work() {
    let nine = fixture(Reply::Models).await;
    let code = fixture(Reply::Native).await;
    let discovery = discovery(&nine, &code);
    let run = || futures_util::future::join_all((0..24).map(|_| discovery.snapshot()));
    let snapshots = run().await;
    assert_eq!(snapshots.iter().filter(|(_, changed)| *changed).count(), 1);
    assert_eq!((nine.count(), code.count()), (1, 1));
    *nine.reply.lock().unwrap() = Reply::Status(401);
    assert!(
        run()
            .await
            .iter()
            .all(|(s, changed)| s.nine_router.status == Status::Available && !changed)
    );
    assert_eq!((nine.count(), code.count()), (1, 1));
    expire(&discovery).await;
    assert_eq!(
        run().await.iter().filter(|(_, changed)| *changed).count(),
        1
    );
    assert_eq!((nine.count(), code.count()), (2, 2));
    assert!(
        run()
            .await
            .iter()
            .all(|(s, changed)| s.nine_router.status == Status::AuthRequired && !changed)
    );
}

#[tokio::test]
async fn authenticated_mobile_discovery_merges_refreshes_and_preserves_local_and_conversations() {
    let nine = fixture(Reply::Models).await;
    let code = fixture(Reply::Native).await;
    let discovery = discovery(&nine, &code);
    let dir = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let state = state(dir.path(), native.path(), discovery.clone()).await;
    assert_eq!((nine.count(), code.count()), (0, 0), "boot must not probe");
    assert_eq!(
        call(&state, "GET", "/api/local/discovery", None, false)
            .await
            .0,
        401
    );
    assert_eq!((nine.count(), code.count()), (0, 0));
    let conv = state
        .store
        .call(Request::CreateConversation(
            json!({"routeId":ROUTE,"presetId":"mobile-api","name":"Preserve"}),
        ))
        .await
        .unwrap();
    // /models must refresh even when its own 60-second cache has not expired.
    let (status, models) = call(&state, "GET", "/api/models", None, true).await;
    assert_eq!(status, 200);
    assert_eq!(models["default"], "mobile-local");
    assert_eq!(models["models"], json!([ROUTE, "mobile-local"]));
    for path in [
        "/api/presets",
        "/api/local/providers",
        "/api/local/status",
        "/api/local/discovery",
    ] {
        assert_eq!(call(&state, "GET", path, None, true).await.0, 200);
    }
    assert_eq!((nine.count(), code.count()), (1, 1));
    assert!(
        state
            .store
            .call(Request::LoadMobileProviders)
            .await
            .unwrap()
            .is_null()
    );
    let (_, public) = call(&state, "GET", "/api/local/providers", None, true).await;
    assert_eq!(public["providers"], json!([]));
    assert_eq!(
        public["discovery"]["openCode"]["status"],
        "detected_native_api"
    );
    // Read-only discovery is not part of the writable sealed schema.
    assert!(MobileProviders::parse(&public).is_err());
    let snapshot = state.providers.snapshot().unwrap();
    for legacy in [
        MODEL,
        "9router-local:org/exact-model:revision",
        "cbai/glm-5.2",
        "openrouter/old-cloud",
    ] {
        assert!(matches!(
            snapshot.select(legacy).await,
            SelectedBackend::Local
        ));
    }
    assert!(matches!(
        snapshot.select(ROUTE).await,
        SelectedBackend::Cloud { .. }
    ));
    *nine.reply.lock().unwrap() = Reply::Status(404);
    expire(&discovery).await;
    let (_, models) = call(&state, "GET", "/api/models", None, true).await;
    assert_eq!(models["models"], json!(["mobile-local"]));
    assert_eq!(models["default"], "mobile-local");
    assert!(state.providers.snapshot().is_none());
    let (_, presets) = call(&state, "GET", "/api/presets", None, true).await;
    assert!(
        presets["presets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "mobile-local" && p["available"] == true)
    );
    let saved = state
        .store
        .call(Request::GetConversation(crate::s(&conv, "id")))
        .await
        .unwrap();
    assert_eq!(saved["routeId"], ROUTE);
    assert_eq!(nine.posts(), 0);
    nine.anonymous_gets();
    code.anonymous_gets();
    state.shutdown().await;
}

#[tokio::test]
async fn saved_config_wins_collisions_and_edits_preserve_the_ephemeral_catalog_without_persisting_it()
 {
    let nine = fixture(Reply::Models).await;
    let code = fixture(Reply::Native).await;
    let manual = fixture(Reply::Models).await;
    let discovery = discovery(&nine, &code);
    let dir = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let state = state(dir.path(), native.path(), discovery.clone()).await;
    call(&state, "GET", "/api/local/discovery", None, true).await;
    let body = json!({"default":"manual","providers":[{"id":"manual","baseUrl":format!("{}/v1",manual.url),"models":[MODEL],"apiKey":SECRET,"allowLoopback":true}],
        "aliases":{"saved-alias":{"provider":"manual","model":MODEL}}});
    assert_eq!(
        call(
            &state,
            "PUT",
            "/api/local/providers",
            Some(body.clone()),
            true
        )
        .await
        .0,
        200
    );
    let (_, models) = call(&state, "GET", "/api/models", None, true).await;
    assert!(models["models"].as_array().unwrap().contains(&json!(ROUTE)));
    assert_eq!(models["default"], format!("mobile-api:manual:{MODEL}"));
    let persisted = state
        .store
        .call(Request::LoadMobileProviders)
        .await
        .unwrap();
    assert_eq!(persisted["providers"].as_array().unwrap().len(), 1);
    assert_eq!(persisted["providers"][0]["id"], "manual");
    assert_eq!(persisted["providers"][0]["apiKey"], SECRET);
    assert_eq!(persisted["default"], body["default"]);
    assert_eq!(persisted["aliases"], body["aliases"]);
    let mut collision = body.clone();
    collision["default"] = json!(PROVIDER_ID);
    collision["providers"][0]["id"] = json!(PROVIDER_ID);
    collision["aliases"]["saved-alias"]["provider"] = json!(PROVIDER_ID);
    assert_eq!(
        call(&state, "PUT", "/api/local/providers", Some(collision), true)
            .await
            .0,
        200
    );
    expire(&discovery).await;
    call(&state, "GET", "/api/local/discovery", None, true).await;
    let runtime = state.providers.snapshot().unwrap();
    let (provider, _) = runtime
        .target(&crate::providers::RouteTarget {
            provider: PROVIDER_ID.into(),
            model: MODEL.into(),
        })
        .unwrap();
    assert_eq!(provider.base_url, format!("{}/v1", manual.url));
    assert_eq!(provider.api_key.as_deref(), Some(SECRET));
    assert!(matches!(
        runtime.select("saved-alias").await,
        SelectedBackend::Cloud { .. }
    ));
    let (_, public) = call(&state, "GET", "/api/local/providers", None, true).await;
    assert!(!public.to_string().contains(SECRET));
    assert_eq!(public["providers"].as_array().unwrap().len(), 1);
    nine.anonymous_gets();
    code.anonymous_gets();
    assert_eq!(nine.posts(), 0);
    state.shutdown().await;
}

#[tokio::test]
async fn chosen_discovered_route_sends_one_real_fixture_post_and_never_retries_after_acceptance() {
    for reply in [Reply::Models, Reply::PostError] {
        let post_error = matches!(reply, Reply::PostError);
        let nine = fixture(reply).await;
        let code = fixture(Reply::Native).await;
        let dir = tempfile::tempdir().unwrap();
        let native = tempfile::tempdir().unwrap();
        let state = state(dir.path(), native.path(), discovery(&nine, &code)).await;
        state
            .store
            .call(Request::SetGlobalSettings(
                json!({"botChars":500,"lengthMode":"maximum","dialoguePct":100}),
            ))
            .await
            .unwrap();
        call(&state, "GET", "/api/models", None, true).await;
        assert_eq!(nine.posts(), 0);
        let conv = state.store.call(Request::CreateConversation(json!({"routeId":ROUTE,"presetId":"mobile-api","name":"Explicit selection","settings":{"context":{"hardLimit":1}}}))).await.unwrap();
        let turn: TurnRequest = serde_json::from_value(json!({"convId":conv["id"],"mode":"send","content":"Olá da fixture.","opId":crate::id()})).unwrap();
        let (generation, _) = state.generation.start(turn).await.unwrap();
        assert_eq!(
            generation.ack["context"]["routing"]["provider"],
            PROVIDER_ID
        );
        // Runtime changes cannot retarget the already selected provider.
        state.providers.set(None);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if generation.output.lock().await.final_event.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(nine.posts(), 1);
        assert_eq!(code.posts(), 0);
        let post = nine
            .calls
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.method == "POST")
            .unwrap()
            .clone();
        assert_eq!(post.path, "/v1/chat/completions");
        assert_eq!(post.body["model"], MODEL);
        assert!(post.headers.get("authorization").is_none());
        let output = generation.output.lock().await;
        assert_eq!(
            output.final_event.as_ref().unwrap()["status"],
            if post_error { "error" } else { "complete" }
        );
        assert!(
            !output
                .final_event
                .as_ref()
                .unwrap()
                .to_string()
                .contains(SECRET)
        );
        drop(output);
        let stored = state
            .store
            .call(Request::GetConversation(generation.conv.clone()))
            .await
            .unwrap();
        assert_eq!(stored["routeId"], ROUTE);
        assert!(!stored.to_string().contains(SECRET));
        state.shutdown().await;
    }
}

#[tokio::test]
async fn absent_loopback_and_bad_provider_keep_local_catalog_and_desktop_never_discovers() {
    let nine = fixture(Reply::Models).await;
    let code = fixture(Reply::Native).await;
    let mut absent = Discovery::new();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let closed = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    absent.nine_router = format!("{closed}/v1/models");
    absent.open_code = format!("{closed}/global/health");
    let dir = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let state = state(dir.path(), native.path(), Arc::new(absent)).await;
    let (_, snapshot) = call(&state, "GET", "/api/local/discovery", None, true).await;
    assert_eq!(snapshot["nineRouter"]["status"], "absent");
    assert!(
        snapshot["nineRouter"]["message"]
            .as_str()
            .unwrap()
            .contains("onde está instalado")
    );
    // Defensive check: even an unavailable runtime snapshot cannot remove the local option.
    state.providers.set(Some(Arc::new(
        Providers::from_json(&json!({"providers":[{"id":"bad","baseUrl":closed}]})).unwrap(),
    )));
    let (_, models) = call(&state, "GET", "/api/models", None, true).await;
    assert_eq!(models["models"], json!(["mobile-local"]));
    state.shutdown().await;
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        data_dir: dir.path().into(),
        dist: dir.path().join("dist"),
        address: "127.0.0.1:3011".parse().unwrap(),
        require_pairing: false,
        providers: None,
        max_generations: 1,
        production: false,
        native_lib_dir: None,
        review: false,
    };
    let mut desktop = AppState::new(config).await.unwrap();
    desktop
        .store
        .call(Request::AddDevice {
            name: "Lote4A desktop fixture".into(),
            token_hash: crate::rules::hash(TOKEN),
        })
        .await
        .unwrap();
    assert!(desktop.discovery.is_none());
    // Force a fixture into the test state to verify the Android guard as well as construction.
    desktop.discovery = Some(discovery(&nine, &code));
    assert_eq!(
        call(&desktop, "GET", "/api/local/discovery", None, true)
            .await
            .0,
        409
    );
    for path in [
        "/api/models",
        "/api/presets",
        "/api/local/status",
        "/api/status",
    ] {
        assert_eq!(call(&desktop, "GET", path, None, true).await.0, 200);
    }
    assert_eq!((nine.count(), code.count()), (0, 0));
    desktop.shutdown().await;
}

// All environment mutation occurs in a fresh process. Never touch the caller's env or personal files.
#[test]
fn android_bootstrap_ignores_env_and_plain_provider_files_in_isolated_subprocesses() {
    const CHILD: &str = "MC_LOTE4A_ISOLATION_CHILD";
    if let Some(dir) = std::env::var_os(CHILD) {
        let dir = std::path::PathBuf::from(dir);
        let mut config = Config::for_android(&dir.join("data"), &dir.join("native")).unwrap();
        assert!(config.providers.is_none());
        // AppState must also reject injected desktop config as a bootstrap fallback.
        config.providers = Some(Arc::new(Providers::from_json(&json!({"providers":[{"id":"cloud","baseUrl":"https://example.invalid/v1","apiKey":SECRET,"models":[MODEL]}]})).unwrap()));
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let state = AppState::new(config).await.unwrap();
                assert!(state.providers.snapshot().is_none());
                assert!(
                    state
                        .store
                        .call(Request::LoadMobileProviders)
                        .await
                        .unwrap()
                        .is_null()
                );
                let nine = fixture(Reply::Models).await;
                let code = fixture(Reply::Native).await;
                let (snapshot, _) = discovery(&nine, &code).snapshot().await;
                assert_eq!(snapshot.nine_router.status, Status::Available);
                assert_eq!(snapshot.open_code.status, Status::DetectedNativeApi);
                nine.anonymous_gets();
                code.anonymous_gets();
                state.shutdown().await;
            });
        return;
    }
    let fake = json!({"default":"cloud","providers":[{"id":"cloud","baseUrl":"https://example.invalid/v1","apiKey":SECRET,"models":[MODEL]}]});
    for mode in ["file", "env-file", "env-url", "invalid-file", "empty"] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        std::fs::create_dir_all(dir.path().join("native")).unwrap();
        let external = dir.path().join("fake-external.json");
        std::fs::write(&external, fake.to_string()).unwrap();
        if mode == "file" || mode == "invalid-file" {
            std::fs::write(
                dir.path().join("data/providers.json"),
                if mode == "file" {
                    fake.to_string()
                } else {
                    "invalid JSON".into()
                },
            )
            .unwrap();
        }
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        proxy.set_nonblocking(true).unwrap();
        let proxy_url = format!("http://fake-user:{SECRET}@{}", proxy.local_addr().unwrap());
        command.args(["--exact","local_discovery::tests::android_bootstrap_ignores_env_and_plain_provider_files_in_isolated_subprocesses"])
            .env(CHILD,dir.path()).env("MC_UPSTREAM_API_KEY",SECRET).env("NINEROUTER_API_KEY",SECRET).env("NINEROUTER_KEY",SECRET)
            .env_remove("MC_PROVIDERS_FILE").env_remove("MC_UPSTREAM_URL").env_remove("NINEROUTER_URL")
            .env("NINEROUTER_DB",dir.path().join("must-not-read.sqlite"));
        for name in [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ] {
            command.env(name, &proxy_url);
        }
        command.env_remove("NO_PROXY").env_remove("no_proxy");
        if mode == "env-file" {
            command.env("MC_PROVIDERS_FILE", external);
        }
        if mode == "env-url" {
            command
                .env("MC_UPSTREAM_URL", "https://example.invalid/v1")
                .env("NINEROUTER_URL", "https://example.invalid/v1");
        }
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "isolated bootstrap case {mode} failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!String::from_utf8_lossy(&result.stdout).contains(SECRET));
        assert!(!String::from_utf8_lossy(&result.stderr).contains(SECRET));
        assert!(
            proxy.accept().is_err(),
            "discovery must bypass even a credentialed environment proxy"
        );
    }
}
