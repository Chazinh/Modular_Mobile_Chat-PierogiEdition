use super::*;
use axum::{Router, body::Body, http::StatusCode, response::Response, routing::get};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{sync::Notify, task::JoinHandle};

struct HttpFixture {
    url: String,
    requests: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}
impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn fixture(
    bytes: Vec<u8>,
    status: StatusCode,
    gate: Option<Arc<Notify>>,
    length: Option<u64>,
) -> HttpFixture {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/model", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let calls = requests.clone();
    let router = Router::new().route(
        "/model",
        get(move || {
            let calls = calls.clone();
            let bytes = bytes.clone();
            let gate = gate.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let stream = futures_util::stream::unfold(
                    (0, bytes, gate),
                    |(index, bytes, gate)| async move {
                        let mid = bytes.len() / 2;
                        let chunk = match index {
                            0 => bytes[..mid].to_vec(),
                            1 => {
                                if let Some(gate) = &gate {
                                    gate.notified().await;
                                }
                                bytes[mid..].to_vec()
                            }
                            _ => return None,
                        };
                        Some((Ok::<_, std::io::Error>(chunk), (index + 1, bytes, gate)))
                    },
                );
                let mut response = Response::builder().status(status);
                if let Some(length) = length {
                    response = response.header("content-length", length);
                }
                response.body(Body::from_stream(stream)).unwrap()
            }
        }),
    );
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    HttpFixture {
        url,
        requests,
        task,
    }
}
fn pin(bytes: &[u8]) -> Pin {
    Pin {
        repo: "fixture",
        revision: "immutable",
        filename: "fixture.gguf",
        size: bytes.len() as u64,
        sha256: Box::leak(format!("{:x}", Sha256::digest(bytes)).into_boxed_str()),
    }
}
async fn wait_for_progress(file: &ModelFile) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let progress = file.status();
            if progress.phase == "downloading" && progress.downloaded > 0 {
                break;
            }
            assert_ne!(progress.phase, "error", "{:?}", progress.error);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn progress_atomic_publish_dedup_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = vec![42u8; 128 * 1024];
    let pin = pin(&bytes);
    let gate = Arc::new(Notify::new());
    let server = fixture(bytes.clone(), StatusCode::OK, Some(gate.clone()), None).await;
    let root = dir.path().join("models");
    let file = Arc::new(ModelFile::fixture(root.clone(), pin, server.url.clone()));
    let first = file.clone();
    let a = tokio::spawn(async move { first.provision(&Cancel::new()).await });
    let second = file.clone();
    let b = tokio::spawn(async move { second.provision(&Cancel::new()).await });
    wait_for_progress(&file).await;
    let progress = file.status();
    assert!(progress.downloaded < progress.total);
    assert_eq!(progress.total, pin.size);
    assert!(!file.path.exists());
    assert!(!file.ready());
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    gate.notify_one();
    a.await.unwrap().unwrap();
    b.await.unwrap().unwrap();
    assert!(file.ready());
    assert!(!file.part().exists());
    assert_eq!(tokio::fs::read(&file.path).await.unwrap(), bytes);
    assert_eq!(file.status().phase, "ready");
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    // Reopening does not trust metadata from a previous instance. It hashes the file once.
    let reopened = ModelFile::fixture(root, pin, server.url.clone());
    assert!(!reopened.ready());
    reopened.provision(&Cancel::new()).await.unwrap();
    reopened.provision(&Cancel::new()).await.unwrap();
    assert_eq!(reopened.digest.hashes.load(Ordering::Relaxed), 1);
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn corrupt_or_truncated_existing_files_are_replaced_without_resuming() {
    let good = b"verified compact fixture".to_vec();
    let pin = pin(&good);
    let server = fixture(good.clone(), StatusCode::OK, None, None).await;
    for bad in [b"truncated".to_vec(), vec![b'?'; good.len()]] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("models");
        tokio::fs::create_dir(&root).await.unwrap();
        tokio::fs::write(root.join(pin.filename), bad)
            .await
            .unwrap();
        tokio::fs::write(root.join(format!("{}.part", pin.filename)), b"old partial")
            .await
            .unwrap();
        let file = ModelFile::fixture(root, pin, server.url.clone());
        file.provision(&Cancel::new()).await.unwrap();
        assert_eq!(tokio::fs::read(&file.path).await.unwrap(), good);
        assert!(file.ready());
        assert!(!file.part().exists());
    }
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn hash_size_http_and_disk_failures_never_publish_a_model() {
    let bytes = b"integrity fixture".to_vec();
    let pin = pin(&bytes);
    let cases = [
        (vec![b'!'; bytes.len()], StatusCode::OK, None, "SHA-256"),
        (b"short".to_vec(), StatusCode::OK, None, "truncado"),
        (vec![1; bytes.len() + 2], StatusCode::OK, None, "maior"),
        (
            b"private API text that must not be exposed".to_vec(),
            StatusCode::FORBIDDEN,
            None,
            "HTTP 403",
        ),
        (
            vec![1; bytes.len() + 3],
            StatusCode::OK,
            Some(pin.size + 3),
            "Tamanho HTTP",
        ),
        (bytes.clone(), StatusCode::FOUND, None, "HTTP 302"),
    ];
    for (body, status, length, expected) in cases {
        let dir = tempfile::tempdir().unwrap();
        let server = fixture(body, status, None, length).await;
        let file = ModelFile::fixture(dir.path().join("models"), pin, server.url.clone());
        let error = file.provision(&Cancel::new()).await.unwrap_err();
        assert!(
            error.message.contains(expected),
            "caso {expected}: {}",
            error.message
        );
        assert!(!file.path.exists());
        assert!(!file.part().exists());
        assert!(!file.ready());
        assert_eq!(file.status().phase, "error");
        assert!(!file.status().error.unwrap().contains("private API"));
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("models");
    tokio::fs::write(&root, b"directory collision")
        .await
        .unwrap();
    let file = ModelFile::fixture(root.clone(), pin, "http://127.0.0.1:1/unused".into());
    assert!(file.provision(&Cancel::new()).await.is_err());
    assert_eq!(tokio::fs::read(root).await.unwrap(), b"directory collision");
}

#[tokio::test]
async fn cancelled_download_is_cleaned_and_explicit_retry_starts_from_zero() {
    let bytes = vec![7; 128 * 1024];
    let pin = pin(&bytes);
    let dir = tempfile::tempdir().unwrap();
    let gate = Arc::new(Notify::new());
    let server = fixture(bytes, StatusCode::OK, Some(gate.clone()), None).await;
    let file = Arc::new(ModelFile::fixture(
        dir.path().join("models"),
        pin,
        server.url.clone(),
    ));
    let cancel = Cancel::new();
    let task_file = file.clone();
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move { task_file.provision(&task_cancel).await });
    wait_for_progress(&file).await;
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(file.status().phase, "cancelled");
    assert!(!file.path.exists());
    assert!(!file.part().exists());
    gate.notify_waiters();
    gate.notify_one();
    file.provision(&Cancel::new()).await.unwrap();
    assert!(file.ready());
    assert_eq!(file.status().downloaded, pin.size);
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn digest_identity_is_portable_cached_and_invalidates_on_changes() {
    let bytes = b"same verified content across directories";
    let pin = pin(bytes);
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let paths = [
        a.path().join("embeddinggemma.gguf"),
        b.path().join("renamed.gguf"),
    ];
    let cache = DigestCache::default();
    let mut fingerprints = Vec::new();
    for path in &paths {
        tokio::fs::write(path, bytes).await.unwrap();
        let digest = cache
            .digest(path, pin.size, &Cancel::new())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(digest, pin.sha256);
        assert!(cache.matches(path, pin.sha256));
        fingerprints.push(super::super::embedding_fingerprint(&digest));
    }
    assert_eq!(fingerprints[0], fingerprints[1]);
    println!("portable embedding fingerprint: {}", fingerprints[0]);
    cache
        .digest(&paths[1], pin.size, &Cancel::new())
        .await
        .unwrap();
    assert_eq!(cache.hashes.load(Ordering::Relaxed), 2);
    tokio::time::sleep(Duration::from_millis(3)).await;
    tokio::fs::write(&paths[1], vec![b'?'; bytes.len()])
        .await
        .unwrap();
    assert!(!cache.matches(&paths[1], pin.sha256));
    assert_ne!(
        cache
            .digest(&paths[1], pin.size, &Cancel::new())
            .await
            .unwrap()
            .unwrap(),
        pin.sha256
    );
    assert_eq!(cache.hashes.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn production_downloader_refuses_plain_http_without_a_request() {
    let server = fixture(b"unused".to_vec(), StatusCode::OK, None, None).await;
    assert!(
        download_client()
            .unwrap()
            .get(&server.url)
            .send()
            .await
            .is_err()
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn sandbox_rejects_symlinked_models_directory_or_final_file() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = dir.path().join("models");
    std::os::unix::fs::symlink(outside.path(), &root).unwrap();
    let file = ModelFile::fixture(
        root.clone(),
        pin(b"fixture"),
        "http://127.0.0.1:1/unused".into(),
    );
    assert!(file.provision(&Cancel::new()).await.is_err());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    std::fs::remove_file(&root).unwrap();
    std::fs::create_dir(&root).unwrap();
    let external = outside.path().join("preserve");
    std::fs::write(&external, b"fixture").unwrap();
    std::os::unix::fs::symlink(&external, &file.path).unwrap();
    assert!(file.provision(&Cancel::new()).await.is_err());
    assert_eq!(std::fs::read(external).unwrap(), b"fixture");
}
