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
    api::{self, AppState},
    config::Config,
    memory_backend::{MemoryApi, MemoryBackend, MemoryPolicy, validate_schema},
    providers::{MobileProviders, RuntimeProviders, SelectedBackend},
    store::{Request, TurnRequest},
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tower::ServiceExt;

const TOKEN: &str = "lote3r_fixture_0123456789abcdef0123456789abcdef0123456789abcdef";
const KEY: &str = "fixture-provider-private-key";
const MODEL: &str = "org/exact-model:revision";

#[derive(Clone, Copy)]
enum Reply {
    Ok,
    Error,
    ContextWindow,
    Partial,
    Required,
    Extra,
    Length,
    BadModels,
    WrongModel,
    SlowModels,
    FormatAndReasoning,
    AllFormats,
    SchemaIgnored,
    WrappedSchemaRejection,
    WrappedErrorWithPartialResult,
    CapabilityAfterServerError,
    GenericBadRequest,
    SlowPost,
    Redirect,
}

struct Fixture {
    url: String,
    calls: Arc<Mutex<Vec<(String, String, Value)>>>,
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
            .filter(|(method, _, _)| method == "POST")
            .count()
    }
}
async fn fixture(reply: Reply) -> Fixture {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let gets = calls.clone();
    let posts = calls.clone();
    let app = Router::new()
        .route("/v1/models", get(move |headers: HeaderMap| {
            let calls = gets.clone();
            async move {
                calls.lock().unwrap().push(("GET".into(), auth(&headers), Value::Null));
                match reply {
                    Reply::BadModels => Json(json!({"ok":true})).into_response(),
                    Reply::WrongModel => Json(json!({"data":[{"id":"another-model"}]})).into_response(),
                    Reply::Redirect => (StatusCode::FOUND, [("location", "/v1/chat/completions")]).into_response(),
                    Reply::SlowModels => {
                        let stream = futures_util::stream::once(async {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            Ok::<_, std::io::Error>(format!("{{\"data\":[{{\"id\":\"{MODEL}\"}}]}}"))
                        });
                        Response::new(Body::from_stream(stream))
                    },
                    _ => Json(json!({"data":[{"id":MODEL}]})).into_response(),
                }
            }
        }))
        .route("/v1/chat/completions", post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let calls = posts.clone();
            async move {
                calls.lock().unwrap().push(("POST".into(), auth(&headers), body.clone()));
                if matches!(reply, Reply::Error | Reply::CapabilityAfterServerError) {
                    return (StatusCode::BAD_GATEWAY, "json_schema unsupported; not a capability rejection").into_response();
                }
                if matches!(reply, Reply::GenericBadRequest) {
                    return (StatusCode::BAD_REQUEST, "unavailable account; invalid input").into_response();
                }
                if matches!(reply, Reply::SlowPost) {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                if matches!(reply, Reply::WrappedErrorWithPartialResult) {
                    return Json(json!({"error":{"message":"invalid params, Mismatch type string with value array", "code":400},"choices":[{"finish_reason":"stop","message":{"content":"{}"}}]})).into_response();
                }
                if matches!(reply, Reply::WrappedSchemaRejection) && body["response_format"]["type"] == "json_schema" {
                    return Json(json!({"error":{"message":"Minimax midstream error: invalid params, Mismatch type string with value array", "code":400},"object":"chat.completion"})).into_response();
                }
                if matches!(reply, Reply::SchemaIgnored) {
                    // The upstream accepts json_schema but answers with a bare list: the schema is ignored.
                    let described = body["messages"].as_array().unwrap().iter().any(|m| {
                        m["content"].as_str().is_some_and(|text| text.contains(&schema().to_string()))
                    });
                    if described {
                        return Json(json!({"choices":[{"finish_reason":"stop","message":{"content":"{\"answer\":\"ok\"}"}}]})).into_response();
                    }
                    return Json(json!({"choices":[{"finish_reason":"stop","message":{"content":"[\"Mira\",\"Nora\"]"}}]})).into_response();
                }
                if matches!(reply, Reply::FormatAndReasoning | Reply::AllFormats) {
                    if body["response_format"]["type"] == "json_schema" {
                        return (StatusCode::BAD_REQUEST, "response_format json_schema is unsupported").into_response();
                    }
                    if body["thinking"]["type"] == "disabled" {
                        return (StatusCode::BAD_REQUEST, "Reasoning is mandatory for this endpoint and cannot be disabled.").into_response();
                    }
                    if matches!(reply, Reply::AllFormats) && body.get("response_format").is_some() {
                        return (StatusCode::BAD_REQUEST, "response_format json_object is unavailable").into_response();
                    }
                    let described = body["messages"].as_array().unwrap().iter().any(|m| {
                        m["content"].as_str().is_some_and(|text| text.contains(&schema().to_string()))
                    });
                    if !described {
                        return Json(json!({"choices":[{"finish_reason":"stop","message":{"content":"{}"}}]})).into_response();
                    }
                }
                if matches!(reply, Reply::ContextWindow) { return (StatusCode::BAD_REQUEST, "physical context window exceeded").into_response(); }
                if matches!(reply, Reply::Partial) {
                    let event = format!("data: {}\n\n", json!({"choices":[{"delta":{"content":"Resposta parcial preservada."}}]}));
                    use futures_util::StreamExt;
                    let stream = futures_util::stream::once(async move { Ok::<_,std::io::Error>(event) })
                        .chain(futures_util::stream::once(async {
                            tokio::time::sleep(Duration::from_millis(40)).await;
                            Err(std::io::Error::other("fixture stream interrupted"))
                        }));
                    return ([("content-type", "text/event-stream")], Body::from_stream(stream)).into_response();
                }
                let content = match reply {
                    Reply::Required => json!({}),
                    Reply::Extra => json!({"answer":"ok","unapproved":true}),
                    _ => json!({"answer":"ok"}),
                };
                Json(json!({"choices":[{"finish_reason":if matches!(reply,Reply::Length){"length"}else{"stop"},"message":{"content":content.to_string()}}]})).into_response()
            }
        }));
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Fixture { url, calls, task }
}
fn auth(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .into()
}
fn provider(id: &str, url: &str, key: &str) -> Value {
    json!({"id":id,"baseUrl":url,"apiKey":key,"models":[MODEL],"allowLoopback":true})
}
fn config(url: &str) -> Value {
    json!({"default":"phone","providers":[provider("phone",url,KEY)],"aliases":{"legacy/route":{"provider":"phone","model":MODEL}}})
}
fn schema() -> Value {
    json!({"type":"object","required":["answer"],"additionalProperties":false,"properties":{"answer":{"type":"string"}}})
}
async fn state(dir: &std::path::Path, native: &std::path::Path) -> AppState {
    let state = AppState::new(Config::for_android(dir, native).unwrap())
        .await
        .unwrap();
    if state.auth.verify(TOKEN).await.unwrap().is_null() {
        state
            .store
            .call(Request::AddDevice {
                name: "Lote3R isolated fixture".into(),
                token_hash: modularchat_server::rules::hash(TOKEN),
            })
            .await
            .unwrap();
    }
    state
}
async fn call(state: &AppState, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    let request = HttpRequest::builder()
        .uri(path)
        .method(method)
        .extension(ConnectInfo(
            "127.0.0.1:43210".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(
            body.map(|v| Body::from(v.to_string()))
                .unwrap_or_else(Body::empty),
        )
        .unwrap();
    let response = api::router(state.clone()).oneshot(request).await.unwrap();
    let code = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (code, serde_json::from_slice(&bytes).unwrap())
}

#[test]
fn mobile_config_rejects_desktop_fields_unknowns_unsafe_urls_and_native_anthropic() {
    let valid = config("https://openrouter.ai/api/v1");
    assert!(MobileProviders::parse(&valid).is_ok());
    for field in ["requestLog", "apiKeyEnv", "unknown"] {
        let mut value = valid.clone();
        value["providers"][0][field] = json!("not-permitted");
        assert!(MobileProviders::parse(&value).is_err(), "{field}");
    }
    for field in ["env", "requestLog", "unexpected"] {
        let mut value = valid.clone();
        value[field] = json!(true);
        assert!(MobileProviders::parse(&value).is_err(), "{field}");
    }
    for url in [
        "http://192.168.0.2/v1",
        "http://example.com/v1",
        "https://api.anthropic.com/v1",
        "https://example.com/v1/messages/",
        "https://example.com/v1/chat/completions/",
        "https://user:secret@example.com/v1",
        "https://example.com/v1?api_key=secret",
    ] {
        let mut value = valid.clone();
        value["providers"][0]["baseUrl"] = json!(url);
        let error = MobileProviders::parse(&value).unwrap_err();
        assert!(!error.message.contains("secret"));
    }
    let mut no_opt_in = config("http://127.0.0.1:12345/v1");
    no_opt_in["providers"][0]["allowLoopback"] = json!(false);
    assert!(MobileProviders::parse(&no_opt_in).is_err());
    let mut alias = valid.clone();
    alias["aliases"]["legacy/route"]["unexpected"] = json!(true);
    assert!(MobileProviders::parse(&alias).is_err());
    let config = MobileProviders::parse(&valid).unwrap();
    assert!(!format!("{config:?}").contains(KEY));
    assert!(!format!("{:?}", config.runtime().unwrap()).contains(KEY));
    assert!(!config.redacted().to_string().contains(KEY));
}

#[tokio::test]
async fn alias_selection_and_cloud_post_use_exact_model_and_own_key() {
    let upstream = fixture(Reply::Ok).await;
    let providers = MobileProviders::parse(&config(&upstream.url))
        .unwrap()
        .runtime()
        .unwrap()
        .unwrap();
    let SelectedBackend::Cloud {
        provider,
        model,
        preflight_fallback,
    } = providers.select("legacy/route").await
    else {
        panic!("API was not selected");
    };
    assert_eq!(
        (provider.id.as_str(), model.as_str(), preflight_fallback),
        ("phone", MODEL, false)
    );
    provider
        .chat_request(
            &reqwest::Client::new(),
            &model,
            &json!({"model":"legacy/route","messages":[],"stream":false}),
            Duration::from_secs(2),
        )
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let calls = upstream.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, "GET");
    assert_eq!(calls[0].1, format!("Bearer {KEY}"));
    assert_eq!(calls[1].1, format!("Bearer {KEY}"));
    assert_eq!(calls[1].2["model"], MODEL);
}

#[tokio::test]
async fn closed_loopback_uses_only_explicit_fallback_and_snapshot_stays_pinned() {
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let closed = format!("http://{}/v1", socket.local_addr().unwrap());
    drop(socket);
    let backup = fixture(Reply::Ok).await;
    let mut value = config(&closed);
    value["providers"].as_array_mut().unwrap().push(provider(
        "backup",
        &backup.url,
        "fixture-backup-key",
    ));
    value["fallbacks"] = json!({"legacy/route":{"provider":"backup","model":MODEL}});
    let runtime = RuntimeProviders::new(MobileProviders::parse(&value).unwrap().runtime().unwrap());
    let snapshot = runtime.snapshot().unwrap();
    runtime.set(None);
    let selected = snapshot.select("legacy/route").await;
    let SelectedBackend::Cloud {
        provider,
        model,
        preflight_fallback,
    } = &selected
    else {
        panic!("explicit fallback missing");
    };
    assert_eq!(
        (provider.id.as_str(), model.as_str(), *preflight_fallback),
        ("backup", MODEL, true)
    );
    assert!(
        !format!(
            "{snapshot:?}{selected:?}{}",
            selected.diagnostic("legacy/route")
        )
        .contains("fixture-backup-key")
    );
    assert_eq!(backup.posts(), 0, "selection must never POST");
    assert!(runtime.snapshot().is_none());
    value.as_object_mut().unwrap().remove("fallbacks");
    let providers = MobileProviders::parse(&value)
        .unwrap()
        .runtime()
        .unwrap()
        .unwrap();
    assert!(matches!(
        providers.select("legacy/route").await,
        SelectedBackend::Local
    ));
    assert!(matches!(
        providers.select("unmapped-old-alias").await,
        SelectedBackend::Local
    ));
    value["fallbacks"] = json!({"unmapped-old-alias":{"provider":"backup","model":MODEL}});
    assert!(MobileProviders::parse(&value).is_err());
}

#[tokio::test]
async fn preflight_rejects_wrong_identity_redirects_and_slow_body_without_post() {
    for reply in [
        Reply::BadModels,
        Reply::WrongModel,
        Reply::SlowModels,
        Reply::Redirect,
    ] {
        let upstream = fixture(reply).await;
        let providers = MobileProviders::parse(&config(&upstream.url))
            .unwrap()
            .runtime()
            .unwrap()
            .unwrap();
        let start = Instant::now();
        assert!(matches!(
            providers.select("legacy/route").await,
            SelectedBackend::Local
        ));
        assert!(start.elapsed() < Duration::from_millis(650));
        assert_eq!(upstream.posts(), 0);
        assert_eq!(upstream.calls.lock().unwrap().len(), 1);
    }
}

#[test]
fn policy_requires_consent_destination_and_bounded_budgets() {
    assert_eq!(
        serde_json::to_value(MemoryPolicy::default()).unwrap(),
        json!({"mode":"deferred"})
    );
    for value in [
        json!({"mode":"api"}),
        json!({"mode":"remote"}),
        json!({"mode":"deferred","unknown":true}),
        json!({"mode":"api","api":{"provider":"p","model":"m","historyConsent":false}}),
        json!({"mode":"api","api":{"provider":"p","model":"m","historyConsent":true,"maxTokens":255}}),
        json!({"mode":"api","api":{"provider":"p","model":"m","historyConsent":true,"maxInputTokens":16385}}),
        json!({"mode":"api","api":{"provider":"p","model":"m","historyConsent":true,"timeoutSeconds":121}}),
        json!({"mode":"api","api":{"provider":"p","model":"m","historyConsent":true,"apiKey":"forbidden"}}),
    ] {
        assert!(MemoryPolicy::parse(&value).is_err(), "{value}");
    }
    let runtime = RuntimeProviders::new(
        MobileProviders::parse(&config("https://openrouter.ai/api/v1"))
            .unwrap()
            .runtime()
            .unwrap(),
    );
    let valid =
        json!({"mode":"api","api":{"provider":"phone","model":MODEL,"historyConsent":true}});
    assert!(
        MemoryPolicy::parse(&valid)
            .unwrap()
            .check_destination(&runtime)
            .is_ok()
    );
    for (field, value) in [("provider", "unknown"), ("model", "not-configured")] {
        let mut invalid = valid.clone();
        invalid["api"][field] = json!(value);
        assert!(
            MemoryPolicy::parse(&invalid)
                .unwrap()
                .check_destination(&runtime)
                .is_err()
        );
    }
    assert!(
        MemoryPolicy::parse(&json!({"mode":"local"}))
            .unwrap()
            .check_destination(&RuntimeProviders::default())
            .is_ok()
    );
}

#[test]
fn schema_requires_fields_and_rejects_extra_nested_or_wrong_types() {
    let extraction: Value =
        serde_json::from_str(include_str!("../src/extraction-schema.json")).unwrap();
    let mut empty_scene = json!({"summary":"Sem mudança de cena.","cast":[],"scene":null,"facts":[],"relationships":[],"open_threads":[],"resolved_threads":[]});
    assert!(validate_schema(&empty_scene, &extraction).is_ok());
    empty_scene.as_object_mut().unwrap().remove("facts");
    assert!(validate_schema(&empty_scene, &extraction).is_err());
    assert!(validate_schema(&json!({"answer":"ok"}), &schema()).is_ok());
    for value in [
        json!({}),
        json!({"answer":null}),
        json!({"answer":"ok","extra":true}),
    ] {
        assert!(validate_schema(&value, &schema()).is_err());
    }
    let nested = json!({"type":"array","items":schema()});
    assert!(validate_schema(&json!([{"answer":"ok"},{}]), &nested).is_err());
}

#[tokio::test]
async fn memory_api_rejects_incomplete_schema_length_and_errors_without_rerouting() {
    for reply in [
        Reply::Ok,
        Reply::Required,
        Reply::Extra,
        Reply::Length,
        Reply::Error,
        Reply::Partial,
        Reply::CapabilityAfterServerError,
        Reply::WrappedErrorWithPartialResult,
        Reply::GenericBadRequest,
        Reply::SlowPost,
    ] {
        let upstream = fixture(reply).await;
        let providers = MobileProviders::parse(&config(&upstream.url))
            .unwrap()
            .runtime()
            .unwrap()
            .unwrap();
        let (provider, _) = providers
            .target(&modularchat_server::providers::RouteTarget {
                provider: "phone".into(),
                model: MODEL.into(),
            })
            .unwrap();
        let backend = MemoryBackend::Api {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            provider,
            api: MemoryApi {
                provider: "phone".into(),
                model: MODEL.into(),
                history_consent: true,
                max_tokens: 600,
                max_input_tokens: 1024,
                timeout_seconds: if matches!(reply, Reply::SlowPost) { 1 } else { 5 },
            },
            preferred_format: std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0)),
            runtime: None,
        };
        let result = backend
            .chat_json(
                vec![json!({"role":"user","content":"Fixture only."})],
                schema(),
                1200,
                0.0,
            )
            .await;
        assert_eq!(result.is_ok(), matches!(reply, Reply::Ok));
        assert_eq!(upstream.posts(), 1);
        let calls = upstream.calls.lock().unwrap();
        assert_eq!(calls[0].1, format!("Bearer {KEY}"));
        assert_eq!(calls[0].2["max_tokens"], 600);
        assert_eq!(calls[0].2["model"], MODEL);
        assert_eq!(calls[0].2["response_format"]["json_schema"]["strict"], true);
    }
}

#[tokio::test]
async fn memory_capability_fallback_preserves_schema_prompt_and_required_reasoning() {
    for (reply, attempts) in [(Reply::FormatAndReasoning, 3), (Reply::AllFormats, 4)] {
        let upstream = fixture(reply).await;
        let providers = MobileProviders::parse(&config(&upstream.url)).unwrap().runtime().unwrap().unwrap();
        let (provider, _) = providers.target(&modularchat_server::providers::RouteTarget {
            provider: "phone".into(), model: MODEL.into(),
        }).unwrap();
        let backend = MemoryBackend::Api {
            client: reqwest::Client::new(), provider,
            api: MemoryApi { provider: "phone".into(), model: MODEL.into(), history_consent: true,
                max_tokens: 600, max_input_tokens: 4096, timeout_seconds: 5 },
            preferred_format: std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0)),
            runtime: None,
        };
        let value = backend.chat_json(vec![
            json!({"role":"system","content":"Preserve original instructions."}),
            json!({"role":"user","content":"Synthetic memory turn."}),
        ], schema(), 600, 0.1).await.unwrap();
        assert_eq!(value, json!({"answer":"ok"}));
        assert_eq!(upstream.posts(), attempts);
        let calls = upstream.calls.lock().unwrap();
        let original = calls[0].2["messages"].as_array().unwrap();
        for (index, (_, authorization, body)) in calls.iter().enumerate() {
            assert_eq!(authorization, &format!("Bearer {KEY}"));
            assert_eq!(body["model"], MODEL);
            assert_eq!(body["max_tokens"], 600);
            assert_eq!(body["stream"], false);
            let messages = body["messages"].as_array().unwrap();
            assert_eq!(&messages[..original.len()], original.as_slice());
            assert_eq!(messages.len(), original.len(), "schema is included exactly once");
            assert!(messages.last().unwrap()["content"].as_str().unwrap().contains(&schema().to_string()));
            assert!(index < attempts);
        }
        assert_eq!(calls[0].2["response_format"]["type"], "json_schema");
        assert_eq!(calls[1].2["response_format"]["type"], "json_object");
        assert_eq!(calls[1].2["thinking"]["type"], "disabled");
        assert!(calls[2].2.get("thinking").is_none());
        assert!(calls[2].2.get("chat_template_kwargs").is_none());
        assert_eq!(calls[2].2["reasoning_effort"], "low");
        if attempts == 4 { assert!(calls[3].2.get("response_format").is_none()); }
    }
}

#[tokio::test]
async fn memory_ignored_schema_receives_the_schema_in_the_first_prompt() {
    for (reply, count) in [(Reply::SchemaIgnored, 1), (Reply::WrappedSchemaRejection, 2)] {
    let upstream = fixture(reply).await;
    let providers = MobileProviders::parse(&config(&upstream.url)).unwrap().runtime().unwrap().unwrap();
    let (provider, _) = providers.target(&modularchat_server::providers::RouteTarget {
        provider: "phone".into(), model: MODEL.into(),
    }).unwrap();
    let backend = MemoryBackend::Api {
        client: reqwest::Client::new(), provider,
        api: MemoryApi { provider: "phone".into(), model: MODEL.into(), history_consent: true,
            max_tokens: 600, max_input_tokens: 4096, timeout_seconds: 5 },
        preferred_format: std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0)),
        runtime: None,
    };
    let value = backend.chat_json(vec![json!({"role":"user","content":"Synthetic memory turn."})], schema(), 600, 0.1).await.unwrap();
    assert_eq!(value, json!({"answer":"ok"}));
    assert_eq!(upstream.posts(), count);
    let calls = upstream.calls.lock().unwrap();
    assert_eq!(calls[0].2["response_format"]["type"], "json_schema");
    assert_eq!(calls[0].2["messages"][0]["content"], "Synthetic memory turn.");
    assert!(calls[0].2["messages"].as_array().unwrap().iter().any(|m| m["content"].as_str().is_some_and(|t| t.contains(&schema().to_string()))));
    if count == 2 {
        assert_eq!(calls[1].2["response_format"]["type"], "json_object");
        assert_eq!(calls[0].2["messages"], calls[1].2["messages"]);
    }
    }
}

#[tokio::test]
async fn providers_get_is_redacted_keys_persist_only_for_same_endpoint_and_reload() {
    let dir = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let state = state(dir.path(), native.path()).await;
    let mut body = config("https://openrouter.ai/api/v1");
    for field in ["requestLog", "apiKeyEnv", "unknown"] {
        let mut invalid = body.clone();
        invalid["providers"][0][field] = json!(KEY);
        let (code, error) = call(&state, "PUT", "/api/local/providers", Some(invalid)).await;
        assert_eq!(code, 400);
        assert!(!error.to_string().contains(KEY));
    }
    assert_eq!(call(&state,"POST","/api/local/policy",Some(json!({"mode":"api","api":{"provider":"phone","model":MODEL,"historyConsent":false}}))).await.0,400);
    assert_eq!(
        call(&state, "PUT", "/api/local/providers", Some(body.clone()))
            .await
            .0,
        200
    );
    let (code, public) = call(&state, "GET", "/api/local/providers", None).await;
    assert_eq!(code, 200);
    assert_eq!(public["providers"][0]["hasApiKey"], true);
    assert!(public["providers"][0].get("apiKey").is_none());
    assert!(!public.to_string().contains(KEY));
    assert!(!format!("{:?}", state.providers.snapshot()).contains(KEY));
    let export = state.store.call(Request::Export(None)).await.unwrap();
    assert!(!export.to_string().contains(KEY));
    body["providers"][0]
        .as_object_mut()
        .unwrap()
        .remove("apiKey");
    assert_eq!(
        call(&state, "PUT", "/api/local/providers", Some(body.clone()))
            .await
            .1["providers"][0]["hasApiKey"],
        true
    );
    assert_eq!(
        call(
            &state,
            "PUT",
            "/api/local/policy",
            Some(
                json!({"mode":"api","api":{"provider":"phone","model":MODEL,"historyConsent":true}})
            )
        )
        .await
        .0,
        200
    );
    state.shutdown().await;
    let encrypted = std::fs::read(dir.path().join("modularchat.db.vault")).unwrap();
    assert!(!encrypted.windows(KEY.len()).any(|w| w == KEY.as_bytes()));
    let state = self::state(dir.path(), native.path()).await;
    assert_eq!(
        call(&state, "GET", "/api/local/providers", None).await.1["providers"][0]["hasApiKey"],
        true
    );
    assert_eq!(
        call(&state, "GET", "/api/local/policy", None).await.1["memoryPolicy"]["mode"],
        "api"
    );
    let snapshot = state.providers.snapshot().unwrap();
    let (saved, _) = snapshot
        .target(&modularchat_server::providers::RouteTarget {
            provider: "phone".into(),
            model: MODEL.into(),
        })
        .unwrap();
    assert_eq!(saved.api_key.as_deref(), Some(KEY));
    body["providers"][0]["baseUrl"] = json!("https://another.example/v1");
    assert_eq!(
        call(&state, "PUT", "/api/local/providers", Some(body.clone()))
            .await
            .1["providers"][0]["hasApiKey"],
        false
    );
    body["providers"][0]["apiKey"] = json!(KEY);
    assert_eq!(
        call(&state, "PUT", "/api/local/providers", Some(body.clone()))
            .await
            .0,
        200
    );
    body["providers"][0]["apiKey"] = Value::Null;
    assert_eq!(
        call(&state, "PUT", "/api/local/providers", Some(body))
            .await
            .1["providers"][0]["hasApiKey"],
        false
    );
    assert_eq!(
        saved.api_key.as_deref(),
        Some(KEY),
        "in-flight snapshot must remain intact"
    );
    state.shutdown().await;
}

async fn generation_finishes(
    state: &AppState,
    route: &str,
) -> Arc<modularchat_server::generation::Generation> {
    let conv = state
        .store
        .call(Request::CreateConversation(
            json!({"routeId":route,"presetId":"glm-52","name":"Lote3R fixture","settings":{"context":{"hardLimit":1}}}),
        ))
        .await
        .unwrap();
    let request: TurnRequest = serde_json::from_value(json!({"convId":conv["id"],"mode":"send","content":"Olá, personagem.","opId":modularchat_server::id()})).unwrap();
    let (generation, _) = state.generation.start(request).await.unwrap();
    // Reconfiguration after selection must not alter the accepted route.
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
    generation
}

#[tokio::test]
async fn generation_post_error_and_partial_stream_never_send_to_fallback() {
    for reply in [Reply::Error, Reply::ContextWindow, Reply::Partial] {
        let primary = fixture(reply).await;
        let backup = fixture(Reply::Ok).await;
        let dir = tempfile::tempdir().unwrap();
        let native = tempfile::tempdir().unwrap();
        let state = state(dir.path(), native.path()).await;
        let mut value = config(&primary.url);
        value["aliases"] = json!({"cbai/glm-5.2":{"provider":"phone","model":MODEL}});
        value["providers"].as_array_mut().unwrap().push(provider(
            "backup",
            &backup.url,
            "fixture-backup-key",
        ));
        value["fallbacks"] = json!({"cbai/glm-5.2":{"provider":"backup","model":MODEL}});
        assert_eq!(
            call(&state, "PUT", "/api/local/providers", Some(value))
                .await
                .0,
            200
        );
        let generation = generation_finishes(&state, "cbai/glm-5.2").await;
        assert_eq!(primary.posts(), 1);
        assert_eq!(backup.posts(), 0);
        assert_eq!(generation.ack["context"]["exceededEstimatedBudget"], true);
        if matches!(reply, Reply::ContextWindow) {
            let output = generation.output.lock().await;
            let done = output.final_event.as_ref().unwrap();
            assert_eq!(done["status"], "error");
            assert!(done["error"].as_str().unwrap().contains("HTTP 400"));
            drop(output);
            let conv = state.store.call(Request::GetConversation(generation.conv.clone())).await.unwrap();
            assert_eq!(conv["history"].as_array().unwrap().len(), 1);
            assert_eq!(conv["history"][0]["content"], "Olá, personagem.");
            assert!(primary.calls.lock().unwrap().iter().find(|c| c.0 == "POST").unwrap().2["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().contains("Olá, personagem."));
            let request: TurnRequest = serde_json::from_value(json!({
                "convId":generation.conv,"mode":"send","content":"Olá, personagem.",
                "opId":generation.ack["opId"]
            })).unwrap();
            let (replayed, replay) = state.generation.start(request).await.unwrap();
            assert!(replay);
            assert_eq!(replayed.id, generation.id);
            assert_eq!(replayed.ack["opId"], generation.ack["opId"]);
            assert_eq!(replayed.output.lock().await.final_event.as_ref().unwrap()["status"], "error");
            assert_eq!(primary.posts(), 1);
            assert_eq!(backup.posts(), 0);
        }
        if matches!(reply, Reply::Partial) {
            assert_eq!(
                generation.output.lock().await.buffer,
                "Resposta parcial preservada."
            );
        }
        assert_eq!(
            primary
                .calls
                .lock()
                .unwrap()
                .iter()
                .find(|c| c.0 == "POST")
                .unwrap()
                .2["model"],
            MODEL
        );
        assert_eq!(
            generation.ack["context"]["routing"]["requestedRoute"],
            "cbai/glm-5.2"
        );
        let meta = state
            .store
            .call(Request::GetMeta(generation.conv.clone()))
            .await
            .unwrap();
        assert_eq!(meta["routeId"], "cbai/glm-5.2");
        state.shutdown().await;
    }
}

#[tokio::test]
async fn mobile_without_provider_requires_explicit_local_model_without_mutating_remote_chat() {
    let dir = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let state = state(dir.path(), native.path()).await;
    for route in ["cbai/glm-5.2", "oc/deepseek-v4.1-flash-free", "mobile-api:missing:model"] {
        let conv = state.store.call(Request::CreateConversation(json!({"routeId":route,"presetId":"glm-52","name":"Remote model fixture"}))).await.unwrap();
        let request: TurnRequest = serde_json::from_value(json!({"convId":conv["id"],"mode":"send","content":"Mensagem preservada.","opId":modularchat_server::id()})).unwrap();
        let error = match state.generation.start(request).await { Err(e) => e, Ok(_) => panic!("Remote route ran with local model") };
        assert_eq!(error.status,409);
        assert_eq!(error.extra["code"],"REMOTE_MODEL_UNAVAILABLE");
        let saved = state.store.call(Request::GetConversation(conv["id"].as_str().unwrap().into())).await.unwrap();
        assert_eq!(saved["routeId"],route);
        assert_eq!(saved["history"].as_array().unwrap().len(),0);
    }
    let generation = generation_finishes(&state, "mobile-local").await;
    assert_eq!(
        generation.ack["context"]["routing"]["effectiveRoute"],
        "mobile-local"
    );
    assert_eq!(
        generation.ack["context"]["routing"]["requestedRoute"],
        "mobile-local"
    );
    let conv = state
        .store
        .call(Request::GetConversation(generation.conv.clone()))
        .await
        .unwrap();
    assert_eq!(conv["routeId"], "mobile-local");
    assert!(conv.to_string().contains("Olá, personagem."));
    assert!(generation.output.lock().await.final_event.is_some());
    state.shutdown().await;
}
