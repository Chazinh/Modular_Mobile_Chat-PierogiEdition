use modularchat_server::providers::{Provider, ReasoningFormat};
use serde_json::json;
use std::time::Duration;

#[test]
fn explicit_cache_marker_is_scoped_to_openrouter_and_the_stable_system_prefix() {
    for base in [
        "https://openrouter.ai/api/v1",
        "http://127.0.0.1:20128/v1",
        "https://api.deepseek.com/v1",
        "https://openrouter.ai.example.com/v1",
    ] {
        let provider = Provider {
            id: "fixture".into(),
            base_url: base.into(),
            api_key: None,
            models: vec![],
            request_log: None,
            reasoning_format: ReasoningFormat::Omit,
            context_window: 128000,
            max_output: 8000,
        };
        let original = json!({"messages":[{"role":"system","content":"STABLE_RULES_AND_CARD"},{"role":"user","content":"DYNAMIC_SCENE"}]});
        let request = provider
            .chat_request(
                &reqwest::Client::new(),
                "fixture-model",
                &original,
                Duration::from_secs(1),
            )
            .build()
            .unwrap();
        let sent: serde_json::Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        if base == "https://openrouter.ai/api/v1" {
            assert_eq!(
                sent["messages"][0]["content"][0]["cache_control"]["type"],
                "ephemeral"
            );
            assert_eq!(
                sent["messages"][0]["content"][0]["text"],
                "STABLE_RULES_AND_CARD"
            );
        } else {
            assert_eq!(sent["messages"][0], original["messages"][0]);
        }
        assert_eq!(sent["messages"][1], original["messages"][1]);
        assert_eq!(original["messages"][0]["content"], "STABLE_RULES_AND_CARD");
    }
}
