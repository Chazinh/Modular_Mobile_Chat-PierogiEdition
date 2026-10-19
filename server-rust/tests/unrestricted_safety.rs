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
fn presets_request_applies_unrestricted_safety_globally_to_all_models() {
    // Test across various model families: generic, claude, deepseek, glm, and gemini
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

        // Verify it was applied to the request body in both Google REST (camelCase)
        // and OpenAI proxy/wrapper (snake_case) formats.
        assert!(
            plan.body.get("safetySettings").is_some(),
            "safetySettings must be present for preset {preset_id}"
        );
        assert!(
            plan.body.get("safety_settings").is_some(),
            "safety_settings must be present for preset {preset_id}"
        );

        let body_settings = &plan.body["safetySettings"];
        assert_eq!(body_settings, &default_safety_settings());
        assert_eq!(&plan.body["safety_settings"], &default_safety_settings());

        // Verify controls metadata declares unrestrictedSafety as true
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
fn presets_request_preserves_custom_safety_settings() {
    let custom = json!([
        {
            "category": "HARM_CATEGORY_HARASSMENT",
            "threshold": "BLOCK_ONLY_HIGH"
        }
    ]);

    let conv = json!({
        "presetId": "generic",
        "routeId": "openai/gpt-4o",
        "settings": {
            "safetySettings": custom
        }
    });

    let plan = request(&conv, vec![], &json!({}));
    assert_eq!(plan.body["safetySettings"], custom);
    assert_eq!(plan.body["safety_settings"], custom);
}

#[test]
fn provider_chat_request_injects_safety_settings_globally() {
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

    let original = json!({
        "messages": [{"role": "user", "content": "Hello world"}]
    });

    let request = provider
        .chat_request(
            &reqwest::Client::new(),
            "any-model",
            &original,
            Duration::from_secs(1),
        )
        .build()
        .unwrap();

    let sent: serde_json::Value =
        serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();

    assert_eq!(sent["safetySettings"], default_safety_settings());
    assert_eq!(sent["safety_settings"], default_safety_settings());
    assert!(sent.get("unrestrictedSafety").is_none(), "internal flag must be cleaned up");
}

#[test]
fn provider_chat_request_respects_explicit_disabled_safety() {
    let provider = Provider {
        id: "fixture-provider".into(),
        base_url: "https://openrouter.ai/api/v1".into(),
        api_key: None,
        models: vec![],
        request_log: None,
        reasoning_format: ReasoningFormat::Omit,
        context_window: 128000,
        max_output: 8000,
    };

    let disabled_body = json!({
        "messages": [{"role": "user", "content": "Hello"}],
        "unrestrictedSafety": false
    });

    let request = provider
        .chat_request(
            &reqwest::Client::new(),
            "any-model",
            &disabled_body,
            Duration::from_secs(1),
        )
        .build()
        .unwrap();

    let sent: serde_json::Value =
        serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();

    assert!(sent.get("safetySettings").is_none());
    assert!(sent.get("safety_settings").is_none());
    assert!(sent.get("unrestrictedSafety").is_none());
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
