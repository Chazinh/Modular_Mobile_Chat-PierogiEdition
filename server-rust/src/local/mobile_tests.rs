use super::*;
use crate::{
    api::{self, AppState},
    store::Request,
};
use axum::{
    Json, Router,
    body::Body,
    extract::ConnectInfo,
    http::Request as HttpRequest,
    routing::{get, post},
};
use http_body_util::BodyExt;
use model_files::Pin;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const TOKEN: &str = "android_fixture_0123456789abcdef0123456789abcdef0123456789abcdef";

// Spawned by Process::ensure in these tests. It is a real owned OS child and HTTP fixture,
// not llama.cpp or device evidence. Nothing downloads a GGUF in this test binary.
#[test]
#[ignore = "owned HTTP subprocess fixture only"]
fn owned_model_fixture() {
    let Ok(port) = std::env::var("MC_LIFECYCLE_FIXTURE_PORT") else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await.unwrap();
        let router = Router::new()
            .route("/health", get(|| async { Json(json!({"ok":true})) }))
            .route("/apply-template", post(|Json(body): Json<Value>| async move {
                Json(json!({"prompt":body["messages"].as_array().unwrap().iter().filter_map(|m|m["content"].as_str()).collect::<Vec<_>>().join("\n")}))
            }))
            .route("/tokenize", post(|Json(body): Json<Value>| async move {
                Json(json!({"tokens":vec![1; body["content"].as_str().unwrap().chars().count()]}))
            }))
            .route("/v1/chat/completions", post(|Json(body): Json<Value>| async move {
                use axum::response::IntoResponse;
                if body["stream"] == true {
                    let stream = futures_util::stream::once(async {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        Ok::<_,std::io::Error>("data: [DONE]\n\n")
                    });
                    return ([("content-type","text/event-stream")], Body::from_stream(stream)).into_response();
                }
                Json(json!({"choices":[{"finish_reason":"stop","message":{"content":json!({"budget":body["max_tokens"]}).to_string()}}]})).into_response()
            }))
            .route("/v1/embeddings", post(|Json(body): Json<Value>| async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let input = body["input"][0].as_str().unwrap();
                let vector = if input.starts_with("task: search result | query:") || input.starts_with("title: História | text:") {
                    vec![3.0, 4.0]
                } else { vec![0.0, 0.0] };
                Json(json!({"data":[{"embedding":vector}]}))
            }));
        axum::serve(listener, router).await.unwrap();
    });
}

async fn mobile_fixture(dir: &std::path::Path, idle: Duration) -> LocalModels {
    let listener_chat = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listener_embed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ports = [
        listener_chat.local_addr().unwrap().port(),
        listener_embed.local_addr().unwrap().port(),
    ];
    let root = dir.join("models");
    tokio::fs::create_dir(&root).await.unwrap();
    let bytes = b"tiny integrity fixture";
    let sha256 = Box::leak(format!("{:x}", Sha256::digest(bytes)).into_boxed_str());
    let pin = |filename| Pin {
        repo: "fixture",
        revision: "immutable",
        filename,
        size: bytes.len() as u64,
        sha256,
    };
    let chat_file = Arc::new(ModelFile::fixture(
        root.clone(),
        pin("chat-fixture.gguf"),
        "http://127.0.0.1:1/unused".into(),
    ));
    let embed_file = Arc::new(ModelFile::fixture(
        root,
        pin("embeddinggemma-fixture.gguf"),
        "http://127.0.0.1:1/unused".into(),
    ));
    for file in [&chat_file, &embed_file] {
        tokio::fs::write(&file.path, bytes).await.unwrap();
        file.provision(&Cancel::new()).await.unwrap();
    }
    let client = reqwest::Client::new();
    let process = |file: Arc<ModelFile>, port| {
        Arc::new(Process {
            name: "owned fixture".into(),
            exe: std::env::current_exe().unwrap(),
            model: file.path.clone(),
            port,
            args: vec![strings(&["--pooling", "mean"])],
            idle: 0,
            child: Mutex::new(None),
            enabled: true,
            client: client.clone(),
            startup_timeout: Duration::from_secs(5),
            mobile_file: Some(file),
            fixture_command: Some(strings(&[
                "--ignored",
                "--exact",
                "local::mobile_tests::owned_model_fixture",
                "--nocapture",
            ])),
        })
    };
    let chat = process(chat_file.clone(), ports[0]);
    let embed = process(embed_file.clone(), ports[1]);
    drop(listener_chat);
    drop(listener_embed);
    let mut mobile = Mobile::new(chat.clone(), embed.clone(), chat_file, embed_file.clone());
    Mobile::test_deadline(&mut mobile, idle);
    LocalModels {
        chat,
        embed,
        mock: None,
        client,
        chat_timeout: Duration::from_secs(5),
        mobile: Some(mobile),
        embed_digest: embed_file.digest.clone(),
    }
}
#[tokio::test]
async fn chat_session_holds_operation_through_stream_and_standby_until_drop() {
    let dir = tempfile::tempdir().unwrap();
    let local = mobile_fixture(dir.path(), Duration::from_secs(240)).await;
    local.mobile.as_ref().unwrap().activate().await.unwrap();
    let session = local.chat_session().await.unwrap();
    let response = session
        .send(
            &json!({"messages":[{"role":"user","content":"hello"}],"max_tokens":64,"stream":true}),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    assert_eq!(local.status().await["activeOperations"], 1);
    local.standby().await.unwrap();
    assert_eq!(local.status().await["activeOperations"], 1);
    assert!(local.chat.child.lock().await.is_some());
    assert!(response.text().await.unwrap().contains("[DONE]"));
    assert!(
        local.chat.child.lock().await.is_some(),
        "stream owner must prevent standby unload"
    );
    drop(session);
    assert_eq!(local.status().await["activeOperations"], 0);
    unloaded(&local.chat).await;
    assert!(local.chat_session().await.is_err());
    local.stop().await.unwrap();
}

async fn active_child(local: &LocalModels) -> u32 {
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            if local.mobile.as_ref().unwrap().status()["activeOperations"] == 1
                && let Ok(owner) = local.embed.child.try_lock()
                && let Some(child) = owner.as_ref()
            {
                break child.id().unwrap();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
async fn unloaded(process: &Process) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if process.child.lock().await.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn idle_unload_protects_real_active_request_then_reaps_and_respawns() {
    let dir = tempfile::tempdir().unwrap();
    let local = mobile_fixture(dir.path(), Duration::from_millis(80)).await;
    local.activate().await.unwrap();
    local.activate().await.unwrap();
    let work = local.clone();
    let request = tokio::spawn(async move { work.embed_text("história", true).await });
    let first_pid = active_child(&local).await;
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(
        local.mobile.as_ref().unwrap().status()["activeOperations"],
        1
    );
    assert!(
        local
            .embed
            .child
            .lock()
            .await
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none()
    );
    assert_eq!(request.await.unwrap().unwrap(), vec![0.6, 0.8]);
    assert_eq!(
        local.mobile.as_ref().unwrap().status()["activeOperations"],
        0
    );
    assert!(
        local.embed.child.lock().await.is_some(),
        "idle starts at completion, not request start"
    );
    unloaded(&local.embed).await;
    assert!(
        local.embed_available(),
        "unload preserves provisioning and activation"
    );
    let work = local.clone();
    let request = tokio::spawn(async move { work.embed_text("próxima consulta", true).await });
    let second_pid = active_child(&local).await;
    assert_ne!(first_pid, second_pid);
    assert_eq!(request.await.unwrap().unwrap(), vec![0.6, 0.8]);
    local.stop().await.unwrap();
    assert!(local.embed.child.lock().await.is_none());
}

#[tokio::test]
async fn standby_waits_for_operation_and_cancellation_drops_the_raii_count() {
    let dir = tempfile::tempdir().unwrap();
    let local = mobile_fixture(dir.path(), Duration::from_secs(10)).await;
    for cancel in [false, true] {
        local.activate().await.unwrap();
        let work = local.clone();
        let request = tokio::spawn(async move { work.embed_text("história", false).await });
        active_child(&local).await;
        let status = local.standby().await.unwrap();
        assert_eq!(status["activated"], false);
        assert_eq!(status["activeOperations"], 1);
        assert!(!local.embed_available());
        assert!(
            local
                .embed
                .child
                .lock()
                .await
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_none()
        );
        if cancel {
            request.abort();
            assert!(request.await.unwrap_err().is_cancelled());
        } else {
            assert_eq!(request.await.unwrap().unwrap(), vec![0.6, 0.8]);
        }
        assert_eq!(
            local.mobile.as_ref().unwrap().status()["activeOperations"],
            0
        );
        unloaded(&local.embed).await;
        local.standby().await.unwrap();
    }
    local.stop().await.unwrap();
    local.stop().await.unwrap();
    assert!(local.activate().await.is_err());
}

#[tokio::test]
async fn mobile_template_budget_and_single_resident_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let local = mobile_fixture(dir.path(), Duration::from_secs(10)).await;
    local.activate().await.unwrap();
    local.embed_text("consulta", true).await.unwrap();
    assert!(local.embed.child.lock().await.is_some());
    let result = local
        .chat_json(
            vec![json!({"role":"user","content":"x".repeat(2600)})],
            json!({"type":"object"}),
            2200,
            0.1,
        )
        .await
        .unwrap();
    assert_eq!(result["budget"], 4096 - 2600 - 64);
    assert!(local.embed.child.lock().await.is_none());
    assert!(local.chat.child.lock().await.is_some());
    let error = local
        .chat_json(
            vec![json!({"role":"user","content":"x".repeat(3900)})],
            json!({"type":"object"}),
            3000,
            0.1,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status, 502);
    assert_ne!(error.extra["code"], "LOCAL_UNAVAILABLE");
    assert_eq!(
        local.mobile.as_ref().unwrap().status()["activeOperations"],
        0
    );
    local.stop().await.unwrap();
}

#[tokio::test]
async fn drop_does_not_leave_a_strong_housekeeping_owner() {
    let dir = tempfile::tempdir().unwrap();
    let local = mobile_fixture(dir.path(), Duration::from_secs(10)).await;
    let weak = Arc::downgrade(local.mobile.as_ref().unwrap());
    local.activate().await.unwrap();
    drop(local);
    tokio::time::timeout(Duration::from_secs(3), async {
        while weak.upgrade().is_some() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn android_profile_pins_cpu_context_and_standby_without_io() {
    let dir = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let config = Config::for_android(dir.path(), native.path()).unwrap();
    let local = LocalModels::new(&config, reqwest::Client::new()).unwrap();
    assert_eq!(local.chat.exe.file_name().unwrap(), "libllama_server.so");
    assert_eq!(
        local.chat.model,
        config.data_dir.join("models").join(CHAT.filename)
    );
    assert_eq!(
        local.embed.model,
        config.data_dir.join("models").join(EMBED.filename)
    );
    let arg = |process: &Process, key: &str| {
        process.args[0].windows(2).find(|p| p[0] == key).unwrap()[1].clone()
    };
    for process in [&local.chat, &local.embed] {
        assert_eq!(arg(process, "-ngl"), "0");
        assert_eq!(arg(process, "--device"), "none");
        assert!(arg(process, "-t").parse::<usize>().unwrap() <= 4);
        assert!(arg(process, "-tb").parse::<usize>().unwrap() <= 4);
        assert_eq!(arg(process, "-np"), "1");
        assert!(
            !process.args[0]
                .iter()
                .any(|a| a == "--fit" || a == "--fit-target")
        );
    }
    assert_eq!(arg(&local.chat, "-c"), "4096");
    assert_eq!(arg(&local.chat, "-b"), "128");
    assert_eq!(arg(&local.chat, "-ub"), "64");
    assert_eq!(arg(&local.embed, "--pooling"), "mean");
    assert_eq!(EMBED.size, 333_590_944);
    assert_eq!(CHAT.size, 6_180_446_208);
    assert_eq!(
        local.mobile.as_ref().unwrap().status()["idleUnloadSeconds"],
        240
    );
    assert!(!local.chat_available());
    assert!(!local.embed_available());
    assert!(!config.data_dir.join("models").exists());
    assert!(local.chat.child.lock().await.is_none());
    assert!(local.embed.child.lock().await.is_none());
    // Matching the filename alone cannot publish the pin.
    tokio::fs::create_dir(config.data_dir.join("models"))
        .await
        .unwrap();
    tokio::fs::write(
        &local.embed.model,
        b"invalid model with a trusted-looking filename",
    )
    .await
    .unwrap();
    assert_ne!(
        local.fingerprint().await,
        embedding_fingerprint(EMBED.sha256)
    );
    local.stop().await.unwrap();
}

async fn api_call(
    state: &AppState,
    endpoint: &str,
    method: &str,
    token: Option<&str>,
) -> (u16, Value) {
    let mut builder = HttpRequest::builder()
        .uri(endpoint)
        .method(method)
        .extension(ConnectInfo(
            "127.0.0.1:54321".parse::<std::net::SocketAddr>().unwrap(),
        ));
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let response = api::router(state.clone())
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let code = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (code, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn android_local_api_auth_activation_and_standby_are_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let state = AppState::new(Config::for_android(dir.path(), native.path()).unwrap())
        .await
        .unwrap();
    state.auth.register_android_host(TOKEN).await.unwrap();
    assert!(!dir.path().join("models").exists());
    for (path, method) in [
        ("/api/local/status", "GET"),
        ("/api/local/activate", "POST"),
        ("/api/local/standby", "POST"),
    ] {
        assert_eq!(api_call(&state, path, method, None).await.0, 401);
    }
    let (code, initial) = api_call(&state, "/api/local/status", "GET", Some(TOKEN)).await;
    assert_eq!(code, 200);
    assert_eq!(initial["activated"], false);
    assert_eq!(initial["chat"]["download"]["phase"], "pending");
    assert_eq!(initial["embed"]["download"]["total"], EMBED.size);
    let conversation = state
        .store
        .call(Request::CreateConversation(
            json!({"name":"Android pending fixture","presetId":"glm-52","routeId":"cbai/glm-5.2"}),
        ))
        .await
        .unwrap();
    let job = state
        .store
        .call(Request::EnqueueJob(
            json!({"convId":conversation["id"],"kind":"curate"}),
        ))
        .await
        .unwrap();
    state.runner.kick();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let runner = state.runner.state().await;
            if runner["running"] == false {
                assert_eq!(runner["policy"]["mode"], "deferred");
                assert!(runner["lastError"].is_null());
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    for _ in 0..2 {
        let (code, status) = api_call(&state, "/api/local/activate", "POST", Some(TOKEN)).await;
        assert_eq!(code, 202);
        assert_eq!(status["activated"], true);
        assert_eq!(status["chat"]["download"]["phase"], "error");
        assert_eq!(status["chat"]["running"], false);
    }
    assert!(
        !dir.path().join("models").exists(),
        "missing runtime must not download on this PC"
    );
    for _ in 0..2 {
        let (code, status) = api_call(&state, "/api/local/standby", "POST", Some(TOKEN)).await;
        assert_eq!(code, 200);
        assert_eq!(status["activated"], false);
    }
    let status = state.local.status().await;
    assert!(status["chat"]["ok"].is_boolean());
    assert!(status["embed"]["ok"].is_boolean());
    state.store.call(Request::Status).await.unwrap();
    state.shutdown().await;
    let db = crate::privacy::read_snapshot(&dir.path().join("modularchat.db.vault")).unwrap();
    let (status, attempts, error): (String, u32, Option<String>) = db
        .query_row(
            "SELECT status,attempts,error FROM jobs WHERE id=?",
            [job["id"].as_str().unwrap()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "pending");
    assert_eq!(attempts, 0);
    assert!(
        error.is_none(),
        "deferred must leave the pending job untouched"
    );
}

#[tokio::test]
async fn android_local_api_model_download_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let state = AppState::new(Config::for_android(dir.path(), native.path()).unwrap())
        .await
        .unwrap();
    state.auth.register_android_host(TOKEN).await.unwrap();

    // Test download chat model
    let (code, status) = api_call(&state, "/api/local/models/download?model=chat", "POST", Some(TOKEN)).await;
    assert_eq!(code, 202);
    assert!(status["chat"].is_object());

    // Test download embed model
    let (code, status) = api_call(&state, "/api/local/models/download?model=embed", "POST", Some(TOKEN)).await;
    assert_eq!(code, 202);
    assert!(status["embed"].is_object());

    // Test delete chat model via query
    let (code, status) = api_call(&state, "/api/local/models/delete?model=chat", "POST", Some(TOKEN)).await;
    assert_eq!(code, 200);
    assert!(status["chat"].is_object());

    // Test delete embed model via path
    let (code, status) = api_call(&state, "/api/local/models/embed", "DELETE", Some(TOKEN)).await;
    assert_eq!(code, 200);
    assert!(status["embed"].is_object());

    // Test delete invalid model returns 400
    let (code, _) = api_call(&state, "/api/local/models/delete?model=invalid", "POST", Some(TOKEN)).await;
    assert_eq!(code, 400);

    state.shutdown().await;
}

#[tokio::test]
async fn activate_does_not_automatically_download_models() {
    let dir = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    // Simulate runtime binary being present
    let fake_exe = native.path().join("libllama_server.so");
    tokio::fs::write(&fake_exe, b"fake runtime binary").await.unwrap();

    let state = AppState::new(Config::for_android(dir.path(), native.path()).unwrap())
        .await
        .unwrap();
    state.auth.register_android_host(TOKEN).await.unwrap();

    // Calling activate must NOT start provisioning/downloading missing models
    let (code, status) = api_call(&state, "/api/local/activate", "POST", Some(TOKEN)).await;
    assert_eq!(code, 202);
    assert_eq!(status["activated"], true);
    // Phase should remain "pending", NOT "downloading"
    assert_eq!(status["chat"]["download"]["phase"], "pending");
    assert_eq!(status["embed"]["download"]["phase"], "pending");
    assert_eq!(status["chat"]["download"]["downloaded"], 0);

    // Verify models directory has no downloaded files
    let models_dir = dir.path().join("models");
    if models_dir.exists() {
        let mut entries = tokio::fs::read_dir(&models_dir).await.unwrap();
        let mut count = 0;
        while let Some(_) = entries.next_entry().await.unwrap() {
            count += 1;
        }
        assert_eq!(count, 0, "No model files or parts should be created on activate");
    }

    state.shutdown().await;
}

#[tokio::test]
async fn regression_nemotron_35_bolt_pin_and_diagnostic() {
    assert_eq!(CHAT.repo, "nvidia/NVIDIA-Nemotron-3.5-Bolt-E4B-4GB");
    assert_eq!(CHAT.filename, "Nemotron-3.5-Bolt-E4B-Q4_K_M.gguf");
    assert_eq!(CHAT.revision, "4008514238a0b4ae17af3988818f75bf084541a5");
    assert_eq!(CHAT.size, 6_180_446_208);
    assert_eq!(
        CHAT.sha256,
        "5442bfb3b1384ff3774c9989e9457dad81bbb5830fbcf96adfdde40078831d5a"
    );
    assert_eq!(
        CHAT.url(),
        "https://huggingface.co/nvidia/NVIDIA-Nemotron-3.5-Bolt-E4B-4GB/resolve/4008514238a0b4ae17af3988818f75bf084541a5/Nemotron-3.5-Bolt-E4B-Q4_K_M.gguf"
    );

    let diagnostic = crate::providers::SelectedBackend::Local.diagnostic("mobile-local");
    assert_eq!(diagnostic["model"], "Nemotron-3.5-Bolt-E4B-4GB");
    assert_eq!(diagnostic["backend"], "local");
    assert_eq!(diagnostic["effectiveRoute"], "mobile-local");
}
