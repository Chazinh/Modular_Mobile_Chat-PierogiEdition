//! Owns only children started by this server, including the compact mobile conversation backend.
mod mobile;
mod model_files;
use crate::{Error, Result, config::Config, rules};
use mobile::Mobile;
use model_files::{CHAT, Cancel, DigestCache, EMBED, ModelFile};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::{
    process::{Child, Command},
    sync::Mutex,
};

/// The local models live in the same Windows "job" as the server: if the server is killed (closing the console,
/// Stop-Process, restarting through start-server.bat), Windows ends the llama-server processes too. Without this they were left
/// orphaned, held ports 20481/20482 and the new server (which does not adopt foreign processes) ran without a local model, with the
/// memory queue stalled. Best effort: a failure here never prevents the model from starting.
#[cfg(windows)]
fn tie_to_server(child: &Child) {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    static JOB: OnceLock<usize> = OnceLock::new();
    // SAFETY: null names/security attributes request an unnamed job with default security.
    // The zeroed Win32 information struct is valid, and its pointer/size match the API.
    // Only a successfully configured handle is retained for the server's lifetime.
    let job = *JOB.get_or_init(|| unsafe {
        let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if handle.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            std::ptr::addr_of!(info).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0 {
            CloseHandle(handle);
            return 0;
        }
        handle as usize
    });
    if job != 0
        && let Some(raw) = child.raw_handle()
    {
        // SAFETY: JOB contains a live configured job handle; raw_handle borrows the
        // live Child's process handle. Neither handle is closed by this call.
        unsafe {
            AssignProcessToJobObject(job as _, raw as _);
        }
    }
}

struct Process {
    name: String,
    exe: PathBuf,
    model: PathBuf,
    port: u16,
    args: Vec<Vec<String>>,
    idle: u64,
    child: Mutex<Option<Child>>,
    enabled: bool,
    client: reqwest::Client,
    startup_timeout: Duration,
    mobile_file: Option<Arc<ModelFile>>,
    #[cfg(test)]
    fixture_command: Option<Vec<String>>,
}
#[derive(Clone)]
pub struct LocalModels {
    chat: Arc<Process>,
    embed: Arc<Process>,
    mock: Option<String>,
    client: reqwest::Client,
    /// Per-request deadline of the memory model (LOCAL_CHAT_TIMEOUT_S, default 600 s: the 9B partly runs on the CPU).
    chat_timeout: Duration,
    mobile: Option<Arc<Mobile>>,
    embed_digest: Arc<DigestCache>,
}
/// Keeps the mobile inference permit through the complete SSE stream and any style repair.
pub(crate) struct ChatSession {
    local: LocalModels,
    _operation: mobile::Operation,
}
impl ChatSession {
    pub(crate) async fn send(&self, body: &Value, timeout: Duration) -> Result<reqwest::Response> {
        let mut body = body.clone();
        let messages = body["messages"]
            .as_array()
            .cloned()
            .ok_or_else(|| Error::new(400, "Mensagens locais inválidas."))?;
        let requested = body["max_tokens"].as_u64().unwrap_or(1024).min(1024);
        let budget = self
            .local
            .mobile_budget(&messages, requested, requested as usize)
            .await?;
        let object = body.as_object_mut().unwrap();
        for key in ["reasoning_effort", "reasoning", "thinking", "output_config"] {
            object.remove(key);
        }
        body["model"] = json!("mobile-local");
        body["max_tokens"] = json!(budget);
        body["chat_template_kwargs"] = json!({"enable_thinking":false});
        let response = self
            .local
            .client
            .post(format!(
                "http://127.0.0.1:{}/v1/chat/completions",
                self.local.chat.port
            ))
            .timeout(timeout)
            .json(&body)
            .send()
            .await
            .map_err(|_| {
                Error::new(503, "Conversa local indisponível.").extra("code", "LOCAL_UNAVAILABLE")
            })?;
        if !response.status().is_success() {
            return Err(Error::new(502, "Modelo compacto recusou a requisição."));
        }
        Ok(response)
    }
}
fn env_path(name: &str, default: PathBuf) -> PathBuf {
    std::env::var_os(name).map(PathBuf::from).unwrap_or(default)
}
fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}
impl Process {
    fn available(&self) -> bool {
        self.enabled
            && self.exe.is_file()
            && self.model.is_file()
            && self.mobile_file.as_ref().is_none_or(|file| file.ready())
    }
    async fn healthy(&self) -> bool {
        self.client
            .get(format!("http://127.0.0.1:{}/health", self.port))
            .timeout(Duration::from_millis(1500))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }
    async fn ensure(&self) -> Result<()> {
        if !self.available() {
            return Err(Error::new(
                503,
                "Modelo local desabilitado ou runtime/GGUF indisponível.",
            )
            .extra("code", "LOCAL_UNAVAILABLE"));
        }
        let mut owner = self.child.lock().await;
        if let Some(child) = owner.as_mut() {
            if child.try_wait()?.is_none() {
                // A failed /health never drops the ownership of a living process.
                let deadline = tokio::time::Instant::now() + self.startup_timeout;
                while tokio::time::Instant::now() < deadline {
                    if self.healthy().await {
                        return Ok(());
                    }
                    if child.try_wait()?.is_some() {
                        break;
                    }
                    tokio::time::sleep_until(
                        deadline.min(tokio::time::Instant::now() + Duration::from_millis(700)),
                    )
                    .await;
                }
                if child.try_wait()?.is_none() {
                    child.kill().await?;
                    child.wait().await?;
                }
            }
            *owner = None;
        }
        if tokio::net::TcpStream::connect(("127.0.0.1", self.port))
            .await
            .is_ok()
        {
            return Err(Error::new(
                503,
                format!(
                    "Porta local {} ocupada; não assumirei outro processo.",
                    self.port
                ),
            )
            .extra("code", "LOCAL_UNAVAILABLE"));
        }
        for args in &self.args {
            let mut command = Command::new(&self.exe);
            #[cfg(test)]
            if let Some(fixture) = &self.fixture_command {
                command
                    .args(fixture)
                    .env("MC_LIFECYCLE_FIXTURE_PORT", self.port.to_string());
            }
            #[cfg(test)]
            let native_args = self.fixture_command.is_none();
            #[cfg(not(test))]
            let native_args = true;
            if native_args {
                // Mobile owns unloading; pinned llama.cpp disables its internal sleep with -1.
                let sleep_idle_seconds = if self.mobile_file.is_some() {
                    "-1".to_owned()
                } else {
                    self.idle.to_string()
                };
                command
                    .args([
                        "-m",
                        self.model
                            .to_str()
                            .ok_or_else(|| Error::new(400, "Caminho GGUF inválido."))?,
                        "--host",
                        "127.0.0.1",
                        "--port",
                        &self.port.to_string(),
                        "--no-webui",
                        "--sleep-idle-seconds",
                        &sleep_idle_seconds,
                    ])
                    .args(args);
            }
            command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.as_std_mut().creation_flags(0x08000000);
            }
            *owner = Some(command.spawn()?);
            #[cfg(windows)]
            if let Some(child) = owner.as_ref() {
                tie_to_server(child);
            }
            let deadline = tokio::time::Instant::now() + self.startup_timeout;
            let mut ready = false;
            while tokio::time::Instant::now() < deadline {
                if owner.as_mut().unwrap().try_wait()?.is_some() {
                    break;
                }
                if self.healthy().await {
                    ready = true;
                    break;
                }
                tokio::time::sleep_until(
                    deadline.min(tokio::time::Instant::now() + Duration::from_millis(700)),
                )
                .await;
            }
            if ready {
                return Ok(());
            }
            let child = owner.as_mut().unwrap();
            if child.try_wait()?.is_none() {
                child.kill().await?;
                child.wait().await?;
            }
            *owner = None;
        }
        Err(Error::new(
            503,
            format!("Modelo {} não ficou pronto no prazo.", self.name),
        )
        .extra("code", "LOCAL_UNAVAILABLE"))
    }
    async fn status(&self) -> Value {
        // Provisioning progress remains readable while a native runtime starts or stops.
        let mut owner = if self.mobile_file.is_some() {
            match self.child.try_lock() {
                Ok(owner) => owner,
                Err(_) => {
                    return json!({"name":self.name,"running":true,"pid":null,"ok":self.available(),"starting":true,"reason":null});
                }
            }
        } else {
            self.child.lock().await
        };
        if owner
            .as_mut()
            .is_some_and(|child| child.try_wait().is_ok_and(|exit| exit.is_some()))
        {
            *owner = None;
        }
        json!({"name":self.name,"running":owner.is_some(),"pid":owner.as_ref().and_then(Child::id),"ok":self.available(),"reason":if self.available(){Value::Null}else{json!("runtime/GGUF indisponível ou modelo desabilitado")}})
    }
    async fn stop(&self) -> Result<()> {
        let mut owner = self.child.lock().await;
        if let Some(child) = owner.as_mut() {
            if child.try_wait()?.is_none() {
                child.kill().await?;
                child.wait().await?;
            }
            *owner = None;
        }
        Ok(())
    }
}
impl LocalModels {
    pub fn is_mobile(&self) -> bool {
        self.mobile.is_some()
    }
    pub fn mobile_activated(&self) -> bool {
        self.mobile.as_ref().is_some_and(|m| m.activated())
    }
    pub fn chat_readiness(&self) -> Result<()> {
        if self.chat_available() {
            return Ok(());
        }
        let downloading = self.mobile.as_ref().is_some_and(|m| {
            m.activated()
                && matches!(
                    m.chat_file.status().phase,
                    "pending" | "downloading" | "verifying"
                )
        });
        Err(Error::new(
            503,
            if downloading {
                "Modelo compacto em download; mensagem preservada. Ative e aguarde o modelo."
            } else {
                "Modelo compacto em standby ou runtime/GGUF indisponível; mensagem preservada."
            },
        )
        .extra(
            "code",
            if downloading {
                "LOCAL_DOWNLOADING"
            } else {
                "LOCAL_UNAVAILABLE"
            },
        ))
    }
    pub(crate) async fn chat_session(&self) -> Result<ChatSession> {
        self.chat_readiness()?;
        let mobile = self
            .mobile
            .as_ref()
            .ok_or_else(|| Error::new(409, "Conversa compacta exige perfil mobile."))?;
        let operation = mobile.begin().await?;
        self.embed.stop().await?;
        self.chat.ensure().await?;
        Ok(ChatSession {
            local: self.clone(),
            _operation: operation,
        })
    }
    async fn mobile_budget(
        &self,
        messages: &[Value],
        requested: u64,
        minimum: usize,
    ) -> Result<u64> {
        let rendered = self.post(&self.chat, "/apply-template", json!({"messages":messages,"add_generation_prompt":true,"chat_template_kwargs":{"enable_thinking":false}}), self.chat_timeout).await?;
        let prompt = rendered["prompt"]
            .as_str()
            .ok_or_else(|| Error::new(502, "Template local inválido."))?;
        let tokens = self
            .post(
                &self.chat,
                "/tokenize",
                json!({"content":prompt,"add_special":true}),
                self.chat_timeout,
            )
            .await?;
        let used = tokens["tokens"]
            .as_array()
            .ok_or_else(|| Error::new(502, "Contagem local de tokens inválida."))?
            .len();
        let remaining = 4096usize.saturating_sub(used + 64);
        if remaining < minimum {
            return Err(Error::new(
                502,
                "Entrada local excedeu ctx4096; redução explícita necessária.",
            )
            .extra("code", "LOCAL_CONTEXT_TOO_LARGE")
            .extra("promptTokens", used)
            .extra("remainingTokens", remaining));
        }
        Ok(requested.min(remaining as u64))
    }
    pub fn new(config: &Config, client: reqwest::Client) -> Result<Self> {
        let embed_digest = Arc::new(DigestCache::default());
        if let Some(native_dir) = &config.native_lib_dir {
            // No desktop overrides and no I/O, downloads or processes until explicit activation.
            let downloader = model_files::download_client()?;
            let root = config.data_dir.join("models");
            let chat_file = Arc::new(ModelFile::new(
                root.clone(),
                CHAT,
                downloader.clone(),
                Arc::new(DigestCache::default()),
            ));
            let embed_file = Arc::new(ModelFile::new(
                root,
                EMBED,
                downloader,
                embed_digest.clone(),
            ));
            let process = |name: &str, file: Arc<ModelFile>, port, args| {
                Arc::new(Process {
                    name: name.into(),
                    exe: native_dir.join("libllama_server.so"),
                    model: file.path.clone(),
                    port,
                    args: vec![args],
                    idle: 0,
                    child: Mutex::new(None),
                    enabled: true,
                    client: client.clone(),
                    startup_timeout: Duration::from_secs(150),
                    mobile_file: Some(file),
                    #[cfg(test)]
                    fixture_command: None,
                })
            };
            let chat = process(
                "chat-memory",
                chat_file.clone(),
                3012,
                strings(&[
                    "-ngl",
                    "0",
                    "--device",
                    "none",
                    "-t",
                    "4",
                    "-tb",
                    "4",
                    "-c",
                    "4096",
                    "-np",
                    "1",
                    "-b",
                    "128",
                    "-ub",
                    "64",
                    "--cache-ram",
                    "0",
                    "--jinja",
                    "--reasoning",
                    "off",
                ]),
            );
            // Non-causal embedding requires a whole input in a physical batch. Keep the existing
            // 1600-character chunks and mean pooling; only one mobile process is resident at a time.
            let embed = process(
                "embed-memory",
                embed_file.clone(),
                3013,
                strings(&[
                    "--embeddings",
                    "--pooling",
                    "mean",
                    "-ngl",
                    "0",
                    "--device",
                    "none",
                    "-t",
                    "4",
                    "-tb",
                    "4",
                    "-c",
                    "2048",
                    "-np",
                    "1",
                    "-b",
                    "2048",
                    "-ub",
                    "2048",
                ]),
            );
            let mobile = Some(Mobile::new(
                chat.clone(),
                embed.clone(),
                chat_file,
                embed_file,
            ));
            return Ok(Self {
                chat,
                embed,
                mock: None,
                client,
                chat_timeout: Duration::from_secs(600),
                mobile,
                embed_digest,
            });
        }
        let base_profile = PathBuf::from(
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .unwrap_or_default(),
        );
        let default_home = base_profile.join("ModularChat-local");
        let home = env_path("MC_LOCAL_HOME", default_home);
        let exe = env_path("LLAMA_SERVER", home.join("llama-cuda/llama-server.exe"));
        // Memory model: Nemotron 3.5 Bolt E4B 4GB (Q4_K_M, ~6.18 GiB) is optimized for local memory extraction,
        // voice and structured tasks. `--fit` keeps as many layers as fit in VRAM and the rest in RAM.
        // Falling back to Qwen3.5-9B or 4B if present.
        let models = home.join("models");
        let chat_model = env_path(
            "LOCAL_CHAT_MODEL",
            [
                "Nemotron-3.5-Bolt-E4B-Q4_K_M.gguf",
                "Qwen3.5-9B-Q4_K_M.gguf",
                "Qwen3.5-4B-Q5_K_M.gguf",
            ]
            .iter()
            .map(|f| models.join(f))
            .find(|p| p.is_file())
            .unwrap_or_else(|| models.join("Nemotron-3.5-Bolt-E4B-Q4_K_M.gguf")),
        );
        let embed_model = env_path(
            "LOCAL_EMBED_MODEL",
            home.join("models/embeddinggemma-300M-Q8_0.gguf"),
        );
        let chat_port = std::env::var("LOCAL_CHAT_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(if config.production { 20481 } else { 20581 });
        let embed_port = std::env::var("LOCAL_EMBED_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(if config.production { 20482 } else { 20582 });
        if chat_port == embed_port
            || !config.production
                && ([20481, 20482].contains(&chat_port) || [20481, 20482].contains(&embed_port))
        {
            return Err(Error::new(
                400,
                "Portas de modelos devem ser distintas e isoladas da produção.",
            ));
        }
        // Qwen3.5 is hybrid (few full-attention layers): 16k of q8 KV costs only a few hundred MiB and avoids the
        // "prompt larger than the context" retries that shrank the extraction.
        let ctx = std::env::var("LOCAL_CHAT_CTX").unwrap_or("16384".into());
        let fit_margin = std::env::var("LOCAL_FIT_MARGIN_MIB").unwrap_or("768".into());
        let idle = std::env::var("LOCAL_IDLE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(900000)
            / 1000;
        let enabled =
            std::env::var("MC_LOCAL_MODELS_ENABLED").as_deref() == Ok("1") || config.production;
        let mut args = Vec::new();
        // 1st: automatic GPU/RAM split sized to the free VRAM (leaves `fit_margin` MiB for the desktop/other apps);
        // 2nd: CPU only, if the GPU attempt does not come up.
        for gpu in [true, false] {
            let mut a = strings(&["-c", &ctx]);
            if gpu {
                a.extend(strings(&[
                    "-ngl",
                    "auto",
                    "--fit",
                    "on",
                    "--fit-target",
                    &fit_margin,
                    "-fa",
                    "on",
                    "-ctk",
                    "q8_0",
                    "-ctv",
                    "q8_0",
                ]));
            } else {
                a.extend(strings(&["-ngl", "0", "--fit", "off"]));
            }
            a.extend(strings(&[
                "--reasoning",
                "off",
                "--jinja",
                "-np",
                "1",
                "--cache-ram",
                "0",
            ]));
            args.push(a);
        }
        let name = embed_model.to_string_lossy().to_lowercase();
        let pooling = if name.contains("bge") {
            "cls"
        } else if name.contains("qwen") {
            "last"
        } else {
            "mean"
        };
        let embed_args = vec![strings(&[
            "--embeddings",
            "--pooling",
            pooling,
            "-ngl",
            "0",
            "--device",
            "none",
            "-c",
            "2048",
            "-b",
            "2048",
            "-ub",
            "2048",
            "-np",
            "1",
            "-t",
            "6",
        ])];
        let mock = if !config.production {
            std::env::var("MC_TEST_LOCAL_URL")
                .ok()
                .filter(|v| !v.trim().is_empty())
        } else {
            None
        };
        if let Some(url) = &mock {
            let url = reqwest::Url::parse(url)
                .map_err(|_| Error::new(400, "URL local de fixture inválida."))?;
            if !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                || [20128, 20481, 20482, 3000].contains(&url.port_or_known_default().unwrap_or(0))
            {
                return Err(Error::new(
                    400,
                    "Fixture local deve usar loopback e porta isolada.",
                ));
            }
        }
        Ok(Self {
            chat: Arc::new(Process {
                name: "chat-memory".into(),
                exe: exe.clone(),
                model: chat_model,
                port: chat_port,
                args,
                idle,
                child: Mutex::new(None),
                enabled,
                client: client.clone(),
                startup_timeout: Duration::from_secs(150),
                mobile_file: None,
                #[cfg(test)]
                fixture_command: None,
            }),
            embed: Arc::new(Process {
                name: "embed-memory".into(),
                exe,
                model: embed_model,
                port: embed_port,
                args: embed_args,
                idle,
                child: Mutex::new(None),
                enabled,
                client: client.clone(),
                startup_timeout: Duration::from_secs(150),
                mobile_file: None,
                #[cfg(test)]
                fixture_command: None,
            }),
            mock,
            client,
            mobile: None,
            embed_digest,
            chat_timeout: Duration::from_secs(
                std::env::var("LOCAL_CHAT_TIMEOUT_S")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(600),
            ),
        })
    }
    pub fn chat_available(&self) -> bool {
        self.mock.is_some()
            || self.mobile.as_ref().is_none_or(|m| m.activated()) && self.chat.available()
    }
    pub fn embed_available(&self) -> bool {
        self.mock.is_some()
            || self.mobile.as_ref().is_none_or(|m| m.activated()) && self.embed.available()
    }
    pub async fn status(&self) -> Value {
        if self.mock.is_some() {
            json!({"chat":{"ok":true,"fixture":true},"embed":{"ok":true,"fixture":true}})
        } else {
            let mut status =
                json!({"chat":self.chat.status().await,"embed":self.embed.status().await});
            if let Some(mobile) = &self.mobile {
                let lifecycle = mobile.status();
                for (key, value) in lifecycle.as_object().unwrap() {
                    status[key] = value.clone();
                }
                status["chat"]["ok"] = json!(self.chat_available());
                status["embed"]["ok"] = json!(self.embed_available());
                status["chat"]["download"] = json!(mobile.chat_file.status());
                status["embed"]["download"] = json!(mobile.embed_file.status());
            }
            status
        }
    }
    pub async fn activate(&self) -> Result<Value> {
        let mobile = self
            .mobile
            .as_ref()
            .ok_or_else(|| Error::new(409, "Ativação disponível apenas no perfil Android."))?;
        mobile.activate().await?;
        Ok(self.status().await)
    }
    pub async fn standby(&self) -> Result<Value> {
        let mobile = self
            .mobile
            .as_ref()
            .ok_or_else(|| Error::new(409, "Standby disponível apenas no perfil Android."))?;
        mobile.standby().await;
        Ok(self.status().await)
    }
    pub async fn download_model(&self, kind: &str) -> Result<Value> {
        if self.mock.is_some() {
            return Ok(self.status().await);
        }
        let mobile = self
            .mobile
            .as_ref()
            .ok_or_else(|| Error::new(409, "Download disponível apenas no perfil Android."))?;
        mobile.download_file(kind).await?;
        Ok(self.status().await)
    }
    pub async fn delete_model(&self, kind: &str) -> Result<Value> {
        if self.mock.is_some() {
            return Ok(self.status().await);
        }
        let mobile = self
            .mobile
            .as_ref()
            .ok_or_else(|| Error::new(409, "Exclusão disponível apenas no perfil Android."))?;
        mobile.delete_file(kind).await?;
        Ok(self.status().await)
    }
    async fn pinned_embedding(&self) -> bool {
        if self.mock.is_some() {
            return false;
        }
        if self.mobile.is_some() {
            return self.embed_digest.matches(&self.embed.model, EMBED.sha256);
        }
        // Keep custom desktop pooling semantics. A compatible renamed copy may use the pin,
        // but no filename (including embeddinggemma) can prove content identity.
        let mean = self.embed.args[0]
            .windows(2)
            .any(|pair| pair == ["--pooling", "mean"]);
        mean && self
            .embed_digest
            .digest(&self.embed.model, EMBED.size, &Cancel::new())
            .await
            .ok()
            .flatten()
            .as_deref()
            == Some(EMBED.sha256)
    }
    pub async fn fingerprint(&self) -> String {
        if self.pinned_embedding().await {
            return embedding_fingerprint(EMBED.sha256);
        }
        let metadata = self.embed.model.metadata().ok();
        let size = metadata.as_ref().map(|m| m.len());
        let mtime = metadata
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_millis());
        rules::hash(
            &json!({"file":self.embed.model,"size":size,"mtime":mtime,"format":2}).to_string(),
        )
    }
    async fn post(
        &self,
        process: &Process,
        path: &str,
        body: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let base = if let Some(mock) = &self.mock {
            mock.clone()
        } else {
            if self.mobile.is_some() {
                // The operation gate covers the whole chat/embedding operation, including all chunks.
                // Unload the other owned runtime before loading this one on a phone.
                if std::ptr::eq(process, self.chat.as_ref()) {
                    self.embed.stop().await?;
                } else {
                    self.chat.stop().await?;
                }
            }
            process.ensure().await?;
            format!("http://127.0.0.1:{}", process.port)
        };
        let response = self
            .client
            .post(format!("{}{path}", base.trim_end_matches('/')))
            .timeout(timeout)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                // A timeout means the model is alive but slow for this input: it must follow the normal retry flow
                // (smaller context, then "failed") instead of LOCAL_UNAVAILABLE, which re-queued the same job forever.
                if e.is_timeout() {
                    Error::new(504, "Modelo local excedeu o prazo para esta entrada.")
                } else {
                    Error::new(503, "Requisição local falhou.").extra("code", "LOCAL_UNAVAILABLE")
                }
            })?;
        if response.status().is_client_error() {
            // 4xx (usually 400: prompt larger than the model context) is NOT unavailability: repeating the same request
            // forever stalled the memory queue. It follows the normal retry flow (the job shrinks the context).
            return Err(Error::new(
                502,
                format!(
                    "Modelo local recusou a requisição (HTTP {}).",
                    response.status()
                ),
            ));
        }
        if !response.status().is_success() {
            return Err(
                Error::new(503, format!("Modelo local HTTP {}", response.status()))
                    .extra("code", "LOCAL_UNAVAILABLE"),
            );
        }
        crate::bounded_http::json(response, crate::bounded_http::JSON_LIMIT)
            .await
            .map_err(|_| Error::new(502, "JSON local inválido ou excedeu o limite de leitura."))
    }
    pub async fn chat_json(
        &self,
        messages: Vec<Value>,
        schema: Value,
        max_tokens: u64,
        temperature: f64,
    ) -> Result<Value> {
        let _operation = if let Some(mobile) = &self.mobile {
            Some(mobile.begin().await?)
        } else {
            None
        };
        let max_tokens = if self.mobile.is_some() {
            self.mobile_budget(
                &messages,
                max_tokens,
                512usize.max(max_tokens.min(1200) as usize),
            )
            .await?
        } else {
            max_tokens
        };
        let data=self.post(&self.chat,"/v1/chat/completions",json!({"model":"local","messages":messages,"temperature":temperature,"max_tokens":max_tokens,"stream":false,"chat_template_kwargs":{"enable_thinking":false},"response_format":{"type":"json_schema","json_schema":{"name":"saida","strict":true,"schema":schema}}}),self.chat_timeout).await?;
        if data["choices"][0]["finish_reason"] == "length" {
            return Err(Error::new(
                502,
                "JSON local excedeu o orçamento e ficou incompleto.",
            ));
        }
        let text = data["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default();
        let raw = serde_json::from_str(text)
            .map_err(|_| Error::new(502, "Modelo local não devolveu JSON completo."))?;
        crate::memory_backend::validate_schema(&raw, &schema)?;
        Ok(raw)
    }
    pub async fn embed_text(&self, text: &str, query: bool) -> Result<Vec<f32>> {
        let _operation = if let Some(mobile) = &self.mobile {
            Some(mobile.begin().await?)
        } else {
            None
        };
        let name = self.embed.model.to_string_lossy().to_lowercase();
        let gemma = name.contains("embeddinggemma") || self.pinned_embedding().await;
        let chunks = rules::chunks(if text.is_empty() { " " } else { text }, 1600, 80);
        let mut partial = Vec::<Vec<f32>>::new();
        for (_, _, text) in chunks {
            let input = if gemma {
                if query {
                    format!("task: search result | query: {text}")
                } else {
                    format!("title: História | text: {text}")
                }
            } else if name.contains("qwen") && query {
                format!(
                    "Instruct: Retrieve relevant story facts and events for the query.\nQuery: {text}"
                )
            } else {
                text
            };
            let data = self
                .post(
                    &self.embed,
                    "/v1/embeddings",
                    json!({"model":"local","input":[input]}),
                    Duration::from_secs(30),
                )
                .await?;
            let vector = data["data"][0]["embedding"]
                .as_array()
                .ok_or_else(|| Error::new(502, "Embedding local ausente."))?
                .iter()
                .map(|v| {
                    v.as_f64()
                        .map(|v| v as f32)
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| Error::new(502, "Embedding local inválido."))
                })
                .collect::<Result<Vec<_>>>()?;
            rules::vector_blob(&vector)?;
            if partial.first().is_some_and(|v| v.len() != vector.len()) {
                return Err(Error::new(502, "Dimensão do embedding mudou."));
            }
            partial.push(vector);
        }
        let mut result = vec![0f32; partial[0].len()];
        for row in &partial {
            for (i, v) in row.iter().enumerate() {
                result[i] += v / partial.len() as f32;
            }
        }
        let norm = result
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            .sqrt();
        if !norm.is_finite() || norm == 0.0 {
            return Err(Error::new(502, "Embedding vazio."));
        }
        for v in &mut result {
            *v = (f64::from(*v) / norm) as f32;
        }
        Ok(result)
    }
    pub async fn stop(&self) -> Result<()> {
        if let Some(mobile) = &self.mobile {
            mobile.stop().await;
        }
        // Attempt both even if one child fails to reap.
        let chat = self.chat.stop().await;
        let embed = self.embed.stop().await;
        chat.and(embed)
    }
}

fn embedding_fingerprint(sha256: &str) -> String {
    rules::hash(&json!({"sha256":sha256,"pooling":"mean","prefixes":"embeddinggemma-search-story-v1","chunks":[1600,80],"format":2}).to_string())
}

pub use crate::cast::from_card as cast_from_card;

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    fn process(port: u16, model: PathBuf) -> Process {
        Process {
            name: "isolated fixture".into(),
            exe: std::env::current_exe().unwrap(),
            model,
            port,
            args: vec![],
            idle: 1,
            child: Mutex::new(None),
            enabled: true,
            client: reqwest::Client::new(),
            startup_timeout: Duration::from_millis(180),
            mobile_file: None,
            fixture_command: None,
        }
    }

    #[tokio::test]
    async fn local_json_paths_are_bounded_without_reclassifying_them_as_unavailable() {
        use crate::bounded_http::{
            JSON_LIMIT,
            tests::{chunked_oversize, serve},
        };
        let value = json!({"data":[{"embedding":[3.0,4.0]}],"prompt":"template intacto","tokens":vec![1;8192],"choices":[{"message":{"content":"{\"ok\":true}"}}]});
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for path in [
            "/v1/embeddings",
            "/apply-template",
            "/tokenize",
            "/v1/chat/completions",
        ] {
            let normal = serde_json::to_vec(&value).unwrap();
            for (headers, body, ok) in [
                (
                    format!("Content-Length: {}\r\n", normal.len()),
                    normal,
                    true,
                ),
                (
                    format!("Content-Length: {}\r\n", JSON_LIMIT + 1),
                    vec![],
                    false,
                ),
                (
                    "Transfer-Encoding: chunked\r\n".into(),
                    chunked_oversize(JSON_LIMIT),
                    false,
                ),
                (
                    "Content-Length: 19\r\n".into(),
                    b"SECRET_INVALID_JSON".to_vec(),
                    false,
                ),
            ] {
                let fixture = serve(&headers, body).await;
                let local = LocalModels {
                    chat: Arc::new(process(0, PathBuf::new())),
                    embed: Arc::new(process(0, PathBuf::new())),
                    mock: Some(fixture.url.clone()),
                    client: client.clone(),
                    chat_timeout: Duration::from_secs(2),
                    mobile: None,
                    embed_digest: Arc::default(),
                };
                let result = local
                    .post(
                        &local.chat,
                        path,
                        json!({"fixture":true}),
                        Duration::from_secs(2),
                    )
                    .await;
                if ok {
                    assert_eq!(result.unwrap(), value, "{path}");
                } else {
                    let error = result.unwrap_err();
                    assert_eq!(error.status, 502, "{path}");
                    assert_ne!(error.extra["code"], "LOCAL_UNAVAILABLE");
                    assert!(!error.message.contains("SECRET"));
                }
            }
        }
    }

    #[tokio::test]
    async fn mobile_budget_uses_tokenized_ctx4096_instead_of_the_text_estimate() {
        use axum::{Json, Router, routing::post};
        let messages = vec![json!({"role":"user","content":"contexto 🚀 ".repeat(5000)})];
        assert!(rules::estimate_messages(&messages) > 4096);
        for (used, requested, minimum, expected) in [
            (3008usize, 1024u64, 1024usize, Some(1024u64)),
            (3009, 1024, 1024, None),
            (3900, 1024, 128, Some(132)),
            (4096, 1024, 1, None),
        ] {
            let expected_messages = messages.clone();
            let app = Router::new()
                .route("/apply-template", post(move |Json(body): Json<Value>| {
                    let expected = expected_messages.clone();
                    async move {
                        assert_eq!(body["messages"], json!(expected));
                        assert_eq!(body["add_generation_prompt"], true);
                        Json(json!({"prompt":"TEMPLATE_🚀"}))
                    }
                }))
                .route("/tokenize", post(move |Json(body): Json<Value>| async move {
                    assert_eq!(body["content"], "TEMPLATE_🚀");
                    assert_eq!(body["add_special"], true);
                    Json(json!({"tokens":vec![1;used]}))
                }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let local = LocalModels {
                chat: Arc::new(process(0, PathBuf::new())),
                embed: Arc::new(process(0, PathBuf::new())),
                mock: Some(url),
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                chat_timeout: Duration::from_secs(2),
                mobile: None,
                embed_digest: Arc::default(),
            };
            let result = local.mobile_budget(&messages, requested, minimum).await;
            server.abort();
            if let Some(budget) = expected {
                assert_eq!(result.unwrap(), budget);
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.status, 502);
                assert_eq!(error.extra["code"], "LOCAL_CONTEXT_TOO_LARGE");
                assert_eq!(error.extra["promptTokens"], used);
                assert_eq!(error.extra["remainingTokens"], 4096usize.saturating_sub(used + 64));
            }
        }
    }

    #[tokio::test]
    async fn occupied_port_is_not_adopted_or_stopped() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        let p = process(listener.local_addr().unwrap().port(), file.path().into());
        assert!(p.ensure().await.unwrap_err().message.contains("ocupada"));
        assert!(p.child.lock().await.is_none());
        p.stop().await.unwrap();
        assert!(
            tokio::net::TcpStream::connect(listener.local_addr().unwrap())
                .await
                .is_ok()
        );
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn failed_health_reaps_owned_child_before_replacement() {
        use std::os::windows::process::CommandExt;
        let file = tempfile::NamedTempFile::new().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let p = process(port, file.path().into());
        let exe = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut command = Command::new(&exe);
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ])
            .kill_on_drop(true);
        command.as_std_mut().creation_flags(0x08000000);
        let child = command.spawn().unwrap();
        let pid = child.id().unwrap();
        *p.child.lock().await = Some(child);
        assert!(
            tokio::time::timeout(Duration::from_secs(4), p.ensure())
                .await
                .unwrap()
                .is_err()
        );
        assert!(p.child.lock().await.is_none());
        let mut check = Command::new(exe);
        check.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "if(Get-Process -Id {pid} -ErrorAction SilentlyContinue){{exit 1}}else{{exit 0}}"
            ),
        ]);
        check.as_std_mut().creation_flags(0x08000000);
        assert!(
            tokio::time::timeout(Duration::from_secs(8), check.status())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }
}

#[cfg(test)]
mod mobile_tests;
