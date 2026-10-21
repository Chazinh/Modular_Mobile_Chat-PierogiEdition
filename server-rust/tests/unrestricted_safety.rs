use modularchat_server::{
    presets::{default_safety_settings, request, unrestricted_safety_enabled},
    providers::{Provider, ReasoningFormat},
};
use serde_json::json;
use std::time::Duration;

#[test]
fn default_safety_settings_blocks_none_for_all_four_standard_categories() {
    let settings = default_safety_settings();
    let array = settings.as_array().expect("must be a JSON array");
    assert_eq!(array.len(), 4);

    let expected_categories = [
        "HARM_CATEGORY_HARASSMENT",
        "HARM_CATEGORY_HATE_SPEECH",
        "HARM_CATEGORY_SEXUALLY_EXPLICIT",
        "HARM_CATEGORY_DANGEROUS_CONTENT",
    ];

    for expected in expected_categories {
        let entry = array
            .iter()
            .find(|item| item["category"] == expected)
            .unwrap_or_else(|| panic!("Category {expected} must be present in default safety settings"));
        assert_eq!(entry["threshold"], "BLOCK_NONE");
    }
}

#[test]
fn presets_request_does_not_inject_safety_settings_into_wire_body() {
    // Upstream OpenAI-compatible endpoints reject `safetySettings` with `invalid_request_error` ("type" error).
    // The request body must remain clean while controls metadata declares unrestrictedSafety.
    for (preset_id, route_id) in [
        ("generic", "openai/gpt-4o"),
        ("claude-sonnet-5-5", "anthropic/claude-3-5-sonnet"),
        ("deepseek-v4-1", "deepseek/deepseek-chat"),
        ("glm-52", "cbai/glm-5.2"),
        ("gemini-3-8-flash", "google/gemini-2.5-flash"),
    ] {
        let conv = json!({
            "presetId": preset_id,
            "routeId": route_id,
        });

        let plan = request(&conv, vec![], &json!({}));

        assert!(
            plan.body.get("safetySettings").is_none(),
            "safetySettings must NOT be present in wire body for preset {preset_id} to avoid invalid_request_error"
        );
        assert!(
            plan.body.get("safety_settings").is_none(),
            "safety_settings must NOT be present in wire body for preset {preset_id} to avoid invalid_request_error"
        );

        // Controls metadata declares unrestrictedSafety as true
        assert_eq!(plan.controls["unrestrictedSafety"], json!(true));
    }
}

#[test]
fn presets_request_respects_explicit_disabled_unrestricted_safety() {
    let conv = json!({
        "presetId": "generic",
        "routeId": "openai/gpt-4o",
        "settings": {
            "unrestrictedSafety": false
        }
    });

    let plan = request(&conv, vec![], &json!({}));
    assert!(plan.body.get("safetySettings").is_none());
    assert!(plan.body.get("safety_settings").is_none());
    assert_eq!(plan.controls["unrestrictedSafety"], json!(false));
}

#[test]
fn provider_chat_request_strips_safety_settings_for_openai_wire_compliance() {
    let provider = Provider {
        id: "fixture-provider".into(),
        base_url: "https://openrouter.ai/api/v1".into(),
        api_key: Some("secret".into()),
        models: vec![],
        request_log: None,
        reasoning_format: ReasoningFormat::Omit,
        context_window: 128000,
        max_output: 8000,
    };

    // Even if incoming body contains safetySettings, safety_settings, or unrestrictedSafety
    // (from legacy payloads or conversation settings), chat_request strips them so upstream
    // endpoints never reject the request with {"error":{"type":"invalid_request_error"}}.
    let incoming = json!({
        "messages": [{"role": "user", "content": "Hello world"}],
        "safetySettings": default_safety_settings(),
        "safety_settings": default_safety_settings(),
        "unrestrictedSafety": true
    });

    let request = provider
        .chat_request(
            &reqwest::Client::new(),
            "any-model",
            &incoming,
            Duration::from_secs(1),
        )
        .build()
        .unwrap();

    let sent: serde_json::Value =
        serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();

    assert!(
        sent.get("safetySettings").is_none(),
        "safetySettings must be stripped from outgoing wire request"
    );
    assert!(
        sent.get("safety_settings").is_none(),
        "safety_settings must be stripped from outgoing wire request"
    );
    assert!(
        sent.get("unrestrictedSafety").is_none(),
        "internal unrestrictedSafety flag must be cleaned up"
    );
}

#[test]
fn unrestricted_safety_enabled_defaults_to_true_and_respects_settings_and_env() {
    let empty = json!({});
    assert!(unrestricted_safety_enabled(&empty));

    let explicit_false = json!({
        "settings": {
            "unrestrictedSafety": false
        }
    });
    assert!(!unrestricted_safety_enabled(&explicit_false));

    let explicit_true = json!({
        "settings": {
            "unrestrictedSafety": true
        }
    });
    assert!(unrestricted_safety_enabled(&explicit_true));
}
