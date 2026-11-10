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

/// Wrong guesses a single code tolerates before it is burned. Counted globally, not per IP: behind the
/// HTTPS gateway every client arrives from 127.0.0.1, and IPv6 address rotation defeats per-IP limits.
pub const MAX_CODE_GUESSES: u32 = 10;

/// DNS-rebinding guard. A browser always sends, in `Host`, the name it resolved. IP literals and
/// `localhost` cannot be rebound to this machine by a third party; any other name must be listed in
/// `MC_ALLOWED_HOSTS` (comma-separated, e.g. the HTTPS gateway's domain). A missing Host (non-browser
/// clients, in-process tests) cannot come from a rebound page.
pub fn host_allowed(host: &str) -> bool {
    static ALLOWED: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    host_allowed_in(host, ALLOWED.get_or_init(|| {
        std::env::var("MC_ALLOWED_HOSTS").unwrap_or_default().split(',')
            .map(|h| h.trim().trim_end_matches('.').to_ascii_lowercase()).filter(|h| !h.is_empty()).collect()
    }))
}
pub fn host_allowed_in(host: &str, allowed: &[String]) -> bool {
    let host = host.trim();
    if host.is_empty() {
        return true;
    }
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split_once(']').is_some_and(|(ip, _)| ip.parse::<std::net::Ipv6Addr>().is_ok());
    }
    let name = match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    };
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    name == "localhost" || name.parse::<std::net::Ipv4Addr>().is_ok() || allowed.iter().any(|a| *a == name)
}

#[derive(Default)]
struct PairState {
    code: Option<(String, Instant)>,
    /// Wrong guesses against the current code (see `MAX_CODE_GUESSES`).
    wrong: u32,
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
        let current = state.code.as_ref()
            .filter(|(_, expiry)| *expiry > now)
            .map(|(code, _)| code.clone());
        if current.is_none() {
            state.wrong = 0;
        }
        let code = current.unwrap_or_else(|| format!("{:06}", rand::random_range(0..1_000_000u32)));
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
        // Loopback is the gateway (shared by every remote client) or this machine: a per-IP lock there only
        // lets a stranger lock the owner out. The global guess budget below bounds brute force for everyone.
        let per_ip = !loopback(ip);
        if per_ip {
            if state.fails.len() >= 2048 && !state.fails.contains_key(&ip) {
                return Err(Error::new(429, "Muitas tentativas de pareamento."));
            }
            if state.fails.get(&ip).is_some_and(|f| f.len() >= 8) {
                return Err(Error::new(
                    429,
                    "Muitas tentativas; aguarde alguns minutos.",
                ));
            }
        }
        let fail = |state: &mut PairState| {
            if per_ip {
                state.fails.entry(ip).or_default().push_back(now);
            }
        };
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
                fail(&mut state);
                return Err(Error::new(401, "Nenhum código ativo no momento. Gere um novo código no PC."));
            }
            Some((_, expiry)) if *expiry <= now => {
                fail(&mut state);
                return Err(Error::new(401, "Código expirado. Gere um novo código no PC."));
            }
            Some((active_code, _)) if active_code != &code => {
                fail(&mut state);
                state.wrong += 1;
                if state.wrong >= MAX_CODE_GUESSES {
                    // Burn the code: at most MAX_CODE_GUESSES guesses per 1,000,000 combinations.
                    state.code = None;
                    return Err(Error::new(401, "Código invalidado após muitas tentativas. Gere um novo código no PC."));
                }
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

    fn wrong_code(code: &Value) -> Value {
        let n: u32 = code.as_str().unwrap().parse().unwrap();
        json!({"code": format!("{:06}", (n + 1) % 1_000_000)})
    }

    // Regression: behind the gateway every client is 127.0.0.1, and rotating IPv6 addresses bypassed the
    // per-IP limit. A code must be burned after MAX_CODE_GUESSES wrong guesses, whatever the source.
    #[tokio::test]
    async fn pairing_code_is_burned_after_global_guess_budget_across_ips() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::new(Store::open(dir.path()).unwrap(), true);
        let now = Instant::now();
        let first = auth.start_at(now).await;
        for i in 0..MAX_CODE_GUESSES {
            let ip: IpAddr = format!("fd00::{:x}", i + 1).parse().unwrap();
            assert_eq!(auth.pair_at(&wrong_code(&first["code"]), ip, now).await.unwrap_err().status, 401);
        }
        let other: IpAddr = "fd00::ffff".parse().unwrap();
        let burned = auth.pair_at(&json!({"code": first["code"]}), other, now).await.unwrap_err();
        assert_eq!(burned.message, "Nenhum código ativo no momento. Gere um novo código no PC.");
        let fresh = auth.start_at(now).await;
        auth.pair_at(&json!({"code": fresh["code"]}), other, now).await.unwrap();
    }

    // Regression: the gateway's shared 127.0.0.1 must not be locked by a stranger's failed attempts.
    #[tokio::test]
    async fn loopback_is_not_locked_out_by_per_ip_failures() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::new(Store::open(dir.path()).unwrap(), true);
        let now = Instant::now();
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        for _ in 0..8 {
            let _ = auth.pair_at(&json!({"code":"000000"}), ip, now).await;
        }
        let code = auth.start_at(now).await;
        auth.pair_at(&json!({"code": code["code"]}), ip, now).await.unwrap();
    }

    #[test]
    fn host_allowlist_blocks_rebound_names_only() {
        let allowed = vec!["chat.example.org".to_string()];
        for ok in ["", "localhost", "localhost:3000", "127.0.0.1:3000", "192.168.1.5", "[::1]:3000",
                   "Chat.Example.org:8443", "chat.example.org."] {
            assert!(host_allowed_in(ok, &allowed), "{ok}");
        }
        for bad in ["evil.com", "evil.com:3000", "localhost.evil.com", "127.0.0.1.nip.io", "[evil]:1"] {
            assert!(!host_allowed_in(bad, &allowed), "{bad}");
        }
    }
}
