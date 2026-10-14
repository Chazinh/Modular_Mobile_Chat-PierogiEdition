//! Immutable pins and private, verified files. No provider credentials or mutable manifest.
use crate::{Error, Result};
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, SystemTime},
};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, watch},
};

#[derive(Clone, Copy)]
pub(super) struct Pin {
    pub repo: &'static str,
    pub revision: &'static str,
    pub filename: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}
impl Pin {
    fn url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repo, self.revision, self.filename
        )
    }
}
pub(super) const EMBED: Pin = Pin {
    repo: "ggml-org/embeddinggemma-300M-GGUF",
    revision: "0f741b5a6585bd53aeb15cd1372c56f2a0f65e12",
    filename: "embeddinggemma-300M-Q8_0.gguf",
    size: 333_590_944,
    sha256: "b5ce9d77a3fc4b3b39ccb5643c36777911cc4eb46a66962eadfa3f5f60490d63",
};
pub(super) const CHAT: Pin = Pin {
    repo: "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
    revision: "91cad51170dc346986eccefdc2dd33a9da36ead9",
    filename: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
    size: 1_117_320_736,
    sha256: "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
};

pub(super) fn unavailable(message: impl Into<String>) -> Error {
    Error::new(503, message).extra("code", "LOCAL_UNAVAILABLE")
}
fn disk_error(error: std::io::Error) -> Error {
    unavailable(format!("Arquivo do modelo: {:?}.", error.kind()))
}

#[derive(Clone)]
pub(super) struct Cancel(watch::Sender<bool>);
impl Cancel {
    pub fn new() -> Self {
        Self(watch::channel(false).0)
    }
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
    pub async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        let _ = receiver.wait_for(|value| *value).await;
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}
impl Stamp {
    fn read(path: &Path) -> std::io::Result<Option<Self>> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a regular file",
            ));
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        }))
    }
}
struct CachedDigest {
    path: PathBuf,
    stamp: Stamp,
    sha256: String,
}
#[derive(Default)]
pub(super) struct DigestCache {
    cached: StdMutex<Option<CachedDigest>>,
    hashing: Mutex<()>,
    #[cfg(test)]
    hashes: std::sync::atomic::AtomicUsize,
}
impl DigestCache {
    pub fn matches(&self, path: &Path, sha256: &str) -> bool {
        self.cached.lock().unwrap().as_ref().is_some_and(|cached| {
            cached.path == path
                && cached.sha256 == sha256
                && Stamp::read(path).ok().flatten().as_ref() == Some(&cached.stamp)
        })
    }
    pub async fn digest(&self, path: &Path, size: u64, cancel: &Cancel) -> Result<Option<String>> {
        let _lock = tokio::select! {
            lock = self.hashing.lock() => lock,
            _ = cancel.cancelled() => return Err(unavailable("Validação cancelada.")),
        };
        let stamp = match Stamp::read(path).map_err(disk_error)? {
            Some(stamp) if stamp.len == size => stamp,
            _ => return Ok(None),
        };
        if let Some(cached) = self.cached.lock().unwrap().as_ref()
            && cached.path == path
            && cached.stamp == stamp
        {
            return Ok(Some(cached.sha256.clone()));
        }
        let path_owned = path.to_owned();
        let before = stamp.clone();
        let cancellation = cancel.clone();
        #[cfg(test)]
        self.hashes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let digest = tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let mut file = std::fs::File::open(&path_owned).map_err(disk_error)?;
            let mut hash = Sha256::new();
            let mut buffer = vec![0u8; 256 * 1024];
            loop {
                if cancellation.is_cancelled() {
                    return Err(unavailable("Validação cancelada."));
                }
                let read = file.read(&mut buffer).map_err(disk_error)?;
                if read == 0 {
                    break;
                }
                hash.update(&buffer[..read]);
            }
            if Stamp::read(&path_owned).map_err(disk_error)?.as_ref() != Some(&before) {
                return Err(unavailable("Arquivo do modelo mudou durante a validação."));
            }
            Ok(format!("{:x}", hash.finalize()))
        })
        .await
        .map_err(|_| unavailable("Validação do modelo interrompida."))??;
        *self.cached.lock().unwrap() = Some(CachedDigest {
            path: path.to_owned(),
            stamp,
            sha256: digest.clone(),
        });
        Ok(Some(digest))
    }
    fn remember(&self, path: &Path, sha256: &str) -> Result<()> {
        let stamp = Stamp::read(path)
            .map_err(disk_error)?
            .ok_or_else(|| unavailable("Modelo ausente após download."))?;
        *self.cached.lock().unwrap() = Some(CachedDigest {
            path: path.to_owned(),
            stamp,
            sha256: sha256.into(),
        });
        Ok(())
    }
}

#[derive(Clone, Serialize)]
pub(super) struct Progress {
    pub phase: &'static str,
    pub downloaded: u64,
    pub total: u64,
    pub error: Option<String>,
}
pub(super) struct ModelFile {
    pub pin: Pin,
    pub path: PathBuf,
    pub digest: Arc<DigestCache>,
    root: PathBuf,
    url: String,
    client: reqwest::Client,
    gate: Mutex<()>,
    progress: StdMutex<Progress>,
}
impl ModelFile {
    pub fn new(root: PathBuf, pin: Pin, client: reqwest::Client, digest: Arc<DigestCache>) -> Self {
        // Runtime pins are compiled in. Even test pins cannot introduce a relative path.
        assert!(
            !pin.filename.is_empty()
                && !pin.filename.contains(['/', '\\'])
                && !matches!(pin.filename, "." | "..")
        );
        Self {
            path: root.join(pin.filename),
            root,
            pin,
            digest,
            url: pin.url(),
            client,
            gate: Mutex::new(()),
            progress: StdMutex::new(Progress {
                phase: "pending",
                downloaded: 0,
                total: pin.size,
                error: None,
            }),
        }
    }
    pub fn ready(&self) -> bool {
        self.digest.matches(&self.path, self.pin.sha256)
    }
    #[cfg(test)]
    pub fn fixture(root: PathBuf, pin: Pin, url: String) -> Self {
        let mut file = Self::new(
            root,
            pin,
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .read_timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            Arc::new(DigestCache::default()),
        );
        file.url = url;
        file
    }
    pub fn status(&self) -> Progress {
        self.progress.lock().unwrap().clone()
    }
    fn progress(&self, phase: &'static str, downloaded: u64, error: Option<String>) {
        *self.progress.lock().unwrap() = Progress {
            phase,
            downloaded,
            total: self.pin.size,
            error,
        };
    }
    pub fn fail(&self, message: &str) {
        self.progress("error", self.status().downloaded, Some(message.into()));
    }
    pub async fn provision(&self, cancel: &Cancel) -> Result<()> {
        let _gate = tokio::select! {
            lock = self.gate.lock() => lock,
            _ = cancel.cancelled() => return Err(unavailable("Download cancelado.")),
        };
        if cancel.is_cancelled() {
            return Err(unavailable("Download cancelado."));
        }
        let outcome = tokio::select! {
            outcome = self.prepare_and_download(cancel) => outcome,
            _ = cancel.cancelled() => Err(unavailable("Download cancelado.")),
        };
        if let Err(error) = &outcome {
            // Only unlink a task-owned .part beneath a real models directory; never follow a link.
            if tokio::fs::symlink_metadata(&self.root)
                .await
                .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
            {
                let _ = tokio::fs::remove_file(self.part()).await;
            }
            self.progress(
                if cancel.is_cancelled() {
                    "cancelled"
                } else {
                    "error"
                },
                self.status().downloaded,
                Some(error.message.clone()),
            );
        }
        outcome
    }
    fn part(&self) -> PathBuf {
        self.root.join(format!("{}.part", self.pin.filename))
    }
    async fn prepare_and_download(&self, cancel: &Cancel) -> Result<()> {
        match tokio::fs::create_dir(&self.root).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(disk_error(error)),
        }
        let metadata = tokio::fs::symlink_metadata(&self.root)
            .await
            .map_err(disk_error)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(unavailable("Diretório privado dos modelos inválido."));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))
                .await
                .map_err(disk_error)?;
        }
        self.progress("verifying", 0, None);
        if self
            .digest
            .digest(&self.path, self.pin.size, cancel)
            .await?
            .as_deref()
            == Some(self.pin.sha256)
        {
            // A stale .part is never a resume source or a model, even when the final file is valid.
            let _ = tokio::fs::remove_file(self.part()).await;
            self.progress("ready", self.pin.size, None);
            return Ok(());
        }
        let part = self.part();
        match tokio::fs::remove_file(&part).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(disk_error(error)),
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&part).await.map_err(disk_error)?;
        self.progress("downloading", 0, None);
        let response = self
            .client
            .get(&self.url)
            .send()
            .await
            .map_err(|_| unavailable("Conexão HTTPS do modelo falhou."))?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(unavailable(format!(
                "Download do modelo: HTTP {}.",
                response.status().as_u16()
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length != self.pin.size)
        {
            return Err(unavailable("Tamanho HTTP do modelo não confere."));
        }
        let mut stream = response.bytes_stream();
        let mut hash = Sha256::new();
        let mut downloaded = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| unavailable("Download do modelo interrompido."))?;
            downloaded = downloaded
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| unavailable("Tamanho do modelo inválido."))?;
            if downloaded > self.pin.size {
                return Err(unavailable("Modelo maior que o tamanho pinado."));
            }
            file.write_all(&chunk).await.map_err(disk_error)?;
            hash.update(&chunk);
            self.progress("downloading", downloaded, None);
        }
        self.progress("verifying", downloaded, None);
        if downloaded != self.pin.size {
            return Err(unavailable("Download do modelo truncado."));
        }
        if format!("{:x}", hash.finalize()) != self.pin.sha256 {
            return Err(unavailable("SHA-256 do modelo não confere."));
        }
        file.flush().await.map_err(disk_error)?;
        file.sync_all().await.map_err(disk_error)?;
        drop(file);
        if cancel.is_cancelled() {
            return Err(unavailable("Download cancelado."));
        }
        // Windows cannot replace an existing destination. Only discard a previously invalid regular file.
        if Stamp::read(&self.path).map_err(disk_error)?.is_some() {
            tokio::fs::remove_file(&self.path)
                .await
                .map_err(disk_error)?;
        }
        tokio::fs::rename(&part, &self.path)
            .await
            .map_err(disk_error)?;
        #[cfg(unix)]
        {
            let root = self.root.clone();
            tokio::task::spawn_blocking(move || std::fs::File::open(root)?.sync_all())
                .await
                .map_err(|_| unavailable("Persistência do modelo interrompida."))?
                .map_err(disk_error)?;
        }
        self.digest.remember(&self.path, self.pin.sha256)?;
        self.progress("ready", downloaded, None);
        Ok(())
    }
}

#[cfg(test)]
mod tests;

pub(super) fn download_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" {
                attempt.error("HTTPS required")
            } else if attempt.previous().len() >= 5 {
                attempt.error("redirect limit")
            } else {
                attempt.follow()
            }
        }))
        .referer(false)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|_| unavailable("Cliente HTTPS de modelos indisponível."))
}
