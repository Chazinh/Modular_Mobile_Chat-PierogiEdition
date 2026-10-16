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

pub(crate) fn valid_android_token(token: &str) -> bool {
    (32..=256).contains(&token.len())
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

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
        self.start_at(Instant::now()).await
    }
    async fn start_at(&self, now: Instant) -> Value {
        let mut state = self.state.lock().await;
        // Reopening the code on the PC must not invalidate the code already
        // being typed on the phone. Each explicit request renews its full TTL.
        let code = state.code.as_ref()
            .filter(|(_, expiry)| *expiry > now)
            .map(|(code, _)| code.clone())
            .unwrap_or_else(|| format!("{:06}", rand::random_range(0..1_000_000u32)));
        state.code = Some((code.clone(), now + Duration::from_secs(300)));
        json!({"code":code,"expiresIn":300,"expiresAt":(chrono::Utc::now()+chrono::Duration::minutes(5)).to_rfc3339()})
    }
    pub async fn verify(&self, token: &str) -> Result<Value> {
        if token.is_empty() || token.len() > 256 {
            return Ok(Value::Null);
        }
        self.store
            .call(Request::FindDevice(rules::hash(token)))
            .await
    }
    /// Register the app-local secret supplied by the native bridge, storing only its hash.
    /// The bridge must generate >=256 random bits, persist them in its own sandbox and never
    /// import a desktop pairing token. Reopening with the same secret preserves the device ID.
    pub(crate) async fn register_android_host(&self, token: &str) -> Result<()> {
        if !valid_android_token(token) {
            return Err(Error::new(400, "Token local inválido."));
        }
        if !self.store.encrypted || !self.required {
            return Err(Error::new(
                409,
                "O host Android exige cofre e autenticação.",
            ));
        }
        if self.verify(token).await?.is_null() {
            self.store
                .call(Request::AddDevice {
                    name: "Android local".into(),
                    token_hash: rules::hash(token),
                })
                .await?;
        }
        Ok(())
    }
    pub async fn pair(&self, p: &Value, ip: IpAddr) -> Result<Value> {
        self.pair_at(p, ip, Instant::now()).await
    }
    async fn pair_at(&self, p: &Value, ip: IpAddr, now: Instant) -> Result<Value> {
        let mut state = self.state.lock().await;
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
        let token = format!("{}{}", id().replace('-', ""), id().replace('-', ""));
        let dev = self
            .store
            .call(Request::AddDevice {
                name: p["deviceName"].as_str().unwrap_or("Dispositivo").to_owned(),
                token_hash: rules::hash(&token),
            })
            .await?;
        // Consume only after the credential has been saved successfully.
        state.code = None;
        Ok(json!({"token":token,"deviceId":dev["id"]}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pairing_code_survives_reopening_and_gets_five_full_minutes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let auth = Auth::new(store, true);
        let now = Instant::now();
        let first = auth.start_at(now).await;
        let renewed = auth.start_at(now + Duration::from_secs(299)).await;
        assert_eq!(first["code"], renewed["code"]);
        assert_eq!(renewed["expiresIn"], 300);
        let body = json!({"code":first["code"],"deviceName":"Phone"});
        let ip = "192.168.0.9".parse().unwrap();
        let paired = auth.pair_at(&body, ip, now + Duration::from_secs(598)).await.unwrap();
        assert!(!auth.verify(paired["token"].as_str().unwrap()).await.unwrap().is_null());
        assert!(auth.pair_at(&body, ip, now + Duration::from_secs(598)).await.is_err());
    }

    #[tokio::test]
    async fn pairing_code_accepts_299_seconds_and_expires_at_300() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::new(Store::open(dir.path()).unwrap(), true);
        let now = Instant::now();
        let first = auth.start_at(now).await;
        let ip = "192.168.0.9".parse().unwrap();
        let body = json!({"code":first["code"]});
        let failure = auth.pair_at(&body, ip, now + Duration::from_secs(300)).await.unwrap_err();
        assert_eq!(failure.status, 401);
        assert_eq!(failure.message, "Código inválido ou expirado.");
        let fresh = auth.start_at(now + Duration::from_secs(301)).await;
        auth.pair_at(&json!({"code":fresh["code"]}), ip, now + Duration::from_secs(600)).await.unwrap();
    }
}
