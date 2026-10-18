use axum::{
    Json, Router,
    body::Body,
    extract::ConnectInfo,
    http::{HeaderMap, Request as HttpRequest},
    response::IntoResponse,
    routing::{get, post},
};
use http_body_util::BodyExt;
use modularchat_server::{
    api::{self, AppState},
    config::Config,
    local::LocalModels,
    memory_backend::{MemoryBackend, MemoryMode, MemoryPolicy},
    providers::{Providers, RuntimeProviders},
    store::Request,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

struct FixtureServer {
    url: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start_fixture(model_id: &str, large_catalog: bool) -> FixtureServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/v1");

    let model_id_owned = model_id.to_string();
    let app = Router::new()
        .route(
            "/v1/models",
            get(move || {
                let m = model_id_owned.clone();
                async move {
                    if large_catalog {
                        // Simulate 9Router with >400 models (>150KB payload)
                        let mut models: Vec<Value> = Vec::new();
                        for i in 0..500 {
                            models.push(json!({
                                "id": format!("provider-{i}/model-variant-description-name-number-{i}"),
                                "object": "model",
                                "created": 1700000000,
                                "owned_by": "9router"
                            }));
                        }
                        models.push(json!({
                            "id": m,
                            "object": "model",
                            "created": 1700000000,
                            "owned_by": "fixture"
                        }));
                        Json(json!({ "data": models })).into_response()
                    } else {
                        Json(json!({
                            "data": [{ "id": m }]
                        }))
                        .into_response()
                    }
                }
            }),
        )
        .route(
            "/v1/chat/completions",
            post(|_headers: HeaderMap, Json(body): Json<Value>| async move {
                // Return response with thinking tags and markdown code fence
                let content = r#"<think>
Analyzing turn facts and location...
The user and companion are at the harbor.
</think>
```json
{
  "location": "Porto da Cidade",
  "time": "Entardecer",
  "mood": "Calmo",
  "characters": [
    {
      "key": "alex",
      "name": "Alex",
      "position": "ao lado do cais",
      "location": "Porto da Cidade",
      "holding": ["caderno"]
    }
  ],
  "memories": [
    {
      "fact": "Alex observa os barcos de pesca no porto.",
      "importance": 0.8,
      "kind": "CHARACTER_FACT",
      "target": "alex",
      "via": "witnessed",
      "visibility": "public"
    }
  ]
}
```"#;
                Json(json!({
                    "id": "chatcmpl-test",
                    "object": "chat.completion",
                    "created": 1700000000,
                    "model": body["model"],
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": content
                        },
                        "finish_reason": "stop"
                    }]
                }))
                .into_response()
            }),
        );

    let task = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service()).await.ok();
    });

    FixtureServer { url, task }
}

#[tokio::test]
async fn regression_desktop_memory_fallback_resolves_and_processes() {
    let model = "oc/deepseek-v4.1-flash-free";
    let fixture = start_fixture(model, true).await;

    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().canonicalize().unwrap();
    // Desktop configuration: native_lib_dir is None, is_android is false
    let mut config = Config {
        dist: data_dir.join("client-dist"),
        data_dir: data_dir.clone(),
        address: "127.0.0.1:3011".parse().unwrap(),
        require_pairing: false,
        providers: None,
        max_generations: 1,
        production: false,
        native_lib_dir: None,
        review: false,
    };

    let client = reqwest::Client::new();
    let local = LocalModels::new(&config, client.clone()).unwrap();
    assert!(
        !local.chat_available(),
        "Local GGUF model must NOT be running/available in this test"
    );

    let providers_json = json!({
        "providers": [{
            "id": "fixture-provider",
            "baseUrl": fixture.url,
            "apiKey": "dummy-key",
            "models": [model]
        }],
        "default": "fixture-provider"
    });
    let providers_obj = Arc::new(Providers::from_json(&providers_json).unwrap());
    let providers = RuntimeProviders::new(Some(providers_obj.clone()));

    // 1. Resolve on Desktop with default Local policy -> falls back to desktop upstream API
    let default_policy = MemoryPolicy {
        mode: MemoryMode::Local,
        api: None,
    };

    let backend = MemoryBackend::resolve(&local, &providers, &default_policy, &client)
        .await
        .expect("Resolution must succeed");

    assert!(
        backend.is_some(),
        "MemoryBackend must resolve to upstream API when local is unavailable on desktop"
    );

    let backend = backend.unwrap();
    assert!(
        backend.available(),
        "Resolved backend must report available() == true"
    );

    // 2. Resolve on Desktop with Deferred policy -> returns None
    let deferred_policy = MemoryPolicy {
        mode: MemoryMode::Deferred,
        api: None,
    };
    let deferred_backend = MemoryBackend::resolve(&local, &providers, &deferred_policy, &client)
        .await
        .unwrap();
    assert!(
        deferred_backend.is_none(),
        "Deferred policy must return None"
    );

    // 3. Test chat_json execution through the resolved backend with thinking tags & markdown code fence
    let schema = json!({
        "type": "object",
        "properties": {
            "location": {"type": "string"},
            "mood": {"type": "string"}
        },
        "required": ["location", "mood"]
    });
    let messages = vec![
        json!({"role": "system", "content": "Extraia a cena da conversa."}),
        json!({"role": "user", "content": "Estamos no porto ao entardecer."}),
    ];

    let extraction = backend
        .chat_json(messages, schema, 1000, 0.1)
        .await
        .expect("chat_json must succeed and parse through <think> and markdown code fences");

    assert_eq!(extraction["location"], "Porto da Cidade");
    assert_eq!(extraction["mood"], "Calmo");

    // 4. Test desktop_upstream_model helper
    let upstream_model = MemoryBackend::desktop_upstream_model(&providers, &default_policy);
    assert_eq!(upstream_model, Some(model.to_string()));

    // 5. Test AppState status reporting via /api/status on desktop
    config.providers = Some(providers_obj);
    let app_state = AppState::new(config).await.unwrap();

    let request = HttpRequest::builder()
        .uri("/api/status")
        .method("GET")
        .extension(ConnectInfo(
            "127.0.0.1:43210".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .body(Body::empty())
        .unwrap();

    let response = api::router(app_state.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.status().as_u16(), 200);

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let status: Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(
        status["local"]["chat"]["ok"], true,
        "Desktop /api/status must report local.chat.ok = true when upstream is available"
    );
    assert!(
        status["local"]["chat"]["model"]
            .as_str()
            .unwrap()
            .contains(model),
        "Desktop /api/status must report the upstream model name"
    );

    app_state.store.call(Request::Close).await.unwrap();
}

#[test]
fn test_regression_sanitize_extraction_resilience() {
    // 1. Missing summary: synthesized from facts
    let raw_no_summary = json!({
        "facts": [
            {
                "kind": "EVENT",
                "content": "Alex conversou com Maria sobre o mapa.",
                "importance": 0.8
            },
            {
                "kind": "LOCATION",
                "content": "Eles se encontraram na praça central.",
                "importance": 0.5
            }
        ]
    });
    let sanitized = modularchat_server::jobs::sanitize_extraction(&raw_no_summary)
        .expect("Must synthesize summary from facts when summary field is omitted");
    assert_eq!(
        sanitized["summary"],
        "Alex conversou com Maria sobre o mapa. Eles se encontraram na praça central."
    );
    assert_eq!(sanitized["facts"].as_array().unwrap().len(), 2);

    // 2. Portuguese aliases: "resumo", "fatos", "cena", "personagens"
    let raw_portuguese = json!({
        "resumo": "Encontro na biblioteca.",
        "personagens": [
            { "name": "Maria", "description": "Historiadora" }
        ],
        "cena": {
            "location": "Biblioteca Municipal",
            "presentes": ["Maria"]
        },
        "fatos": [
            {
                "fato": "Maria encontrou o tomo antigo na estante de história.",
                "sujeito": "Maria"
            }
        ],
        "relacionamentos": [
            { "a": "Alex", "b": "Maria", "description": "Colegas de pesquisa" }
        ],
        "pendencias": ["Examinar o tomo antigo"]
    });
    let sanitized_pt = modularchat_server::jobs::sanitize_extraction(&raw_portuguese)
        .expect("Must accept Portuguese alias keys");
    assert_eq!(sanitized_pt["summary"], "Encontro na biblioteca.");
    assert_eq!(sanitized_pt["cast"][0]["name"], "Maria");
    assert_eq!(sanitized_pt["scene"]["location"], "Biblioteca Municipal");
    assert_eq!(sanitized_pt["facts"][0]["content"], "Maria encontrou o tomo antigo na estante de história.");
    assert_eq!(sanitized_pt["relationships"].as_array().unwrap().len(), 1);
    assert_eq!(sanitized_pt["openThreads"][0], "Examinar o tomo antigo");

    // 3. Valid summary with empty/omitted facts does not 502
    let raw_empty_facts = json!({
        "summary": "Uma conversa amigável sobre o clima."
    });
    let sanitized_empty = modularchat_server::jobs::sanitize_extraction(&raw_empty_facts)
        .expect("Must not 502 when facts array is omitted but summary is valid");
    assert_eq!(sanitized_empty["summary"], "Uma conversa amigável sobre o clima.");
    assert_eq!(sanitized_empty["facts"].as_array().unwrap().len(), 0);
}

