//! Embedded loopback host, C ABI v1 and direct Rust JNI exports.
//!
//! C declarations (all results are fixed-width signed 32-bit integers):
//! ```c
//! int32_t mc_android_start(const char *data_dir, const char *native_lib_dir, const char *local_token);
//! int32_t mc_android_stop(void);
//! int32_t mc_android_status(void);
//! int32_t mc_android_last_error(void);
//! ```
//! Start/stop return 0 when accepted, 1 when already in the requested state,
//! -1 for invalid input, -2 for a conflicting start/stopping host, -3 for thread
//! creation failure, -4 for an intercepted Rust panic. Start is asynchronous.
//! Status: 0 stopped, 1 initializing, 2 ready, 3 stopping, 4 failed. Last error
//! is a numeric internal error class (HTTP-style), zero on a successful start.
//! It exposes no token, path, conversation content or error text. No HTTP status
//! endpoint is added. Failed/finished threads are joined before any new start.
//!
//! Input strings are copied before returning. They must be valid readable,
//! NUL-terminated UTF-8 C strings (paths <=4096 bytes, token <=256 bytes).
//! Null pointers, missing terminators within the bounds, invalid UTF-8, relative
//! paths and malformed tokens are rejected. As with any pointer ABI, arbitrary
//! unreadable addresses cannot be validated by Rust; the caller must uphold the
//! memory contract. This module never sets process environment variables.
//!
//! The bridge must generate/persist a distinct app-local random token
//! (>=256 bits; hex/base64url), pass it on each start and keep it out of logs and
//! desktop synchronization. Only its hash is stored as a paired device. Identical
//! concurrent starts are idempotent; replacing a running host's inputs is refused.
//! Stop is asynchronous: it cancels serving and runs AppState::shutdown with a
//! bounded deadline. Keep the library loaded for the app process lifetime.
//! The SQLite worker remains the existing dedicated store thread;
//! the host owns exactly one thread with a current-thread Tokio runtime.

use crate::{
    Error, Result,
    api::{self, AppState},
    config::Config,
    rules,
};
use futures_util::FutureExt;
use std::{
    ffi::c_char,
    future::IntoFuture,
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicI32, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};
use tokio::sync::oneshot;
use zeroize::Zeroizing;

#[cfg(target_os = "android")]
#[path = "android_jni.rs"]
pub(crate) mod jni;

pub const STOPPED: i32 = 0;
pub const INITIALIZING: i32 = 1;
pub const READY: i32 = 2;
pub const STOPPING: i32 = 3;
pub const FAILED: i32 = 4;
const INVALID_INPUT: i32 = -1;
const CONFLICT: i32 = -2;
const SPAWN_FAILED: i32 = -3;
const PANIC: i32 = -4;

struct StartOptions {
    data_dir: PathBuf,
    native_lib_dir: PathBuf,
    local_token: Zeroizing<String>,
    #[cfg(test)]
    address: std::net::SocketAddr,
}
#[derive(PartialEq, Eq)]
struct Identity {
    data_dir: PathBuf,
    native_lib_dir: PathBuf,
    token_hash: String,
    #[cfg(test)]
    address: std::net::SocketAddr,
}
impl StartOptions {
    fn new(
        data_dir: PathBuf,
        native_lib_dir: PathBuf,
        local_token: Zeroizing<String>,
    ) -> Result<Self> {
        for path in [&data_dir, &native_lib_dir] {
            if !path.is_absolute()
                || path.file_name().is_none()
                || path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(Error::new(400, "Caminho local inválido."));
            }
        }
        if !crate::auth::valid_android_token(&local_token) {
            return Err(Error::new(400, "Token local inválido."));
        }
        Ok(Self {
            data_dir,
            native_lib_dir,
            local_token,
            #[cfg(test)]
            address: "127.0.0.1:3011".parse().unwrap(),
        })
    }
    fn identity(&self) -> Identity {
        Identity {
            data_dir: self.data_dir.clone(),
            native_lib_dir: self.native_lib_dir.clone(),
            token_hash: rules::hash(&self.local_token),
            #[cfg(test)]
            address: self.address,
        }
    }
}
struct Run {
    identity: Identity,
    stop: Option<oneshot::Sender<()>>,
    thread: JoinHandle<()>,
}
#[cfg(target_os = "android")]
pub(crate) fn log_android(prio: i32, msg: &str) {
    unsafe extern "C" {
        fn __android_log_write(
            prio: std::ffi::c_int,
            tag: *const std::ffi::c_char,
            text: *const std::ffi::c_char,
        ) -> std::ffi::c_int;
    }
    if let Ok(c_msg) = std::ffi::CString::new(msg) {
        unsafe {
            __android_log_write(prio, b"modularchat\0".as_ptr().cast(), c_msg.as_ptr());
        }
    }
}
#[cfg(not(target_os = "android"))]
pub(crate) fn log_android(_prio: i32, _msg: &str) {}

struct Host {
    state: AtomicI32,
    error: AtomicI32,
    run: Mutex<Option<Run>>,
    #[cfg(test)]
    port: std::sync::atomic::AtomicU16,
}
impl Host {
    fn new() -> Self {
        Self {
            state: AtomicI32::new(STOPPED),
            error: AtomicI32::new(0),
            run: Mutex::new(None),
            #[cfg(test)]
            port: std::sync::atomic::AtomicU16::new(0),
        }
    }
    fn status(&self) -> i32 {
        self.state.load(Ordering::Acquire)
    }
    fn observed_status(&self) -> i32 {
        if matches!(self.status(), STOPPED | FAILED) {
            let mut run = self.run.lock().unwrap_or_else(|p| p.into_inner());
            if matches!(self.status(), STOPPED | FAILED)
                && let Some(finished) = run.take()
                && finished.thread.join().is_err()
            {
                self.error.store(500, Ordering::Release);
                self.state.store(FAILED, Ordering::Release);
            }
        }
        self.status()
    }
    fn start(self: &Arc<Self>, options: StartOptions) -> i32 {
        let identity = options.identity();
        let mut run = self.run.lock().unwrap_or_else(|p| p.into_inner());
        match self.status() {
            INITIALIZING | READY => {
                return if run.as_ref().is_some_and(|r| r.identity == identity) {
                    1
                } else {
                    CONFLICT
                };
            }
            STOPPING => return CONFLICT,
            _ => {}
        }
        if let Some(previous) = run.take() {
            let _ = previous.thread.join();
        }
        self.error.store(0, Ordering::Release);
        self.state.store(INITIALIZING, Ordering::Release);
        let (stop_tx, stop_rx) = oneshot::channel();
        let host = self.clone();
        match std::thread::Builder::new()
            .name("mc-android-host".into())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| host.run_thread(options, stop_rx)));
                let (error, msg) = match result {
                    Ok(Ok(())) => (0, "run_thread finished successfully".to_string()),
                    Ok(Err(e)) => {
                        let text = format!("run_thread failed: status={}, message={}", e.status, e.message);
                        (i32::from(e.status), text)
                    }
                    Err(p) => {
                        let text = if let Some(s) = p.downcast_ref::<&str>() {
                            format!("run_thread panicked: {s}")
                        } else if let Some(s) = p.downcast_ref::<String>() {
                            format!("run_thread panicked: {s}")
                        } else {
                            "run_thread panicked with unknown payload".to_string()
                        };
                        (500, text)
                    }
                };
                log_android(if error == 0 { 4 } else { 6 }, &msg);
                host.error.store(error, Ordering::Release);
                host.state
                    .store(if error == 0 { STOPPED } else { FAILED }, Ordering::Release);
            }) {
            Ok(thread) => {
                *run = Some(Run {
                    identity,
                    stop: Some(stop_tx),
                    thread,
                });
                0
            }
            Err(_) => {
                log_android(6, "Failed to spawn mc-android-host thread");
                self.error.store(500, Ordering::Release);
                self.state.store(FAILED, Ordering::Release);
                SPAWN_FAILED
            }
        }
    }
    fn stop(&self) -> i32 {
        let mut run = self.run.lock().unwrap_or_else(|p| p.into_inner());
        match self.status() {
            STOPPED | FAILED | STOPPING => 1,
            _ => {
                self.state.store(STOPPING, Ordering::Release);
                if let Some(sender) = run.as_mut().and_then(|r| r.stop.take()) {
                    let _ = sender.send(());
                }
                0
            }
        }
    }
    fn run_thread(&self, options: StartOptions, stop: oneshot::Receiver<()>) -> Result<()> {
        log_android(4, "run_thread: entering");
        let config = Config::for_android(&options.data_dir, &options.native_lib_dir)?;
        log_android(4, "run_thread: Config::for_android ok");
        #[cfg(test)]
        let config = Config {
            address: options.address,
            ..config
        };
        // Only tests may override the fixed production loopback address.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        log_android(4, "run_thread: tokio runtime built, entering block_on serve");
        let result = runtime.block_on(self.serve(config, options.local_token, stop));
        log_android(4, "run_thread: serve returned");
        runtime.shutdown_timeout(Duration::from_secs(5));
        result
    }
    async fn serve(
        &self,
        config: Config,
        local_token: Zeroizing<String>,
        mut stop: oneshot::Receiver<()>,
    ) -> Result<()> {
        log_android(4, &format!("serve: binding to {}", config.address));
        // Bind first: a port conflict must not initialize or modify a vault.
        let listener = tokio::net::TcpListener::bind(config.address).await?;
        log_android(4, "serve: listener bound successfully");
        #[cfg(test)]
        self.port
            .store(listener.local_addr()?.port(), Ordering::Release);
        log_android(4, "serve: creating AppState");
        let app = AppState::new(config).await?;
        log_android(4, "serve: AppState created successfully");
        let served = AssertUnwindSafe(async {
            app.auth.register_android_host(&local_token).await?;
            drop(local_token);
            let _ = self.state.compare_exchange(
                INITIALIZING,
                READY,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            let server = axum::serve(
                listener,
                api::router(app.clone())
                    .into_make_service_with_connect_info::<std::net::SocketAddr>(),
            );
            // As in the desktop bootstrap, cancel persistent SSE connections on shutdown.
            tokio::select! {
                biased;
                _ = &mut stop => Ok(()),
                result = server.into_future() => result.map_err(Error::from),
            }
        })
        .catch_unwind()
        .await
        .unwrap_or_else(|_| Err(Error::new(500, "Falha interna do host local.")));
        let shutdown = tokio::time::timeout(Duration::from_secs(40), app.shutdown())
            .await
            .map_err(|_| Error::new(504, "Encerramento local excedeu o prazo."));
        served.and(shutdown)
    }
}
fn host() -> &'static Arc<Host> {
    static HOST: OnceLock<Arc<Host>> = OnceLock::new();
    HOST.get_or_init(|| Arc::new(Host::new()))
}
fn ffi_result(f: impl FnOnce() -> i32) -> i32 {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(PANIC)
}
/// Caller must provide a readable C string until NUL or `max + 1` bytes, whichever comes first.
unsafe fn c_string(ptr: *const c_char, max: usize) -> Result<Zeroizing<String>> {
    if ptr.is_null() {
        return Err(Error::new(400, "String local ausente."));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    for index in 0..=max {
        // SAFETY: the caller upholds the readable, terminated string contract documented above.
        let byte = unsafe { *ptr.add(index) } as u8;
        if byte == 0 {
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| Error::new(400, "String local não é UTF-8."))?;
            return Ok(Zeroizing::new(text.to_owned()));
        }
        bytes.push(byte);
    }
    Err(Error::new(400, "String local excedeu o limite."))
}
/// Schedule one Android host. See the module-level ABI contract for results and limits.
///
/// # Safety
/// Each non-null pointer must remain readable until its NUL terminator (or the documented
/// maximum plus one byte) for the duration of the call. Input buffers remain caller-owned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mc_android_start(
    data_dir: *const c_char,
    native_lib_dir: *const c_char,
    local_token: *const c_char,
) -> i32 {
    ffi_result(|| {
        let options = (|| {
            // SAFETY: forwarded from this function's C ABI memory contract.
            let data = unsafe { c_string(data_dir, 4096)? };
            let native = unsafe { c_string(native_lib_dir, 4096)? };
            let token = unsafe { c_string(local_token, 256)? };
            StartOptions::new(
                PathBuf::from(data.as_str()),
                PathBuf::from(native.as_str()),
                token,
            )
        })();
        match options {
            Ok(options) => host().start(options),
            Err(_) => INVALID_INPUT,
        }
    })
}
/// Schedule shutdown/flush; keep the native library loaded for the app process lifetime.
#[unsafe(no_mangle)]
pub extern "C" fn mc_android_stop() -> i32 {
    ffi_result(|| host().stop())
}
/// Numeric lifecycle only; never includes secrets or runtime paths.
#[unsafe(no_mangle)]
pub extern "C" fn mc_android_status() -> i32 {
    ffi_result(|| host().observed_status())
}
/// Numeric failure class, zero after a successful new start.
#[unsafe(no_mangle)]
pub extern "C" fn mc_android_last_error() -> i32 {
    ffi_result(|| host().error.load(Ordering::Acquire))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    const TOKEN: &str = "android_fixture_0123456789abcdef0123456789abcdef0123456789abcdef";
    fn options(dir: &std::path::Path) -> StartOptions {
        let mut options =
            StartOptions::new(dir.into(), dir.into(), Zeroizing::new(TOKEN.into())).unwrap();
        options.address = "127.0.0.1:0".parse().unwrap();
        options
    }
    async fn wait(host: &Host, expected: i32) {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let status = host.observed_status();
                assert!(
                    status != FAILED || expected == FAILED,
                    "host error {}",
                    host.error.load(Ordering::Acquire)
                );
                if status == expected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    struct StopOnDrop(Arc<Host>);
    impl Drop for StopOnDrop {
        fn drop(&mut self) {
            self.0.stop();
        }
    }

    #[test]
    fn android_abi_rejects_null_invalid_utf8_and_unterminated_strings_and_contains_panics() {
        assert_eq!(
            unsafe { mc_android_start(std::ptr::null(), std::ptr::null(), std::ptr::null()) },
            INVALID_INPUT
        );
        let invalid = [0xffu8, 0];
        assert!(unsafe { c_string(invalid.as_ptr().cast(), 1) }.is_err());
        let unterminated = [b'x'; 258];
        assert!(unsafe { c_string(unterminated.as_ptr().cast(), 256) }.is_err());
        assert!(
            StartOptions::new(
                "relative".into(),
                "relative".into(),
                Zeroizing::new(TOKEN.into())
            )
            .is_err()
        );
        assert_eq!(ffi_result(|| panic!("test ABI panic")), PANIC);
    }
    #[tokio::test]
    async fn android_concurrent_start_is_idempotent_auth_is_required_and_vault_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new());
        let _cleanup = StopOnDrop(host.clone());
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let mut codes = std::thread::scope(|s| {
            let start = || {
                barrier.wait();
                host.start(options(dir.path()))
            };
            let a = s.spawn(start);
            let b = s.spawn(start);
            vec![a.join().unwrap(), b.join().unwrap()]
        });
        codes.sort();
        assert_eq!(codes, [0, 1]);
        wait(&host, READY).await;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let base = format!("http://127.0.0.1:{}", host.port.load(Ordering::Acquire));
        assert_eq!(
            client
                .get(format!("{base}/api/chats"))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            client
                .get(format!("{base}/api/privacy/cache-key"))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            client
                .get(format!("{base}/api/chats"))
                .bearer_auth("desktop_fixture_other_token_0123456789")
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        let response = client
            .get(format!("{base}/api/chats"))
            .bearer_auth(TOKEN)
            .header("origin", "capacitor://localhost")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()["access-control-allow-origin"],
            "capacitor://localhost"
        );
        let health = client
            .get(format!("{base}/api/health"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(!health.contains(TOKEN));
        assert!(!health.contains("android_status"));
        let created: Value = client.post(format!("{base}/api/chats")).bearer_auth(TOKEN)
            .json(&json!({"name":"native host fixture","prompt":"PRIVATE_ANDROID_CARD","persona":"Alex"}))
            .send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
        let mut changed = options(dir.path());
        changed.local_token = Zeroizing::new("different_native_token_0123456789abcdef".into());
        assert_eq!(host.start(changed), CONFLICT);
        assert_eq!(host.stop(), 0);
        wait(&host, STOPPED).await;
        assert_eq!(host.stop(), 1);
        assert!(!dir.path().join("modularchat.db").exists());
        assert!(dir.path().join("modularchat.db.vault").is_file());
        assert_eq!(host.start(options(dir.path())), 0);
        wait(&host, READY).await;
        let base = format!("http://127.0.0.1:{}", host.port.load(Ordering::Acquire));
        let chat: Value = client
            .get(format!(
                "{base}/api/chats/{}",
                created["id"].as_str().unwrap()
            ))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(chat["prompt"], "PRIVATE_ANDROID_CARD");
        let devices: Value = client
            .get(format!("{base}/api/devices"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(devices["devices"].as_array().unwrap().len(), 1);
        host.stop();
        wait(&host, STOPPED).await;
        let db = crate::privacy::read_snapshot(&dir.path().join("modularchat.db.vault")).unwrap();
        db.execute("UPDATE devices SET revoked_at='2026-01-01T00:00:00Z'", []).unwrap();
        drop(db);
        assert_eq!(host.start(options(dir.path())), 0);
        wait(&host, READY).await;
        host.stop();
        wait(&host, STOPPED).await;
        let db = crate::privacy::read_snapshot(&dir.path().join("modularchat.db.vault")).unwrap();
        let stored_hash: String = db
            .query_row("SELECT token_hash FROM devices WHERE revoked_at IS NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored_hash, rules::hash(TOKEN));
        assert_ne!(stored_hash, TOKEN);
    }
    #[tokio::test]
    async fn android_failed_bind_is_observable_and_can_restart_without_touching_vault() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new());
        let _cleanup = StopOnDrop(host.clone());
        let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut input = options(dir.path());
        input.address = blocker.local_addr().unwrap();
        assert_eq!(host.start(input), 0);
        wait(&host, FAILED).await;
        assert_ne!(host.error.load(Ordering::Acquire), 0);
        assert!(!dir.path().join("modularchat.db.vault").exists());
        drop(blocker);
        assert_eq!(host.start(options(dir.path())), 0);
        wait(&host, READY).await;
        host.stop();
        wait(&host, STOPPED).await;
        assert_eq!(host.error.load(Ordering::Acquire), 0);
    }
    #[tokio::test]
    async fn android_stop_during_initialization_finishes_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(Host::new());
        let _cleanup = StopOnDrop(host.clone());
        assert_eq!(host.start(options(dir.path())), 0);
        assert_eq!(host.stop(), 0);
        wait(&host, STOPPED).await;
        let (vault, db) = crate::privacy::Vault::open(dir.path()).unwrap();
        assert!(
            db.query_row("SELECT count(*) FROM devices", [], |r| r.get::<_, i64>(0))
                .unwrap()
                <= 1
        );
        drop(db);
        drop(vault);
    }
}
