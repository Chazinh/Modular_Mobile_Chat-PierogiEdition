use axum::{
    Json, Router,
    body::Body,
    extract::ConnectInfo,
    http::{HeaderMap, Request as HttpRequest, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use http_body_util::BodyExt;
use modularchat_server::{
    api::{AppState, router},
    config::Config,
    context::{self, default_context_hard_limit},
    generation::{Manager, classify_token_limit},
    local::LocalModels,
    memory_backend::{MemoryApi, MemoryBackend, parse_retry_after},
    presets,
    providers::{
        Provider, Providers, ReasoningFormat, RuntimeProviders, SelectedBackend, normalize_base_url,
    },
    rules,
    store::{Request, Store, TurnRequest},
};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, broadcast};
use tower::ServiceExt;

struct MockServer {
    url: String,
    posts: Arc<AtomicUsize>,
    _gets: Arc<AtomicUsize>,
    _in_flight: Arc<AtomicUsize>,
    peak_in_flight: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn spawn_mock<F, Fut>(handler: F) -> MockServer
where
    F: Fn(usize, Value, Arc<AtomicUsize>, Arc<AtomicUsize>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Response> + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let posts = Arc::new(AtomicUsize::new(0));
    let gets = Arc::new(AtomicUsize::new(0));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let peak_in_flight = Arc::new(AtomicUsize::new(0));
    let bodies = Arc::new(Mutex::new(Vec::new()));

    let handler = Arc::new(handler);
    let gets_srv = gets.clone();
    let posts_srv = posts.clone();
    let in_flight_srv = in_flight.clone();
    let peak_srv = peak_in_flight.clone();
    let bodies_srv = bodies.clone();

    let app = Router::new()
        .route(
            "/v1/models",
            get(move || {
                let gets_srv = gets_srv.clone();
                async move {
                    gets_srv.fetch_add(1, Ordering::SeqCst);
                    Json(json!({
                        "data": [
                            { "id": "custom/model-alpha" },
                            { "id": "custom/model-beta" },
                            { "id": "custom/muse-alpha" }
                        ]
                    }))
                }
            }),
        )
        .route(
            "/v1/chat/completions",
            post(move |Json(body): Json<Value>| {
                let handler = handler.clone();
                let posts_srv = posts_srv.clone();
                let in_flight_srv = in_flight_srv.clone();
                let peak_srv = peak_srv.clone();
                let bodies_srv = bodies_srv.clone();
                async move {
                    let seq = posts_srv.fetch_add(1, Ordering::SeqCst) + 1;
                    bodies_srv.lock().unwrap().push(body.clone());
                    let cur = in_flight_srv.fetch_add(1, Ordering::SeqCst) + 1;
                    peak_srv.fetch_max(cur, Ordering::SeqCst);
                    let resp = handler(seq, body, in_flight_srv.clone(), peak_srv.clone()).await;
                    in_flight_srv.fetch_sub(1, Ordering::SeqCst);
                    resp
                }
            }),
        );
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    MockServer {
        url,
        posts,
        _gets: gets,
        _in_flight: in_flight,
        peak_in_flight,
        bodies,
        task,
    }
}

fn sse_reply(text: &str) -> Response {
    let chunk1 = format!(
        "data: {}\n\n",
        json!({"choices":[{"delta":{"content": text}}]})
    );
    let chunk2 = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"delta":{},"finish_reason":"stop"}]})
    );
    let body = format!("{chunk1}{chunk2}");
    ([("content-type", "text/event-stream")], body).into_response()
}

fn test_manager_with_store(
    store: Store,
    dir: &std::path::Path,
    providers: RuntimeProviders,
    max_generations: usize,
) -> Manager {
    let local_cfg = Config::for_android(dir, dir).unwrap();
    let config = Config {
        data_dir: dir.to_path_buf(),
        dist: dir.join("dist"),
        address: "127.0.0.1:0".parse().unwrap(),
        require_pairing: false,
        providers: None,
        max_generations,
        production: false,
        native_lib_dir: None,
        review: false,
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let local = LocalModels::new(&local_cfg, client.clone()).unwrap();
    let (events, _) = broadcast::channel(32);
    let mut manager = Manager::new(store, local, client, config, events, Arc::new(Notify::new()));
    manager.providers = providers;
    manager
}

fn test_manager(dir: &std::path::Path, providers: RuntimeProviders, max_generations: usize) -> (Store, Manager) {
    let config = Config {
        data_dir: dir.to_path_buf(),
        dist: dir.join("dist"),
        address: "127.0.0.1:0".parse().unwrap(),
        require_pairing: false,
        providers: None,
        max_generations,
        production: false,
        native_lib_dir: None,
        review: false,
    };
    let store = Store::open_for_config(&config).unwrap();
    let manager = test_manager_with_store(store.clone(), dir, providers, max_generations);
    (store, manager)
}

async fn finish_generation(manager: &Manager, conv_id: &str, content: &str) -> (String, Value) {
    let req: TurnRequest = serde_json::from_value(json!({
        "convId": conv_id,
        "mode": "send",
        "content": content,
        "opId": modularchat_server::id()
    }))
    .unwrap();
    let (g, _) = manager.start(req).await.unwrap();
    let final_event = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(ev) = g.output.lock().await.final_event.clone() {
                return ev;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    (g.id.clone(), final_event)
}

#[tokio::test]
async fn per_base_url_serializes_same_server_and_allows_distinct_servers_in_parallel() {
    let hold_a = Arc::new(Notify::new());
    let hold_a_srv = hold_a.clone();
    let started_a = Arc::new(Notify::new());
    let started_a_srv = started_a.clone();

    let server_a = spawn_mock(move |seq, _body, _cur, _peak| {
        let hold_a_srv = hold_a_srv.clone();
        let started_a_srv = started_a_srv.clone();
        async move {
            if seq == 1 {
                started_a_srv.notify_one();
                hold_a_srv.notified().await;
            }
            sse_reply("*Lia respira fundo.*\n\nLia: Olá do servidor A por aqui hoje.")
        }
    })
    .await;

    let server_b = spawn_mock(move |_seq, _body, _cur, _peak| async move {
        sse_reply("*Lia respira fundo.*\n\nLia: Olá paralelo do servidor B hoje.")
    })
    .await;

    let providers = Arc::new(
        Providers::from_json(&json!({
            "default": "srv_a1",
            "providers": [
                { "id": "srv_a1", "baseUrl": server_a.url, "maxConcurrent": 1 },
                { "id": "srv_a2", "baseUrl": format!("{}/", server_a.url) },
                { "id": "srv_b", "baseUrl": server_b.url, "maxConcurrent": 1 }
            ]
        }))
        .unwrap(),
    );
    let runtime = RuntimeProviders::with_default_concurrency(Some(providers), 4);
    let dir = tempfile::tempdir().unwrap();
    let (store, manager) = test_manager(dir.path(), runtime, 4);

    let c1 = store
        .call(Request::CreateConversation(json!({
            "routeId": "srv_a1:custom/model-alpha",
            "presetId": "generic"
        })))
        .await
        .unwrap();
    let c2 = store
        .call(Request::CreateConversation(json!({
            "routeId": "srv_a2:custom/model-beta",
            "presetId": "generic"
        })))
        .await
        .unwrap();
    let c3 = store
        .call(Request::CreateConversation(json!({
            "routeId": "srv_b:custom/model-gamma",
            "presetId": "generic"
        })))
        .await
        .unwrap();

    let t1 = tokio::spawn({
        let manager = manager.clone();
        let id = c1["id"].as_str().unwrap().to_owned();
        async move { finish_generation(&manager, &id, "oi 1").await }
    });
    tokio::time::timeout(Duration::from_secs(3), started_a.notified())
        .await
        .unwrap();

    // Queue c2 on same normalized baseUrl as c1 (different logical provider id srv_a2 with trailing slash)
    let t2 = tokio::spawn({
        let manager = manager.clone();
        let id = c2["id"].as_str().unwrap().to_owned();
        async move { finish_generation(&manager, &id, "oi 2").await }
    });
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(server_a.posts.load(Ordering::SeqCst), 1, "srv_a2 must wait in same baseUrl queue");

    // c3 on distinct server_b completes immediately without waiting on server_a!
    let (_, ev3) = tokio::time::timeout(
        Duration::from_secs(2),
        finish_generation(&manager, c3["id"].as_str().unwrap(), "oi 3"),
    )
    .await
    .unwrap();
    assert_eq!(ev3["status"], "complete");
    assert_eq!(server_b.posts.load(Ordering::SeqCst), 1);
    assert_eq!(server_a.posts.load(Ordering::SeqCst), 1);

    // Release server_a -> c1 completes, then c2 executes sequentially
    hold_a.notify_one();
    let (r1, r2) = tokio::join!(t1, t2);
    let (_, ev1) = r1.unwrap();
    let (_, ev2) = r2.unwrap();
    assert_eq!(ev1["status"], "complete");
    assert_eq!(ev2["status"], "complete");
    assert_eq!(server_a.posts.load(Ordering::SeqCst), 2);
    assert_eq!(server_a.peak_in_flight.load(Ordering::SeqCst), 1);

    store.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn foreground_chat_memory_job_and_style_repair_share_outbound_gate() {
    let hold_first = Arc::new(Notify::new());
    let hold_first_srv = hold_first.clone();
    let started_first = Arc::new(Notify::new());
    let started_first_srv = started_first.clone();

    let server = spawn_mock(move |seq, body, _cur, _peak| {
        let hold_first_srv = hold_first_srv.clone();
        let started_first_srv = started_first_srv.clone();
        async move {
            if seq == 1 {
                started_first_srv.notify_one();
                hold_first_srv.notified().await;
                // Pure narration violates dialoguePct=100, triggering style_repair!
                return sse_reply("*Lia olha pela janela em silêncio absoluto.*");
            }
            if body["stream"] == false && body.get("response_format").is_some() {
                return Json(json!({
                    "choices": [{
                        "finish_reason": "stop",
                        "message": { "content": "{\"answer\":\"ok\"}" }
                    }]
                }))
                .into_response();
            }
            // style_repair response
            Json(json!({
                "choices": [{
                    "finish_reason": "stop",
                    "message": { "content": "Lia: O céu está limpo hoje." }
                }]
            }))
            .into_response()
        }
    })
    .await;

    let providers = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 1 }]
        }))
        .unwrap(),
    );
    let runtime = RuntimeProviders::with_default_concurrency(Some(providers.clone()), 1);
    let dir = tempfile::tempdir().unwrap();
    let (store, manager) = test_manager(dir.path(), runtime.clone(), 4);

    store
        .call(Request::SetGlobalSettings(json!({
            "botChars": 1200,
            "dialoguePct": 100,
            "monologueChars": 150,
            "content": "",
            "lengthMode": "maximum",
            "turnPacing": "interactive"
        })))
        .await
        .unwrap();
    let conv = store
        .call(Request::CreateConversation(json!({
            "routeId": "custom/model-alpha",
            "presetId": "generic"
        })))
        .await
        .unwrap();

    let fg_task = tokio::spawn({
        let manager = manager.clone();
        let id = conv["id"].as_str().unwrap().to_owned();
        async move { finish_generation(&manager, &id, "oi").await }
    });
    tokio::time::timeout(Duration::from_secs(3), started_first.notified())
        .await
        .unwrap();

    let (provider, _) = providers.resolve("custom/model-alpha");
    let mem_backend = MemoryBackend::Api {
        client: reqwest::Client::builder().no_proxy().build().unwrap(),
        provider: provider.clone(),
        api: MemoryApi {
            provider: "mock".into(),
            model: "custom/model-alpha".into(),
            history_consent: true,
            max_tokens: 600,
            max_input_tokens: 4096,
            timeout_seconds: 10,
        },
        preferred_format: Arc::new(std::sync::atomic::AtomicU8::new(0)),
        runtime: Some(runtime.clone()),
    };
    let mem_task = tokio::spawn(async move {
        mem_backend
            .chat_json(
                vec![json!({"role":"user","content":"Teste memória"})],
                json!({"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"]}),
                600,
                0.1,
            )
            .await
    });

    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(server.posts.load(Ordering::SeqCst), 1, "memory job must wait while foreground stream holds permit");

    hold_first.notify_one();
    let (fg_res, mem_res) = tokio::join!(fg_task, mem_task);
    let (_, fg_event) = fg_res.unwrap();
    assert_eq!(fg_event["status"], "complete");
    assert_eq!(fg_event["usage"]["style"]["attempted"], true);
    assert_eq!(fg_event["usage"]["style"]["accepted"], true);
    assert_eq!(mem_res.unwrap().unwrap()["answer"], "ok");
    assert_eq!(server.posts.load(Ordering::SeqCst), 3);
    assert_eq!(server.peak_in_flight.load(Ordering::SeqCst), 1);

    store.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn queued_generation_cancellation_exits_immediately_without_leaking_permit() {
    let hold_first = Arc::new(Notify::new());
    let hold_first_srv = hold_first.clone();
    let started_first = Arc::new(Notify::new());
    let started_first_srv = started_first.clone();

    let server = spawn_mock(move |seq, _body, _cur, _peak| {
        let hold_first_srv = hold_first_srv.clone();
        let started_first_srv = started_first_srv.clone();
        async move {
            if seq == 1 {
                started_first_srv.notify_one();
                hold_first_srv.notified().await;
            }
            sse_reply("*Lia respira fundo.*\n\nLia: Tudo está em ordem por aqui hoje.")
        }
    })
    .await;

    let providers = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 1 }]
        }))
        .unwrap(),
    );
    let runtime = RuntimeProviders::with_default_concurrency(Some(providers), 1);
    let dir = tempfile::tempdir().unwrap();
    let (store, manager) = test_manager(dir.path(), runtime, 4);

    let c1 = store.call(Request::CreateConversation(json!({"routeId":"custom/m","presetId":"generic"}))).await.unwrap();
    let c2 = store.call(Request::CreateConversation(json!({"routeId":"custom/m","presetId":"generic"}))).await.unwrap();
    let c3 = store.call(Request::CreateConversation(json!({"routeId":"custom/m","presetId":"generic"}))).await.unwrap();

    let (g1, _) = manager
        .start(serde_json::from_value(json!({"convId":c1["id"],"mode":"send","content":"1","opId":"q1"})).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), started_first.notified())
        .await
        .unwrap();

    let (g2, _) = manager
        .start(serde_json::from_value(json!({"convId":c2["id"],"mode":"send","content":"2","opId":"q2"})).unwrap())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(server.posts.load(Ordering::SeqCst), 1);

    // Stop queued g2 while g1 holds the semaphore permit -> must finish promptly (<< 35s stop deadline) without hitting upstream
    tokio::time::timeout(
        Duration::from_secs(5),
        manager.stop(c2["id"].as_str().unwrap(), &g2.id),
    )
    .await
    .unwrap()
    .unwrap();
    let g2_status = g2.output.lock().await.final_event.as_ref().unwrap()["status"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        matches!(g2_status.as_str(), "interrupted" | "empty"),
        "unexpected g2 status: {g2_status}"
    );
    assert_eq!(server.posts.load(Ordering::SeqCst), 1);

    // Release g1 -> completes, and g3 succeeds (no leaked permit!)
    hold_first.notify_one();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if g1.output.lock().await.final_event.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let (_, ev3) = finish_generation(&manager, c3["id"].as_str().unwrap(), "3").await;
    assert_eq!(ev3["status"], "complete");
    assert_eq!(server.posts.load(Ordering::SeqCst), 2);

    store.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn max_concurrent_zero_is_unbounded_and_slots_503_preserved() {
    let (hold_tx, hold_rx) = tokio::sync::watch::channel(false);
    let two_arrived = Arc::new(Notify::new());
    let two_arrived_srv = two_arrived.clone();

    let server = spawn_mock(move |seq, _body, _cur, _peak| {
        let mut hold_rx = hold_rx.clone();
        let two_arrived_srv = two_arrived_srv.clone();
        async move {
            if seq == 2 {
                two_arrived_srv.notify_one();
            }
            let _ = hold_rx.wait_for(|v| *v).await;
            sse_reply("*Lia respira fundo.*\n\nLia: Concorrência livre por aqui hoje.")
        }
    })
    .await;

    let providers = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 0 }]
        }))
        .unwrap(),
    );
    let runtime = RuntimeProviders::with_default_concurrency(Some(providers), 1);
    let dir = tempfile::tempdir().unwrap();
    let (store, manager) = test_manager(dir.path(), runtime.clone(), 4);

    let c1 = store.call(Request::CreateConversation(json!({"routeId":"custom/m","presetId":"generic"}))).await.unwrap();
    let c2 = store.call(Request::CreateConversation(json!({"routeId":"custom/m","presetId":"generic"}))).await.unwrap();
    let t1 = tokio::spawn({
        let manager = manager.clone();
        let id = c1["id"].as_str().unwrap().to_owned();
        async move { finish_generation(&manager, &id, "1").await }
    });
    let t2 = tokio::spawn({
        let manager = manager.clone();
        let id = c2["id"].as_str().unwrap().to_owned();
        async move { finish_generation(&manager, &id, "2").await }
    });
    tokio::time::timeout(Duration::from_secs(3), two_arrived.notified())
        .await
        .unwrap();
    assert_eq!(server.peak_in_flight.load(Ordering::SeqCst), 2);
    let _ = hold_tx.send(true);
    let (r1, r2) = tokio::join!(t1, t2);
    let (_, e1) = r1.unwrap();
    let (_, e2) = r2.unwrap();
    assert_eq!(e1["status"], "complete");
    assert_eq!(e2["status"], "complete");
    store.call(Request::Close).await.unwrap();

    // Verify Manager::slots 503 GENERATION_OVERLOADED contract preserved (with a held stream)
    let (hold2_tx, hold2_rx) = tokio::sync::watch::channel(false);
    let started2 = Arc::new(Notify::new());
    let started2_srv = started2.clone();
    let server2 = spawn_mock(move |_seq, _body, _cur, _peak| {
        let mut hold2_rx = hold2_rx.clone();
        let started2_srv = started2_srv.clone();
        async move {
            started2_srv.notify_one();
            let _ = hold2_rx.wait_for(|v| *v).await;
            sse_reply("*Lia respira fundo.*\n\nLia: Concluído.")
        }
    })
    .await;
    let providers2 = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server2.url, "maxConcurrent": 1 }]
        }))
        .unwrap(),
    );
    let runtime2 = RuntimeProviders::with_default_concurrency(Some(providers2), 1);
    let dir2 = tempfile::tempdir().unwrap();
    let (store2, manager2) = test_manager(dir2.path(), runtime2, 1);
    let ca = store2.call(Request::CreateConversation(json!({"routeId":"custom/m","presetId":"generic"}))).await.unwrap();
    let cb = store2.call(Request::CreateConversation(json!({"routeId":"custom/m","presetId":"generic"}))).await.unwrap();
    let g1 = manager2
        .start(serde_json::from_value(json!({"convId":ca["id"],"mode":"send","content":"a","opId":"s1"})).unwrap())
        .await
        .unwrap()
        .0;
    tokio::time::timeout(Duration::from_secs(3), started2.notified())
        .await
        .unwrap();
    let Err(err) = manager2
        .start(serde_json::from_value(json!({"convId":cb["id"],"mode":"send","content":"b","opId":"s2"})).unwrap())
        .await
    else {
        panic!("expected 503 GENERATION_OVERLOADED");
    };
    assert_eq!(err.status, 503);
    assert_eq!(err.extra["code"], "GENERATION_OVERLOADED");
    let _ = hold2_tx.send(true);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if g1.output.lock().await.final_event.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    store2.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn classify_token_limit_and_429_rate_limit_no_pruning_or_muse_fallback() {
    // Unit matrix for classify_token_limit
    for (status, body) in [
        (StatusCode::TOO_MANY_REQUESTS, r#"{"error":{"message":"Rate limit exceeded: too many tokens per minute"}}"#),
        (StatusCode::TOO_MANY_REQUESTS, "tokens per min (TPM) exceeded"),
        (StatusCode::TOO_MANY_REQUESTS, "maximum context length"),
        (StatusCode::UNAUTHORIZED, "invalid token"),
        (StatusCode::FORBIDDEN, "Quota exceeded: token limit exceeded"),
        (StatusCode::BAD_GATEWAY, "rate limit token"),
        (StatusCode::SERVICE_UNAVAILABLE, "token limit exceeded"),
        (StatusCode::BAD_REQUEST, "boom"),
        (StatusCode::BAD_REQUEST, "physical context window exceeded"),
        (StatusCode::BAD_REQUEST, "Rate limit reached on tokens per min (TPM)"),
        (StatusCode::BAD_REQUEST, "insufficient_quota: token limit exceeded"),
    ] {
        assert!(!classify_token_limit(status, body), "must be false for {status} {body:?}");
    }
    for (status, body) in [
        (StatusCode::BAD_REQUEST, "This model's maximum context length is 128000 tokens"),
        (StatusCode::UNPROCESSABLE_ENTITY, "prompt is too long: 210000 tokens"),
        (StatusCode::BAD_REQUEST, "Prompt tokens limit exceeded"),
        (StatusCode::PAYLOAD_TOO_LARGE, ""),
        (StatusCode::PAYMENT_REQUIRED, "This request requires more credits, or fewer max_tokens."),
    ] {
        assert!(classify_token_limit(status, body), "must be true for {status} {body:?}");
    }

    // End-to-end: 429 TPM -> 1 POST only, explicit error; 400 context_length on muse route -> never switches model
    let server = spawn_mock(move |seq, body, _cur, _peak| async move {
        if body["model"] == "custom/rate-limited" {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                r#"{"error":{"message":"Rate limit exceeded: too many tokens per minute (TPM)"}}"#,
            )
                .into_response();
        }
        if seq == 3 {
            return (
                StatusCode::BAD_REQUEST,
                r#"{"error":{"message":"This model's maximum context length is 128000 tokens"}}"#,
            )
                .into_response();
        }
        sse_reply("*Lia respira fundo.*\n\nLia: Modelo muse preservado por aqui hoje.")
    })
    .await;

    let providers = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 1 }]
        }))
        .unwrap(),
    );
    let runtime = RuntimeProviders::with_default_concurrency(Some(providers), 1);
    let dir = tempfile::tempdir().unwrap();
    let (store, manager) = test_manager(dir.path(), runtime, 4);

    let c_rate = store.call(Request::CreateConversation(json!({"routeId":"custom/rate-limited","presetId":"generic"}))).await.unwrap();
    let (_, ev_rate) = finish_generation(&manager, c_rate["id"].as_str().unwrap(), "oi").await;
    assert_eq!(ev_rate["status"], "error");
    assert!(ev_rate["error"].as_str().unwrap().contains("HTTP 429"));
    assert_eq!(server.posts.load(Ordering::SeqCst), 1, "429 TPM must NOT trigger retry/pruning");

    // Muse-named custom route with a prior turn so pruning + max_tokens reduction run on 400 context_length
    let c_muse = store.call(Request::CreateConversation(json!({"routeId":"custom/muse-alpha","presetId":"muse-spark-13"}))).await.unwrap();
    let (_, ev_seed) = finish_generation(&manager, c_muse["id"].as_str().unwrap(), "turno inicial").await;
    assert_eq!(ev_seed["status"], "complete");

    let (_, ev_muse) = finish_generation(&manager, c_muse["id"].as_str().unwrap(), "oi muse").await;
    assert_eq!(ev_muse["status"], "complete");
    let bodies = server.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 4);
    assert_eq!(bodies[2]["model"], "custom/muse-alpha");
    assert_eq!(bodies[3]["model"], "custom/muse-alpha", "must NEVER fall back to oc/muse-spark-1.3-contributor-free");

    store.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn empty_streaming_response_has_bounded_zero_retries_and_releases_permit() {
    let server = spawn_mock(move |seq, _body, _cur, _peak| async move {
        if seq == 1 {
            let body = format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"delta":{},"finish_reason":"stop"}]})
            );
            return ([("content-type", "text/event-stream")], body).into_response();
        }
        sse_reply("*Lia respira fundo.*\n\nLia: Próxima resposta está em ordem hoje.")
    })
    .await;

    let providers = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 1 }]
        }))
        .unwrap(),
    );
    let runtime = RuntimeProviders::with_default_concurrency(Some(providers), 1);
    let dir = tempfile::tempdir().unwrap();
    let (store, manager) = test_manager(dir.path(), runtime, 4);

    let c1 = store.call(Request::CreateConversation(json!({"routeId":"custom/m","presetId":"generic"}))).await.unwrap();
    let (_, ev1) = finish_generation(&manager, c1["id"].as_str().unwrap(), "oi").await;
    assert_eq!(ev1["status"], "empty");
    assert_eq!(ev1["error"], "O modelo retornou uma resposta vazia.");
    assert_eq!(server.posts.load(Ordering::SeqCst), 1, "empty SSE stream must make <= 1 attempt (0 retries)");

    // Permit must be released cleanly so next generation succeeds
    let c2 = store.call(Request::CreateConversation(json!({"routeId":"custom/m","presetId":"generic"}))).await.unwrap();
    let (_, ev2) = finish_generation(&manager, c2["id"].as_str().unwrap(), "oi 2").await;
    assert_eq!(ev2["status"], "complete");
    assert_eq!(server.posts.load(Ordering::SeqCst), 2);
    assert_eq!(server.peak_in_flight.load(Ordering::SeqCst), 1);

    store.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn preflight_cache_positive_only_ttl_and_invalidation_on_set() {
    let models_ok = Arc::new(AtomicUsize::new(0));
    let models_ok_srv = models_ok.clone();
    let gets = Arc::new(AtomicUsize::new(0));
    let gets_srv = gets.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let app = Router::new().route(
        "/v1/models",
        get(move || {
            let gets_srv = gets_srv.clone();
            let models_ok_srv = models_ok_srv.clone();
            async move {
                gets_srv.fetch_add(1, Ordering::SeqCst);
                if models_ok_srv.load(Ordering::SeqCst) == 0 {
                    (StatusCode::SERVICE_UNAVAILABLE, "booting").into_response()
                } else {
                    Json(json!({"data":[{"id":"Custom/Model"}]})).into_response()
                }
            }
        }),
    );
    let srv = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let p1 = Provider {
        id: "p1".into(),
        base_url: url.clone(),
        api_key: Some("key-1".into()),
        models: vec![],
        request_log: None,
        reasoning_format: ReasoningFormat::Omit,
        context_window: 128000,
        max_output: 8192,
        max_concurrent: Some(1),
    };
    let mut p1_slash_lower = p1.clone();
    p1_slash_lower.base_url = format!("{}/", url);
    let mut p1_other_key = p1.clone();
    p1_other_key.api_key = Some("key-2".into());

    let runtime = RuntimeProviders::with_default_concurrency(None, 1);

    // Negative result must NOT be cached
    assert!(!runtime.preflight(&p1, "custom/model").await);
    assert_eq!(gets.load(Ordering::SeqCst), 1);

    models_ok.store(1, Ordering::SeqCst);
    assert!(runtime.preflight(&p1, "custom/model").await, "must recover immediately without negative cache");
    assert_eq!(gets.load(Ordering::SeqCst), 2);

    // Rapid repeated calls + trailing slash + case-insensitive model hit cache -> 0 new GETs
    assert!(runtime.preflight(&p1, "Custom/Model").await);
    assert!(runtime.preflight(&p1_slash_lower, "CUSTOM/MODEL").await);
    assert_eq!(gets.load(Ordering::SeqCst), 2);

    // Different API key misses cache
    assert!(runtime.preflight(&p1_other_key, "custom/model").await);
    assert_eq!(gets.load(Ordering::SeqCst), 3);

    // RuntimeProviders::set clears PreflightCache immediately
    runtime.set(None);
    assert!(runtime.preflight(&p1, "custom/model").await);
    assert_eq!(gets.load(Ordering::SeqCst), 4);

    assert_eq!(
        normalize_base_url("HTTP://127.0.0.1:20128/v1/"),
        "http://127.0.0.1:20128/v1"
    );
    assert_eq!(
        normalize_base_url("https://openrouter.ai:443/api/v1/"),
        "https://openrouter.ai/api/v1"
    );

    srv.abort();
}

#[test]
fn context_default_cap_and_explicit_override() {
    let conv_1m = json!({
        "id": "c1",
        "presetId": "gemini-4-hydrogen",
        "routeId": "ag/gemini-4-hydrogen-high",
        "persona": "Alex",
        "prompt": "Ficha curta.",
        "settings": {}
    });
    let plan = presets::request(&conv_1m, vec![], &json!({}));
    assert_eq!(plan.capacity["contextWindow"], 1_000_000);
    let data = json!({
        "conv": conv_1m,
        "current": { "seq": 1, "userText": "Olá." },
        "pairs": [],
        "globalSettings": {}
    });
    let built = context::assemble(&data, &json!({}), "send", &plan, None).unwrap();
    assert_eq!(built.report["limits"]["hard"], 128_000);
    assert_eq!(built.report["limits"]["soft"], 108_800);
    assert_eq!(built.report["limits"]["target"], 94_656);
    assert_eq!(built.report["limits"]["capacity"], 1_000_000);

    // Explicit user override up to input_capacity is preserved; beyond input_capacity clamps
    let mut data_override = data.clone();
    data_override["conv"]["settings"]["context"]["hardLimit"] = json!(400_000);
    let built_override = context::assemble(&data_override, &json!({}), "send", &plan, None).unwrap();
    assert_eq!(built_override.report["limits"]["hard"], 400_000);
    assert_eq!(built_override.report["limits"]["soft"], 340_000);

    data_override["conv"]["settings"]["context"]["hardLimit"] = json!(2_000_000);
    let built_clamped = context::assemble(&data_override, &json!({}), "send", &plan, None).unwrap();
    let input_cap = 1_000_000u64 - (plan.body["max_tokens"].as_u64().unwrap() + 64);
    assert_eq!(built_clamped.report["limits"]["hard"], input_cap);

    // Compact mobile preset stays capped at input_capacity
    let conv_mobile = json!({
        "id": "c2",
        "presetId": "mobile-local",
        "routeId": "mobile-local",
        "persona": "Alex",
        "prompt": "Ficha.",
        "settings": {}
    });
    let plan_mobile = presets::mobile_local_plan(&conv_mobile, &json!({}));
    let data_mobile = json!({
        "conv": conv_mobile,
        "current": { "seq": 1, "userText": "Oi." },
        "pairs": [],
        "globalSettings": {}
    });
    let built_mobile = context::assemble(&data_mobile, &json!({}), "send", &plan_mobile, None).unwrap();
    assert!(built_mobile.report["limits"]["hard"].as_u64().unwrap() <= 4096);

    // retrieval_window_start_for aligns to 128k/108.8k on 1M preset when hardLimit is omitted:
    // 30 pairs * ~5k tokens = ~150k tokens (> 108.8k softLimit, < 850k old 1M softLimit) -> steps down to 20 pairs (seq 11)
    let pair_text = "palavra ".repeat(930); // ~7440 bytes -> ~4968 tokens per pair
    let pairs: Vec<Value> = (1..=30)
        .map(|seq| {
            json!({
                "seq": seq,
                "user": &pair_text,
                "assistant": &pair_text
            })
        })
        .collect();
    let data_ret = json!({
        "conv": {
            "presetId": "gemini-4-hydrogen",
            "settings": {}
        },
        "pairs": pairs,
        "current": { "seq": 31 }
    });
    assert_eq!(
        context::retrieval_window_start_for(&data_ret, false),
        11,
        "1M preset without explicit hardLimit must compact retrieval window at 108.8k, not 850k"
    );

    // Isolated subprocess verification for MC_DEFAULT_CONTEXT_HARD_LIMIT
    for (val, expected) in [("64000", "64000"), ("0", "128000"), ("abc", "128000")] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "context_hard_limit_env_fixture",
                "--ignored",
            ])
            .env_clear()
            .env("MC_DEFAULT_CONTEXT_HARD_LIMIT", val)
            .env("MC_EXPECTED_HARD_LIMIT", expected);
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
fn context_hard_limit_env_fixture() {
    let expected: u64 = std::env::var("MC_EXPECTED_HARD_LIMIT").unwrap().parse().unwrap();
    assert_eq!(default_context_hard_limit(), expected);
}

#[test]
fn retry_after_header_parsing_supports_seconds_and_http_dates() {
    let mut h = HeaderMap::new();
    assert_eq!(parse_retry_after(&h), None);

    h.insert("retry-after", "0".parse().unwrap());
    assert_eq!(parse_retry_after(&h), Some(1));

    h.insert("retry-after", "45".parse().unwrap());
    assert_eq!(parse_retry_after(&h), Some(45));

    h.insert("retry-after", "99999".parse().unwrap());
    assert_eq!(parse_retry_after(&h), Some(900));

    let future = (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc2822();
    h.insert("retry-after", future.parse().unwrap());
    let parsed = parse_retry_after(&h).unwrap();
    assert!((55..=65).contains(&parsed), "got {parsed}");

    h.insert("retry-after", "not-a-date".parse().unwrap());
    assert_eq!(parse_retry_after(&h), None);
}

#[tokio::test]
async fn chat_json_classifies_transient_retry_after_and_auth_config_errors() {
    for (status, retry_after, expected_kind) in [
        (StatusCode::TOO_MANY_REQUESTS, Some("42"), "transient"),
        (StatusCode::SERVICE_UNAVAILABLE, None, "transient"),
        (StatusCode::UNAUTHORIZED, None, "auth_config"),
        (StatusCode::FORBIDDEN, None, "auth_config"),
        (StatusCode::NOT_FOUND, None, "auth_config"),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move || async move {
                if let Some(ra) = retry_after {
                    (status, [("retry-after", ra)], r#"{"error":{"message":"err"}}"#).into_response()
                } else {
                    (status, r#"{"error":{"message":"err"}}"#).into_response()
                }
            }),
        );
        let srv = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let provider = Provider {
            id: "mock".into(),
            base_url: url,
            api_key: Some("k".into()),
            models: vec![],
            request_log: None,
            reasoning_format: ReasoningFormat::Omit,
            context_window: 128000,
            max_output: 8192,
            max_concurrent: Some(1),
        };
        let backend = MemoryBackend::Api {
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            provider,
            api: MemoryApi {
                provider: "mock".into(),
                model: "custom/m".into(),
                history_consent: true,
                max_tokens: 600,
                max_input_tokens: 4096,
                timeout_seconds: 5,
            },
            preferred_format: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            runtime: None,
        };
        let err = backend
            .chat_json(
                vec![json!({"role":"user","content":"teste"})],
                json!({"type":"object","properties":{"a":{"type":"string"}},"required":["a"]}),
                600,
                0.1,
            )
            .await
            .unwrap_err();
        assert_eq!(err.status, 503);
        assert_eq!(err.extra["code"], "MEMORY_BACKEND_UNAVAILABLE");
        assert_eq!(err.extra["unavailableKind"], expected_kind);
        assert_eq!(err.extra["upstreamStatus"], status.as_u16());
        if let Some(ra) = retry_after {
            assert_eq!(err.extra["retryAfterSeconds"], ra.parse::<u64>().unwrap());
        }
        srv.abort();
    }
}

#[tokio::test]
async fn dynamic_lowering_2_to_1_via_set_and_http_put_preserves_mutual_exclusion() {
    // Part 1: RuntimeProviders::set lowering 2 -> 1 while 2 streams are in-flight
    let (hold_1_tx, hold_1_rx) = tokio::sync::watch::channel(false);
    let (hold_2_tx, hold_2_rx) = tokio::sync::watch::channel(false);
    let (hold_4_tx, hold_4_rx) = tokio::sync::watch::channel(false);
    let (started_1_tx, mut started_1_rx) = tokio::sync::watch::channel(false);
    let (started_2_tx, mut started_2_rx) = tokio::sync::watch::channel(false);
    let (started_4_tx, mut started_4_rx) = tokio::sync::watch::channel(false);

    let server = spawn_mock(move |_seq, body, _cur, _peak| {
        let mut hold_1_rx = hold_1_rx.clone();
        let mut hold_2_rx = hold_2_rx.clone();
        let mut hold_4_rx = hold_4_rx.clone();
        let started_1_tx = started_1_tx.clone();
        let started_2_tx = started_2_tx.clone();
        let started_4_tx = started_4_tx.clone();
        async move {
            let prompt = body["messages"]
                .as_array()
                .and_then(|arr| arr.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or_default()
                .to_owned();
            if prompt.starts_with("slot-1") {
                let _ = started_1_tx.send(true);
                let _ = hold_1_rx.wait_for(|v| *v).await;
            } else if prompt.starts_with("slot-2") {
                let _ = started_2_tx.send(true);
                let _ = hold_2_rx.wait_for(|v| *v).await;
            } else if prompt.starts_with("slot-4") {
                let _ = started_4_tx.send(true);
                let _ = hold_4_rx.wait_for(|v| *v).await;
            }
            sse_reply("*Lia respira fundo.*\n\nLia: Resposta sequencial segura.")
        }
    })
    .await;

    let providers_2 = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 2 }]
        }))
        .unwrap(),
    );
    let providers_1 = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 1 }]
        }))
        .unwrap(),
    );

    let runtime = RuntimeProviders::with_default_concurrency(Some(providers_2), 1);
    let dir = tempfile::tempdir().unwrap();
    let (store, manager) = test_manager(dir.path(), runtime.clone(), 4);

    let mut conv_ids = Vec::new();
    for _ in 0..5 {
        let c = store
            .call(Request::CreateConversation(json!({
                "routeId": "custom/m",
                "presetId": "generic"
            })))
            .await
            .unwrap();
        conv_ids.push(c["id"].as_str().unwrap().to_owned());
    }

    let t1 = tokio::spawn({
        let manager = manager.clone();
        let id = conv_ids[0].clone();
        async move { finish_generation(&manager, &id, "slot-1").await }
    });
    let t2 = tokio::spawn({
        let manager = manager.clone();
        let id = conv_ids[1].clone();
        async move { finish_generation(&manager, &id, "slot-2").await }
    });

    tokio::time::timeout(Duration::from_secs(3), async {
        let _ = started_1_rx.wait_for(|v| *v).await;
        let _ = started_2_rx.wait_for(|v| *v).await;
    })
    .await
    .unwrap();
    assert_eq!(server.posts.load(Ordering::SeqCst), 2);
    assert_eq!(server.peak_in_flight.load(Ordering::SeqCst), 2);

    // Lower maxConcurrent 2 -> 1 while both streams are still in-flight
    runtime.set(Some(providers_1));

    let t3 = tokio::spawn({
        let manager = manager.clone();
        let id = conv_ids[2].clone();
        async move { finish_generation(&manager, &id, "slot-3").await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        server.posts.load(Ordering::SeqCst),
        2,
        "3rd request must block while 2 active streams exceed new limit=1"
    );
    assert_eq!(server.peak_in_flight.load(Ordering::SeqCst), 2);

    // Drop ONLY the 1st stream -> 1 stream still in-flight (1 < 1 is false), so 3rd MUST STILL BLOCK!
    let _ = hold_1_tx.send(true);
    let (_, ev1) = t1.await.unwrap();
    assert_eq!(ev1["status"], "complete");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        server.posts.load(Ordering::SeqCst),
        2,
        "3rd request must stay blocked until BOTH active permits drop (in_flight == 0)"
    );
    assert_eq!(server.peak_in_flight.load(Ordering::SeqCst), 2);

    // Drop the 2nd stream -> 3rd enters and completes
    let _ = hold_2_tx.send(true);
    let (_, ev2) = t2.await.unwrap();
    let (_, ev3) = t3.await.unwrap();
    assert_eq!(ev2["status"], "complete");
    assert_eq!(ev3["status"], "complete");
    assert_eq!(server.posts.load(Ordering::SeqCst), 3);
    assert_eq!(server.peak_in_flight.load(Ordering::SeqCst), 2);

    // Subsequent requests run strictly 1 at a time
    server.peak_in_flight.store(0, Ordering::SeqCst);
    let t4 = tokio::spawn({
        let manager = manager.clone();
        let id = conv_ids[3].clone();
        async move { finish_generation(&manager, &id, "slot-4").await }
    });
    tokio::time::timeout(Duration::from_secs(3), started_4_rx.wait_for(|v| *v))
        .await
        .unwrap()
        .unwrap();
    let t5 = tokio::spawn({
        let manager = manager.clone();
        let id = conv_ids[4].clone();
        async move { finish_generation(&manager, &id, "slot-5").await }
    });
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(server.posts.load(Ordering::SeqCst), 4);
    assert_eq!(server.peak_in_flight.load(Ordering::SeqCst), 1);

    let _ = hold_4_tx.send(true);
    let (_, ev4) = t4.await.unwrap();
    let (_, ev5) = t5.await.unwrap();
    assert_eq!(ev4["status"], "complete");
    assert_eq!(ev5["status"], "complete");
    assert_eq!(server.posts.load(Ordering::SeqCst), 5);
    assert_eq!(server.peak_in_flight.load(Ordering::SeqCst), 1);

    store.call(Request::Close).await.unwrap();

    // Part 2: End-to-end via PUT /api/local/providers + POST /api/chats/:id/message
    let (h1_hold_tx, h1_hold_rx) = tokio::sync::watch::channel(false);
    let (h2_hold_tx, h2_hold_rx) = tokio::sync::watch::channel(false);
    let (h1_started_tx, mut h1_started_rx) = tokio::sync::watch::channel(false);
    let (h2_started_tx, mut h2_started_rx) = tokio::sync::watch::channel(false);

    let http_mock = spawn_mock(move |_seq, body, _cur, _peak| {
        let mut h1_hold_rx = h1_hold_rx.clone();
        let mut h2_hold_rx = h2_hold_rx.clone();
        let h1_started_tx = h1_started_tx.clone();
        let h2_started_tx = h2_started_tx.clone();
        async move {
            let prompt = body["messages"]
                .as_array()
                .and_then(|arr| arr.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or_default()
                .to_owned();
            if prompt.starts_with("http-1") {
                let _ = h1_started_tx.send(true);
                let _ = h1_hold_rx.wait_for(|v| *v).await;
            } else if prompt.starts_with("http-2") {
                let _ = h2_started_tx.send(true);
                let _ = h2_hold_rx.wait_for(|v| *v).await;
            }
            sse_reply("*Lia respira fundo.*\n\nLia: Fluxo HTTP validado com sucesso.")
        }
    })
    .await;

    let app_dir = tempfile::tempdir().unwrap();
    let native_dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::for_android(app_dir.path(), native_dir.path()).unwrap();
    cfg.address = "127.0.0.1:0".parse().unwrap();
    cfg.max_generations = 4;
    let state = AppState::new(cfg).await.unwrap();
    state.runner.stop();
    let token = "m2_http_lowering_token_0123456789abcdef0123456789abcdef";
    state
        .store
        .call(Request::AddDevice {
            name: "M2 regression".into(),
            token_hash: rules::hash(token),
        })
        .await
        .unwrap();

    let put_req = |max_concurrent: usize| {
        HttpRequest::builder()
            .method("PUT")
            .uri("/api/local/providers")
            .extension(ConnectInfo("127.0.0.1:0".parse::<SocketAddr>().unwrap()))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "default": "mock",
                    "providers": [{
                        "id": "mock",
                        "baseUrl": http_mock.url,
                        "models": ["custom/model-alpha"],
                        "allowLoopback": true,
                        "maxConcurrent": max_concurrent
                    }]
                })
                .to_string(),
            ))
            .unwrap()
    };

    let resp = router(state.clone()).oneshot(put_req(2)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let mut chat_ids = Vec::new();
    for _ in 0..3 {
        let req = HttpRequest::builder()
            .method("POST")
            .uri("/api/chats")
            .extension(ConnectInfo("127.0.0.1:0".parse::<SocketAddr>().unwrap()))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "routeId": "mobile-api:mock:custom/model-alpha",
                    "presetId": "generic"
                })
                .to_string(),
            ))
            .unwrap();
        let r = router(state.clone()).oneshot(req).await.unwrap();
        assert_eq!(r.status(), StatusCode::CREATED);
        let v: Value = serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
        chat_ids.push(v["id"].as_str().unwrap().to_owned());
    }

    let send_msg = |state: AppState, chat_id: String, content: String, op_id: String| async move {
        let req = HttpRequest::builder()
            .method("POST")
            .uri(format!("/api/chats/{chat_id}/message"))
            .extension(ConnectInfo("127.0.0.1:0".parse::<SocketAddr>().unwrap()))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "mode": "send",
                    "content": content,
                    "opId": op_id
                })
                .to_string(),
            ))
            .unwrap();
        let r = router(state).oneshot(req).await.unwrap();
        let status = r.status();
        let bytes = r.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    };

    let s1 = tokio::spawn(send_msg(state.clone(), chat_ids[0].clone(), "http-1".into(), "h1".into()));
    let s2 = tokio::spawn(send_msg(state.clone(), chat_ids[1].clone(), "http-2".into(), "h2".into()));
    tokio::time::timeout(Duration::from_secs(3), async {
        let _ = h1_started_rx.wait_for(|v| *v).await;
        let _ = h2_started_rx.wait_for(|v| *v).await;
    })
    .await
    .unwrap();
    assert_eq!(http_mock.peak_in_flight.load(Ordering::SeqCst), 2);

    let resp = router(state.clone()).oneshot(put_req(1)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let s3 = tokio::spawn(send_msg(state.clone(), chat_ids[2].clone(), "http-3".into(), "h3".into()));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(http_mock.posts.load(Ordering::SeqCst), 2);
    assert_eq!(http_mock.peak_in_flight.load(Ordering::SeqCst), 2);

    let _ = h1_hold_tx.send(true);
    let (st1, _) = s1.await.unwrap();
    assert_eq!(st1, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        http_mock.posts.load(Ordering::SeqCst),
        2,
        "HTTP 3rd message must remain queued while 2nd stream is still active"
    );

    let _ = h2_hold_tx.send(true);
    let (st2, _) = s2.await.unwrap();
    let (st3, body3) = s3.await.unwrap();
    assert_eq!(st2, StatusCode::OK);
    assert_eq!(st3, StatusCode::OK);
    assert!(body3.contains("\"status\":\"complete\""), "body3={body3}");
    assert_eq!(http_mock.posts.load(Ordering::SeqCst), 3);
    assert_eq!(http_mock.peak_in_flight.load(Ordering::SeqCst), 2);

    state.shutdown().await;
}

#[tokio::test]
async fn dynamic_1_to_3_wakes_queued_callers_immediately_in_fifo_order() {
    let (hold_tx, hold_rx) = tokio::sync::watch::channel(false);
    let first_started = Arc::new(Notify::new());
    let first_started_srv = first_started.clone();
    let three_in_flight = Arc::new(Notify::new());
    let three_in_flight_srv = three_in_flight.clone();
    let arrival_order = Arc::new(Mutex::new(Vec::<String>::new()));
    let arrival_srv = arrival_order.clone();

    let server = spawn_mock(move |_seq, body, cur, _peak| {
        let mut hold_rx = hold_rx.clone();
        let first_started_srv = first_started_srv.clone();
        let three_in_flight_srv = three_in_flight_srv.clone();
        let arrival_srv = arrival_srv.clone();
        async move {
            let prompt = body["messages"]
                .as_array()
                .and_then(|arr| arr.last())
                .and_then(|m| m["content"].as_str())
                .and_then(|s| s.lines().next())
                .unwrap_or_default()
                .to_owned();
            arrival_srv.lock().unwrap().push(prompt);
            let now_cur = cur.load(Ordering::SeqCst);
            if now_cur == 1 {
                first_started_srv.notify_one();
            } else if now_cur == 3 {
                three_in_flight_srv.notify_one();
            }
            let _ = hold_rx.wait_for(|v| *v).await;
            sse_reply("*Lia respira fundo.*\n\nLia: Ordem FIFO confirmada aqui.")
        }
    })
    .await;

    let providers_1 = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 1 }]
        }))
        .unwrap(),
    );
    let providers_3 = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 3 }]
        }))
        .unwrap(),
    );
    let runtime = RuntimeProviders::with_default_concurrency(Some(providers_1), 1);
    let dir = tempfile::tempdir().unwrap();
    let (store, manager) = test_manager(dir.path(), runtime.clone(), 4);

    let mut convs = Vec::new();
    for _ in 0..4 {
        let c = store
            .call(Request::CreateConversation(json!({
                "routeId": "custom/m",
                "presetId": "generic"
            })))
            .await
            .unwrap();
        convs.push(c["id"].as_str().unwrap().to_owned());
    }

    let t1 = tokio::spawn({
        let manager = manager.clone();
        let id = convs[0].clone();
        async move { finish_generation(&manager, &id, "turn-1").await }
    });
    tokio::time::timeout(Duration::from_secs(3), first_started.notified())
        .await
        .unwrap();

    let t2 = tokio::spawn({
        let manager = manager.clone();
        let id = convs[1].clone();
        async move { finish_generation(&manager, &id, "turn-2").await }
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    let t3 = tokio::spawn({
        let manager = manager.clone();
        let id = convs[2].clone();
        async move { finish_generation(&manager, &id, "turn-3").await }
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    let t4 = tokio::spawn({
        let manager = manager.clone();
        let id = convs[3].clone();
        async move { finish_generation(&manager, &id, "turn-4").await }
    });
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(server.posts.load(Ordering::SeqCst), 1);

    // Raise 1 -> 3 while t1 is held: t2 and t3 must immediately wake in FIFO order; t4 stays queued
    runtime.set(Some(providers_3));
    tokio::time::timeout(Duration::from_secs(3), three_in_flight.notified())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(server.posts.load(Ordering::SeqCst), 3);
    assert_eq!(server.peak_in_flight.load(Ordering::SeqCst), 3);
    assert_eq!(
        *arrival_order.lock().unwrap(),
        vec!["turn-1", "turn-2", "turn-3"]
    );

    let _ = hold_tx.send(true);
    for t in [t1, t2, t3, t4] {
        let (_, ev) = t.await.unwrap();
        assert_eq!(ev["status"], "complete");
    }
    assert_eq!(server.posts.load(Ordering::SeqCst), 4);
    assert_eq!(server.peak_in_flight.load(Ordering::SeqCst), 3);

    store.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn dynamic_1_to_0_to_1_tracks_unbounded_in_flight_permits() {
    let url = "http://127.0.0.1:20128/v1";
    let providers_1 = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": url, "maxConcurrent": 1 }]
        }))
        .unwrap(),
    );
    let providers_0 = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": url, "maxConcurrent": 0 }]
        }))
        .unwrap(),
    );
    let runtime = RuntimeProviders::with_default_concurrency(Some(providers_1.clone()), 1);
    let p = providers_1.default_provider().clone();

    let permit1 = runtime
        .acquire_outbound(&p, Duration::from_secs(2))
        .await
        .unwrap()
        .expect("permit1");

    // Switch 1 -> 0: unbounded requests succeed immediately while permit1 is still held
    runtime.set(Some(providers_0));
    let permit2 = tokio::time::timeout(
        Duration::from_secs(1),
        runtime.acquire_outbound(&p, Duration::from_secs(2)),
    )
    .await
    .unwrap()
    .unwrap()
    .expect("permit2");
    let permit3 = tokio::time::timeout(
        Duration::from_secs(1),
        runtime.acquire_outbound(&p, Duration::from_secs(2)),
    )
    .await
    .unwrap()
    .unwrap()
    .expect("permit3");

    drop(permit1);

    // Switch 0 -> 1 while permit2 & permit3 are STILL in-flight (in_flight == 2)
    runtime.set(Some(providers_1));

    let acquired4 = Arc::new(AtomicUsize::new(0));
    let acquired4_task = acquired4.clone();
    let rt4 = runtime.clone();
    let p4 = p.clone();
    let t4 = tokio::spawn(async move {
        let permit4 = rt4
            .acquire_outbound(&p4, Duration::from_secs(5))
            .await
            .unwrap()
            .expect("permit4");
        acquired4_task.store(1, Ordering::SeqCst);
        drop(permit4);
    });

    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(acquired4.load(Ordering::SeqCst), 0, "must wait while 2 unbounded permits are in-flight");

    drop(permit2);
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(acquired4.load(Ordering::SeqCst), 0, "must wait until remaining unbounded permit drops (in_flight == 0)");

    drop(permit3);
    tokio::time::timeout(Duration::from_secs(2), t4)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(acquired4.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn effective_limit_omitted_inherits_default_even_when_sibling_is_zero() {
    let hold_shared = Arc::new(Notify::new());
    let hold_shared_srv = hold_shared.clone();
    let started_shared = Arc::new(Notify::new());
    let started_shared_srv = started_shared.clone();

    let server_shared = spawn_mock(move |seq, _body, _cur, _peak| {
        let hold_shared_srv = hold_shared_srv.clone();
        let started_shared_srv = started_shared_srv.clone();
        async move {
            if seq == 1 {
                started_shared_srv.notify_one();
                hold_shared_srv.notified().await;
            }
            sse_reply("*Lia respira fundo.*\n\nLia: Compartilhado serializado corretamente.")
        }
    })
    .await;

    let providers_shared = Arc::new(
        Providers::from_json(&json!({
            "default": "p2",
            "providers": [
                { "id": "p1", "baseUrl": server_shared.url, "maxConcurrent": 0 },
                { "id": "p2", "baseUrl": format!("{}/", server_shared.url) }
            ]
        }))
        .unwrap(),
    );
    let runtime_shared = RuntimeProviders::with_default_concurrency(Some(providers_shared), 1);
    let dir1 = tempfile::tempdir().unwrap();
    let (store1, manager1) = test_manager(dir1.path(), runtime_shared, 4);

    let c1 = store1
        .call(Request::CreateConversation(json!({
            "routeId": "p2:custom/m1",
            "presetId": "generic"
        })))
        .await
        .unwrap();
    let c2 = store1
        .call(Request::CreateConversation(json!({
            "routeId": "p2:custom/m2",
            "presetId": "generic"
        })))
        .await
        .unwrap();

    let t1 = tokio::spawn({
        let manager = manager1.clone();
        let id = c1["id"].as_str().unwrap().to_owned();
        async move { finish_generation(&manager, &id, "1").await }
    });
    tokio::time::timeout(Duration::from_secs(3), started_shared.notified())
        .await
        .unwrap();

    let t2 = tokio::spawn({
        let manager = manager1.clone();
        let id = c2["id"].as_str().unwrap().to_owned();
        async move { finish_generation(&manager, &id, "2").await }
    });
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(
        server_shared.posts.load(Ordering::SeqCst),
        1,
        "p2 (omitted) must inherit default=1 even when sibling p1 sets maxConcurrent=0"
    );
    assert_eq!(server_shared.peak_in_flight.load(Ordering::SeqCst), 1);

    hold_shared.notify_one();
    let (_, ev1) = t1.await.unwrap();
    let (_, ev2) = t2.await.unwrap();
    assert_eq!(ev1["status"], "complete");
    assert_eq!(ev2["status"], "complete");
    assert_eq!(server_shared.posts.load(Ordering::SeqCst), 2);
    assert_eq!(server_shared.peak_in_flight.load(Ordering::SeqCst), 1);
    store1.call(Request::Close).await.unwrap();

    // Single p1 (maxConcurrent: 0) on its own base_url runs concurrently
    let (hold_solo_tx, hold_solo_rx) = tokio::sync::watch::channel(false);
    let two_solo = Arc::new(Notify::new());
    let two_solo_srv = two_solo.clone();
    let server_solo = spawn_mock(move |seq, _body, _cur, _peak| {
        let mut hold_solo_rx = hold_solo_rx.clone();
        let two_solo_srv = two_solo_srv.clone();
        async move {
            if seq == 2 {
                two_solo_srv.notify_one();
            }
            let _ = hold_solo_rx.wait_for(|v| *v).await;
            sse_reply("*Lia respira fundo.*\n\nLia: Solo sem limite roda em paralelo.")
        }
    })
    .await;

    let providers_solo = Arc::new(
        Providers::from_json(&json!({
            "default": "p1",
            "providers": [
                { "id": "p1", "baseUrl": server_solo.url, "maxConcurrent": 0 }
            ]
        }))
        .unwrap(),
    );
    let runtime_solo = RuntimeProviders::with_default_concurrency(Some(providers_solo), 1);
    let dir2 = tempfile::tempdir().unwrap();
    let (store2, manager2) = test_manager(dir2.path(), runtime_solo, 4);

    let s1 = store2
        .call(Request::CreateConversation(json!({
            "routeId": "custom/m1",
            "presetId": "generic"
        })))
        .await
        .unwrap();
    let s2 = store2
        .call(Request::CreateConversation(json!({
            "routeId": "custom/m2",
            "presetId": "generic"
        })))
        .await
        .unwrap();

    let ts1 = tokio::spawn({
        let manager = manager2.clone();
        let id = s1["id"].as_str().unwrap().to_owned();
        async move { finish_generation(&manager, &id, "1").await }
    });
    let ts2 = tokio::spawn({
        let manager = manager2.clone();
        let id = s2["id"].as_str().unwrap().to_owned();
        async move { finish_generation(&manager, &id, "2").await }
    });
    tokio::time::timeout(Duration::from_secs(3), two_solo.notified())
        .await
        .unwrap();
    assert_eq!(server_solo.peak_in_flight.load(Ordering::SeqCst), 2);
    let _ = hold_solo_tx.send(true);
    let (_, es1) = ts1.await.unwrap();
    let (_, es2) = ts2.await.unwrap();
    assert_eq!(es1["status"], "complete");
    assert_eq!(es2["status"], "complete");
    store2.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn loopback_aliases_share_outbound_gate() {
    let providers = Arc::new(
        Providers::from_json(&json!({
            "default": "ip4",
            "providers": [
                { "id": "ip4", "baseUrl": "http://127.0.0.1:20128/v1/", "maxConcurrent": 1 },
                { "id": "host", "baseUrl": "HTTP://LOCALHOST:20128/v1", "maxConcurrent": 1 },
                { "id": "ip6", "baseUrl": "http://[::1]:20128/v1/", "maxConcurrent": 1 }
            ]
        }))
        .unwrap(),
    );
    let (p_ip4, _) = providers.resolve("ip4:m");
    let (p_host, _) = providers.resolve("host:m");
    let (p_ip6, _) = providers.resolve("ip6:m");

    // Verify original provider.base_url values were NOT mutated
    assert_eq!(p_ip4.base_url, "http://127.0.0.1:20128/v1");
    assert_eq!(p_host.base_url, "HTTP://LOCALHOST:20128/v1");
    assert_eq!(p_ip6.base_url, "http://[::1]:20128/v1");

    let runtime = RuntimeProviders::with_default_concurrency(Some(providers.clone()), 1);

    let permit1 = runtime
        .acquire_outbound(p_ip4, Duration::from_secs(1))
        .await
        .unwrap()
        .expect("permit1");

    // HTTP://LOCALHOST:<port>/v1 shares gate with http://127.0.0.1:<port>/v1/
    let err_host = runtime
        .acquire_outbound(p_host, Duration::from_millis(80))
        .await
        .err()
        .expect("localhost must block while 127.0.0.1 holds gate");
    assert_eq!(err_host.status, 504);
    assert_eq!(err_host.extra["code"], "UPSTREAM_QUEUE_TIMEOUT");

    // http://[::1]:<port>/v1/ also shares gate
    let err_ip6 = runtime
        .acquire_outbound(p_ip6, Duration::from_millis(80))
        .await
        .err()
        .expect("::1 must block while 127.0.0.1 holds gate");
    assert_eq!(err_ip6.status, 504);

    drop(permit1);

    let permit2 = runtime
        .acquire_outbound(p_host, Duration::from_secs(1))
        .await
        .unwrap()
        .expect("permit2");
    drop(permit2);
}

#[tokio::test]
async fn preflight_cache_isolates_ipv4_ipv6_loopback_and_preserves_fallback() {
    // Bind an IPv4-only listener (127.0.0.1:0) while leaving [::1]:<port> unbound.
    let v4_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = v4_listener.local_addr().unwrap().port();
    let v4_gets = Arc::new(AtomicUsize::new(0));
    let v4_gets_srv = v4_gets.clone();
    let v4_app = Router::new().route(
        "/v1/models",
        get(move || {
            let v4_gets_srv = v4_gets_srv.clone();
            async move {
                v4_gets_srv.fetch_add(1, Ordering::SeqCst);
                Json(json!({ "data": [{ "id": "custom/m1" }] }))
            }
        }),
    );
    let v4_srv = tokio::spawn(async move { axum::serve(v4_listener, v4_app).await.unwrap() });

    let fb_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fb_url = format!("http://{}/v1", fb_listener.local_addr().unwrap());
    let fb_app = Router::new().route(
        "/v1/models",
        get(|| async { Json(json!({ "data": [{ "id": "custom/fb" }] })) }),
    );
    let fb_srv = tokio::spawn(async move { axum::serve(fb_listener, fb_app).await.unwrap() });

    let providers = Arc::new(
        Providers::from_json(&json!({
            "default": "v4",
            "providers": [
                { "id": "v4", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "maxConcurrent": 1 },
                { "id": "v6", "baseUrl": format!("http://[::1]:{port}/v1"), "maxConcurrent": 1 },
                { "id": "host", "baseUrl": format!("http://localhost:{port}/v1"), "maxConcurrent": 1 },
                { "id": "cloud", "baseUrl": fb_url, "maxConcurrent": 1 }
            ],
            "fallbacks": {
                "v6:custom/m1": { "provider": "cloud", "model": "custom/fb" }
            }
        }))
        .unwrap(),
    );
    let runtime = RuntimeProviders::with_default_concurrency(Some(providers.clone()), 1);
    let (p_v4, _) = providers.resolve("v4:custom/m1");
    let (p_v6, _) = providers.resolve("v6:custom/m1");
    let (p_host, _) = providers.resolve("host:custom/m1");

    // Precondition: [::1]:<port> is unbound, so v6 preflight returns false.
    assert!(!runtime.preflight(&p_v6, "custom/m1").await);

    // Warm PreflightCache via v4 (127.0.0.1).
    assert!(runtime.preflight(&p_v4, "custom/m1").await);
    assert_eq!(v4_gets.load(Ordering::SeqCst), 1);
    // Repeated v4 preflight hits cache (0 new GETs).
    assert!(runtime.preflight(&p_v4, "custom/m1").await);
    assert_eq!(v4_gets.load(Ordering::SeqCst), 1);

    // v6 must NOT hit v4's cache entry and select() must route to fallback with preflight_fallback=true.
    assert!(!runtime.preflight(&p_v6, "custom/m1").await);
    match runtime.select(&providers, "v6:custom/m1").await {
        SelectedBackend::Cloud {
            provider,
            model,
            preflight_fallback,
        } => {
            assert_eq!(provider.id, "cloud");
            assert_eq!(model, "custom/fb");
            assert!(preflight_fallback);
        }
        SelectedBackend::Local => panic!("expected fallback cloud backend"),
    }

    // localhost must NOT hit 127.0.0.1's cache entry either:
    // if localhost resolves to 127.0.0.1 on this host, it issues a real GET (v4_gets -> 2);
    // if localhost resolves to ::1, it returns false.
    let host_ok = runtime.preflight(&p_host, "custom/m1").await;
    if host_ok {
        assert_eq!(v4_gets.load(Ordering::SeqCst), 2);
    } else {
        assert_eq!(v4_gets.load(Ordering::SeqCst), 1);
    }

    v4_srv.abort();
    fb_srv.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manager_stop_cancellation_wakeup_stress_multi_thread() {
    let hold_gate = Arc::new(tokio::sync::Mutex::new(Some(Arc::new(Notify::new()))));
    let started_gate = Arc::new(tokio::sync::Mutex::new(None::<Arc<Notify>>));
    let hold_srv = hold_gate.clone();
    let started_srv = started_gate.clone();

    let server = spawn_mock(move |_seq, body, _cur, _peak| {
        let hold_srv = hold_srv.clone();
        let started_srv = started_srv.clone();
        async move {
            let prompt = body["messages"]
                .as_array()
                .and_then(|arr| arr.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or_default()
                .to_owned();
            if prompt.starts_with("hold") {
                let s = started_srv.lock().await.clone();
                if let Some(s) = s {
                    s.notify_one();
                }
                let h = hold_srv.lock().await.clone();
                if let Some(h) = h {
                    h.notified().await;
                }
            }
            sse_reply("*Lia respira fundo.*\n\nLia: Parada validada sem perda de sinal.")
        }
    })
    .await;

    let providers = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 1 }]
        }))
        .unwrap(),
    );
    let runtime = RuntimeProviders::with_default_concurrency(Some(providers), 1);
    let dir = tempfile::tempdir().unwrap();
    let (store, manager) = test_manager(dir.path(), runtime, 8);

    let mut expected_posts = 0usize;

    // 32 iterations mixing queued cancellation behind maxConcurrent=1 and fast-finishing stop() races.
    for iter in 0..32usize {
        if iter % 2 == 0 {
            let hold = Arc::new(Notify::new());
            let started = Arc::new(Notify::new());
            *hold_gate.lock().await = Some(hold.clone());
            *started_gate.lock().await = Some(started.clone());

            let c_hold = store
                .call(Request::CreateConversation(json!({
                    "routeId": "custom/m",
                    "presetId": "generic"
                })))
                .await
                .unwrap();
            let c_queued = store
                .call(Request::CreateConversation(json!({
                    "routeId": "custom/m",
                    "presetId": "generic"
                })))
                .await
                .unwrap();

            let req1: TurnRequest = serde_json::from_value(json!({
                "convId": c_hold["id"],
                "mode": "send",
                "content": "hold",
                "opId": format!("hold-{iter}")
            }))
            .unwrap();
            let (g1, _) = manager.start(req1).await.unwrap();
            tokio::time::timeout(Duration::from_secs(2), started.notified())
                .await
                .unwrap();
            expected_posts += 1;

            let req2: TurnRequest = serde_json::from_value(json!({
                "convId": c_queued["id"],
                "mode": "send",
                "content": format!("queued-{iter}"),
                "opId": format!("queued-{iter}")
            }))
            .unwrap();
            let (g2, _) = manager.start(req2).await.unwrap();
            if iter % 4 == 0 {
                tokio::task::yield_now().await;
            } else {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }

            let stop_res = match tokio::time::timeout(
                Duration::from_secs(1),
                manager.stop(c_queued["id"].as_str().unwrap(), &g2.id),
            )
            .await
            {
                Ok(res) => res,
                Err(e) => {
                    let has_final = g2.output.lock().await.final_event.is_some();
                    let active_ids = manager.busy_ids().await;
                    let posts = server.posts.load(Ordering::SeqCst);
                    panic!(
                        "queued Manager::stop timed out on iter {iter}: {e}; has_final={has_final}, active_ids={active_ids:?}, posts={posts}"
                    );
                }
            };
            assert!(stop_res.is_ok(), "queued stop failed on iter {iter}: {stop_res:?}");

            hold.notify_one();
            tokio::time::timeout(Duration::from_secs(2), async {
                while g1.output.lock().await.final_event.is_none() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
        } else {
            *hold_gate.lock().await = None;
            *started_gate.lock().await = None;

            let c_fast = store
                .call(Request::CreateConversation(json!({
                    "routeId": "custom/m",
                    "presetId": "generic"
                })))
                .await
                .unwrap();
            let req: TurnRequest = serde_json::from_value(json!({
                "convId": c_fast["id"],
                "mode": "send",
                "content": format!("fast-{iter}"),
                "opId": format!("fast-{iter}")
            }))
            .unwrap();
            let (g_fast, _) = manager.start(req).await.unwrap();
            if iter % 4 == 3 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            let stop_res = tokio::time::timeout(
                Duration::from_secs(1),
                manager.stop(c_fast["id"].as_str().unwrap(), &g_fast.id),
            )
            .await
            .expect("fast-finishing Manager::stop must never lose wakeup");
            assert!(stop_res.is_ok(), "fast stop failed on iter {iter}: {stop_res:?}");
            assert!(g_fast.output.lock().await.final_event.is_some());
            expected_posts = server.posts.load(Ordering::SeqCst);
        }
    }

    assert_eq!(server.posts.load(Ordering::SeqCst), expected_posts);
    // Sanity turn after 32 stop cycles proves zero leaked OutboundPermits or generation slots.
    let c_sanity = store
        .call(Request::CreateConversation(json!({
            "routeId": "custom/m",
            "presetId": "generic"
        })))
        .await
        .unwrap();
    let (_, ev_sanity) = finish_generation(&manager, c_sanity["id"].as_str().unwrap(), "sanity").await;
    assert_eq!(ev_sanity["status"], "complete");

    store.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn put_local_providers_with_active_runner_429_cooldown_and_2_to_1_lowering() {
    let bad_posts = Arc::new(AtomicUsize::new(0));
    let bad_posts_srv = bad_posts.clone();
    let bad_hold_started = Arc::new(Notify::new());
    let bad_hold_started_srv = bad_hold_started.clone();
    let bad_hold_release = Arc::new(Notify::new());
    let bad_hold_release_srv = bad_hold_release.clone();
    let hold_bad_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hold_bad_flag_srv = hold_bad_flag.clone();

    let bad_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bad_url = format!("http://{}/v1", bad_listener.local_addr().unwrap());
    let bad_app = Router::new()
        .route(
            "/v1/models",
            get(|| async { Json(json!({ "data": [{ "id": "custom/memory-model" }] })) }),
        )
        .route(
            "/v1/chat/completions",
            post(move || {
                let bad_posts_srv = bad_posts_srv.clone();
                let bad_hold_started_srv = bad_hold_started_srv.clone();
                let bad_hold_release_srv = bad_hold_release_srv.clone();
                let hold_bad_flag_srv = hold_bad_flag_srv.clone();
                async move {
                    bad_posts_srv.fetch_add(1, Ordering::SeqCst);
                    if hold_bad_flag_srv.load(Ordering::SeqCst) {
                        bad_hold_started_srv.notify_one();
                        bad_hold_release_srv.notified().await;
                    }
                    (
                        StatusCode::TOO_MANY_REQUESTS,
                        [("retry-after", "30")],
                        r#"{"error":{"message":"rate limit exceeded"}}"#,
                    )
                        .into_response()
                }
            }),
        );
    let bad_srv = tokio::spawn(async move { axum::serve(bad_listener, bad_app).await.unwrap() });

    let good_posts = Arc::new(AtomicUsize::new(0));
    let good_posts_srv = good_posts.clone();
    let good_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let good_url = format!("http://{}/v1", good_listener.local_addr().unwrap());
    let good_app = Router::new()
        .route(
            "/v1/models",
            get(|| async { Json(json!({ "data": [{ "id": "custom/memory-model" }] })) }),
        )
        .route(
            "/v1/chat/completions",
            post(move || {
                let good_posts_srv = good_posts_srv.clone();
                async move {
                    good_posts_srv.fetch_add(1, Ordering::SeqCst);
                    Json(json!({
                        "choices": [{
                            "finish_reason": "stop",
                            "message": { "content": "{\"cast\":[]}" }
                        }]
                    }))
                }
            }),
        );
    let good_srv = tokio::spawn(async move { axum::serve(good_listener, good_app).await.unwrap() });

    let app_dir = tempfile::tempdir().unwrap();
    let native_dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::for_android(app_dir.path(), native_dir.path()).unwrap();
    cfg.address = "127.0.0.1:0".parse().unwrap();
    cfg.max_generations = 4;
    let state = AppState::new(cfg).await.unwrap();
    let token = "m2_runner_put_token_0123456789abcdef0123456789abcdef0123456789";
    state
        .store
        .call(Request::AddDevice {
            name: "M2 runner test".into(),
            token_hash: rules::hash(token),
        })
        .await
        .unwrap();

    let put_providers = |state: AppState, url: String, max_concurrent: usize| async move {
        let req = HttpRequest::builder()
            .method("PUT")
            .uri("/api/local/providers")
            .extension(ConnectInfo("127.0.0.1:0".parse::<SocketAddr>().unwrap()))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "default": "mock",
                    "providers": [{
                        "id": "mock",
                        "baseUrl": url,
                        "models": ["custom/memory-model"],
                        "allowLoopback": true,
                        "maxConcurrent": max_concurrent
                    }]
                })
                .to_string(),
            ))
            .unwrap();
        router(state).oneshot(req).await.unwrap()
    };

    // Activate mobile local lifecycle so MemoryBackend::mobile runs MemoryMode::Api.
    let act_req = HttpRequest::builder()
        .method("POST")
        .uri("/api/local/activate")
        .extension(ConnectInfo("127.0.0.1:0".parse::<SocketAddr>().unwrap()))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router(state.clone()).oneshot(act_req).await.unwrap().status(),
        StatusCode::ACCEPTED
    );

    // Part A: Runner enters 429 cooldown (Retry-After: 30) on bad provider, then PUT switches to good (2 -> 1) and recovers immediately.
    let resp = put_providers(state.clone(), bad_url.clone(), 2).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let policy_req = HttpRequest::builder()
        .method("PUT")
        .uri("/api/local/policy")
        .extension(ConnectInfo("127.0.0.1:0".parse::<SocketAddr>().unwrap()))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "mode": "api",
                "api": {
                    "provider": "mock",
                    "model": "custom/memory-model",
                    "historyConsent": true
                }
            })
            .to_string(),
        ))
        .unwrap();
    assert_eq!(
        router(state.clone()).oneshot(policy_req).await.unwrap().status(),
        StatusCode::OK
    );

    let conv = state
        .store
        .call(Request::CreateConversation(json!({
            "routeId": "mobile-api:mock:custom/memory-model",
            "presetId": "generic",
            "prompt": "Uma exploradora silenciosa."
        })))
        .await
        .unwrap();
    let job1 = state
        .store
        .call(Request::EnqueueJob(json!({
            "convId": conv["id"],
            "kind": "cast"
        })))
        .await
        .unwrap();
    let jid1 = job1["id"].as_str().unwrap().to_owned();
    state.runner.kick();

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let st = state.runner.state().await;
            let row = state.store.call(Request::JobInput(jid1.clone())).await.unwrap();
            if st["unavailableStreak"].as_u64() == Some(1)
                && row["job"]["deferrals"].as_i64() == Some(1)
                && row["job"]["status"] == "pending"
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        state.runner.state().await["cooldownRemainingSeconds"]
            .as_u64()
            .unwrap_or(0)
            >= 20
    );

    // Switch to good provider + lower 2 -> 1 while 30s cooldown is active: must wake immediately.
    let resp = put_providers(state.clone(), good_url.clone(), 1).await;
    assert_eq!(resp.status(), StatusCode::OK);

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let row = state.store.call(Request::JobInput(jid1.clone())).await.unwrap();
            if row["job"]["status"] == "done" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(good_posts.load(Ordering::SeqCst), 1);
    assert_eq!(state.runner.state().await["unavailableStreak"], 0);
    assert_eq!(state.runner.state().await["lastError"], Value::Null);

    // Part B: Concurrent PUT /api/local/providers (2 -> 1) WHILE mock_bad chat_json is in-flight -> stale 429 must NOT install cooldown or increment deferrals.
    hold_bad_flag.store(true, Ordering::SeqCst);
    let resp = put_providers(state.clone(), bad_url.clone(), 2).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let job2 = state
        .store
        .call(Request::EnqueueJob(json!({
            "convId": conv["id"],
            "kind": "cast"
        })))
        .await
        .unwrap();
    let jid2 = job2["id"].as_str().unwrap().to_owned();
    state.runner.kick();

    tokio::time::timeout(Duration::from_secs(5), bad_hold_started.notified())
        .await
        .unwrap();

    // Switch to good_url (maxConcurrent: 1) while job2's old-provider chat_json is still blocked in-flight.
    let resp = put_providers(state.clone(), good_url.clone(), 1).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Now release the old bad provider so it returns 429 Retry-After: 30.
    bad_hold_release.notify_one();

    // Because config_epoch changed during in-flight handle(), runner must NOT enter 30s cooldown
    // and must immediately complete job2 on good_url with deferrals == 0.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let row = state.store.call(Request::JobInput(jid2.clone())).await.unwrap();
            if row["job"]["status"] == "done" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .unwrap();

    let done2 = state.store.call(Request::JobInput(jid2)).await.unwrap();
    assert_eq!(done2["job"]["deferrals"], 0);
    assert_eq!(state.runner.state().await["unavailableStreak"], 0);
    assert_eq!(state.runner.state().await["cooldownRemainingSeconds"], 0);
    assert_eq!(state.runner.state().await["lastError"], Value::Null);
    assert_eq!(good_posts.load(Ordering::SeqCst), 2);

    state.shutdown().await;
    bad_srv.abort();
    good_srv.abort();
}

#[tokio::test]
async fn style_repair_and_empty_sse_under_2_to_1_lowering() {
    let hold_g1 = Arc::new(Notify::new());
    let hold_g1_srv = hold_g1.clone();
    let hold_g2 = Arc::new(Notify::new());
    let hold_g2_srv = hold_g2.clone();
    let two_in_flight = Arc::new(Notify::new());
    let two_in_flight_srv = two_in_flight.clone();

    let server = spawn_mock(move |_seq, body, cur, _peak| {
        let hold_g1_srv = hold_g1_srv.clone();
        let hold_g2_srv = hold_g2_srv.clone();
        let two_in_flight_srv = two_in_flight_srv.clone();
        async move {
            let is_repair = body["messages"]
                .as_array()
                .and_then(|arr| arr.last())
                .and_then(|m| m["content"].as_str())
                .is_some_and(|s| s.contains("AJUSTE DA RESPOSTA ANTERIOR"));
            if is_repair {
                return Json(json!({
                    "choices": [{
                        "finish_reason": "stop",
                        "message": { "content": "Lia: Vamos continuar a conversa agora com calma." }
                    }]
                }))
                .into_response();
            }
            let prompt = body["messages"]
                .as_array()
                .and_then(|arr| arr.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or_default()
                .to_owned();
            if prompt.starts_with("empty-turn") {
                let done = format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"delta":{},"finish_reason":"stop"}]})
                );
                return ([("content-type", "text/event-stream")], done).into_response();
            }
            if prompt.starts_with("g2-hold") {
                if cur.load(Ordering::SeqCst) == 2 {
                    two_in_flight_srv.notify_one();
                }
                hold_g2_srv.notified().await;
                return sse_reply("Lia: Segundo turno liberado com segurança.");
            }
            if prompt.starts_with("g1-narration") {
                if cur.load(Ordering::SeqCst) == 2 {
                    two_in_flight_srv.notify_one();
                }
                hold_g1_srv.notified().await;
                // g1 returns pure narration so dialoguePct=100 triggers style_repair
                return sse_reply("*Lia observa a chuva silenciosa pela janela do corredor.*");
            }
            sse_reply("Lia: Turno concluído normalmente.")
        }
    })
    .await;

    let providers_2 = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 2 }]
        }))
        .unwrap(),
    );
    let providers_1 = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 1 }]
        }))
        .unwrap(),
    );
    let runtime = RuntimeProviders::with_default_concurrency(Some(providers_2), 1);
    let dir = tempfile::tempdir().unwrap();
    let (store, manager) = test_manager(dir.path(), runtime.clone(), 4);
    store
        .call(Request::SetGlobalSettings(json!({ "dialoguePct": 100 })))
        .await
        .unwrap();

    let c1 = store
        .call(Request::CreateConversation(json!({
            "routeId": "custom/m",
            "presetId": "generic"
        })))
        .await
        .unwrap();
    let c2 = store
        .call(Request::CreateConversation(json!({
            "routeId": "custom/m",
            "presetId": "generic"
        })))
        .await
        .unwrap();
    let c3 = store
        .call(Request::CreateConversation(json!({
            "routeId": "custom/m",
            "presetId": "generic"
        })))
        .await
        .unwrap();
    let c4 = store
        .call(Request::CreateConversation(json!({
            "routeId": "custom/m",
            "presetId": "generic"
        })))
        .await
        .unwrap();

    let t1 = tokio::spawn({
        let manager = manager.clone();
        let id = c1["id"].as_str().unwrap().to_owned();
        async move { finish_generation(&manager, &id, "g1-narration").await }
    });
    let t2 = tokio::spawn({
        let manager = manager.clone();
        let id = c2["id"].as_str().unwrap().to_owned();
        async move { finish_generation(&manager, &id, "g2-hold").await }
    });

    tokio::time::timeout(Duration::from_secs(3), two_in_flight.notified())
        .await
        .unwrap();
    assert_eq!(server.peak_in_flight.load(Ordering::SeqCst), 2);

    // Lower 2 -> 1 while g1 and g2 are both in-flight, then finish g1's primary stream while g2 stays held.
    runtime.set(Some(providers_1));
    hold_g1.notify_one();

    // g1's primary stream finishes (0 repair posts yet while g2 still holds 1 permit under new limit=1).
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(
        server.posts.load(Ordering::SeqCst),
        2,
        "g1 style_repair must queue behind g2 under lowered maxConcurrent=1 without self-deadlock"
    );

    // Release g2 -> g2 completes, freeing the sole permit, and g1's style_repair enters and succeeds.
    hold_g2.notify_one();
    let (_, ev2) = t2.await.unwrap();
    let (_, ev1) = t1.await.unwrap();
    assert_eq!(ev2["status"], "complete");
    assert_eq!(ev1["status"], "complete");
    assert_eq!(ev1["usage"]["style"]["attempted"], true);
    assert_eq!(ev1["usage"]["style"]["accepted"], true);
    assert_eq!(server.posts.load(Ordering::SeqCst), 3);

    // Empty SSE under limit=1 returns status "empty" with <= 1 POST and zero leaked permits.
    let posts_before_empty = server.posts.load(Ordering::SeqCst);
    let (_, ev_empty) = finish_generation(&manager, c3["id"].as_str().unwrap(), "empty-turn").await;
    assert_eq!(ev_empty["status"], "empty");
    assert_eq!(server.posts.load(Ordering::SeqCst), posts_before_empty + 1);

    // Follow-up turn on a fresh conversation proves zero permits leaked after empty SSE.
    let (_, ev_after) = finish_generation(&manager, c4["id"].as_str().unwrap(), "after-empty").await;
    assert_eq!(ev_after["status"], "complete");

    store.call(Request::Close).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manager_shutdown_cancels_queued_before_releasing_streaming_permits() {
    use futures_util::StreamExt;

    let hold_gate: Arc<tokio::sync::Mutex<Option<tokio::sync::watch::Receiver<bool>>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let started_gate: Arc<tokio::sync::Mutex<Option<tokio::sync::watch::Sender<bool>>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let hold_srv = hold_gate.clone();
    let started_srv = started_gate.clone();

    let server = spawn_mock(move |_seq, body, _cur, _peak| {
        let hold_srv = hold_srv.clone();
        let started_srv = started_srv.clone();
        async move {
            let prompt = body["messages"]
                .as_array()
                .and_then(|arr| arr.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or_default()
                .to_owned();
            if prompt.starts_with("stream-sse-") {
                let mut hold_rx = hold_srv.lock().await.clone().unwrap();
                let started_tx = started_srv.lock().await.clone().unwrap();
                let s = futures_util::stream::once(async move {
                    Ok::<_, std::convert::Infallible>(format!(
                        "data: {}\n\n",
                        json!({"choices":[{"delta":{"content": "*Lia respira.* "}}]})
                    ))
                })
                .chain(futures_util::stream::once(async move {
                    let _ = started_tx.send(true);
                    let _ = hold_rx.wait_for(|v| *v).await;
                    Ok::<_, std::convert::Infallible>(format!(
                        "data: {}\n\ndata: [DONE]\n\n",
                        json!({"choices":[{"delta":{},"finish_reason":"stop"}]})
                    ))
                }));
                return Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "text/event-stream")
                    .body(Body::from_stream(s))
                    .unwrap();
            }
            if prompt.starts_with("stream-hdr-") {
                let mut hold_rx = hold_srv.lock().await.clone().unwrap();
                let started_tx = started_srv.lock().await.clone().unwrap();
                let _ = started_tx.send(true);
                let _ = hold_rx.wait_for(|v| *v).await;
            }
            sse_reply("*Lia respira fundo.*\n\nLia: Encerramento limpo confirmado.")
        }
    })
    .await;

    let providers = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 1 }]
        }))
        .unwrap(),
    );

    for iter in 0..16usize {
        let runtime = RuntimeProviders::with_default_concurrency(Some(providers.clone()), 1);
        let dir = tempfile::tempdir().unwrap();
        let (store, manager) = test_manager(dir.path(), runtime.clone(), 8);

        let (hold_tx, hold_rx) = tokio::sync::watch::channel(false);
        let (started_tx, mut started_rx) = tokio::sync::watch::channel(false);
        *hold_gate.lock().await = Some(hold_rx);
        *started_gate.lock().await = Some(started_tx);

        let mut convs = Vec::with_capacity(7);
        for _ in 0..7 {
            let c = store
                .call(Request::CreateConversation(json!({
                    "routeId": "custom/m",
                    "presetId": "generic"
                })))
                .await
                .unwrap();
            convs.push(c["id"].as_str().unwrap().to_owned());
        }

        let base_posts = server.posts.load(Ordering::SeqCst);
        let g0_prompt = if iter % 2 == 0 {
            format!("stream-hdr-{iter}")
        } else {
            format!("stream-sse-{iter}")
        };
        let (g0, _) = manager
            .start(
                serde_json::from_value(json!({
                    "convId": convs[0],
                    "mode": "send",
                    "content": g0_prompt,
                    "opId": format!("g0-{iter}")
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), started_rx.wait_for(|v| *v))
            .await
            .unwrap()
            .unwrap();

        let mut queued = Vec::with_capacity(5);
        for k in 1..=5usize {
            let (gk, _) = manager
                .start(
                    serde_json::from_value(json!({
                        "convId": convs[k],
                        "mode": "send",
                        "content": format!("queued-{iter}-{k}"),
                        "opId": format!("g{k}-{iter}")
                    }))
                    .unwrap(),
                )
                .await
                .unwrap();
            queued.push(gk);
        }

        // Wait until all 5 queued generations are parked in OutboundGate behind g0.
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert_eq!(server.posts.load(Ordering::SeqCst), base_posts + 1);

        tokio::time::timeout(Duration::from_secs(5), manager.shutdown())
            .await
            .expect("Manager::shutdown() must finalize all active & queued generations");

        assert_eq!(
            server.posts.load(Ordering::SeqCst),
            base_posts + 1,
            "iter {iter}: Manager::shutdown() must cancel queued generations BEFORE releasing g0's OutboundPermit"
        );
        assert!(manager.busy_ids().await.is_empty());
        let g0_status = g0.output.lock().await.final_event.as_ref().unwrap()["status"]
            .as_str()
            .unwrap()
            .to_owned();
        let expected_g0 = if iter % 2 == 1 { "interrupted" } else { "empty" };
        assert_eq!(g0_status, expected_g0, "unexpected g0 status on iter {iter}");
        for gk in &queued {
            let gk_status = gk.output.lock().await.final_event.as_ref().unwrap()["status"]
                .as_str()
                .unwrap()
                .to_owned();
            assert!(
                matches!(gk_status.as_str(), "interrupted" | "empty"),
                "unexpected queued status on iter {iter}: {gk_status}"
            );
        }

        let _ = hold_tx.send(true);
        assert_eq!(
            manager.available_slots(),
            8,
            "iter {iter}: Manager::shutdown() must release all Manager::slots permits"
        );
        let post_shutdown_err = manager
            .start(
                serde_json::from_value(json!({
                    "convId": convs[6],
                    "mode": "send",
                    "content": format!("after-shutdown-{iter}"),
                    "opId": format!("after-shutdown-{iter}")
                }))
                .unwrap(),
            )
            .await
            .err()
            .expect("Manager::start() after Manager::shutdown() must fail with 503");
        assert_eq!(post_shutdown_err.status, 503);
        assert_eq!(server.posts.load(Ordering::SeqCst), base_posts + 1);

        // Sanity turn on a fresh Manager sharing the same RuntimeProviders confirms zero leaked OutboundGate permits.
        let sanity_manager = test_manager_with_store(store.clone(), dir.path(), runtime.clone(), 8);
        let (_, ev_sanity) =
            finish_generation(&sanity_manager, &convs[6], &format!("sanity-{iter}")).await;
        assert_eq!(ev_sanity["status"], "complete");
        assert_eq!(server.posts.load(Ordering::SeqCst), base_posts + 2);

        store.call(Request::Close).await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn style_repair_cancellation_race_suppresses_spurious_repair_posts() {
    use futures_util::StreamExt;

    let repair_posts = Arc::new(AtomicUsize::new(0));
    let repair_posts_srv = repair_posts.clone();

    let race_a_hold: Arc<tokio::sync::Mutex<Option<tokio::sync::watch::Receiver<bool>>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let race_a_chunk1: Arc<tokio::sync::Mutex<Option<tokio::sync::watch::Sender<bool>>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let race_b_g1_entered: Arc<tokio::sync::Mutex<Option<tokio::sync::watch::Sender<bool>>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let race_b_g1_hold: Arc<tokio::sync::Mutex<Option<tokio::sync::watch::Receiver<bool>>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let race_b_g0_entered: Arc<tokio::sync::Mutex<Option<tokio::sync::watch::Sender<bool>>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let race_b_g0_hold: Arc<tokio::sync::Mutex<Option<tokio::sync::watch::Receiver<bool>>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    let race_a_hold_srv = race_a_hold.clone();
    let race_a_chunk1_srv = race_a_chunk1.clone();
    let race_b_g1_entered_srv = race_b_g1_entered.clone();
    let race_b_g1_hold_srv = race_b_g1_hold.clone();
    let race_b_g0_entered_srv = race_b_g0_entered.clone();
    let race_b_g0_hold_srv = race_b_g0_hold.clone();

    let server = spawn_mock(move |_seq, body, _cur, _peak| {
        let repair_posts_srv = repair_posts_srv.clone();
        let race_a_hold_srv = race_a_hold_srv.clone();
        let race_a_chunk1_srv = race_a_chunk1_srv.clone();
        let race_b_g1_entered_srv = race_b_g1_entered_srv.clone();
        let race_b_g1_hold_srv = race_b_g1_hold_srv.clone();
        let race_b_g0_entered_srv = race_b_g0_entered_srv.clone();
        let race_b_g0_hold_srv = race_b_g0_hold_srv.clone();
        async move {
            let is_repair = body["messages"]
                .as_array()
                .and_then(|arr| arr.last())
                .and_then(|m| m["content"].as_str())
                .is_some_and(|s| s.contains("AJUSTE DA RESPOSTA ANTERIOR"));
            if is_repair {
                repair_posts_srv.fetch_add(1, Ordering::SeqCst);
                return Json(json!({
                    "choices": [{
                        "finish_reason": "stop",
                        "message": { "content": "Lia: Vamos conversar com calma agora." }
                    }]
                }))
                .into_response();
            }
            let prompt = body["messages"]
                .as_array()
                .and_then(|arr| arr.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or_default()
                .to_owned();
            if prompt.contains("race-a-") {
                let mut hold_rx = race_a_hold_srv.lock().await.clone().unwrap();
                let chunk1_tx = race_a_chunk1_srv.lock().await.clone().unwrap();
                let s = futures_util::stream::once(async move {
                    let _ = chunk1_tx.send(true);
                    Ok::<_, std::convert::Infallible>(format!(
                        "data: {}\n\n",
                        json!({"choices":[{"delta":{"content": "*Lia olha pela janela em silêncio absoluto.*"}}]})
                    ))
                })
                .chain(futures_util::stream::once(async move {
                    let _ = hold_rx.wait_for(|v| *v).await;
                    Ok::<_, std::convert::Infallible>(format!(
                        "data: {}\n\ndata: [DONE]\n\n",
                        json!({"choices":[{"delta":{},"finish_reason":"stop"}]})
                    ))
                }));
                return Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "text/event-stream")
                    .body(Body::from_stream(s))
                    .unwrap();
            }
            if prompt.contains("race-b-g1-") {
                let entered_tx = race_b_g1_entered_srv.lock().await.clone().unwrap();
                let mut hold_rx = race_b_g1_hold_srv.lock().await.clone().unwrap();
                let _ = entered_tx.send(true);
                let _ = hold_rx.wait_for(|v| *v).await;
                return sse_reply("Lia: G1 concluído normalmente.");
            }
            if prompt.contains("race-b-g0-") {
                let entered_tx = race_b_g0_entered_srv.lock().await.clone().unwrap();
                let mut hold_rx = race_b_g0_hold_srv.lock().await.clone().unwrap();
                let _ = entered_tx.send(true);
                let _ = hold_rx.wait_for(|v| *v).await;
                return sse_reply("*Lia observa o corredor em silêncio absoluto.*");
            }
            sse_reply("Lia: Turno de sanidade concluído com sucesso.")
        }
    })
    .await;

    let providers_2 = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 2 }]
        }))
        .unwrap(),
    );
    let providers_1 = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 1 }]
        }))
        .unwrap(),
    );
    let runtime = RuntimeProviders::with_default_concurrency(Some(providers_1.clone()), 1);
    let dir = tempfile::tempdir().unwrap();
    let (store, manager) = test_manager(dir.path(), runtime.clone(), 4);
    store
        .call(Request::SetGlobalSettings(json!({ "dialoguePct": 100 })))
        .await
        .unwrap();

    let conv_a = store
        .call(Request::CreateConversation(json!({
            "routeId": "custom/m",
            "presetId": "generic"
        })))
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let conv_b0 = store
        .call(Request::CreateConversation(json!({
            "routeId": "custom/m",
            "presetId": "generic"
        })))
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let conv_b1 = store
        .call(Request::CreateConversation(json!({
            "routeId": "custom/m",
            "presetId": "generic"
        })))
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let conv_sanity = store
        .call(Request::CreateConversation(json!({
            "routeId": "custom/m",
            "presetId": "generic"
        })))
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Race A: cancel while g0.output is contended between primary completion and style_repair (32 iters).
    for iter in 0..32usize {
        let (hold_tx, hold_rx) = tokio::sync::watch::channel(false);
        let (chunk1_tx, mut chunk1_rx) = tokio::sync::watch::channel(false);
        *race_a_hold.lock().await = Some(hold_rx);
        *race_a_chunk1.lock().await = Some(chunk1_tx);

        let (g0, _) = manager
            .start(
                serde_json::from_value(json!({
                    "convId": conv_a,
                    "mode": "send",
                    "content": format!("race-a-{iter}"),
                    "opId": format!("race-a-{iter}")
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), chunk1_rx.wait_for(|v| *v))
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if g0.output.lock().await.units > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let output_guard = g0.output.lock().await;
        let _ = hold_tx.send(true);
        tokio::time::sleep(Duration::from_millis(2)).await;

        let stop_handle = tokio::spawn({
            let manager = manager.clone();
            let conv = conv_a.clone();
            let gid = g0.id.clone();
            async move { manager.stop(&conv, &gid).await }
        });
        tokio::time::sleep(Duration::from_millis(2)).await;
        drop(output_guard);

        tokio::time::timeout(Duration::from_secs(3), stop_handle)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            repair_posts.load(Ordering::SeqCst),
            0,
            "Race A iter {iter}: cancelled turn must not send style_repair POST"
        );
    }

    // Race B: g0 parked in style_repair behind g1, release g1 + stop(g0) simultaneously (32 iters).
    for iter in 0..32usize {
        runtime.set(Some(providers_2.clone()));
        let (g1_entered_tx, mut g1_entered_rx) = tokio::sync::watch::channel(false);
        let (g1_hold_tx, g1_hold_rx) = tokio::sync::watch::channel(false);
        let (g0_entered_tx, mut g0_entered_rx) = tokio::sync::watch::channel(false);
        let (g0_hold_tx, g0_hold_rx) = tokio::sync::watch::channel(false);
        *race_b_g1_entered.lock().await = Some(g1_entered_tx);
        *race_b_g1_hold.lock().await = Some(g1_hold_rx);
        *race_b_g0_entered.lock().await = Some(g0_entered_tx);
        *race_b_g0_hold.lock().await = Some(g0_hold_rx);

        let (g1, _) = manager
            .start(
                serde_json::from_value(json!({
                    "convId": conv_b1,
                    "mode": "send",
                    "content": format!("race-b-g1-{iter}"),
                    "opId": format!("race-b-g1-{iter}")
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        let (g0, _) = manager
            .start(
                serde_json::from_value(json!({
                    "convId": conv_b0,
                    "mode": "send",
                    "content": format!("race-b-g0-{iter}"),
                    "opId": format!("race-b-g0-{iter}")
                }))
                .unwrap(),
            )
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(3), g1_entered_rx.wait_for(|v| *v))
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), g0_entered_rx.wait_for(|v| *v))
            .await
            .unwrap()
            .unwrap();

        runtime.set(Some(providers_1.clone()));
        let _ = g0_hold_tx.send(true);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if g0.output.lock().await.units > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(3)).await;

        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let release_task = tokio::spawn({
            let barrier = barrier.clone();
            async move {
                barrier.wait().await;
                let _ = g1_hold_tx.send(true);
            }
        });
        let stop_task = tokio::spawn({
            let barrier = barrier.clone();
            let manager = manager.clone();
            let conv = conv_b0.clone();
            let gid = g0.id.clone();
            async move {
                barrier.wait().await;
                manager.stop(&conv, &gid).await
            }
        });

        release_task.await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), stop_task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if g1.output.lock().await.final_event.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            repair_posts.load(Ordering::SeqCst),
            0,
            "Race B iter {iter}: stopped style_repair must not fire spurious POST"
        );
    }

    assert_eq!(manager.available_slots(), 4);
    let (_, ev_sanity) = finish_generation(&manager, &conv_sanity, "sanity-after-races").await;
    assert_eq!(ev_sanity["status"], "complete");
    assert_eq!(repair_posts.load(Ordering::SeqCst), 0);

    store.call(Request::Close).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manager_shutdown_vs_concurrent_start_leaves_zero_busy_or_posts() {
    let hold_gate: Arc<tokio::sync::Mutex<Option<tokio::sync::watch::Receiver<bool>>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let hold_srv = hold_gate.clone();

    let server = spawn_mock(move |_seq, body, _cur, _peak| {
        let hold_srv = hold_srv.clone();
        async move {
            let prompt = body["messages"]
                .as_array()
                .and_then(|arr| arr.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or_default()
                .to_owned();
            if prompt.starts_with("bg-") {
                let mut hold_rx = hold_srv.lock().await.clone().unwrap();
                let _ = hold_rx.wait_for(|v| *v).await;
            }
            sse_reply("Lia: Resposta do servidor mock.")
        }
    })
    .await;

    let providers = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 2 }]
        }))
        .unwrap(),
    );

    for iter in 0..32usize {
        let runtime = RuntimeProviders::with_default_concurrency(Some(providers.clone()), 2);
        let dir = tempfile::tempdir().unwrap();
        let (store, manager) = test_manager(dir.path(), runtime, 4);

        let (hold_tx, hold_rx) = tokio::sync::watch::channel(false);
        *hold_gate.lock().await = Some(hold_rx);

        let conv_bg = store
            .call(Request::CreateConversation(json!({
                "routeId": "custom/m",
                "presetId": "generic"
            })))
            .await
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let conv_race = store
            .call(Request::CreateConversation(json!({
                "routeId": "custom/m",
                "presetId": "generic"
            })))
            .await
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();

        if iter % 2 == 1 {
            let _ = manager
                .start(
                    serde_json::from_value(json!({
                        "convId": conv_bg,
                        "mode": "send",
                        "content": format!("bg-{iter}"),
                        "opId": format!("bg-{iter}")
                    }))
                    .unwrap(),
                )
                .await
                .unwrap();
        }

        let req: TurnRequest = serde_json::from_value(json!({
            "convId": conv_race,
            "mode": "send",
            "content": format!("race-{iter}"),
            "opId": format!("race-{iter}")
        }))
        .unwrap();

        let (start_handle, shutdown_handle) = match iter % 3 {
            0 => {
                let start_handle = tokio::spawn({
                    let manager = manager.clone();
                    async move { manager.start(req).await }
                });
                let shutdown_handle = tokio::spawn({
                    let manager = manager.clone();
                    async move { manager.shutdown().await }
                });
                (start_handle, shutdown_handle)
            }
            1 => {
                let start_handle = tokio::spawn({
                    let manager = manager.clone();
                    async move { manager.start(req).await }
                });
                tokio::time::sleep(Duration::from_millis(1)).await;
                let shutdown_handle = tokio::spawn({
                    let manager = manager.clone();
                    async move { manager.shutdown().await }
                });
                (start_handle, shutdown_handle)
            }
            _ => {
                let barrier = Arc::new(tokio::sync::Barrier::new(2));
                let start_handle = tokio::spawn({
                    let barrier = barrier.clone();
                    let manager = manager.clone();
                    async move {
                        barrier.wait().await;
                        manager.start(req).await
                    }
                });
                let shutdown_handle = tokio::spawn({
                    let barrier = barrier.clone();
                    let manager = manager.clone();
                    async move {
                        barrier.wait().await;
                        manager.shutdown().await
                    }
                });
                (start_handle, shutdown_handle)
            }
        };

        tokio::time::timeout(Duration::from_secs(5), shutdown_handle)
            .await
            .expect("Manager::shutdown() must complete promptly")
            .unwrap();

        assert!(
            manager.busy_ids().await.is_empty(),
            "iter {iter}: busy_ids must be empty immediately after Manager::shutdown() returns"
        );
        let posts_at_return = server.posts.load(Ordering::SeqCst);

        match tokio::time::timeout(Duration::from_secs(5), start_handle)
            .await
            .unwrap()
            .unwrap()
        {
            Ok((g, _)) => {
                assert!(
                    g.output.lock().await.final_event.is_some(),
                    "iter {iter}: start() that beat shutdown() must be finalized before shutdown() returns"
                );
            }
            Err(e) => {
                assert_eq!(e.status, 503, "iter {iter}: post-shutdown start() must return 503");
            }
        }

        let _ = hold_tx.send(true);
        tokio::time::sleep(Duration::from_millis(25)).await;

        let posts_later = server.posts.load(Ordering::SeqCst);
        assert_eq!(
            posts_later, posts_at_return,
            "iter {iter}: zero post-shutdown upstream POSTs allowed"
        );
        assert!(manager.busy_ids().await.is_empty());
        assert_eq!(manager.available_slots(), 4);

        store.call(Request::Close).await.unwrap();
    }
}

#[tokio::test]
async fn g1_402_billing_error_fails_fast_without_pruning_while_402_credit_limit_retries() {
    assert!(!classify_token_limit(
        StatusCode::PAYMENT_REQUIRED,
        r#"{"error":{"message":"Account balance is zero. Please top up billing."}}"#,
    ));
    assert!(!classify_token_limit(
        StatusCode::PAYMENT_REQUIRED,
        "Insufficient funds in account",
    ));
    assert!(classify_token_limit(
        StatusCode::PAYMENT_REQUIRED,
        "This request requires more credits, or fewer max_tokens.",
    ));

    let server = spawn_mock(move |seq, body, _cur, _peak| async move {
        if seq <= 2 {
            return sse_reply("*Lia sorri de leve.*\n\nLia: Turno inicial salvo no histórico.");
        }
        if body["model"] == "custom/billing-zero" {
            return (
                StatusCode::PAYMENT_REQUIRED,
                r#"{"error":{"message":"Account balance is zero. Please top up billing."}}"#,
            )
                .into_response();
        }
        if seq == 6 {
            return (
                StatusCode::PAYMENT_REQUIRED,
                r#"{"error":{"message":"This request requires more credits, or fewer max_tokens."}}"#,
            )
                .into_response();
        }
        sse_reply("*Lia acena devagar.*\n\nLia: Recuperado após redução de max_tokens.")
    })
    .await;

    let providers = Arc::new(
        Providers::from_json(&json!({
            "default": "mock",
            "providers": [{ "id": "mock", "baseUrl": server.url, "maxConcurrent": 1 }]
        }))
        .unwrap(),
    );
    let runtime = RuntimeProviders::with_default_concurrency(Some(providers), 1);
    let dir = tempfile::tempdir().unwrap();
    let (store, manager) = test_manager(dir.path(), runtime, 4);

    // Billing 402 with 2 seeded turns -> must fail fast in 1 POST (no pruning retries)
    let c_bill = store
        .call(Request::CreateConversation(json!({"routeId":"custom/billing-zero","presetId":"generic"})))
        .await
        .unwrap();
    let cid_bill = c_bill["id"].as_str().unwrap();
    assert_eq!(finish_generation(&manager, cid_bill, "t1").await.1["status"], "complete");
    assert_eq!(finish_generation(&manager, cid_bill, "t2").await.1["status"], "complete");
    let posts_before_402 = server.posts.load(Ordering::SeqCst);
    let (_, ev_bill) = finish_generation(&manager, cid_bill, "t3").await;
    assert_eq!(ev_bill["status"], "error");
    assert!(ev_bill["error"].as_str().unwrap().contains("HTTP 402"));
    assert_eq!(
        server.posts.load(Ordering::SeqCst) - posts_before_402,
        1,
        "Billing 402 must send exactly 1 POST without history pruning"
    );

    // Credit/max_tokens 402 -> classifies as token limit and succeeds on retry
    let c_cred = store
        .call(Request::CreateConversation(json!({"routeId":"custom/credit-cap","presetId":"generic"})))
        .await
        .unwrap();
    let cid_cred = c_cred["id"].as_str().unwrap();
    assert_eq!(finish_generation(&manager, cid_cred, "seed").await.1["status"], "complete");
    assert_eq!(finish_generation(&manager, cid_cred, "seed2").await.1["status"], "complete");
    let posts_before_cred = server.posts.load(Ordering::SeqCst);
    let (_, ev_cred) = finish_generation(&manager, cid_cred, "retry me").await;
    assert_eq!(ev_cred["status"], "complete");
    assert_eq!(server.posts.load(Ordering::SeqCst) - posts_before_cred, 2);

    store.call(Request::Close).await.unwrap();
}

#[test]
fn g2_retrieval_window_start_for_plan_matches_assemble_limits_under_validation_override() {
    // 30 pairs * ~461 tokens/pair = ~13830 tokens (> 10390 soft for capacity=16384, max_tokens=4096,
    // and < 108800 old 128k default soft) -> both assemble and retrieval_window_start_for_plan step down to 20 pairs (seq 11).
    let pair_text = "palavra ".repeat(85);
    let pairs: Vec<Value> = (1..=30)
        .map(|seq| {
            json!({
                "seq": seq,
                "user": &pair_text,
                "assistant": &pair_text
            })
        })
        .collect();
    let conv = json!({
        "id": "c-g2",
        "presetId": "generic",
        "routeId": "custom/test-model",
        "persona": "Alex",
        "prompt": "Ficha curta.",
        "settings": {}
    });
    let mut plan = presets::request(&conv, vec![], &json!({}));
    plan.capacity["contextWindow"] = json!(16384);
    plan.body["max_tokens"] = json!(4096);

    let data = json!({
        "conv": conv,
        "current": { "seq": 31, "userText": "Qual o segredo do turno 1?" },
        "pairs": pairs,
        "globalSettings": {}
    });

    let built = context::assemble(&data, &json!({}), "send", &plan, None).unwrap();
    assert_eq!(built.report["limits"]["hard"], 12224);
    assert_eq!(built.report["limits"]["soft"], 10390);
    let pairs_included = built.report["pairsIncluded"].as_u64().unwrap() as i64;
    assert!(pairs_included < 30, "expected compaction under 16384/4096 override, got {pairs_included}");

    let win_start = context::retrieval_window_start_for_plan(&data, &plan);
    assert_eq!(
        win_start,
        31 - pairs_included,
        "retrieval_window_start_for_plan must match assemble's compacted history boundary"
    );

    // Also verify compact=true clamps explicit hardLimit > input_capacity identically to assemble
    let mut data_compact = data.clone();
    data_compact["conv"]["settings"]["context"]["hardLimit"] = json!(500_000);
    plan.controls["mobileCompact"] = json!(true);
    let (h_eff, s_eff) = context::effective_context_limits(
        &data_compact["conv"],
        plan.capacity["contextWindow"].as_u64(),
        plan.body["max_tokens"].as_u64().unwrap_or(0),
        true,
    );
    let built_compact = context::assemble(&data_compact, &json!({}), "send", &plan, None).unwrap();
    assert_eq!(h_eff as u64, built_compact.report["limits"]["hard"].as_u64().unwrap());
    assert_eq!(s_eff as u64, built_compact.report["limits"]["soft"].as_u64().unwrap());
}
