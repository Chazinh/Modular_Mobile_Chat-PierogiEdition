use axum::{body::Body, extract::ConnectInfo, http::Request};
use http_body_util::BodyExt;
use modularchat_server::{
    api::{self, AppState},
    config::Config,
    presets,
    providers::Providers,
};
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc};
use tower::ServiceExt;

async fn request(state: &AppState, method: &str, path: &str, body: Value) -> (u16, Value) {
    let response = api::router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .extension(ConnectInfo("127.0.0.1:1".parse::<SocketAddr>().unwrap()))
                .body(if body.is_null() {
                    Body::empty()
                } else {
                    Body::from(body.to_string())
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[test]
fn test_glm_dual_routes_public_when_provider_advertises_only_cbai() {
    let ids = vec!["cbai/glm-5.2".to_string()];
    let catalog = presets::public(&ids, &json!({}));
    let glm = catalog
        .iter()
        .find(|p| p["id"] == "glm-52")
        .expect("glm-52 preset must exist in catalog");

    assert_eq!(glm["available"], true, "glm-52 must be available");
    assert_eq!(glm["groupRoute"], "GLM", "glm-52 must have groupRoute GLM");

    let routes = glm["routes"].as_array().expect("routes must be array");
    assert_eq!(routes.len(), 2, "glm-52 must expose exactly 2 active routes");

    let route_ids: Vec<&str> = routes.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(route_ids[0], "GLM", "groupRoute GLM must be first at index 0");
    assert_eq!(route_ids[1], "cbai/glm-5.2", "cbai/glm-5.2 must be present at index 1");
    assert!(!route_ids.contains(&"nvidia/z-ai/glm-5.2"), "Retired route must not be exposed");
}

#[test]
fn test_glm_dual_routes_public_when_provider_advertises_only_glm() {
    let ids = vec!["GLM".to_string()];
    let catalog = presets::public(&ids, &json!({}));
    let glm = catalog
        .iter()
        .find(|p| p["id"] == "glm-52")
        .expect("glm-52 preset must exist in catalog");

    assert_eq!(glm["available"], true);
    let routes = glm["routes"].as_array().unwrap();
    assert_eq!(routes.len(), 2, "Must expose both routes even when provider lists only GLM");

    let route_ids: Vec<&str> = routes.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert!(route_ids.contains(&"GLM"));
    assert!(route_ids.contains(&"cbai/glm-5.2"));
}

#[test]
fn test_glm_dual_routes_public_when_provider_advertises_both() {
    let ids = vec!["GLM".to_string(), "cbai/glm-5.2".to_string()];
    let catalog = presets::public(&ids, &json!({}));
    let glm = catalog.iter().find(|p| p["id"] == "glm-52").unwrap();

    assert_eq!(glm["available"], true);
    let routes = glm["routes"].as_array().unwrap();
    assert_eq!(routes.len(), 2, "Must not duplicate routes when both are advertised");

    let route_ids: Vec<&str> = routes.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(route_ids[0], "GLM");
    assert_eq!(route_ids[1], "cbai/glm-5.2");
}

#[test]
fn test_glm_dual_routes_public_filters_retired_routes() {
    let ids = vec![
        "oc/deepseek-v4.1-flash".to_string(),
        "cbai/glm-5.2".to_string(),
        "nvidia/z-ai/glm-5.2".to_string(),
    ];
    let catalog = presets::public(&ids, &json!({}));
    let glm = catalog.iter().find(|p| p["id"] == "glm-52").unwrap();

    let route_ids: Vec<&str> = glm["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert!(!route_ids.contains(&"nvidia/z-ai/glm-5.2"), "Retired nvidia route must be excluded");
    assert!(route_ids.contains(&"GLM"));
    assert!(route_ids.contains(&"cbai/glm-5.2"));
}

#[test]
fn test_glm_dual_routes_public_empty_provider_routes() {
    let ids: Vec<String> = vec![];
    let catalog = presets::public(&ids, &json!({}));
    let glm = catalog.iter().find(|p| p["id"] == "glm-52").unwrap();

    assert!(glm["available"].is_null(), "available must be null when no provider routes configured");
    assert_eq!(glm["routes"].as_array().unwrap().len(), 0, "routes must be empty");
}

#[test]
fn test_glm_route_resolution_and_validation() {
    // 1. Resolve GLM route
    let res_glm = presets::resolve(&json!({"presetId": "glm-52", "routeId": "GLM"}), None)
        .expect("Resolving GLM route must succeed");
    assert_eq!(res_glm["routeId"], "GLM");
    assert_eq!(res_glm["presetId"], "glm-52");
    assert_eq!(res_glm["effort"], "high");

    // 2. Resolve cbai/glm-5.2 route
    let res_cbai = presets::resolve(&json!({"presetId": "glm-52", "routeId": "cbai/glm-5.2"}), None)
        .expect("Resolving cbai/glm-5.2 route must succeed");
    assert_eq!(res_cbai["routeId"], "cbai/glm-5.2");
    assert_eq!(res_cbai["presetId"], "glm-52");
    assert_eq!(res_cbai["effort"], "high");

    // 3. Switch route on existing conversation preserving effort
    let current = json!({"presetId": "glm-52", "routeId": "GLM", "effort": "low"});
    let switched = presets::resolve(
        &json!({"presetId": "glm-52", "routeId": "cbai/glm-5.2"}),
        Some(&current),
    )
    .expect("Route switch must succeed");
    assert_eq!(switched["routeId"], "cbai/glm-5.2");
    assert_eq!(switched["presetId"], "glm-52");
    assert_eq!(switched["effort"], "low", "Effort must be preserved across route switch");

    // 4. Reject foreign route under glm-52
    let err = presets::resolve(
        &json!({"presetId": "glm-52", "routeId": "oc/deepseek-v4.1-flash"}),
        None,
    );
    assert!(err.is_err());
    assert_eq!(err.unwrap_err().status, 400);

    // 5. Reject retired route
    let err_ret = presets::resolve(
        &json!({"presetId": "glm-52", "routeId": "nvidia/z-ai/glm-5.2"}),
        None,
    );
    assert!(err_ret.is_err());
}

#[tokio::test]
async fn test_glm_api_endpoints_and_chat_route_switching() {
    let dir = tempfile::tempdir().unwrap();
    let providers = Providers::from_json(&json!({
        "providers": [
            {
                "id": "router",
                "baseUrl": "http://127.0.0.1:1/v1",
                "models": ["cbai/glm-5.2"]
            }
        ]
    }))
    .unwrap();
    let state = AppState::new(Config {
        data_dir: dir.path().into(),
        dist: dir.path().join("absent"),
        address: "127.0.0.1:0".parse().unwrap(),
        require_pairing: false,
        providers: Some(Arc::new(providers)),
        max_generations: 1,
        production: false,
        native_lib_dir: None,
        review: false,
    })
    .await
    .unwrap();
    state.runner.stop();

    // 1. GET /api/presets exposes dual GLM routes
    let (status, presets_body) = request(&state, "GET", "/api/presets", Value::Null).await;
    assert_eq!(status, 200, "{presets_body}");
    let presets_arr = presets_body["presets"].as_array().unwrap();
    let glm_preset = presets_arr.iter().find(|p| p["id"] == "glm-52").expect("glm-52 in /api/presets");
    let routes = glm_preset["routes"].as_array().unwrap();
    assert_eq!(routes.len(), 2, "/api/presets must return 2 routes for glm-52");
    let route_ids: Vec<&str> = routes.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(route_ids[0], "GLM");
    assert_eq!(route_ids[1], "cbai/glm-5.2");

    // 2. GET /api/models exposes dual GLM routes in catalog
    let (status, models_body) = request(&state, "GET", "/api/models", Value::Null).await;
    assert_eq!(status, 200, "{models_body}");
    let models_presets = models_body["presets"].as_array().unwrap();
    let glm_models_preset = models_presets.iter().find(|p| p["id"] == "glm-52").unwrap();
    assert_eq!(glm_models_preset["routes"].as_array().unwrap().len(), 2);

    // 3. POST /api/chats creates conversation with routeId: "GLM"
    let (status, chat) = request(
        &state,
        "POST",
        "/api/chats",
        json!({
            "name": "GLM Route Test",
            "persona": "Alex",
            "presetId": "glm-52",
            "routeId": "GLM"
        }),
    )
    .await;
    assert_eq!(status, 201, "{chat}");
    assert_eq!(chat["presetId"], "glm-52");
    assert_eq!(chat["routeId"], "GLM");
    let chat_id = chat["id"].as_str().unwrap();
    let chat_path = format!("/api/chats/{}", chat_id);

    // 4. PUT /api/chats/{id} switches route to "cbai/glm-5.2"
    let (status, updated) = request(
        &state,
        "PUT",
        &chat_path,
        json!({
            "presetId": "glm-52",
            "routeId": "cbai/glm-5.2"
        }),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["routeId"], "cbai/glm-5.2");
    assert_eq!(updated["presetId"], "glm-52");

    // 5. PUT /api/chats/{id} switches route back to "GLM"
    let (status, reverted) = request(
        &state,
        "PUT",
        &chat_path,
        json!({
            "presetId": "glm-52",
            "routeId": "GLM"
        }),
    )
    .await;
    assert_eq!(status, 200, "{reverted}");
    assert_eq!(reverted["routeId"], "GLM");
    assert_eq!(reverted["presetId"], "glm-52");

    state.shutdown().await;
}
