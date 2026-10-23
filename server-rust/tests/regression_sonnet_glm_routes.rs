use modularchat_server::presets;
use serde_json::json;

#[test]
fn test_regression_glm_53_route_matching_and_catalog() {
    // Both 5.2 and 5.3 must match the glm-52 preset
    for route in [
        "GLM",
        "cbai/glm-5.2",
        "oc/glm-5.3-flash-free",
        "openrouter/z-ai/glm-5.3-flash",
        "openrouter/z-ai/glm-5.2",
        "glm-5.2",
        "glm-5.3",
    ] {
        assert_eq!(
            presets::for_route(route)["id"],
            "glm-52",
            "Route {route} should match glm-52 preset"
        );
    }

    // Boundary check: arbitrary subversions like 5.21 and 5.30 must fall back to generic
    assert_eq!(presets::for_route("cbai/glm-5.21")["id"], "generic");
    assert_eq!(presets::for_route("cbai/glm-5.30")["id"], "generic");
    assert_eq!(presets::for_route("openrouter/z-ai/glm-5.4")["id"], "generic");

    // Public catalog check: oc/glm-5.3-flash-free advertised by 9Router appears under GLM preset
    let available_routes = vec![
        "GLM".to_string(),
        "cbai/glm-5.2".to_string(),
        "oc/glm-5.3-flash-free".to_string(),
    ];
    let catalog = presets::public(&available_routes, &json!({}));
    let glm = catalog.iter().find(|p| p["id"] == "glm-52").expect("glm-52 preset must exist");
    let glm_routes: Vec<&str> = glm["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();

    assert!(
        glm_routes.contains(&"oc/glm-5.3-flash-free"),
        "GLM preset must expose oc/glm-5.3-flash-free in routes: {:?}",
        glm_routes
    );
    assert!(
        glm_routes.contains(&"openrouter/z-ai/glm-5.3-flash"),
        "GLM preset must expose openrouter/z-ai/glm-5.3-flash in routes: {:?}",
        glm_routes
    );
    assert!(
        glm_routes.contains(&"openrouter/z-ai/glm-5.2"),
        "GLM preset must expose openrouter/z-ai/glm-5.2 in routes: {:?}",
        glm_routes
    );
    assert_eq!(glm_routes[0], "GLM", "groupRoute GLM must be first");

    // Generic preset should not steal GLM 5.3
    let generic = catalog.iter().find(|p| p["id"] == "generic");
    if let Some(g_preset) = generic {
        let gen_routes: Vec<&str> = g_preset["routes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert!(
            !gen_routes.contains(&"oc/glm-5.3-flash-free"),
            "Generic preset must not contain oc/glm-5.3-flash-free"
        );
    }
}

#[test]
fn test_regression_sonnet_thinking_disabled_multilingual_headroom_and_sentence_trim() {
    let conv = json!({
        "presetId": "claude-sonnet-5-5",
        "routeId": "Sonnet",
        "effort": "medium",
        "settings": {
            "thinkingEnabled": false
        }
    });

    let mut plan = presets::request(&conv, vec![], &json!({}));
    assert_eq!(plan.controls["thinkingEnabled"], false);
    assert_eq!(plan.body["reasoning_effort"], "none");

    // For a 2000 character target (hard = 2700)
    let length = presets::apply_length(&mut plan, &json!({"botChars": 2000, "lengthMode": "target"}));
    assert_eq!(length["thinkingEnabled"], false);
    assert_eq!(length["reasoningReserve"], 0);

    // Multilingual headroom ensures the token budget is at least 1680 tokens (preventing truncation of Portuguese text)
    let total_tokens = plan.body["max_tokens"].as_u64().unwrap();
    assert!(
        total_tokens >= 1680,
        "Sonnet thinking-disabled for 2000 char target must allocate at least 1680 tokens, got {total_tokens}"
    );
    assert_eq!(length["apiMaxTokens"], total_tokens);
    assert_eq!(plan.controls["sentParams"]["max_tokens"], total_tokens);

    // For a 1500 character target (hard = 2025)
    let mut plan_1500 = presets::request(&conv, vec![], &json!({}));
    let _length_1500 = presets::apply_length(&mut plan_1500, &json!({"botChars": 1500, "lengthMode": "target"}));
    let total_1500 = plan_1500.body["max_tokens"].as_u64().unwrap();
    assert!(
        total_1500 >= 1140,
        "Sonnet thinking-disabled for 1500 char target must allocate at least 1140 tokens (was 810), got {total_1500}"
    );

    // Baseline 600 character contract is strictly preserved (hard = 600 -> exactly 240)
    let mut plan_600 = presets::request(&conv, vec![], &json!({}));
    let length_600 = presets::apply_length(&mut plan_600, &json!({"botChars": 600}));
    assert_eq!(length_600["visibleTokens"], 200);
    assert_eq!(length_600["textTokenBudget"], 240);
    assert_eq!(plan_600.body["max_tokens"], 240);

    // Verify sentence trimmer properly salvages cut dialogue when provider hits length
    let cut_dialogue = "Caio: Torce o quanto quiser, pirralha. Quem decide é o pessoal da Yulia.\n\nIuno:";
    let trimmed = modularchat_server::rules::trim_to_sentence(cut_dialogue);
    assert_eq!(
        trimmed,
        "Caio: Torce o quanto quiser, pirralha. Quem decide é o pessoal da Yulia."
    );
    assert!(!trimmed.ends_dialogue_or_cut_midword("Iuno:"));
}

trait SentenceTestHelper {
    fn ends_dialogue_or_cut_midword(&self, needle: &str) -> bool;
}

impl SentenceTestHelper for str {
    fn ends_dialogue_or_cut_midword(&self, needle: &str) -> bool {
        self.ends_with(needle)
    }
}
