use modularchat_server::presets;
use serde_json::json;

#[test]
fn supported_models_toggle_without_losing_the_selected_effort() {
    for (id, route, effort) in [
        ("deepseek-v41-flash", "cbai/deepseek-v4.1-flash", "high"),
        ("claude-sonnet-5-5", "cc/claude-sonnet-5-5", "medium"),
    ] {
        let mut conv = json!({"presetId":id,"routeId":route,"effort":effort,"settings":{"thinkingEnabled":false}});
        let mut off = presets::request(&conv, vec![], &json!({}));
        assert_eq!(off.body["reasoning_effort"], "none");
        assert!(off.body["thinking"].is_null());
        assert!(off.body["output_config"].is_null());
        assert_eq!(off.controls["requestedEffort"], effort);
        assert_eq!(off.controls["thinkingEnabled"], false);
        assert_eq!(
            presets::apply_length(&mut off, &json!({"botChars":600}))["reasoningReserve"],
            0
        );
        assert_eq!(off.body["max_tokens"], 240);
        conv["settings"]["thinkingEnabled"] = json!(true);
        let mut on = presets::request(&conv, vec![], &json!({}));
        assert_eq!(on.body["reasoning_effort"], effort);
        assert!(
            presets::apply_length(&mut on, &json!({"botChars":600}))["reasoningReserve"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(conv["effort"], effort);
    }
}



#[test]
fn mobile_local_does_not_offer_thinking_controls_or_thinking_can_disable_in_catalog() {
    let public = presets::public(&["mobile-local".into()], &json!({}));
    let local = public.iter().find(|p| p["id"] == "mobile-local").unwrap();
    assert_ne!(local["thinkingCanDisable"], true);
    assert!(local["efforts"].as_array().unwrap().is_empty());
    assert!(local.get("thinking").is_none() || local["thinking"].is_null());

    let raw = presets::for_route("mobile-local");
    assert_eq!(raw["id"], "mobile-local");
    assert_ne!(raw["thinkingCanDisable"], true);

    let conv = json!({
        "presetId": "mobile-local",
        "routeId": "mobile-local",
        "settings": { "thinkingEnabled": false }
    });
    let plan = presets::request(&conv, vec![], &json!({}));
    assert_eq!(plan.controls["thinkingCanDisable"], false);
    assert!(plan.body.get("reasoning_effort").is_none());
    assert!(plan.body.get("thinking").is_none());
    assert!(plan.controls["requestedEffort"].is_null());

    let conv_on = json!({
        "presetId": "mobile-local",
        "routeId": "mobile-local",
        "settings": { "thinkingEnabled": true }
    });
    let plan_on = presets::request(&conv_on, vec![], &json!({}));
    assert_eq!(plan_on.controls["thinkingCanDisable"], false);
    assert!(plan_on.body.get("reasoning_effort").is_none());
    assert!(plan_on.body.get("thinking").is_none());
}

