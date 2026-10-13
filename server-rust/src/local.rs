//! Owns only children started by this server. Local memory never uses the provider gateway.
use crate::{Error, Result, config::Config, rules};
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
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    static JOB: OnceLock<usize> = OnceLock::new();
    let job = *JOB.get_or_init(|| unsafe {
        let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if handle.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            std::ptr::addr_of!(info).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        handle as usize
    });
    if job != 0
        && let Some(raw) = child.raw_handle()
    {
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
}
#[derive(Clone)]
pub struct LocalModels {
    chat: Arc<Process>,
    embed: Arc<Process>,
    mock: Option<String>,
    client: reqwest::Client,
}
fn env_path(name: &str, default: PathBuf) -> PathBuf {
    std::env::var_os(name).map(PathBuf::from).unwrap_or(default)
}
fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}
impl Process {
    fn available(&self) -> bool {
        self.enabled && self.exe.is_file() && self.model.is_file()
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
                    &self.idle.to_string(),
                ])
                .args(args)
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
        let owner = self.child.lock().await;
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
    pub fn new(config: &Config, client: reqwest::Client) -> Result<Self> {
        let home = env_path(
            "MC_LOCAL_HOME",
            PathBuf::from(
                std::env::var_os("USERPROFILE")
                    .or_else(|| std::env::var_os("HOME"))
                    .unwrap_or_default(),
            )
            .join("ModularChat-local"),
        );
        let exe = env_path("LLAMA_SERVER", home.join("llama-cuda/llama-server.exe"));
        let chat_model = env_path(
            "LOCAL_CHAT_MODEL",
            home.join("models/Qwen3.5-4B-Q5_K_M.gguf"),
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
        let ctx = std::env::var("LOCAL_CHAT_CTX").unwrap_or("8192".into());
        let idle = std::env::var("LOCAL_IDLE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(900000)
            / 1000;
        let enabled =
            std::env::var("MC_LOCAL_MODELS_ENABLED").as_deref() == Ok("1") || config.production;
        let mut args = Vec::new();
        for layers in ["99", "24", "0"] {
            let mut a = strings(&["-c", &ctx, "-ngl", layers]);
            if layers != "0" {
                a.extend(strings(&["-fa", "on", "-ctk", "q8_0", "-ctv", "q8_0"]));
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
            }),
            mock,
            client,
        })
    }
    pub fn chat_available(&self) -> bool {
        self.mock.is_some() || self.chat.available()
    }
    pub fn embed_available(&self) -> bool {
        self.mock.is_some() || self.embed.available()
    }
    pub async fn status(&self) -> Value {
        if self.mock.is_some() {
            json!({"chat":{"ok":true,"fixture":true},"embed":{"ok":true,"fixture":true}})
        } else {
            json!({"chat":self.chat.status().await,"embed":self.embed.status().await})
        }
    }
    pub fn fingerprint(&self) -> String {
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
            .map_err(|_| {
                Error::new(503, "Requisição local falhou.").extra("code", "LOCAL_UNAVAILABLE")
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
        response
            .json()
            .await
            .map_err(|_| Error::new(502, "JSON local inválido."))
    }
    pub async fn chat_json(
        &self,
        messages: Vec<Value>,
        schema: Value,
        max_tokens: u64,
        temperature: f64,
    ) -> Result<Value> {
        let data=self.post(&self.chat,"/v1/chat/completions",json!({"model":"local","messages":messages,"temperature":temperature,"max_tokens":max_tokens,"stream":false,"chat_template_kwargs":{"enable_thinking":false},"response_format":{"type":"json_schema","json_schema":{"name":"saida","strict":true,"schema":schema}}}),Duration::from_secs(240)).await?;
        if data["choices"][0]["finish_reason"] == "length" {
            return Err(Error::new(
                502,
                "JSON local excedeu o orçamento e ficou incompleto.",
            ));
        }
        let text = data["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default();
        serde_json::from_str(text)
            .map_err(|_| Error::new(502, "Modelo local não devolveu JSON completo."))
    }
    pub async fn embed_text(&self, text: &str, query: bool) -> Result<Vec<f32>> {
        let name = self.embed.model.to_string_lossy().to_lowercase();
        let chunks = rules::chunks(if text.is_empty() { " " } else { text }, 1600, 80);
        let mut partial = Vec::<Vec<f32>>::new();
        for (_, _, text) in chunks {
            let input = if name.contains("embeddinggemma") {
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
        self.chat.stop().await?;
        self.embed.stop().await
    }
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
