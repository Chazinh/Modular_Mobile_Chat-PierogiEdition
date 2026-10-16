//! Lote 2: isolated HTTP fixtures and disposable stores, never a real provider or model.
use axum::{
    Json, Router,
    body::Body,
    http::{Request as HttpRequest, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, post},
};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use modularchat_server::{
    api::AppState,
    config::Config,
    generation::Generation,
    providers::Providers,
    rules,
    store::{Request, Store, TurnRequest},
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;

const REPLY: &str = "Lia: Olá 🚀.";
const DRAFT: &str = "Lia: You said that she was there with the letter.";
const REPAIRED: &str = "Lia: Vamos examinar essa carta com você.";

struct Server {
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn serve(app: Router) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { url, task }
}

#[derive(Clone, Copy, Debug)]
enum Reply {
    Gated,
    StyleOk,
    StyleLength,
    StyleChunked,
    StyleError,
    StyleInvalid,
    SseError,
}
struct Fixture {
    server: Server,
    requests: Arc<Mutex<Vec<Value>>>,
    release: Arc<Semaphore>,
}
async fn fixture(reply: Reply) -> Fixture {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let release = Arc::new(Semaphore::new(0));
    let gate = release.clone();
    let app = Router::new().route("/v1/chat/completions", post(move |Json(body): Json<Value>| {
        let captured = captured.clone();
        let gate = gate.clone();
        async move {
            captured.lock().unwrap().push(body.clone());
            if body["stream"] == true {
                let event = if matches!(reply, Reply::SseError) {
                    json!({"error":{"message":"SECRET_UPSTREAM_BODY"}})
                } else {
                    json!({"choices":[{"delta":{"content":if matches!(reply,Reply::Gated){REPLY}else{DRAFT}}}]})
                };
                let first = futures_util::stream::once(async move {
                    Ok::<_, std::io::Error>(format!("data: {event}\n\n"))
                });
                let end = futures_util::stream::once(async move {
                    if matches!(reply, Reply::Gated) { gate.acquire().await.unwrap().forget(); }
                    Ok::<_, std::io::Error>(format!("data: {}\n\ndata: [DONE]\n\n", json!({"choices":[{"delta":{},"finish_reason":"stop"}]})))
                });
                return ([("content-type", "text/event-stream")], Body::from_stream(first.chain(end))).into_response();
            }
            match reply {
                Reply::StyleLength => Response::builder().header("content-length", 2 * 1024 * 1024 + 1)
                    .body(Body::from_stream(futures_util::stream::pending::<Result<String, std::io::Error>>())).unwrap(),
                Reply::StyleChunked => {
                    let chunks = (0..33).map(|_| Ok::<_, std::io::Error>(vec![b' '; 64 * 1024]));
                    Response::new(Body::from_stream(futures_util::stream::iter(chunks)))
                }
                Reply::StyleError => (StatusCode::BAD_GATEWAY, "SECRET_UPSTREAM_BODY").into_response(),
                Reply::StyleInvalid => "SECRET_INVALID_JSON".into_response(),
                _ => Json(json!({"choices":[{"finish_reason":"stop","message":{"content":REPAIRED}}]})).into_response(),
            }
        }
    }));
    Fixture {
        server: serve(app).await,
        requests,
        release,
    }
}
async fn state(dir: &Path, upstream: &Fixture) -> AppState {
    let providers = Providers::from_json(&json!({"providers":[{"id":"fixture","baseUrl":format!("{}/v1",upstream.server.url),"apiKey":"fixture-key","models":["cbai/glm-5.2"]}]})).unwrap();
    let state = AppState::new(Config {
        data_dir: dir.into(),
        dist: dir.join("absent"),
        address: "127.0.0.1:0".parse().unwrap(),
        require_pairing: false,
        providers: Some(Arc::new(providers)),
        max_generations: 1,
        production: false,
        native_lib_dir: None,
        review: false,
    })
    .await
    .unwrap();
    state.runner.stop();
    state
        .store
        .call(Request::SetGlobalSettings(
            json!({"botChars":2000,"dialoguePct":100,"monologueChars":300}),
        ))
        .await
        .unwrap();
    state
}
async fn conversation(state: &AppState) -> String {
    state.store.call(Request::CreateConversation(json!({"name":"Lote 2 fixture","persona":"Alex","routeId":"cbai/glm-5.2","presetId":"glm-52","prompt":"SCENE_SENTINEL: Lia examina uma carta."}))).await.unwrap()["id"].as_str().unwrap().into()
}
fn turn(conv: &str, op: &str) -> TurnRequest {
    serde_json::from_value(
        json!({"convId":conv,"opId":op,"mode":"send","content":"USER_SENTINEL 🚀: Olá, Lia."}),
    )
    .unwrap()
}
async fn wait_output(g: &Generation, done: bool) {
    let mut wake = g.wake.subscribe();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let output = g.output.lock().await;
            if if done {
                output.final_event.is_some()
            } else {
                !output.buffer.is_empty()
            } {
                break;
            }
            drop(output);
            wake.recv().await.unwrap();
        }
    })
    .await
    .expect("local generation failed to advance");
}
async fn events(response: impl IntoResponse) -> Vec<Value> {
    let bytes = tokio::time::timeout(
        Duration::from_secs(15),
        response.into_response().into_body().collect(),
    )
    .await
    .unwrap()
    .unwrap()
    .to_bytes();
    std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .filter_map(|line| {
            line.strip_prefix("data:")
                .map(|v| serde_json::from_str(v.trim()).unwrap())
        })
        .collect()
}

#[tokio::test]
async fn generation_detaches_releases_output_and_replays_from_persistence_without_a_new_request() {
    let dir = tempfile::tempdir().unwrap();
    let upstream = fixture(Reply::Gated).await;
    let state = state(dir.path(), &upstream).await;
    let conv = conversation(&state).await;
    tokio::fs::write(
        dir.path().join("route-validation.json"),
        json!({"routes":{"cbai/glm-5.2":{"contextWindow":1}}}).to_string(),
    )
    .await
    .unwrap();
    let request = turn(&conv, "same-op");
    let (g, replay) = state.generation.start(request.clone()).await.unwrap();
    assert!(!replay);
    assert_eq!(g.ack["context"]["overCapacity"], true);
    assert_eq!(g.ack["context"]["exceededEstimatedBudget"], true);
    // Disconnect one consumer; detached work and another consumer remain alive.
    drop(state.generation.stream(g.clone(), 0, false).await.unwrap());
    wait_output(&g, false).await;
    let (same, replay) = state.generation.start(request.clone()).await.unwrap();
    assert!(replay && Arc::ptr_eq(&g, &same));
    assert_eq!(upstream.requests.lock().unwrap().len(), 1);
    let consumer = state.generation.stream(g.clone(), 0, false).await.unwrap();
    upstream.release.add_permits(1);
    wait_output(&g, true).await;
    assert!(state.generation.get(&conv, Some(&g.id)).await.is_none());
    let id = g.id.clone();
    let weak = Arc::downgrade(&g);
    drop(same);
    drop(g);
    assert!(
        weak.upgrade().is_some(),
        "active consumers must retain their output"
    );
    let live = events(consumer).await;
    assert!(
        live.iter()
            .any(|v| v["delta"] == REPLY && v["offset"] == rules::utf16_len(REPLY))
    );
    assert!(
        live.iter()
            .any(|v| v["done"] == true && v["status"] == "complete")
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("supervisor/task must not retain finalized replies");
    state.providers.set(None); // Idempotent replay needs neither a destination nor capacity.
    let (replayed, replay) = state.generation.start(request).await.unwrap();
    assert!(replay);
    assert_eq!(replayed.id, id);
    assert_eq!(replayed.ack["replay"], true);
    assert!(replayed.ack.get("content").is_none());
    let persisted = state.generation.persisted(&conv, &id).await.unwrap();
    let offset = rules::utf16_len("Lia: Olá ");
    let replay_events = events(
        state
            .generation
            .stream(persisted.clone(), offset, false)
            .await
            .unwrap(),
    )
    .await;
    assert!(
        replay_events
            .iter()
            .any(|v| v["delta"] == "🚀." && v["offset"] == rules::utf16_len(REPLY))
    );
    assert!(
        replay_events
            .iter()
            .any(|v| v["done"] == true && v["replay"] == true)
    );
    assert!(
        state
            .generation
            .stream(persisted, rules::utf16_len(REPLY) + 1, false)
            .await
            .is_err()
    );
    for _ in 0..2 {
        assert_eq!(
            state.generation.stop(&conv, &id).await.unwrap(),
            json!({"ok":true})
        );
    }
    assert_eq!(
        state
            .generation
            .stop("wrong-chat", &id)
            .await
            .unwrap_err()
            .status,
        404
    );
    let payload = upstream.requests.lock().unwrap()[0].clone();
    let messages = payload["messages"].to_string();
    assert!(messages.contains("SCENE_SENTINEL") && messages.contains("USER_SENTINEL 🚀"));
    assert_eq!(messages.matches("SCENE_SENTINEL").count(), 1);
    assert_eq!(messages.matches("USER_SENTINEL 🚀").count(), 1);
    assert_eq!(payload["model"], "cbai/glm-5.2");
    assert_eq!(upstream.requests.lock().unwrap().len(), 1);
    let db = rusqlite::Connection::open(dir.path().join("modularchat.db")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM generations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(db);
    state.shutdown().await;
}

#[tokio::test]
async fn generation_checkpoint_cancel_and_shutdown_keep_the_persisted_partial_reply() {
    let dir = tempfile::tempdir().unwrap();
    let upstream = fixture(Reply::Gated).await;
    let state = state(dir.path(), &upstream).await;
    let conv = conversation(&state).await;
    let (first, _) = state
        .generation
        .start(turn(&conv, "cancel-op"))
        .await
        .unwrap();
    wait_output(&first, false).await;
    // The existing two-second checkpoint runs while the provider's stream is still open.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let checkpoint = state
        .store
        .call(Request::GetGeneration(first.id.clone()))
        .await
        .unwrap();
    assert_eq!(checkpoint["status"], "running");
    assert_eq!(checkpoint["content"], REPLY);
    state.generation.stop(&conv, &first.id).await.unwrap();
    assert_eq!(
        first.output.lock().await.final_event.as_ref().unwrap()["status"],
        "interrupted"
    );
    let second_conv = conversation(&state).await;
    let (second, _) = state
        .generation
        .start(turn(&second_conv, "shutdown-op"))
        .await
        .unwrap();
    wait_output(&second, false).await;
    state.shutdown().await;
    assert_eq!(
        second.output.lock().await.final_event.as_ref().unwrap()["status"],
        "interrupted"
    );
    let reopened = Store::open(dir.path()).unwrap();
    for g in [&first, &second] {
        let saved = reopened
            .call(Request::GetGeneration(g.id.clone()))
            .await
            .unwrap();
        assert_eq!(saved["status"], "interrupted");
        assert_eq!(saved["content"], REPLY);
    }
    reopened.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn desktop_post_redirects_never_reach_the_second_destination() {
    let dir = tempfile::tempdir().unwrap();
    let upstream = fixture(Reply::Gated).await;
    let state = state(dir.path(), &upstream).await;
    let hits = Arc::new(AtomicUsize::new(0));
    let capture = hits.clone();
    let sink = serve(Router::new().route(
        "/",
        any(move |_: HttpRequest<Body>| {
            capture.fetch_add(1, Ordering::SeqCst);
            async { StatusCode::OK }
        }),
    ))
    .await;
    for code in [
        StatusCode::TEMPORARY_REDIRECT,
        StatusCode::PERMANENT_REDIRECT,
        StatusCode::FOUND,
    ] {
        let target = sink.url.clone();
        let redirect = serve(Router::new().route(
            "/",
            any(move || {
                let target = target.clone();
                async move { (code, [("location", target)]).into_response() }
            }),
        ))
        .await;
        let response = state
            .generation
            .client
            .post(&redirect.url)
            .bearer_auth("fixture-secret")
            .json(&json!({"messages":[{"role":"user","content":"fixture-prompt"}]}))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), code);
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    state.shutdown().await;
}

#[tokio::test]
async fn generation_style_json_is_bounded_and_upstream_error_bodies_are_not_persisted() {
    for reply in [
        Reply::StyleOk,
        Reply::StyleLength,
        Reply::StyleChunked,
        Reply::StyleError,
        Reply::StyleInvalid,
        Reply::SseError,
    ] {
        eprintln!("isolated style boundary fixture: {reply:?}");
        let dir = tempfile::tempdir().unwrap();
        let upstream = fixture(reply).await;
        let state = state(dir.path(), &upstream).await;
        let conv = conversation(&state).await;
        let (g, _) = state
            .generation
            .start(turn(&conv, "style-op"))
            .await
            .unwrap();
        wait_output(&g, true).await;
        let output = g.output.lock().await;
        let final_event = output.final_event.as_ref().unwrap();
        if matches!(reply, Reply::SseError) {
            assert_eq!(final_event["status"], "error");
            assert_eq!(upstream.requests.lock().unwrap().len(), 1);
        } else {
            assert_eq!(final_event["status"], "complete");
            assert_eq!(final_event["usage"]["style"]["attempted"], true);
            assert_eq!(
                final_event["usage"]["style"]["accepted"],
                matches!(reply, Reply::StyleOk)
            );
            assert_eq!(
                output.buffer,
                if matches!(reply, Reply::StyleOk) {
                    REPAIRED
                } else {
                    DRAFT
                }
            );
            assert_eq!(upstream.requests.lock().unwrap().len(), 2);
        }
        assert!(!final_event.to_string().contains("SECRET"));
        drop(output);
        let saved = state
            .store
            .call(Request::GetGeneration(g.id.clone()))
            .await
            .unwrap();
        assert!(!saved.to_string().contains("SECRET"));
        state.shutdown().await;
    }
}

#[tokio::test]
async fn generation_route_validation_preserves_the_strict_file_size_boundary() {
    for length in [64, 2 * 1024 * 1024 - 1, 2 * 1024 * 1024, 32 * 1024 * 1024] {
        let dir = tempfile::tempdir().unwrap();
        let upstream = fixture(Reply::Gated).await;
        let state = state(dir.path(), &upstream).await;
        let path = dir.path().join("route-validation.json");
        let mut bytes = br#"{"routes":{"fixture":{"maxOutput":8192}}}"#.to_vec();
        if length < 32 * 1024 * 1024 {
            bytes.resize(length, b' ');
            tokio::fs::write(&path, bytes).await.unwrap();
        } else {
            tokio::fs::File::create(&path)
                .await
                .unwrap()
                .set_len(length as u64)
                .await
                .unwrap();
        }
        let validation = state.generation.validation().await;
        assert_eq!(
            validation["routes"]["fixture"]["maxOutput"] == 8192,
            length < 2 * 1024 * 1024
        );
        state.shutdown().await;
    }
}
