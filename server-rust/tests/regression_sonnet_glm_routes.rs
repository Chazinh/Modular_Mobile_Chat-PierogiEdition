use modularchat_server::presets;
use serde_json::json;


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
