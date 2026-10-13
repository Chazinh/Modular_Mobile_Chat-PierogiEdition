use crate::{
    Error, Result, id, rules,
    store::{Request, Store},
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

pub fn private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_loopback() || v.is_private() || v.is_link_local(),
        IpAddr::V6(v) => {
            v.is_loopback()
                || v.to_ipv4_mapped().is_some_and(|v| private(v.into()))
                || v.segments()[0] & 0xfe00 == 0xfc00
                || v.segments()[0] & 0xffc0 == 0xfe80
        }
    }
}
pub fn loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
        || match ip {
            IpAddr::V6(v) => v.to_ipv4_mapped().is_some_and(|v| v.is_loopback()),
            _ => false,
        }
}

#[derive(Default)]
struct PairState {
    code: Option<(String, Instant)>,
    fails: HashMap<IpAddr, VecDeque<Instant>>,
}
#[derive(Clone)]
pub struct Auth {
    pub required: bool,
    store: Store,
    state: Arc<Mutex<PairState>>,
}
impl Auth {
    pub fn new(store: Store, required: bool) -> Self {
        Self {
            store,
            required,
            state: Arc::default(),
        }
    }
    pub async fn start(&self) -> Value {
        let code = format!("{:06}", rand::random_range(0..1_000_000u32));
        self.state.lock().await.code =
            Some((code.clone(), Instant::now() + Duration::from_secs(300)));
        json!({"code":code,"expiresAt":(chrono::Utc::now()+chrono::Duration::minutes(5)).to_rfc3339()})
    }
    pub async fn verify(&self, token: &str) -> Result<Value> {
        if token.is_empty() || token.len() > 256 {
            return Ok(Value::Null);
        }
        self.store
            .call(Request::FindDevice(rules::hash(token)))
            .await
    }
    pub async fn pair(&self, p: &Value, ip: IpAddr) -> Result<Value> {
        let mut state = self.state.lock().await;
        let now = Instant::now();
        state.fails.retain(|_, v| {
            while v
                .front()
                .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(600))
            {
                v.pop_front();
            }
            !v.is_empty()
        });
        if state.fails.len() >= 2048 && !state.fails.contains_key(&ip) {
            return Err(Error::new(429, "Muitas tentativas de pareamento."));
        }
        let fails = state.fails.entry(ip).or_default();
        if fails.len() >= 8 {
            return Err(Error::new(
                429,
                "Muitas tentativas; aguarde alguns minutos.",
            ));
        }
        let code = p["code"].as_str().unwrap_or_default().trim();
        if !state
            .code
            .as_ref()
            .is_some_and(|(c, expiry)| c == code && *expiry > now)
        {
            state.fails.entry(ip).or_default().push_back(now);
            return Err(Error::new(401, "Código inválido ou expirado."));
        }
        state.code = None;
        drop(state);
        let token = format!("{}{}", id().replace('-', ""), id().replace('-', ""));
        let dev = self
            .store
            .call(Request::AddDevice {
                name: p["deviceName"].as_str().unwrap_or("Dispositivo").to_owned(),
                token_hash: rules::hash(&token),
            })
            .await?;
        Ok(json!({"token":token,"deviceId":dev["id"]}))
    }
}
