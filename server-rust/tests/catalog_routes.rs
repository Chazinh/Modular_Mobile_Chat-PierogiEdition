use axum::{body::Body, extract::ConnectInfo, http::Request};
use http_body_util::BodyExt;
use modularchat_server::{api::{self, AppState}, config::Config, providers::Providers};
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc};
use tower::ServiceExt;

async fn request(state: &AppState, method: &str, path: &str, body: Value) -> (u16, Value) {
    let response = api::router(state.clone()).oneshot(Request::builder().method(method).uri(path)
        .header("content-type", "application/json")
        .extension(ConnectInfo("127.0.0.1:1".parse::<SocketAddr>().unwrap()))
        .body(if body.is_null() { Body::empty() } else { Body::from(body.to_string()) }).unwrap()).await.unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn desktop_catalog_lists_and_saves_configured_opencode_routes() {
    let dir = tempfile::tempdir().unwrap();
    let providers = Providers::from_json(&json!({"providers":[{"id":"router","baseUrl":"http://127.0.0.1:1/v1",
        "models":["oc/deepseek-v4-flash", "oc/deepseek-v4.1-flash", "oc/space-bunny-free"]}]})).unwrap();
    let state = AppState::new(Config { data_dir:dir.path().into(), dist:dir.path().join("absent"),
        address:"127.0.0.1:0".parse().unwrap(), require_pairing:false, providers:Some(Arc::new(providers)),
        max_generations:1, production:false, native_lib_dir:None, review:false }).await.unwrap();
    state.runner.stop();
    let (status, catalog) = request(&state,"GET","/api/models",Value::Null).await;
    assert_eq!(status,200,"{catalog}");
    assert!(catalog["models"].as_array().unwrap().contains(&json!("oc/deepseek-v4-flash")));
    assert!(!catalog["models"].as_array().unwrap().contains(&json!("oc/space-bunny-free")));
    let (status, chat) = request(&state,"POST","/api/chats",json!({"name":"Catalog fixture","persona":"Alex",
        "presetId":"generic","routeId":"oc/deepseek-v4-flash","effort":"high"})).await;
    assert_eq!(status,201,"{chat}");
    assert_eq!(chat["routeId"],"oc/deepseek-v4-flash");
    assert!(chat["effort"].is_null());
    let path = format!("/api/chats/{}",chat["id"].as_str().unwrap());
    let (status, _) = request(&state,"PUT",&path,json!({"presetId":"deepseek-v41-flash","routeId":"oc/deepseek-v4.1-flash"})).await;
    assert_eq!(status,200);
    let (status, updated) = request(&state,"PUT",&path,json!({"presetId":"generic","routeId":"oc/deepseek-v4-flash"})).await;
    assert_eq!(status,200,"{updated}");
    let (status, _) = request(&state,"PUT",&path,json!({"presetId":"generic","routeId":"unconfigured/model"})).await;
    assert_eq!(status,400);
    state.shutdown().await;
}
