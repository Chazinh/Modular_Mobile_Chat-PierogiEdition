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
fn disabled_is_only_accepted_for_supported_routes_and_defaults_stay_on() {
    for (id, route) in [
        ("claude-opus-5-5", "cc/claude-opus-5-5"),
        ("kimi-k3", "nvidia/moonshotai/kimi-k3"),
        ("gemini-38-flash", "ag/gemini-3.8-flash-low"),
    ] {
        let mut plan = presets::request(
            &json!({"presetId":id,"routeId":route,"settings":{"thinkingEnabled":false}}),
            vec![],
            &json!({}),
        );
        assert_eq!(plan.controls["thinkingEnabled"], true);
        assert_ne!(plan.body["reasoning_effort"], "none");
        assert!(
            presets::apply_length(&mut plan, &json!({"botChars":600}))["reasoningReserve"]
                .as_u64()
                .unwrap()
                > 0
        );
    }
    let p = presets::by_id("claude-sonnet-5-5");
    assert!(presets::thinking_enabled(&p, &json!({})));
    assert!(presets::thinking_enabled(
        &p,
        &json!({"thinkingEnabled":"false"})
    ));
    assert_eq!(p["thinkingOffType"], "between_tools");
}

#[test]
fn glm_52_starts_without_thinking_by_default() {
    for route in ["cbai/glm-5.2"] {
        assert_eq!(presets::for_route(route)["id"], "glm-52");
        let conv = json!({"presetId":"glm-52","routeId":route,"effort":"high","settings":{}});
        let mut plan = presets::request(&conv, vec![], &json!({}));
        // on GLM any reasoning_effort (even "none") turns thinking on: off = no control at all
        assert!(plan.body.get("reasoning_effort").is_none());
        assert!(plan.body.get("thinking").is_none());
        assert_eq!(plan.controls["thinkingEnabled"], false);
        assert!(plan.body.get("temperature").is_none());
        assert_eq!(
            presets::apply_length(&mut plan, &json!({"botChars":600}))["reasoningReserve"],
            0
        );
        let on = presets::request(
            &json!({"presetId":"glm-52","routeId":route,"effort":"high","settings":{"thinkingEnabled":true}}),
            vec![],
            &json!({}),
        );
        assert_eq!(on.body["reasoning_effort"], "high");
    }
    assert_eq!(presets::for_route("cbai/glm-5.21")["id"], "generic");
    // the NVIDIA route was shut down by the provider (410): it is not offered even if 9Router lists it
    let public = presets::public(&["cbai/glm-5.2".into(), "nvidia/z-ai/glm-5.2".into()], &json!({}));
    let glm = public.iter().find(|p| p["id"] == "glm-52").unwrap();
    assert_eq!(glm["routes"].as_array().unwrap().len(), 1);
    assert_eq!(glm["routes"][0]["id"], "cbai/glm-5.2");
}
