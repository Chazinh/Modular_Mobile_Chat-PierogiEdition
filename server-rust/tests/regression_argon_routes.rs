use modularchat_server::presets;
use serde_json::json;

#[test]
fn test_argon_preset_selectable_and_for_route() {
    assert!(presets::selectable_route("Argon"), "Argon must be selectable");
    assert!(presets::selectable_route("ag/gemini-4-argon-medium"), "ag/gemini-4-argon-medium must be selectable");

    let p_argon = presets::for_route("Argon");
    assert_eq!(p_argon["id"], "gemini-4-argon-medium", "Argon must map to gemini-4-argon-medium preset");

    let p_slug = presets::for_route("ag/gemini-4-argon-medium");
    assert_eq!(p_slug["id"], "gemini-4-argon-medium", "ag/gemini-4-argon-medium must map to gemini-4-argon-medium preset");
}

#[test]
fn test_argon_public_catalog_when_provider_advertises_ag_slug() {
    let ids = vec!["ag/gemini-4-argon-medium".to_string()];
    let catalog = presets::public(&ids, &json!({}));
    let argon = catalog
        .iter()
        .find(|p| p["id"] == "gemini-4-argon-medium")
        .expect("gemini-4-argon-medium preset must exist in catalog");

    assert_eq!(argon["available"], true, "Argon must be available");
    assert_eq!(argon["groupRoute"], "Argon", "Argon groupRoute must be Argon");

    let routes = argon["routes"].as_array().expect("routes must be array");
    assert!(routes.len() >= 1, "Argon must expose routes");

    let route_ids: Vec<&str> = routes.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(route_ids[0], "Argon", "groupRoute Argon must be at index 0");
    assert!(route_ids.contains(&"ag/gemini-4-argon-medium"), "ag/gemini-4-argon-medium must be present");
}

#[test]
fn test_argon_public_catalog_when_provider_advertises_argon() {
    let ids = vec!["Argon".to_string()];
    let catalog = presets::public(&ids, &json!({}));
    let argon = catalog
        .iter()
        .find(|p| p["id"] == "gemini-4-argon-medium")
        .expect("gemini-4-argon-medium preset must exist in catalog");

    assert_eq!(argon["available"], true, "Argon must be available");
    let routes = argon["routes"].as_array().unwrap();
    let route_ids: Vec<&str> = routes.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert!(route_ids.contains(&"Argon"));
    assert!(route_ids.contains(&"ag/gemini-4-argon-medium"));
}

#[test]
fn test_argon_resolve_available_success() {
    let body_argon = json!({
        "presetId": "gemini-4-argon-medium",
        "routeId": "Argon",
        "effort": "low"
    });
    let res = presets::resolve_available(&body_argon, None, &["Argon".to_string()]);
    assert!(res.is_ok(), "Argon route must resolve without 400 error: {:?}", res);
    let resolved = res.unwrap();
    assert_eq!(resolved["presetId"], "gemini-4-argon-medium");
    assert_eq!(resolved["routeId"], "Argon");
    assert_eq!(resolved["effort"], "low");

    let body_slug = json!({
        "presetId": "gemini-4-argon-medium",
        "routeId": "ag/gemini-4-argon-medium",
        "effort": "low"
    });
    let res_slug = presets::resolve_available(&body_slug, None, &["ag/gemini-4-argon-medium".to_string()]);
    assert!(res_slug.is_ok(), "ag/gemini-4-argon-medium route must resolve without 400 error: {:?}", res_slug);
    let resolved_slug = res_slug.unwrap();
    assert_eq!(resolved_slug["presetId"], "gemini-4-argon-medium");
    assert_eq!(resolved_slug["routeId"], "ag/gemini-4-argon-medium");
    assert_eq!(resolved_slug["effort"], "low");
}
