use modularchat_server::presets;
use serde_json::json;

#[test]
fn hydrogen_generic_and_low_effort_preserve_reasoning_controls_and_token_reserve() {
    // Regression: a Hydrogen conversation saved with presetId="generic" (e.g. from a stale server downgrade)
    // must still resolve to gemini-4-hydrogen and emit reasoning_effort + thinking reserve instead of
    // omitting reasoning_effort and starving content at max_tokens=1356.
    let mut from_generic = presets::request(
        &json!({
            "presetId": "generic",
            "routeId": "Hydrogen",
            "effort": null
        }),
        vec![json!({"role": "user", "content": "Oi"})],
        &json!({}),
    );
    assert_eq!(from_generic.body["model"], "Hydrogen");
    assert_eq!(from_generic.body["reasoning_effort"], "high");
    assert!(from_generic.body.get("temperature").is_none());
    let len_high = presets::apply_length(&mut from_generic, &json!({"botChars": 1700, "lengthMode": "target"}));
    assert_eq!(len_high["reasoningReserve"], 8192);
    assert!(from_generic.body["max_tokens"].as_u64().unwrap() > 8192);

    // Explicit low effort on Hydrogen (both under presetId="generic" and "gemini-4-hydrogen")
    for preset_id in ["generic", "gemini-4-hydrogen"] {
        let mut low = presets::request(
            &json!({
                "presetId": preset_id,
                "routeId": "Hydrogen",
                "effort": "low"
            }),
            vec![json!({"role": "user", "content": "Oi"})],
            &json!({}),
        );
        assert_eq!(low.body["model"], "Hydrogen");
        assert_eq!(low.body["reasoning_effort"], "low");
        assert_eq!(low.controls["requestedEffort"], "low");
        let len_low = presets::apply_length(&mut low, &json!({"botChars": 1700, "lengthMode": "target"}));
        assert_eq!(len_low["reasoningReserve"], 2048);
        assert!(low.body["max_tokens"].as_u64().unwrap() > 2048);
    }
}

#[test]
fn migrate_and_resolve_upgrade_generic_hydrogen_routes() {
    assert_eq!(
        presets::migrate("generic", "Hydrogen", &json!(null)),
        ("Hydrogen".into(), "gemini-4-hydrogen".into(), json!("high"))
    );
    assert_eq!(
        presets::migrate("generic", "ag/gemini-4-hydrogen-low", &json!(null)),
        ("ag/gemini-4-hydrogen-low".into(), "gemini-4-hydrogen".into(), json!("low"))
    );
    assert_eq!(
        presets::migrate("generic", "Argon", &json!(null)),
        ("Argon".into(), "gemini-4-argon-medium".into(), json!("low"))
    );
    assert_eq!(
        presets::migrate("generic", "other/model", &json!(null)),
        ("other/model".into(), "generic".into(), json!(null))
    );

    let resolved = presets::resolve(
        &json!({"presetId": "generic", "routeId": "Hydrogen", "effort": "low"}),
        None,
    )
    .unwrap();
    assert_eq!(resolved["presetId"], "gemini-4-hydrogen");
    assert_eq!(resolved["routeId"], "Hydrogen");
    assert_eq!(resolved["effort"], "low");
}
