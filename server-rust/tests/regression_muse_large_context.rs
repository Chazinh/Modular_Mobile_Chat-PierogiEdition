//! Automated regression test suite for Milestone M2: Muse Spark Large Text Support (R2).
//! Tests context assembly preservation under 1M context, ladder compaction, upstream
//! token overflow retry/fallback, and voluminous payload acceptance.

use axum::{
    Json, Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request as HttpRequest, StatusCode},
    response::IntoResponse,
    routing::post,
};
use http_body_util::BodyExt;
use modularchat_server::{
    api::{self, AppState},
    config::Config,
    context, presets,
    providers::Providers,
    store::{FinishRequest, Request, TurnRequest},
};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tower::ServiceExt;

/// Base conversation state fixture for context assembly tests.
fn base_context_data() -> Value {
    json!({
        "conv": {
            "persona": "Alex",
            "prompt": "Lia estuda magia em uma biblioteca antiga.",
            "settings": {
                "context": {
                    "preferredPairs": 30,
                    "minPairs": 20,
                    "maxPairs": 50
                }
            }
        },
        "pairs": [],
        "totalPairs": 0,
        "beforeSeq": 1,
        "pathLength": 0,
        "cast": [],
        "scene": null,
        "pinned": [],
        "relationships": [],
        "threads": [],
        "notes": [],
        "episodes": [],
        "timeline": { "current": null },
        "sync": { "lag": 0, "synced": true },
        "unprocessed": [],
        "current": {
            "interactionId": null,
            "seq": 1,
            "userText": "O que você descobriu na biblioteca?",
            "priorParts": []
        },
        "globalSettings": {
            "dialoguePct": 65,
            "botChars": 2000
        }
    })
}

// ============================================================================
// TEST A: Context assembly with voluminous turns (>130k tokens) under muse-spark-13
// preserves full capacity and does NOT falsely flag overHardLimit or exceededEstimatedBudget.
// ============================================================================
#[test]
fn test_a_muse_large_context_assembly_preserves_capacity() {
    let mut d = base_context_data();
    // Build voluminous pairs totaling > 130,000 tokens
    let mut pairs = Vec::new();
    for seq in 1..=35 {
        // ~15,000 chars per pair -> ~4,500 tokens * 35 = ~157,500 tokens
        let text = format!(
            "Capítulo histórico {seq}: Descrição detalhada dos eventos passados na grande biblioteca antiga, "
        )
        .repeat(160);
        pairs.push(json!({
            "id": format!("pair_{seq}"),
            "seq": seq,
            "user": format!("Pergunta {seq} sobre o capítulo de pesquisa antiga."),
            "assistant": format!("Lia: Análise aprofundada {seq}: {text}"),
            "complete": true
        }));
    }
    d["pairs"] = json!(pairs);
    d["totalPairs"] = json!(35);
    d["beforeSeq"] = json!(36);
    d["pathLength"] = json!(35);
    d["conv"]["settings"]["context"]["preferredPairs"] = json!(35);

    let plan = presets::request(
        &json!({
            "presetId": "muse-spark-13",
            "routeId": "Muse",
            "settings": {
                "context": {
                    "preferredPairs": 35,
                    "minPairs": 20,
                    "maxPairs": 50
                }
            }
        }),
        vec![],
        &json!({}),
    );

    let ctx = context::assemble(&d, &json!({}), "send", &plan, None)
        .expect("Context assembly for muse-spark-13 must succeed with voluminous turns");

    let total_tokens = ctx.report["tokens"]["total"]
        .as_u64()
        .expect("total tokens must be present");
    assert!(
        total_tokens > 130_000,
        "Total tokens must exceed 130k for this test, got: {total_tokens}"
    );

    // Assert that under muse-spark-13 (1M context), full capacity is preserved:
    assert_eq!(
        ctx.report["limits"]["capacity"], 1048576,
        "Capacity must reflect Muse Spark's 1M window"
    );
    assert_eq!(
        ctx.report["overHardLimit"], false,
        "Context exceeding 130k tokens must NOT falsely flag overHardLimit under 1M context model"
    );
    assert_eq!(
        ctx.report["overCapacity"], false,
        "Context within 1M window must NOT flag overCapacity"
    );
    assert_eq!(
        ctx.report["exceededEstimatedBudget"], false,
        "Context must NOT flag exceededEstimatedBudget when within model capacity"
    );
    assert_eq!(
        ctx.report["pairsIncluded"].as_u64().unwrap(),
        35,
        "All 35 turns must be preserved without premature truncation"
    );
}

// ============================================================================
// TEST B: Context assembly ladder correctly compacts and allocates token budgets
// when context approaches actual model capacity.
// ============================================================================
#[test]
fn test_b_muse_context_ladder_compacts_approaching_capacity() {
    let mut d = base_context_data();
    // Configure conversation settings with explicit soft/target boundaries simulating approaching capacity
    d["conv"]["settings"]["context"] = json!({
        "preferredPairs": 30,
        "minPairs": 15,
        "maxPairs": 50,
        "targetTokens": 40000,
        "softLimit": 50000,
        "hardLimit": 70000
    });

    // Provide turns that exceed target and soft limits (e.g. ~60,000 tokens)
    let mut pairs = Vec::new();
    for seq in 1..=30 {
        let text = format!("Turno {seq}: Análise longa e minuciosa dos manuscritos e artefatos. ")
            .repeat(80);
        pairs.push(json!({
            "id": format!("pair_{seq}"),
            "seq": seq,
            "user": format!("Pergunta {seq}"),
            "assistant": format!("Lia: Resposta {seq}: {text}"),
            "complete": true
        }));
    }
    d["pairs"] = json!(pairs);
    d["totalPairs"] = json!(30);
    d["beforeSeq"] = json!(31);

    let plan = presets::request(
        &json!({
            "presetId": "muse-spark-13",
            "routeId": "Muse"
        }),
        vec![],
        &json!({}),
    );

    let ctx = context::assemble(&d, &json!({}), "send", &plan, None)
        .expect("Context assembly must succeed");

    // The ladder must compact the included pairs to fit within soft/hard limits
    let pairs_included = ctx.report["pairsIncluded"]
        .as_u64()
        .expect("pairsIncluded must exist");
    assert!(
        pairs_included < 30,
        "Ladder must reduce pairs when total exceeds soft limit, included: {pairs_included}"
    );

    let compression = ctx.report["compression"]
        .as_array()
        .expect("compression array must exist");
    assert!(
        !compression.is_empty(),
        "Compression log must document the ladder reduction steps"
    );

    // Verify limit allocations
    let limits = &ctx.report["limits"];
    assert_eq!(limits["target"], 40000);
    assert_eq!(limits["soft"], 50000);
    assert_eq!(limits["hard"], 70000);

    let total = ctx.report["tokens"]["total"].as_u64().unwrap();
    assert!(
        total <= limits["hard"].as_u64().unwrap(),
        "Total tokens after ladder compaction ({total}) must be within hard limit ({})",
        limits["hard"]
    );
}

// ============================================================================
// TEST C: Generation simulation with large payload against a mock Axum upstream
// that returns 402/503 "Prompt tokens limit exceeded" on initial attempt:
// verifies that retry with compacted prompt or route fallback succeeds and returns
// complete stream without fatal error.
// ============================================================================
#[tokio::test]
async fn test_c_muse_generation_simulation_retry_on_upstream_token_overflow() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_filter = attempts.clone();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let requests_capture = requests.clone();

    // Start mock Axum server that returns 402 "Prompt tokens limit exceeded" on initial attempt,
    // and returns a valid SSE completion stream on retry / route fallback.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = listener.local_addr().unwrap();
    let mock_url = format!("http://{}", mock_addr);

    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(body): Json<Value>| {
            let attempts = attempts_filter.clone();
            let requests = requests_capture.clone();
            async move {
                requests.lock().unwrap().push(body.clone());
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    // Attempt 0: return HTTP 402 with OpenRouter / 9Router overflow message
                    (
                        StatusCode::PAYMENT_REQUIRED,
                        Json(json!({
                            "error": {
                                "message": "Prompt tokens limit exceeded: 21875 > 8739. To increase, visit https://openrouter.ai/settings/credits, or lower max_tokens / prompt size to fit your remaining balance.",
                                "code": 402
                            }
                        })),
                    )
                        .into_response()
                } else {
                    // Attempt >= 1: retry succeeds with stream
                    let chunk = json!({
                        "choices": [{
                            "delta": {
                                "content": "Lia: Análise processada com sucesso após redução de contexto."
                            }
                        }]
                    });
                    let sse = format!(
                        "data: {chunk}\n\ndata: {}\n\ndata: [DONE]\n\n",
                        json!({"choices": [{"delta": {}, "finish_reason": "stop"}]})
                    );
                    ([("content-type", "text/event-stream")], Body::from(sse)).into_response()
                }
            }
        }),
    );

    let server_task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let dir = tempfile::tempdir().unwrap();
    let providers = Providers::from_json(&json!({
        "providers": [
            {
                "id": "mock-muse-provider",
                "baseUrl": format!("{mock_url}/v1"),
                "apiKey": "mock-key",
                "models": [
                    "Muse",
                    "oc-router/meta/muse-spark-open",
                    "oc/muse-spark-1.3-contributor-free"
                ]
            }
        ]
    }))
    .unwrap();

    let state = AppState::new(Config {
        data_dir: dir.path().into(),
        dist: dir.path().join("absent"),
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

    let chat = state
        .store
        .call(Request::CreateConversation(json!({
            "name": "Muse Retry Conv",
            "persona": "Alex",
            "presetId": "muse-spark-13",
            "routeId": "Muse",
            "prompt": "Lia pesquisa livros proibidos na torre oeste."
        })))
        .await
        .unwrap();
    let conv_id = chat["id"].as_str().unwrap();

    // Populate conversation with history turns so that compaction can take effect on retry
    for i in 1..=5 {
        let req: TurnRequest = serde_json::from_value(json!({
            "convId": conv_id,
            "mode": "send",
            "content": format!("Pergunta histórica {i} com texto detalhado sobre as runas antigas."),
            "opId": modularchat_server::id()
        }))
        .unwrap();
        let b = state.store.call(Request::BeginTurn(req)).await.unwrap();
        state
            .store
            .call(Request::FinishGeneration(
                serde_json::from_value::<FinishRequest>(json!({
                    "generationId": b["generationId"],
                    "status": "complete",
                    "text": format!("Resposta histórica {i} sobre as runas antigas."),
                    "usage": {}
                }))
                .unwrap(),
            ))
            .await
            .unwrap();
    }

    let turn_req: TurnRequest = serde_json::from_value(json!({
        "convId": conv_id,
        "mode": "send",
        "content": "Analise a totalidade dos dados históricos coletados até o momento.",
        "opId": modularchat_server::id()
    }))
    .unwrap();

    let (g, replay) = state
        .generation
        .start(turn_req)
        .await
        .expect("Generation start must succeed");

    let consumer = state
        .generation
        .stream(g.clone(), 0, replay)
        .await
        .expect("Stream consumer must be established");

    let bytes = consumer
        .into_response()
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let stream_text = String::from_utf8_lossy(&bytes).to_string();

    let events: Vec<Value> = stream_text
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .filter_map(|s| serde_json::from_str(s.trim()).ok())
        .collect();

    // Verify retry occurred (at least 2 attempts recorded)
    let total_attempts = attempts.load(Ordering::SeqCst);
    assert!(
        total_attempts >= 2,
        "Upstream must receive retry attempt after initial 402 failure, recorded: {total_attempts}"
    );

    // Verify stream completed successfully rather than terminating with fatal error
    assert!(
        events.iter().any(|e| e["done"] == true && e["status"] == "complete"),
        "Generation must finish with status complete after retry. Stream text: {stream_text}"
    );
    assert!(
        events.iter().any(|e| e["delta"]
            .as_str()
            .unwrap_or_default()
            .contains("Análise processada com sucesso")),
        "Consumer must receive expected assistant content"
    );

    server_task.abort();
    state.shutdown().await;
}

// ============================================================================
// TEST D: Sending voluminous message payload with muse-spark-13 completes
// without fatal request failure (and anti-regression check for standard models).
// ============================================================================
#[tokio::test]
async fn test_d_muse_voluminous_message_payload_accepted_without_refusal() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let requests_capture = requests.clone();

    // Mock upstream server accepting large messages and streaming back reply
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = listener.local_addr().unwrap();
    let mock_url = format!("http://{}", mock_addr);

    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(body): Json<Value>| {
            let requests = requests_capture.clone();
            async move {
                requests.lock().unwrap().push(body);
                let chunk = json!({
                    "choices": [{
                        "delta": {
                            "content": "Lia: Documento volumoso recebido e processado integralmente."
                        }
                    }]
                });
                let sse = format!(
                    "data: {chunk}\n\ndata: {}\n\ndata: [DONE]\n\n",
                    json!({"choices": [{"delta": {}, "finish_reason": "stop"}]})
                );
                ([("content-type", "text/event-stream")], Body::from(sse)).into_response()
            }
        }),
    );

    let server_task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let dir = tempfile::tempdir().unwrap();
    let providers = Providers::from_json(&json!({
        "providers": [
            {
                "id": "mock-muse-provider",
                "baseUrl": format!("{mock_url}/v1"),
                "apiKey": "mock-key",
                "models": ["Muse", "oc/muse-spark-1.3-contributor-free"]
            }
        ]
    }))
    .unwrap();

    let state = AppState::new(Config {
        data_dir: dir.path().into(),
        dist: dir.path().join("absent"),
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

    // Create Muse Spark conversation
    let chat = state
        .store
        .call(Request::CreateConversation(json!({
            "name": "Muse Large Payload Chat",
            "persona": "Alex",
            "presetId": "muse-spark-13",
            "routeId": "Muse",
            "prompt": "Lia analisa extensos relatórios mágicos."
        })))
        .await
        .unwrap();
    let conv_id = chat["id"].as_str().unwrap();

    // Voluminous message payload > 6,000 characters (~10,080 characters)
    let paragraph = "Este é um manuscrito extenso com transcrições de relatórios detalhados. ";
    let voluminous_content = paragraph.repeat(140);
    assert!(
        voluminous_content.len() > 6000,
        "Payload must exceed legacy 6000 character limit"
    );

    // Send turn via HTTP endpoint POST /api/chats/{id}/message
    let response = api::router(state.clone())
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri(format!("/api/chats/{conv_id}/message"))
                .header("content-type", "application/json")
                .extension(ConnectInfo("127.0.0.1:1".parse::<SocketAddr>().unwrap()))
                .body(Body::from(
                    json!({
                        "mode": "send",
                        "content": voluminous_content,
                        "opId": modularchat_server::id()
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&bytes).to_string();

    // Verify that the request is NOT rejected with 422 "Sua mensagem pode ter no máximo 6.000 caracteres"
    assert_ne!(
        status, 422,
        "Voluminous message under Muse Spark must not be rejected with 422. Response: {body_str}"
    );
    assert_eq!(
        status, 200,
        "Voluminous message under Muse Spark must return 200 SSE stream. Response: {body_str}"
    );

    // Verify SSE stream delivers expected reply
    assert!(
        body_str.contains("Documento volumoso recebido e processado integralmente"),
        "Stream must deliver expected content"
    );
    assert!(
        body_str.contains("\"status\":\"complete\""),
        "Generation must finish with status complete"
    );

    // ------------------------------------------------------------------------
    // Anti-regression assertion: non-Muse models (e.g. mobile-local) must still
    // enforce standard limit and reject payloads > 6000 chars with 422.
    // ------------------------------------------------------------------------
    let local_chat = state
        .store
        .call(Request::CreateConversation(json!({
            "name": "Standard Local Chat",
            "persona": "Alex",
            "presetId": "mobile-local",
            "routeId": "mobile-local",
            "prompt": "Prompt padrão."
        })))
        .await
        .unwrap();
    let local_conv_id = local_chat["id"].as_str().unwrap();

    let local_response = api::router(state.clone())
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri(format!("/api/chats/{local_conv_id}/message"))
                .header("content-type", "application/json")
                .extension(ConnectInfo("127.0.0.1:1".parse::<SocketAddr>().unwrap()))
                .body(Body::from(
                    json!({
                        "mode": "send",
                        "content": paragraph.repeat(100), // ~7,200 chars (> 6000)
                        "opId": modularchat_server::id()
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        local_response.status().as_u16(),
        422,
        "Standard presets must continue to enforce USER_LIMIT = 6000 and reject with 422"
    );

    server_task.abort();
    state.shutdown().await;
}
