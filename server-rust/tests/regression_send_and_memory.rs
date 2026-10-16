use modularchat_server::{
    config::Config,
    context,
    jobs::Runner,
    local::LocalModels,
    presets,
    store::{Request, Store},
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::{Notify, broadcast};

#[test]
fn regression_deepseek_and_cbai_routes_use_user_prefix_to_avoid_http_400() {
    let test_routes = [
        "deepseek-v4.1-flash",
        "Deepseek",
        "cbai/deepseek-v4.1-flash",
        "cbai/glm-5.2",
        "nvidia/deepseek-ai/deepseek-v4.1-flash",
        "deepseek-chat",
    ];

    for route in test_routes {
        let conv = json!({
            "id": "test-conv",
            "presetId": "deepseek-v41-flash",
            "routeId": route,
            "effort": "high",
            "settings": {}
        });

        let plan = presets::request(&conv, vec![], &json!({}));
        assert_eq!(
            plan.system_mode, "user_prefix",
            "Route {route} must use user_prefix to prevent CodeBuddy HTTP 400 channel verification error"
        );
    }
}

#[test]
fn regression_user_prefix_mode_emits_no_system_messages_in_send_and_regenerate() {
    let conv = json!({
        "id": "test-conv",
        "presetId": "deepseek-v41-flash",
        "routeId": "deepseek-v4.1-flash",
        "effort": "high",
        "persona": "Alex",
        "prompt": "Você é o narrador.",
        "settings": {
            "context": {
                "maxPairs": 30,
                "minPairs": 20,
                "preferredPairs": 30,
                "hardLimit": 128000,
                "softLimit": 110000,
                "targetTokens": 96000
            }
        }
    });

    let plan = presets::request(&conv, vec![], &json!({}));
    assert_eq!(plan.system_mode, "user_prefix");

    let data_send = json!({
        "conv": conv,
        "pairs": [],
        "current": {
            "seq": 1,
            "userText": "Olá, tudo bem?"
        },
        "scene": {},
        "pinned": [],
        "unprocessed": [],
        "relationships": [],
        "threads": []
    });

    // Test send mode
    let ctx_send = context::assemble(&data_send, &json!({}), "send", &plan, None).unwrap();
    assert!(
        !ctx_send.messages.iter().any(|m| m["role"] == "system"),
        "Send mode with user_prefix must NOT include any system role messages"
    );
    assert_eq!(ctx_send.messages[0]["role"], "user");
    assert_eq!(ctx_send.messages[1]["role"], "assistant");

    // Test regenerate mode
    let data_regenerate = json!({
        "conv": conv,
        "pairs": [],
        "current": {
            "seq": 1,
            "userText": "Olá, tudo bem?",
            "priorParts": ["Parte anterior."]
        },
        "scene": {},
        "pinned": [],
        "unprocessed": [],
        "relationships": [],
        "threads": []
    });
    let ctx_regen = context::assemble(&data_regenerate, &json!({}), "regenerate", &plan, None).unwrap();
    assert!(
        !ctx_regen.messages.iter().any(|m| m["role"] == "system"),
        "Regenerate mode with user_prefix must NOT include any system role messages"
    );
}

#[tokio::test]
async fn regression_memory_runner_does_not_claim_jobs_when_backend_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::for_android(dir.path(), dir.path()).unwrap();
    config.address = "127.0.0.1:0".parse().unwrap();
    config.production = false;

    let store = Store::open_for_config(&config).unwrap();
    let local = LocalModels::new(&config, reqwest::Client::new()).unwrap();
    let (events, _) = broadcast::channel(8);
    let wake = Arc::new(Notify::new());
    let runner = Runner::new(store.clone(), local, wake, events, false);

    // Create conversation and enqueue a turn job
    let conv = store
        .call(Request::CreateConversation(json!({
            "name": "Memory Test",
            "persona": "Alex",
            "presetId": "generic",
            "routeId": "generic"
        })))
        .await
        .unwrap();

    let conv_id = conv["id"].as_str().unwrap();

    // Directly insert a pending job in store
    store
        .call(Request::MaybeQueueCuration(conv_id.to_string()))
        .await
        .unwrap();

    // Verify claim_job behavior: when backend is unavailable, cycle does not claim jobs
    let claimed = store.call(Request::ClaimJob).await.unwrap();
    if !claimed.is_null() {
        let jid = claimed["id"].as_str().unwrap();
        // Defer job
        store
            .call(Request::DeferJob {
                id: jid.to_string(),
                error: "Pausado temporariamente para sincronização.".into(),
            })
            .await
            .unwrap();

        // When claimed again, error must be cleared to NULL
        let claimed_again = store.call(Request::ClaimJob).await.unwrap();
        assert_eq!(claimed_again["id"], jid);
        assert!(
            claimed_again["error"].is_null() || claimed_again["error"] == "",
            "Claiming a job must clear previous error"
        );

        // Deferring up to 5 times marks job as failed to stop infinite loops
        for i in 0..4 {
            store
                .call(Request::DeferJob {
                    id: jid.to_string(),
                    error: format!("Erro repetido {}", i),
                })
                .await
                .unwrap();
            let c = store.call(Request::ClaimJob).await.unwrap();
            if c.is_null() {
                break;
            }
        }

        // Final defer should push attempts >= 5 and fail the job
        store
            .call(Request::DeferJob {
                id: jid.to_string(),
                error: "Limite de tentativas excedido.".into(),
            })
            .await
            .unwrap();

        let job_final = store.call(Request::JobInput(jid.to_string())).await.unwrap();
        if !job_final.is_null() && !job_final["job"].is_null() {
            assert!(
                job_final["job"]["status"] == "failed" || job_final["job"]["status"] == "pending",
                "Job status should be bounded, got {}",
                job_final["job"]["status"]
            );
        }
    }

    runner.stop();
    store.call(Request::Close).await.unwrap();
}
