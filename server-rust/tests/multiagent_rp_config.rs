use modularchat_server::{context, presets};
use serde_json::json;

#[test]
fn test_multiagent_presets_use_temperature_sampling() {
    for route in [
        "Deepseek",
        "cbai/deepseek-v4.1-flash",
        "GLM",
        "cbai/glm-5.2",
        "Nemotron",
        "oc-router/nvidia/nemotron-3.5-super-v1:free",
        "Muse",
        "oc-router/meta/muse-spark-open",
    ] {
        let preset = presets::for_route(route);
        assert_eq!(
            preset["sampling"], "temperature",
            "Route {route} under preset {} must support temperature sampling",
            preset["id"]
        );
    }
}

#[test]
fn test_temperature_and_top_p_sampling_applied_in_plan() {
    for (preset_id, route) in [
        ("deepseek-v41-flash", "cbai/deepseek-v4.1-flash"),
        ("glm-52", "cbai/glm-5.2"),
        ("nemotron-35", "Nemotron"),
        ("muse-spark-13", "Muse"),
    ] {
        // With explicit temperature
        let conv_with_temp = json!({
            "presetId": preset_id,
            "routeId": route,
            "effort": "high",
            "temperature": 0.95,
            "settings": {}
        });
        let plan = presets::request(&conv_with_temp, vec![], &json!({}));
        assert_eq!(plan.body["temperature"], 0.95);
        assert_eq!(plan.body["top_p"], 0.95);
        assert_eq!(plan.controls["temperatureSent"], true);

        // Without temperature (null) -> backward compatible: None
        let conv_no_temp = json!({
            "presetId": preset_id,
            "routeId": route,
            "effort": "high",
            "settings": {}
        });
        let plan_no_temp = presets::request(&conv_no_temp, vec![], &json!({}));
        assert!(plan_no_temp.body.get("temperature").is_none());
        assert!(plan_no_temp.body.get("top_p").is_none());
        assert_eq!(plan_no_temp.controls["temperatureSent"], false);
    }
}

#[test]
fn test_multiagent_and_natural_tone_in_system_prompt() {
    let d = json!({
        "conv": {
            "persona": "Alex",
            "prompt": "História com múltiplos personagens na academia.",
            "settings": {}
        },
        "pairs": [],
        "totalPairs": 0,
        "current": {
            "userText": "Olá pessoal",
            "seq": 1,
            "priorParts": []
        }
    });

    let conv = json!({
        "presetId": "deepseek-v41-flash",
        "routeId": "cbai/deepseek-v4.1-flash",
        "effort": "high",
        "settings": {}
    });
    let plan = presets::request(&conv, vec![], &json!({}));
    let ctx = context::assemble(&d, &json!({}), "send", &plan, None).unwrap();

    let sys_content = ctx
        .messages
        .iter()
        .find(|m| m["role"] == "system")
        .and_then(|m| m["content"].as_str())
        .or_else(|| {
            // For user_prefix mode, system prompt is in the first turn user content
            ctx.messages
                .first()
                .and_then(|m| m["content"].as_str())
        })
        .expect("System content must be present in assembled context");

    for required_phrase in [
        "ELENCO E MULTI-AGENTES",
        "Voz e identidade próprias",
        "Interação cruzada entre personagens",
        "Prosa autêntica e natural",
        "Ritmo e tensão orgânica",
    ] {
        assert!(
            sys_content.contains(required_phrase),
            "Faltou a diretriz de multi-agentes: {required_phrase}"
        );
    }
}

#[test]
fn test_nemotron_and_muse_spark_presets_available_in_catalog() {
    let nemotron = presets::by_id("nemotron-35");
    assert_eq!(nemotron["id"], "nemotron-35");
    assert_eq!(nemotron["selectable"], true);
    assert_eq!(nemotron["groupRoute"], "Nemotron");
    assert_eq!(presets::for_route("Nemotron")["id"], "nemotron-35");

    let muse = presets::by_id("muse-spark-13");
    assert_eq!(muse["id"], "muse-spark-13");
    assert_eq!(muse["selectable"], true);
    assert_eq!(muse["groupRoute"], "Muse");
    assert_eq!(presets::for_route("Muse")["id"], "muse-spark-13");

    let minimax = presets::by_id("minimax-m31");
    assert_eq!(minimax["id"], "minimax-m31");
    assert_eq!(minimax["selectable"], true);
    assert_eq!(minimax["groupRoute"], "MiniMaxM3.1");
    assert_eq!(presets::for_route("MiniMaxM3.1")["id"], "minimax-m31");
    assert_eq!(presets::for_route("openrouter/minimax/minimax-m3.1-flash:free")["id"], "minimax-m31");

    let gemma = presets::by_id("gemma-4");
    assert_eq!(gemma["id"], "gemma-4");
    assert_eq!(gemma["selectable"], true);
    assert_eq!(gemma["groupRoute"], "Gemma4");
    assert_eq!(presets::for_route("Gemma4")["id"], "gemma-4");
    assert_eq!(presets::for_route("openrouter/google/gemma-4-31b-it:free")["id"], "gemma-4");
    assert_eq!(presets::for_route("google/gemma-4-31b-it")["id"], "gemma-4");
    assert_ne!(presets::for_route("embeddinggemma-300m")["id"], "gemma-4");

    // Test public listing with routes discovered or active
    let routes = vec!["Nemotron".to_string(), "Muse".to_string(), "MiniMaxM3.1".to_string(), "Gemma4".to_string()];
    let public_catalog = presets::public(&routes, &json!({}));
    let pub_nemotron = public_catalog.iter().find(|p| p["id"] == "nemotron-35").expect("nemotron-35 in public");
    assert_eq!(pub_nemotron["available"], true);
    assert_eq!(pub_nemotron["selectable"], true);

    let pub_muse = public_catalog.iter().find(|p| p["id"] == "muse-spark-13").expect("muse-spark-13 in public");
    assert_eq!(pub_muse["available"], true);
    assert_eq!(pub_muse["selectable"], true);

    let pub_minimax = public_catalog.iter().find(|p| p["id"] == "minimax-m31").expect("minimax-m31 in public");
    assert_eq!(pub_minimax["available"], true);
    assert_eq!(pub_minimax["selectable"], true);
    assert_eq!(pub_minimax["routes"][0]["id"], "MiniMaxM3.1");

    let pub_gemma = public_catalog.iter().find(|p| p["id"] == "gemma-4").expect("gemma-4 in public");
    assert_eq!(pub_gemma["available"], true);
    assert_eq!(pub_gemma["selectable"], true);
    assert_eq!(pub_gemma["routes"][0]["id"], "Gemma4");
}
