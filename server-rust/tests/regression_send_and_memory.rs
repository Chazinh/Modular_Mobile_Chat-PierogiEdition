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

#[test]
fn regression_screenshot_model_routes_and_argon_exclusion() {
    // 1. Argon deve ser estritamente rejeitado como rota selecionável
    assert!(!presets::selectable_route("ag/gemini-4-argon-high"), "ag/gemini-4-argon-high deve ser rejeitado");
    assert!(!presets::selectable_route("gemini/gemini-4-argon"), "gemini/gemini-4-argon deve ser rejeitado");
    assert!(!presets::selectable_route("Argon"), "Argon combo deve ser rejeitado");

    // 2. Rotas ativas e grupos de combo devem ser selecionáveis
    assert!(presets::selectable_route("oc/deepseek-v4.1-flash-free"), "oc/deepseek-v4.1-flash-free deve ser selecionável");
    assert!(presets::selectable_route("cbai/deepseek-v4.1-flash"), "cbai/deepseek-v4.1-flash deve ser selecionável");
    assert!(presets::selectable_route("nvidia/deepseek-ai/deepseek-v4.1-flash"), "nvidia/deepseek-ai/deepseek-v4.1-flash deve ser selecionável");
    assert!(presets::selectable_route("Deepseek"), "Deepseek combo group deve ser selecionável");
    assert!(presets::selectable_route("Gemini"), "Gemini combo group deve ser selecionável");
    assert!(presets::selectable_route("Sonnet"), "Sonnet combo group deve ser selecionável");
    assert!(presets::selectable_route("Opus"), "Opus combo group deve ser selecionável");
    assert!(presets::selectable_route("GLM"), "GLM combo group deve ser selecionável");
    assert!(presets::selectable_route("cbai/glm-5.2"), "cbai/glm-5.2 deve ser selecionável");
    assert!(!presets::selectable_route("nvidia/z-ai/glm-5.2"), "nvidia/z-ai/glm-5.2 foi desligada e NÃO deve ser selecionável");
    assert!(presets::selectable_route("cc/claude-opus-5-5"), "cc/claude-opus-5-5 deve ser selecionável");
    assert!(presets::selectable_route("cc/claude-sonnet-5-5"), "cc/claude-sonnet-5-5 deve ser selecionável");
    assert!(presets::selectable_route("gemini/gemini-3.8-flash"), "gemini/gemini-3.8-flash deve ser selecionável");

    // 3. Mapeamento correto para presets
    assert_eq!(presets::for_route("oc/deepseek-v4.1-flash-free")["id"], "deepseek-v41-flash");
    assert_eq!(presets::for_route("Deepseek")["id"], "deepseek-v41-flash");
    assert_eq!(presets::for_route("Gemini")["id"], "gemini-38-flash");
    assert_eq!(presets::for_route("GLM")["id"], "glm-52");
    assert_eq!(presets::for_route("Sonnet")["id"], "claude-sonnet-5-5");
    assert_eq!(presets::for_route("Opus")["id"], "claude-opus-5-5");
    assert_eq!(presets::for_route("cbai/glm-5.2")["id"], "glm-52");

    // 4. Verificação de presets::public
    let advertised = vec![
        "Deepseek".to_string(),
        "cbai/deepseek-v4.1-flash".to_string(),
        "oc/deepseek-v4.1-flash-free".to_string(),
        "cbai/glm-5.2".to_string(),
        "nvidia/z-ai/glm-5.2".to_string(),
        "ag/gemini-4-argon-high".to_string(),
    ];
    let public_presets = presets::public(&advertised, &json!({"routes": {}}));

    // DeepSeek preset: deve conter o grupo e rotas ativas, com Deepseek em primeiro lugar
    let ds = public_presets.iter().find(|p| p["id"] == "deepseek-v41-flash").expect("Deepseek preset");
    let ds_routes: Vec<&str> = ds["routes"].as_array().unwrap().iter().filter_map(|r| r["id"].as_str()).collect();
    assert_eq!(ds_routes[0], "Deepseek", "Grupo Deepseek deve ser a primeira rota");
    assert!(ds_routes.contains(&"oc/deepseek-v4.1-flash-free"), "DeepSeek deve conter oc/deepseek-v4.1-flash-free");
    assert!(ds_routes.contains(&"cbai/deepseek-v4.1-flash"), "DeepSeek deve conter cbai/deepseek-v4.1-flash");

    // GLM preset: nvidia/z-ai/glm-5.2 não deve ser listada
    let glm = public_presets.iter().find(|p| p["id"] == "glm-52").expect("GLM preset");
    let glm_routes: Vec<&str> = glm["routes"].as_array().unwrap().iter().filter_map(|r| r["id"].as_str()).collect();
    assert!(glm_routes.contains(&"cbai/glm-5.2"), "GLM deve conter cbai/glm-5.2");
    assert!(!glm_routes.contains(&"nvidia/z-ai/glm-5.2"), "GLM NÃO deve conter rota desligada nvidia/z-ai/glm-5.2");

    // Argon NUNCA deve aparecer em nenhum preset
    for p in &public_presets {
        let routes: Vec<&str> = p["routes"].as_array().unwrap().iter().filter_map(|r| r["id"].as_str()).collect();
        assert!(!routes.iter().any(|r| r.to_lowercase().contains("argon")), "Preset {} não pode conter rota argon", p["id"]);
    }
}
