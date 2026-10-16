//! Fixed, anonymous loopback GETs only. No service launch, environment, credentials or disk reads.
use crate::providers::{Provider, ReasoningFormat};
use serde::Serialize;
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const TTL: Duration = Duration::from_secs(30);
const PROBE_TIMEOUT: Duration = Duration::from_millis(650);
// Same ceiling as the provider preflight; enforced on Content-Length AND streamed bytes before parsing.
const MODELS_LIMIT: usize = 128 * 1024;
const HEALTH_LIMIT: usize = 16 * 1024;
const NINE_ROUTER: &str = "http://127.0.0.1:20128/v1/models";
const OPEN_CODE: &str = "http://127.0.0.1:4096/global/health";
pub(crate) const PROVIDER_ID: &str = "9router-local";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Absent,
    AuthRequired,
    Incompatible,
    Available,
    DetectedNativeApi,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Service {
    status: Status,
    protocol: &'static str,
    models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    message: &'static str,
}
impl Service {
    fn new(status: Status, protocol: &'static str) -> Self {
        let message = match status {
            Status::Absent => {
                "Serviço local indisponível. Inicie-o onde está instalado; o app não iniciou o serviço."
            }
            Status::AuthRequired => {
                "Serviço exige autenticação. A descoberta não envia credenciais; configure um provedor explicitamente."
            }
            Status::Incompatible => "A resposta local não corresponde ao protocolo esperado.",
            Status::Available => {
                "API local detectada. Escolha um modelo explicitamente para usá-la."
            }
            Status::DetectedNativeApi => {
                "OpenCode serve detectado: API própria, incompatível com OpenAI chat completions. OpenCode Zen HTTPS é um serviço diferente."
            }
        };
        Self {
            status,
            protocol,
            models: Vec::new(),
            version: None,
            message,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    checked_at: String,
    ttl_seconds: u64,
    nine_router: Service,
    open_code: Service,
    #[serde(skip)]
    nine_router_base: String,
}
impl Snapshot {
    pub(crate) fn provider(&self) -> Option<Provider> {
        (self.nine_router.status == Status::Available && !self.nine_router.models.is_empty()).then(
            || Provider {
                id: PROVIDER_ID.into(),
                base_url: self.nine_router_base.clone(),
                api_key: None,
                models: self.nine_router.models.clone(),
                request_log: None,
                reasoning_format: ReasoningFormat::Omit,
                context_window: 8192,
                max_output: 2048,
            },
        )
    }
}

pub(crate) struct Discovery {
    client: Option<reqwest::Client>,
    nine_router: String,
    open_code: String,
    ttl: Duration,
    cache: Mutex<Option<(Instant, Snapshot)>>,
}
impl Discovery {
    pub(crate) fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_millis(150))
                .timeout(PROBE_TIMEOUT)
                .build()
                .ok(),
            nine_router: NINE_ROUTER.into(),
            open_code: OPEN_CODE.into(),
            ttl: TTL,
            cache: Mutex::new(None),
        }
    }
    // The mutex spans both bounded probes: concurrent callers share one refresh, including failures.
    // There is no background task and no URL/refresh override exposed by the HTTP API.
    pub(crate) async fn snapshot(&self) -> (Snapshot, bool) {
        let mut cache = self.cache.lock().await;
        if let Some((at, snapshot)) = cache.as_ref()
            && at.elapsed() < self.ttl
        {
            return (snapshot.clone(), false);
        }
        let (nine_router, open_code) = tokio::join!(self.probe(false), self.probe(true));
        let snapshot = Snapshot {
            checked_at: crate::now(),
            ttl_seconds: self.ttl.as_secs(),
            nine_router,
            open_code,
            nine_router_base: self.nine_router.trim_end_matches("/models").into(),
        };
        *cache = Some((Instant::now(), snapshot.clone()));
        (snapshot, true)
    }
    pub(crate) async fn cached_provider(&self) -> Option<Provider> {
        self.cache
            .lock()
            .await
            .as_ref()
            .and_then(|(_, s)| s.provider())
    }
    async fn probe(&self, native: bool) -> Service {
        let protocol = if native {
            "opencode-native"
        } else {
            "openai-compatible"
        };
        let Some(client) = &self.client else {
            return Service::new(Status::Absent, protocol);
        };
        tokio::time::timeout(PROBE_TIMEOUT, async {
            let url = if native {
                &self.open_code
            } else {
                &self.nine_router
            };
            let Ok(response) = client.get(url).send().await else {
                return Service::new(Status::Absent, protocol);
            };
            let status = response.status();
            if matches!(status.as_u16(), 401 | 403) {
                return Service::new(Status::AuthRequired, protocol);
            }
            if !status.is_success() {
                return Service::new(Status::Incompatible, protocol);
            }
            let limit = if native { HEALTH_LIMIT } else { MODELS_LIMIT };
            let Ok(body) = crate::bounded_http::json(response, limit).await else {
                return Service::new(Status::Incompatible, protocol);
            };
            if native {
                if body["healthy"] != true
                    || !body["version"].as_str().is_some_and(|v| valid_id(v, 128))
                {
                    return Service::new(Status::Incompatible, protocol);
                }
                let mut service = Service::new(Status::DetectedNativeApi, protocol);
                service.version = body["version"].as_str().map(str::to_owned);
                service
            } else {
                match model_ids(&body) {
                    Some(models) => {
                        let mut service = Service::new(Status::Available, protocol);
                        service.models = models;
                        service
                    }
                    None => Service::new(Status::Incompatible, protocol),
                }
            }
        })
        .await
        .unwrap_or_else(|_| Service::new(Status::Absent, protocol))
    }
}

fn valid_id(id: &str, limit: usize) -> bool {
    !id.trim().is_empty() && id.len() <= limit && !id.chars().any(char::is_control)
}
fn model_ids(body: &Value) -> Option<Vec<String>> {
    let data = body["data"].as_array()?;
    if data.len() > 128 {
        return None;
    }
    let mut models = Vec::with_capacity(data.len());
    for entry in data {
        let id = entry["id"].as_str().filter(|id| valid_id(id, 256))?;
        models.push(id.to_owned());
    }
    models.sort();
    models.dedup();
    Some(models)
}

#[cfg(test)]
#[path = "local_discovery_tests.rs"]
mod tests;
