use modularchat_server::{presets, rules, style_policy};
use serde_json::json;

#[test]
fn api_budget_follows_characters_and_preserves_effort() {
    // (preset, route, requested effort, effective effort, reserve). On Gemini each level is its own route:
    // without an explicit request the route's level applies (effortByRoute), and the reserve is that level's.
    for (id, route, effort, effective, reserve) in [
        ("gemini-38-flash", "ag/gemini-3.8-flash-low", None, "low", 2048),
        ("claude-sonnet-5-5", "cc/claude-sonnet-5-5", Some("low"), "low", 2048),
        ("claude-sonnet-5-5", "cc/claude-sonnet-5-5", Some("medium"), "medium", 4096),
        ("deepseek-v41-flash", "cbai/deepseek-v4.1-flash", Some("high"), "high", 8192),
    ] {
        let mut p = presets::request(
            &json!({"presetId":id,"routeId":route,"effort":effort}),
            vec![],
            &json!({}),
        );
        let short = presets::apply_length(&mut p, &json!({"botChars":600}));
        assert_eq!(short["visibleTokens"], 200);
        assert_eq!(short["reasoningReserve"], reserve);
        assert_eq!(p.body["max_tokens"], 240 + reserve);
        assert_eq!(p.body["max_tokens"], p.controls["sentParams"]["max_tokens"]);
        assert_eq!(p.controls["requestedEffort"], json!(effective));
        let long = presets::apply_length(&mut p, &json!({"botChars":8000,"lengthMode":"target"}));
        assert!(long["apiMaxTokens"].as_u64().unwrap() > short["apiMaxTokens"].as_u64().unwrap());
        assert_eq!(long["minCharacters"], 6800);
    }
    let mut p = presets::request(
        &json!({"presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5","effort":"low"}),
        vec![],
        &json!({"routes":{"cc/claude-sonnet-5-5":{"maxOutput":128}}}),
    );
    let length = presets::apply_length(&mut p, &json!({"botChars":600}));
    assert_eq!(p.body["max_tokens"], 128);
    assert_eq!(length["reasoningReserve"], 0);
}

#[test]
fn maximum_does_not_expand_a_finished_reaction_but_target_still_checks_size() {
    let short = "Lia: Achei a carta.";
    let a = style_policy::assess_settings(short, 500, 100.0, &json!({}));
    assert_eq!(a["ok"], true);
    assert_eq!(a["minCharacters"], 0);
    assert!(
        style_policy::instruction_settings(&a, 100.0, &json!({})).contains("sem tamanho mínimo")
    );
    assert_eq!(
        style_policy::assess_settings(short, 500, 100.0, &json!({"lengthMode":"target"}))["sizeOk"],
        false
    );
    assert_eq!(
        style_policy::assess_settings("", 500, 100.0, &json!({}))["sizeOk"],
        false
    );
}

#[test]
fn bundled_style_examples_match_the_ratio_they_announce() {
    let examples: serde_json::Value =
        serde_json::from_str(include_str!("../src/style-examples.json")).unwrap();
    // central ratio of each example band
    for (entry, pct) in examples
        .as_array()
        .unwrap()
        .iter()
        .zip([0.0, 35.0, 65.0, 90.0, 100.0])
    {
        let a = style_policy::assess(entry["example"].as_str().unwrap(), 8000, pct);
        assert_eq!(a["ratioOk"], true, "faixa {}: {a}", entry["max"]);
        assert_eq!(a["languageOk"], true);
    }
}

#[test]
fn label_stars_and_multiline_actions_are_classified_consistently() {
    let settings = json!({"monologueChars":300});
    // a label with asterisks is still speech
    let a = style_policy::assess_settings("*Lia:* Olá, tudo bem?", 500, 100.0, &settings);
    assert_eq!(a["dialoguePct"], 100.0);
    assert_eq!(a["longestMonologueCharacters"], 14);
    // an action opened on one line and closed on the next does not become speech
    let a = style_policy::assess_settings(
        "Lia: Oi
*Ela sorri,
então fala.*",
        500,
        65.0,
        &settings,
    );
    assert_eq!(a["longestMonologueCharacters"], 2);
    assert!(a["dialoguePct"].as_f64().unwrap() < 20.0, "{a}");
    // ratio and monologue agree about "*action* speech" on the same line
    let a = style_policy::assess_settings(
        "Lia: Oi
*sorri* Olá, tudo bem?",
        500,
        65.0,
        &settings,
    );
    assert_eq!(a["longestMonologueCharacters"], 16);
    assert_eq!(a["dialoguePct"], 76.2);
}

#[test]
fn excess_over_the_ceiling_makes_the_score_worse() {
    let settings = json!({"monologueChars":0});
    let long = format!("Lia: {}", "a".repeat(1200));
    let short = format!("Lia: {}", "a".repeat(900));
    let before = style_policy::assess_settings(&long, 1000, 100.0, &settings);
    let after = style_policy::assess_settings(&short, 1000, 100.0, &settings);
    assert_eq!(before["sizeOk"], false);
    assert_eq!(after["ok"], true);
    assert!(style_policy::score(&after, 1000, 100.0) < style_policy::score(&before, 1000, 100.0));
}

#[test]
fn monologues_include_continuations_actions_and_utf16_without_combining_other_speakers() {
    let long = format!(
        "Lia: {}\n\n*Ela abre a carta.*\nLia: {}\n{}",
        "a".repeat(130),
        "b".repeat(120),
        "🚀".repeat(30)
    );
    let a = style_policy::assess_settings(&long, 8000, 65.0, &json!({"monologueChars":300}));
    assert_eq!(a["longestMonologueCharacters"], 310);
    assert_eq!(a["monologueOk"], false);
    assert_eq!(
        style_policy::assess_settings(&long, 8000, 65.0, &json!({"monologueChars":0}))["monologueOk"],
        true
    );
    let exchange = format!(
        "Lia: {}\nMara: {}\nLia: {}",
        "a".repeat(250),
        "b".repeat(240),
        "c".repeat(200)
    );
    assert_eq!(
        style_policy::assess_settings(&exchange, 8000, 100.0, &json!({}))["longestMonologueCharacters"],
        250
    );
    assert!(style_policy::turn_instruction(&json!({})).contains("devolva a vez"));
    assert!(
        style_policy::turn_instruction(&json!({"turnPacing":"balanced"}))
            .contains("no máximo um acontecimento")
    );
    assert!(style_policy::turn_instruction(&json!({"turnPacing":"free"})).contains("Ritmo livre"));
}

#[test]
fn approximate_target_is_a_centre_with_room_above_it() {
    let target = json!({"botChars":2000,"lengthMode":"target"});
    assert_eq!(rules::reply_bounds(&target), (1700, 2500, 2700));
    assert_eq!(rules::reply_bounds(&json!({"botChars":2000})), (0, 2000, 2000));
    // every size gets the same 35%, the largest one included; the store accepts it
    assert_eq!(rules::reply_bounds(&json!({"botChars":8000,"lengthMode":"target"})), (6800, 10000, 10800));
    assert_eq!(rules::REPLY_STORE_LIMIT, 10800);
    // the API budget covers the hard cut, not only the centre
    let mut p = presets::request(
        &json!({"presetId":"deepseek-v41-flash","routeId":"cbai/deepseek-v4.1-flash","effort":"high"}),
        vec![],
        &json!({}),
    );
    let length = presets::apply_length(&mut p, &target);
    assert_eq!(length["visibleTokens"], 900);
    assert_eq!(length["hardCharacters"], 2700);
    assert_eq!(length["rangeMaxCharacters"], 2500);
    // 2,400 characters are inside the range; 2,600 are above it but below the cut
    let pct = 0.0;
    let ok = format!("*{}*", "a".repeat(2398));
    assert_eq!(style_policy::assess_settings(&ok, 2000, pct, &target)["sizeOk"], true);
    let long = format!("*{}*", "a".repeat(2598));
    assert_eq!(style_policy::assess_settings(&long, 2000, pct, &target)["sizeOk"], false);
}

#[test]
fn a_cut_reply_ends_at_its_last_complete_sentence() {
    let cut = "*Ela fecha o livro.* Lia: Vamos sair agora. A atmosfera tem se mostrado ult";
    assert_eq!(rules::trim_to_sentence(cut), "*Ela fecha o livro.* Lia: Vamos sair agora.");
    // no boundary in the second half: the text is kept as received
    assert_eq!(rules::trim_to_sentence("Oi. aaaaaaaaaaaaaaaaaaaaaaaaaaaaa"), "Oi. aaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert_eq!(rules::trim_to_sentence("x".repeat(30).as_str()), "x".repeat(30));
}
