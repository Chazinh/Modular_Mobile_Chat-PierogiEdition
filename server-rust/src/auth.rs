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

pub const PAIRING_TTL: Duration = Duration::from_secs(900);

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
        state.code = Some((code.clone(), now + PAIRING_TTL));
        json!({
            "code": code,
            "expiresIn": PAIRING_TTL.as_secs(),
            "expiresAt": (chrono::Utc::now() + chrono::Duration::seconds(PAIRING_TTL.as_secs() as i64)).to_rfc3339()
        })
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
                .is_some_and(|t| now.saturating_duration_since(*t) > Duration::from_secs(600))
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
        let code = match &p["code"] {
            Value::String(s) => s.trim().to_string(),
            Value::Number(n) => {
                if let Some(v) = n.as_u64() {
                    format!("{:06}", v)
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        };
        match &state.code {
            None => {
                state.fails.entry(ip).or_default().push_back(now);
                return Err(Error::new(401, "Nenhum código ativo no momento. Gere um novo código no PC."));
            }
            Some((_, expiry)) if *expiry <= now => {
                state.fails.entry(ip).or_default().push_back(now);
                return Err(Error::new(401, "Código expirado. Gere um novo código no PC."));
            }
            Some((active_code, _)) if active_code != &code => {
                state.fails.entry(ip).or_default().push_back(now);
                return Err(Error::new(401, "Código incorreto. Confira os 6 dígitos."));
            }
            _ => {}
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
        state.fails.remove(&ip);
        Ok(json!({"token":token,"deviceId":dev["id"]}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pairing_code_survives_reopening_and_gets_full_window() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let auth = Auth::new(store, true);
        let now = Instant::now();
        let first = auth.start_at(now).await;
        let renewed = auth.start_at(now + Duration::from_secs(899)).await;
        assert_eq!(first["code"], renewed["code"]);
        assert_eq!(renewed["expiresIn"], 900);
        let body = json!({"code":first["code"],"deviceName":"Phone"});
        let ip = "192.168.0.9".parse().unwrap();
        let paired = auth.pair_at(&body, ip, now + Duration::from_secs(1798)).await.unwrap();
        assert!(!auth.verify(paired["token"].as_str().unwrap()).await.unwrap().is_null());
        assert!(auth.pair_at(&body, ip, now + Duration::from_secs(1798)).await.is_err());
    }

    #[tokio::test]
    async fn pairing_code_accepts_899_seconds_and_expires_at_900() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::new(Store::open(dir.path()).unwrap(), true);
        let now = Instant::now();
        let first = auth.start_at(now).await;
        let ip = "192.168.0.9".parse().unwrap();
        let body = json!({"code":first["code"]});
        let failure = auth.pair_at(&body, ip, now + Duration::from_secs(900)).await.unwrap_err();
        assert_eq!(failure.status, 401);
        assert_eq!(failure.message, "Código expirado. Gere um novo código no PC.");
        let fresh = auth.start_at(now + Duration::from_secs(901)).await;
        auth.pair_at(&json!({"code":fresh["code"]}), ip, now + Duration::from_secs(1800)).await.unwrap();
    }

    #[tokio::test]
    async fn pairing_code_remains_valid_well_past_five_minutes_and_accepts_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::new(Store::open(dir.path()).unwrap(), true);
        let now = Instant::now();
        let first = auth.start_at(now).await;
        let ip = "192.168.0.9".parse().unwrap();
        let code_str = first["code"].as_str().unwrap();
        let code_num: u64 = code_str.parse().unwrap();
        
        // 6 minutes (360s) has passed: under the old 300s TTL this would fail, but now succeeds!
        let body_numeric = json!({"code": code_num, "deviceName": "Phone"});
        let paired = auth.pair_at(&body_numeric, ip, now + Duration::from_secs(360)).await.unwrap();
        assert!(!auth.verify(paired["token"].as_str().unwrap()).await.unwrap().is_null());
    }

    #[tokio::test]
    async fn pairing_reports_accurate_error_messages_and_clears_fails() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::new(Store::open(dir.path()).unwrap(), true);
        let now = Instant::now();
        let first = auth.start_at(now).await;
        let ip = "192.168.0.9".parse().unwrap();
        
        // Wrong code
        let err_wrong = auth.pair_at(&json!({"code":"999999"}), ip, now).await.unwrap_err();
        assert_eq!(err_wrong.message, "Código incorreto. Confira os 6 dígitos.");
        
        // Expired code
        let err_exp = auth.pair_at(&json!({"code":first["code"]}), ip, now + Duration::from_secs(901)).await.unwrap_err();
        assert_eq!(err_exp.message, "Código expirado. Gere um novo código no PC.");
        
        // Successful pair clears previous failed attempts
        let fresh = auth.start_at(now + Duration::from_secs(902)).await;
        auth.pair_at(&json!({"code":fresh["code"]}), ip, now + Duration::from_secs(903)).await.unwrap();
        
        // No active code after consumption
        let err_none = auth.pair_at(&json!({"code":fresh["code"]}), ip, now + Duration::from_secs(904)).await.unwrap_err();
        assert_eq!(err_none.message, "Nenhum código ativo no momento. Gere um novo código no PC.");
    }
}
