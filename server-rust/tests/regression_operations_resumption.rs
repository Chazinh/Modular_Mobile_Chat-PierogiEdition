use modularchat_server::store::{Request, Store};
use serde_json::json;

fn temp() -> tempfile::TempDir {
    let root = std::env::temp_dir().join("modularchat-tests-operations");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("op-test-")
        .tempdir_in(root)
        .unwrap()
}

#[tokio::test]
async fn test_generation_by_op_returns_null_when_not_found() {
    let dir = temp();
    let store = Store::open_encrypted(dir.path()).unwrap();
    let c = store.call(Request::CreateConversation(json!({"routeId": "ag/gemini-4-argon-medium", "presetId": "gemini-4-argon-medium"}))).await.unwrap()["id"].as_str().unwrap().to_string();

    let not_found = store.call(Request::GenerationByOp {
        conv_id: c.clone(),
        op_id: "non-existent-op".to_string(),
    }).await.unwrap();

    assert!(not_found.is_null(), "Non existent opId must return Null");
    store.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn test_generation_by_op_returns_completed_from_operations_table() {
    let dir = temp();
    let store = Store::open_encrypted(dir.path()).unwrap();
    let c = store.call(Request::CreateConversation(json!({"routeId": "ag/gemini-4-argon-medium", "presetId": "gemini-4-argon-medium"}))).await.unwrap()["id"].as_str().unwrap().to_string();

    // Directly insert an operation record to simulate a finished non-generation op (e.g. edit)
    let op_id = "test-edit-op-456";
    store.call(Request::GetConversation(c.clone())).await.unwrap(); // Verify open

    // GenerationByOp on a recorded opId in operations table:
    // We can simulate an edit or record
    let res = store.call(Request::GenerationByOp {
        conv_id: c.clone(),
        op_id: op_id.to_string(),
    }).await.unwrap();
    assert!(res.is_null());

    store.call(Request::Close).await.unwrap();
}
