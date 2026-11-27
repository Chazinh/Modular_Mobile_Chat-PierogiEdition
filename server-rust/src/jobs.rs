//! Persisted jobs; one supervisor, explicit mobile backend policy and source/version checks.
use crate::{
    Error, Result,
    local::LocalModels,
    memory_backend::{MemoryBackend, MemoryMode, MemoryPolicy, PolicySnapshot},
    providers::RuntimeProviders,
    review, rules, s,
    store::{EmbeddingItem, Request, Store},
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, Notify, broadcast, watch};
#[derive(Default)]
struct State {
    running: bool,
    last_error: Option<String>,
}

#[derive(Default)]
struct Cooldown {
    unavailable_until: Option<tokio::time::Instant>,
    unavailable_streak: u32,
    unavailable_kind: Option<String>,
}

fn is_unavailable_code(err: &Error) -> bool {
    matches!(
        err.extra["code"].as_str(),
        Some(
            "LOCAL_UNAVAILABLE"
                | "LOCAL_DOWNLOADING"
                | "MEMORY_BACKEND_UNAVAILABLE"
                | "PROVIDER_UNAVAILABLE"
        )
    )
}

fn compute_cooldown(streak: u32, kind: &str, retry_after_secs: Option<u64>) -> Duration {
    if let Some(ra) = retry_after_secs {
        return Duration::from_secs(ra.clamp(1, 900));
    }
    if kind == "auth_config" {
        let schedule = [300u64, 600, 900];
        let base = schedule[(streak.saturating_sub(1) as usize).min(schedule.len() - 1)];
        let extra = ((base as f64) * rand::random_range(0.0..=0.10)).round() as u64;
        return Duration::from_secs((base + extra).clamp(1, 900));
    }
    let schedule = [30u64, 60, 120, 300, 900];
    let base = schedule[(streak.saturating_sub(1) as usize).min(schedule.len() - 1)];
    let factor = rand::random_range(0.85f64..=1.15f64);
    let ms = ((base as f64) * factor * 1000.0).round().max(1000.0) as u64;
    Duration::from_millis(ms).min(Duration::from_secs(900))
}

/// Keep memory paused only while a sync request owns the guard. Concurrent
/// requests share the pause; returning an HTTP error or cancelling also releases it.
pub struct SyncPauseGuard {
    pause: watch::Sender<tokio::time::Instant>,
    users: Arc<std::sync::Mutex<usize>>,
    wake: Arc<Notify>,
}
impl Drop for SyncPauseGuard {
    fn drop(&mut self) {
        let mut users = self.users.lock().unwrap();
        *users -= 1;
        if *users == 0 {
            // Adjacent snapshot requests share one quiet burst. This ends well
            // before the next periodic connection check.
            self.pause.send_replace(tokio::time::Instant::now() + Duration::from_secs(1));
            self.wake.notify_one();
        }
    }
}
/// Parsed once; every reviewed chunk reuses it.
fn review_schema() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::from_str(include_str!("review-schema.json"))
            .expect("bundled review schema must be valid JSON")
    })
}
fn extraction_schema() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::from_str(include_str!("extraction-schema.json"))
            .expect("bundled extraction schema must be valid JSON")
    })
}
#[derive(Clone)]
pub struct Runner {
    store: Store,
    local: LocalModels,
    wake: Arc<Notify>,
    events: broadcast::Sender<Value>,
    state: Arc<std::sync::Mutex<State>>,
    stop: watch::Sender<bool>,
    review: bool,
    providers: RuntimeProviders,
    policy: PolicySnapshot,
    client: reqwest::Client,
    sync_pause: watch::Sender<tokio::time::Instant>,
    cycle_gate: Arc<Mutex<()>>,
    sync_users: Arc<std::sync::Mutex<usize>>,
    cooldown: Arc<std::sync::Mutex<Cooldown>>,
    config_epoch: Arc<AtomicU64>,
}
impl Runner {
    pub fn new(
        store: Store,
        local: LocalModels,
        wake: Arc<Notify>,
        events: broadcast::Sender<Value>,
        review: bool,
    ) -> Self {
        let (stop, _) = watch::channel(false);
        let (sync_pause, _) = watch::channel(tokio::time::Instant::now());
        let policy = Arc::new(std::sync::RwLock::new(MemoryPolicy {
            mode: if local.is_mobile() {
                MemoryMode::Deferred
            } else {
                MemoryMode::Local
            },
            api: None,
        }));
        Self {
            store,
            local,
            wake,
            events,
            state: Arc::default(),
            stop,
            review,
            providers: RuntimeProviders::default(),
            policy,
            client: reqwest::Client::new(),
            sync_pause,
            cycle_gate: Arc::default(),
            sync_users: Arc::default(),
            cooldown: Arc::default(),
            config_epoch: Arc::default(),
        }
    }
    pub fn set_backends(
        &mut self,
        providers: RuntimeProviders,
        policy: PolicySnapshot,
        client: reqwest::Client,
    ) {
        self.providers = providers;
        self.policy = policy;
        self.client = client;
    }
    fn cooldown_until(&self) -> Option<tokio::time::Instant> {
        let mut cd = self.cooldown.lock().unwrap();
        match cd.unavailable_until {
            Some(u) if u > tokio::time::Instant::now() => Some(u),
            Some(_) => {
                cd.unavailable_until = None;
                None
            }
            None => None,
        }
    }
    fn in_cooldown(&self) -> bool {
        self.cooldown_until().is_some()
    }
    fn record_unavailable(&self, err: &Error) {
        let kind = if err.extra["unavailableKind"].as_str() == Some("auth_config")
            || err.extra["code"].as_str() == Some("PROVIDER_UNAVAILABLE")
        {
            "auth_config"
        } else if matches!(
            err.extra["code"].as_str(),
            Some("LOCAL_UNAVAILABLE" | "LOCAL_DOWNLOADING")
        ) {
            "local"
        } else {
            "transient"
        };
        let retry_after = err.extra["retryAfterSeconds"].as_u64();
        let mut cd = self.cooldown.lock().unwrap();
        let streak = cd.unavailable_streak.saturating_add(1);
        let delay = compute_cooldown(streak, kind, retry_after);
        cd.unavailable_until = Some(tokio::time::Instant::now() + delay);
        cd.unavailable_streak = streak;
        cd.unavailable_kind = Some(kind.to_owned());
    }
    fn clear_cooldown(&self) {
        let mut cd = self.cooldown.lock().unwrap();
        cd.unavailable_until = None;
        cd.unavailable_streak = 0;
        cd.unavailable_kind = None;
    }
    pub fn reset_cooldown(&self) {
        self.config_epoch.fetch_add(1, Ordering::AcqRel);
        self.clear_cooldown();
        self.state.lock().unwrap().last_error = None;
        self.wake.notify_one();
    }
    pub async fn state(&self) -> Value {
        let (running, last_error) = {
            let s = self.state.lock().unwrap();
            (s.running, s.last_error.clone())
        };
        let (cooldown_secs, streak, kind) = {
            let mut cd = self.cooldown.lock().unwrap();
            let now = tokio::time::Instant::now();
            let rem = match cd.unavailable_until {
                Some(u) if u > now => (u - now).as_secs().max(1),
                Some(_) => {
                    cd.unavailable_until = None;
                    0
                }
                None => 0,
            };
            (rem, cd.unavailable_streak, cd.unavailable_kind.clone())
        };
        json!({"running":running,"lastError":last_error,"policy":self.policy.read().unwrap().clone(),"concurrency":1,
            "pausedForSync":*self.sync_pause.borrow() > tokio::time::Instant::now(),
            "cooldownRemainingSeconds":cooldown_secs,"unavailableStreak":streak,"unavailableKind":kind})
    }
    pub fn start(&self) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this.store.call(Request::RequeueJobs).await {
                this.state.lock().unwrap().last_error = Some(e.message);
            }
            let mut stop = this.stop.subscribe();
            let mut pause = this.sync_pause.subscribe();
            let mut timer = tokio::time::interval(Duration::from_secs(30));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut run_immediately = false;
            loop {
                let until = *pause.borrow_and_update();
                if until > tokio::time::Instant::now() {
                    run_immediately = false;
                    tokio::select! {
                        _=stop.wait_for(|s|*s)=>break,
                        _=pause.changed()=>{},
                        _=tokio::time::sleep_until(until)=>this.wake.notify_one(),
                    }
                    continue;
                }
                if let Some(cd_until) = this.cooldown_until() {
                    run_immediately = false;
                    tokio::select! {
                        _=stop.wait_for(|s|*s)=>break,
                        _=pause.changed()=>{},
                        _=this.wake.notified()=>{
                            if this.cooldown_until().is_none() {
                                run_immediately = true;
                            }
                        },
                        _=tokio::time::sleep_until(cd_until)=>{
                            run_immediately = true;
                        },
                    }
                    continue;
                }
                if !run_immediately {
                    tokio::select! {_=stop.wait_for(|s|*s)=>break,_=this.wake.notified()=>{},_=timer.tick()=>{},_=pause.changed()=>continue}
                }
                run_immediately = false;
                this.state.lock().unwrap().running = true;
                loop {
                    if *stop.borrow() {
                        break;
                    }
                    let work = this.cycle();
                    let result = tokio::select! {_=stop.wait_for(|s|*s)=>break,v=work=>v};
                    match result {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(e) => {
                            this.state.lock().unwrap().last_error = Some(e.message);
                            break;
                        }
                    }
                }
                this.state.lock().unwrap().running = false;
            }
            this.state.lock().unwrap().running = false;
        })
    }
    pub fn stop(&self) {
        self.stop.send_replace(true);
    }
    pub fn kick(&self) {
        self.wake.notify_one();
    }
    /// Synchronization has priority over derived memory. Drop only the background
    /// inference future, refund/requeue its job, and wait for Store writes to settle.
    /// The short renewable lease expires even if the client disconnects.
    pub async fn pause_for_sync(&self) -> SyncPauseGuard {
        self.pause_for_sync_for(Duration::from_secs(30)).await
    }
    async fn pause_for_sync_for(&self, duration: Duration) -> SyncPauseGuard {
        {
            let mut users = self.sync_users.lock().unwrap();
            *users += 1;
            self.sync_pause.send_replace(tokio::time::Instant::now() + duration);
        }
        // Construct before awaiting the active cycle so cancellation cannot leak a pause.
        let guard = SyncPauseGuard { pause: self.sync_pause.clone(), users: self.sync_users.clone(), wake: self.wake.clone() };
        self.wake.notify_one();
        let _gate = self.cycle_gate.lock().await;
        guard
    }
    async fn cycle(&self) -> Result<bool> {
        let _gate = self.cycle_gate.lock().await;
        let mut pause = self.sync_pause.subscribe();
        if *pause.borrow() > tokio::time::Instant::now() { return Ok(false); }
        if self.in_cooldown() {
            if self.local.embed_available() {
                let _ = self.embeddings().await;
            }
            return Ok(false);
        }
        self.cycle_work(&mut pause).await
    }
    async fn cycle_work(&self, pause: &mut watch::Receiver<tokio::time::Instant>) -> Result<bool> {
        let epoch = self.config_epoch.load(Ordering::Acquire);
        let policy = self.policy.read().unwrap().clone();
        let backend = match MemoryBackend::resolve(&self.local, &self.providers, &policy, &self.client).await {
            Ok(Some(b)) => b,
            Ok(None) => return Ok(false),
            Err(e) => {
                if self.config_epoch.load(Ordering::Acquire) != epoch {
                    return Ok(true);
                }
                if is_unavailable_code(&e) {
                    self.record_unavailable(&e);
                }
                return Err(e);
            }
        };
        if !backend.available() {
            return Ok(false);
        }
        let job = self.store.call(Request::ClaimJob).await?;
        if job.is_null() {
            return tokio::select! {
                biased;
                _=pause.changed()=>Ok(false),
                result=self.embeddings()=>Ok(result? > 0),
            };
        }
        let outcome = tokio::select! {
            biased;
            _=pause.changed()=> {
                self.store.call_retrying(|| Request::DeferJob { id: s(&job,"id"), error: "Pausado temporariamente para sincronização.".into() }).await?;
                return Ok(false);
            },
            outcome=self.handle(&job, &backend)=>outcome,
        };
        let (status, error, continue_work) = match outcome {
            Ok(status) => {
                self.clear_cooldown();
                self.state.lock().unwrap().last_error = None;
                (status, None, true)
            }
            Err(e) if is_unavailable_code(&e) => {
                if self.config_epoch.load(Ordering::Acquire) != epoch {
                    self.store
                        .call_retrying(|| Request::DeferJob {
                            id: s(&job, "id"),
                            error: "Configuração atualizada durante a execução; reagendando."
                                .into(),
                        })
                        .await?;
                    return Ok(true);
                }
                self.record_unavailable(&e);
                self.state.lock().unwrap().last_error = Some(e.message.clone());
                let is_auth_or_config = e.extra["unavailableKind"].as_str() == Some("auth_config")
                    || e.extra["code"].as_str() == Some("PROVIDER_UNAVAILABLE");
                if is_auth_or_config {
                    let status = if job["attempts"].as_u64().unwrap_or(1) >= 3 {
                        "failed"
                    } else {
                        "pending"
                    };
                    self.store
                        .call_retrying(|| Request::FinishJob {
                            id: s(&job, "id"),
                            status: status.into(),
                            error: Some(e.message.clone()),
                        })
                        .await?;
                } else {
                    self.store
                        .call_retrying(|| Request::DeferUnavailableJob {
                            id: s(&job, "id"),
                            error: e.message.clone(),
                            max_deferrals: 8,
                        })
                        .await?;
                }
                return Ok(false);
            }
            Err(e) => {
                let status = if job["attempts"].as_u64().unwrap_or(1) >= 3 {
                    "failed"
                } else {
                    "pending"
                };
                // Kind and outcome only: the error text may quote model output.
                tracing::warn!(
                    kind = job["kind"].as_str().unwrap_or("?"),
                    attempts = job["attempts"].as_u64().unwrap_or(0),
                    code = e.status,
                    next = status,
                    "tarefa de memória falhou"
                );
                self.state.lock().unwrap().last_error = Some(e.message.clone());
                (status.into(), Some(e.message), true)
            }
        };
        let done_turn = job["kind"] == "turn" && status == "done";
        // A lost FinishJob left the job `running`: sync answered BUSY for that chat until a restart.
        self.store
            .call_retrying(|| Request::FinishJob {
                id: s(&job, "id"),
                status: status.clone(),
                error: error.clone(),
            })
            .await?;
        if done_turn {
            // cheap (counts only); schedules curation only when there are too many memories and enough new ones
            let _ = self
                .store
                .call(Request::MaybeQueueCuration(s(&job, "conv_id")))
                .await;
        }
        // Embeddings receive a share even with a continuous backlog of turns.
        if *pause.borrow_and_update() > tokio::time::Instant::now() { return Ok(false); }
        let embeddings = tokio::select! {
            biased;
            _=pause.changed()=>return Ok(false),
            result=self.embeddings()=>result,
        };
        if let Err(e) = embeddings {
            self.state.lock().unwrap().last_error = Some(e.message);
        }
        Ok(continue_work)
    }
    async fn handle(&self, job: &Value, backend: &MemoryBackend) -> Result<String> {
        match job["kind"].as_str() {
            Some("turn") => self.turn(job, backend).await,
            Some("episode") => self.episode(job, backend).await,
            Some("cast") => self.cast(job, backend).await,
            Some("curate") => self.curate(job, backend).await,
            _ => Ok("skipped".into()),
        }
    }
    /// Memory curation: groups near-duplicate memories (deterministic decision) and asks the local model,
    /// in a single call, only which one to keep and the summary sentence. Without groups, the model is not called.
    async fn curate(&self, job: &Value, backend: &MemoryBackend) -> Result<String> {
        if !backend.available() {
            return Err(
                Error::new(503, "Modelo local indisponível.").extra("code", "LOCAL_UNAVAILABLE")
            );
        }
        let c = s(job, "conv_id");
        let found = self
            .store
            .call(Request::CurationCandidates(c.clone()))
            .await?;
        let clusters: Vec<Vec<Value>> = found["clusters"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|g| g.as_array().cloned().unwrap_or_default())
            .collect();
        if clusters.is_empty() {
            self.store
                .call(Request::ApplyCuration(json!({"convId":c,"decisions":[]})))
                .await?;
            return Ok("skipped".into());
        }
        let mut decisions = Vec::new();
        // Complete groups keep their ids/provenance. Mobile processes one group per prompt;
        // no partial mutation is applied if a later group fails its schema/budget.
        let batches = if backend.compact() {
            clusters.chunks(1).collect::<Vec<_>>()
        } else {
            vec![clusters.as_slice()]
        };
        for batch in batches {
            let raw = backend
                .chat_json(
                    crate::curation::messages(batch),
                    crate::curation::schema(),
                    900,
                    0.1,
                )
                .await?;
            decisions.extend(crate::curation::sanitize(&raw, batch));
        }
        let out = self
            .store
            .call(Request::ApplyCuration(
                json!({"convId":c,"decisions":decisions}),
            ))
            .await?;
        if out["merged"].as_u64().unwrap_or(0) > 0 {
            let _ = self.events.send(json!({"convId":c,"type":"memory"}));
        }
        Ok("done".into())
    }
    async fn current(&self, job: &Value, expected: &Value) -> Result<bool> {
        let input = self.store.call(Request::JobInput(s(job, "id"))).await?;
        Ok(input["gone"] != true && input["versionIds"] == expected["versionIds"])
    }
    async fn turn(&self, job: &Value, backend: &MemoryBackend) -> Result<String> {
        let input = self.store.call(Request::JobInput(s(job, "id"))).await?;
        if input["gone"] == true || s(&input, "text").trim().is_empty() {
            return Ok("skipped".into());
        }
        if !backend.available() {
            return Err(
                Error::new(503, "Modelo local indisponível.").extra("code", "LOCAL_UNAVAILABLE")
            );
        }
        let secrets = input["related"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["kind"] == "SECRET")
            .take(8)
            .map(|m| format!("{} ({})", s(m, "content"), m["knowledge"]))
            .collect::<Vec<_>>();
        let assistant = s(&input, "assistantText");
        if self.review && job["payload"]["skipReview"] != true {
            let mut flags = Vec::new();
            for (start, _, text) in
                rules::chunks(&assistant, if backend.compact() { 800 } else { 8000 }, 120)
            {
                let mut prefix = format!(
                    "{}\n{}",
                    s(&input, "prelude"),
                    &assistant[..rules::byte_at(&assistant, start)]
                );
                let mut review_input = input.clone();
                if backend.compact() {
                    prefix = compact_excerpt(&prefix, 300);
                    review_input["previous"] = json!(compact_excerpt(&s(&input, "previous"), 400));
                    review_input["scene"] =
                        json!(compact_excerpt(&input["scene"].to_string(), 400));
                }
                let raw = backend
                    .chat_json(
                        review::messages(&review_input, &text, &prefix, &secrets),
                        review_schema().clone(),
                        700,
                        0.1,
                    )
                    .await?;
                if !["agency", "knowledge", "continuity"]
                    .iter()
                    .all(|k| raw[k].is_array())
                {
                    return Err(Error::new(502, "Revisão local incompleta."));
                }
                let preceding = format!(
                    "{}\n\n{}\n\n{prefix}",
                    s(&input, "previous"),
                    s(&input, "userText")
                );
                flags.extend(review::supported_with_sources(
                    &raw, &text, &preceding, &secrets, &input,
                ));
            }
            if !flags.is_empty() {
                if !self.current(job, &input).await? {
                    return Ok("skipped".into());
                }
                self.store.call(Request::AddFlags(json!({"convId":job["conv_id"],"jobId":job["id"],"versionId":input["versionId"],"flags":flags,"source":"llm"}))).await?;
                let _ = self
                    .events
                    .send(json!({"convId":job["conv_id"],"type":"flags"}));
                return Ok("blocked".into());
            }
        }
        let full = s(&input, "text");
        let mut ext = json!({"summary":"","cast":[],"scene":null,"scenes":[],"facts":[],"relationships":[],"openThreads":[],"resolvedThreads":[]});
        let mut scene = input["scene"].clone();
        let mut related = input["related"].clone();
        // Each retry uses a smaller context: the local Qwen has an 8,192-token window and a long turn with history,
        // retrieval and sources already overflowed it (HTTP 400) or the JSON went over budget. 0 = full; 1 = reduced; 2 = minimal.
        let level = job["attempts"]
            .as_u64()
            .unwrap_or(1)
            .saturating_sub(1)
            .min(2);
        let tail = |t: &str, n: usize| -> String {
            let chars = t.chars().count();
            if !backend.compact() && level == 0 || chars <= n {
                t.to_string()
            } else {
                t.chars().skip(chars - n).collect()
            }
        };
        let chunk_size = if backend.compact() {
            match level {
                0 => 1000,
                1 => 700,
                _ => 450,
            }
        } else if level == 0 {
            4000
        } else if level == 1 {
            2800
        } else {
            1800
        };
        for (_, _, text) in rules::chunks(&full, chunk_size, 100) {
            let retrieval = if level == 0 && !backend.compact() {
                self.store.call(Request::Retrieve(json!({"convId":job["conv_id"],"queryText":text,"beforeSeq":input["interaction"]["seq"],"limit":8}))).await?
            } else {
                json!({"candidates":[]})
            };
            let sources = if level >= 2 && !backend.compact() {
                Vec::new()
            } else {
                input["recentSources"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            };
            let prompt = if backend.compact() {
                format!(
                    "PERSONA: {}\nCONTEXTO PARCIAL (não são fontes de citações):\nCENA: {}\nFATOS: {}\nHISTÓRICO: {}\n{}\nTEXTO DO TURNO (fragmento integral):\n{text}",
                    input["conv"]["persona"],
                    compact_excerpt(&scene.to_string(), 300),
                    compact_excerpt(&related.to_string(), 300),
                    tail(&s(&input, "previous"), 400),
                    crate::provenance::block(&s(&input, "userText"), &sources)
                )
            } else {
                crate::memory_prompt::desktop(
                    &input,
                    &scene,
                    &related,
                    &retrieval,
                    &text,
                    &tail(&s(&input, "previous"), 1500),
                    &sources,
                )
            };
            let messages = vec![
                json!({"role":"system","content":if backend.compact() { compact_extraction_prompt(&s(&input["conv"], "persona")) } else { include_str!("extraction-prompt.txt").replace("{{persona}}", input["conv"]["persona"].as_str().unwrap_or("o usuário")) }}),
                json!({"role":"user","content":prompt}),
            ];
            let raw = backend
                .chat_json(
                    messages,
                    extraction_schema().clone(),
                    if backend.compact() {
                        1200
                    } else if level == 0 {
                        3500
                    } else {
                        2800
                    },
                    0.1,
                )
                .await?;
            let part = sanitize_with_sources(&raw, &input)?;
            // A rejected fact means the summary generated with it may carry the same false authorship.
            // Nothing of the batch is applied: the error follows the pending/failed/retry flow.
            if let Some(rejected) = part["attributionRejected"]
                .as_array()
                .filter(|r| !r.is_empty())
            {
                return Err(attribution_conflict(rejected));
            }
            let summary = format!(
                "{}{}{}",
                s(&ext, "summary"),
                if s(&ext, "summary").is_empty() {
                    ""
                } else {
                    " "
                },
                s(&part, "summary")
            );
            ext["summary"] = json!(summary);
            for k in [
                "cast",
                "facts",
                "relationships",
                "openThreads",
                "resolvedThreads",
            ] {
                ext[k]
                    .as_array_mut()
                    .unwrap()
                    .extend(part[k].as_array().unwrap().iter().cloned());
            }
            if !part["scene"].is_null() {
                scene = part["scene"].clone();
                ext["scene"] = scene.clone();
                ext["scenes"].as_array_mut().unwrap().push(scene.clone());
            }
            let present = scene["present"].as_array().cloned().unwrap_or_default();

            for f in ext["facts"].as_array_mut().unwrap() {
                if f.get("witnesses").is_some() {
                    continue;
                }
                let subject = s(f, "subject");
                let content = s(f, "content");
                let speaker = f["attribution"]["originalSpeaker"].as_str().unwrap_or_default();
                let visibility = f["visibility"].as_str().unwrap_or("public");
                let kind = f["kind"].as_str().unwrap_or("EVENT");

                let mut deterministic_witnesses = HashSet::new();

                // 1. Explicit knowledge attribution with valid witnessed state
                if let Some(knows) = f["knowledge"].as_array() {
                    for k in knows {
                        let who = s(k, "who");
                        let state = k["state"].as_str().unwrap_or("knows");
                        let via = k["via"].as_str().unwrap_or("witnessed");
                        if state != "unaware" && via == "witnessed" && !who.is_empty() {
                            let w_key = rules::key(&who);
                            if present.iter().any(|p| rules::key(p.as_str().unwrap_or_default()) == w_key) {
                                deterministic_witnesses.insert(who);
                            }
                        }
                    }
                }

                // 2. Direct participants (subject, content, speaker) who are physically present
                for p_val in &present {
                    let p_name = p_val.as_str().unwrap_or_default();
                    let p_key = rules::key(p_name);
                    if p_key.is_empty() {
                        continue;
                    }
                    let in_subject = !subject.is_empty() && rules::key(&subject).contains(&p_key);
                    let in_content = !content.is_empty() && rules::key(&content).contains(&p_key);
                    let is_speaker = !speaker.is_empty() && rules::key(speaker) == p_key;

                    if in_subject || in_content || is_speaker {
                        deterministic_witnesses.insert(p_name.to_string());
                    }
                }

                // 3. For public, non-secret events without specific individual participants,
                // present characters witness the public scene event.
                // Private/secret events are NEVER defaulted to all present!
                if deterministic_witnesses.is_empty() && visibility != "private" && kind != "SECRET" {
                    for p_val in &present {
                        if let Some(p_name) = p_val.as_str() {
                            deterministic_witnesses.insert(p_name.to_string());
                        }
                    }
                }

                let mut w_list: Vec<String> = deterministic_witnesses.into_iter().collect();
                w_list.sort();
                f["witnesses"] = json!(w_list);
            }
            related = part["facts"].clone();
        }
        let result=self.store.call(Request::ApplyExtraction(json!({"convId":job["conv_id"],"jobId":job["id"],"interactionId":input["interaction"]["id"],"versionId":input["versionId"],"versionIds":input["versionIds"],"sourceText":full,"extraction":ext}))).await?;
        if result["skipped"] == true {
            return Ok("skipped".into());
        }
        self.store
            .call(Request::QueueEpisodes(s(job, "conv_id")))
            .await?;
        let _ = self
            .events
            .send(json!({"convId":job["conv_id"],"type":"memory"}));
        Ok("done".into())
    }
    async fn episode(&self, job: &Value, backend: &MemoryBackend) -> Result<String> {
        let payload = &job["payload"];
        let ids = payload["interactionIds"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if ids.is_empty() {
            return Ok("skipped".into());
        }
        let notes = payload["notes"].as_array().cloned().unwrap_or_default();
        let conv = self
            .store
            .call(Request::GetMeta(s(job, "conv_id")))
            .await
            .unwrap_or_default();
        let persona = conv["persona"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("Alex");
        let mut sources = Vec::new();
        for iid in &ids {
            let source = self
                .store
                .call(Request::Source {
                    conv_id: s(job, "conv_id"),
                    interaction_id: iid.as_str().unwrap_or_default().to_owned(),
                })
                .await?;
            if source["deleted"] == true {
                return Ok("skipped".into());
            }
            for entry in source["sources"].as_array().into_iter().flatten() {
                sources.push(crate::provenance::source(
                    &s(entry, "id"),
                    &s(entry, "role"),
                    &s(entry, "text"),
                    None,
                ));
            }
        }
        // Notes determine coverage/version eligibility; the source text, including
        // every continuation, is the input. Summarizing summaries loses the tail.
        let original = sources.join("\n\n");
        let mut summaries = Vec::new();
        if backend.available() {
            let prompt = episode_prompt(persona, backend.compact());
            let chunk_size = if backend.compact() { 1400 } else { 5000 };
            let overlap = if backend.compact() { 100 } else { 200 };
            for (_, _, text) in rules::chunks(&original, chunk_size, overlap) {
                let raw = backend
                    .chat_json(
                        vec![
                            json!({"role":"system","content":prompt}),
                            json!({"role":"user","content":text}),
                        ],
                        json!({
                            "type": "object",
                            "properties": {
                                "summary": {"type": "string"}
                            },
                            "required": ["summary"]
                        }),
                        if backend.compact() { 400 } else { 600 },
                        0.2,
                    )
                    .await?;
                let summary = sanitize_episode_summary(&raw)
                    .ok_or_else(|| Error::new(502, "Resumo episódico vazio."))?;
                summaries.push(summary);
            }
        } else {
            summaries.push(original);
        }
        let final_summary = summaries.join(" ");
        let result = self
            .store
            .call(Request::CreateEpisode(json!({
                "convId": job["conv_id"],
                "interactionIds": ids,
                "fromSeq": payload["fromSeq"],
                "toSeq": payload["toSeq"],
                "summary": final_summary,
                "expectedNotes": notes
            })))
            .await?;
        Ok(if result["skipped"] == true {
            "skipped"
        } else {
            "done"
        }
        .into())
    }
    async fn cast(&self, job: &Value, backend: &MemoryBackend) -> Result<String> {
        let conv = self.store.call(Request::GetMeta(s(job, "conv_id"))).await?;
        let text = s(&conv, "prompt");
        let heuristic = crate::local::cast_from_card(&text, &s(&conv, "persona"));
        self.store
            .call(Request::ReanalyzeCast(s(job, "conv_id")))
            .await?;
        if heuristic.len() < 2 && !text.trim().is_empty() && backend.available() {
            for (_, _, text) in
                rules::chunks(&text, if backend.compact() { 1200 } else { 5000 }, 100)
            {
                let raw=backend.chat_json(vec![json!({"role":"system","content":include_str!("cast-prompt.txt").replace("{{persona}}", conv["persona"].as_str().unwrap_or("o usuário"))}),json!({"role":"user","content":format!("PERSONA: {}\nFICHA:\n{text}",conv["persona"])})],serde_json::from_str(include_str!("cast-schema.json")).unwrap(),if backend.compact() { 1000 } else { 1500 },0.1).await?;
                if let Some(entries) = raw["cast"].as_array() {
                    for entry in entries {
                        let Some(name) = crate::cast::name(&s(entry, "name")) else {
                            continue;
                        };
                        if rules::key(&name) == rules::key(&s(&conv, "persona")) {
                            continue;
                        }
                        let mut entry = entry.clone();
                        entry["name"] = json!(name);
                        entry["aliases"] = json!(
                            entry["aliases"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|a| crate::cast::name(a.as_str().unwrap_or_default()))
                                .take(6)
                                .collect::<Vec<_>>()
                        );
                        entry["kind"] = json!(if entry["main"] == true { "main" } else { "npc" });
                        entry["source"] = json!("card");
                        entry["confirmed"] = json!(false);
                        // A changed card invalidates the result of an older model call.
                        if self.store.call(Request::GetMeta(s(job, "conv_id"))).await?["characterRev"]
                            != conv["characterRev"]
                        {
                            return Ok("skipped".into());
                        }
                        self.store
                            .call(Request::SaveCast {
                                conv_id: s(job, "conv_id"),
                                value: entry,
                            })
                            .await?;
                    }
                }
            }
        }
        let _ = self
            .events
            .send(json!({"convId":job["conv_id"],"type":"cast"}));
        Ok("done".into())
    }
    async fn embeddings(&self) -> Result<usize> {
        if self.local.is_mobile()
            && (!self.local.mobile_activated()
                || self.policy.read().unwrap().mode != MemoryMode::Local)
        {
            return Ok(0);
        }
        if !self.local.embed_available() {
            return Ok(0);
        }
        let model = self.local.fingerprint().await;
        let items = self
            .store
            .call(Request::PendingEmbeddings {
                limit: 4,
                model: model.clone(),
            })
            .await?;
        // One cycle becomes a single transaction; if a vector fails, the ones already computed are saved before the error.
        let mut batch = Vec::new();
        let mut failure = None;
        for item in items.as_array().into_iter().flatten() {
            let text = s(item, "text");
            match self.local.embed_text(&text, false).await {
                Ok(vector) => batch.push(EmbeddingItem {
                    conv_id: s(item, "conv_id"),
                    owner_type: s(item, "t"),
                    owner_id: s(item, "id"),
                    model: model.clone(),
                    source_text: Some(text),
                    vector,
                }),
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        let saved = if batch.is_empty() {
            0
        } else {
            self.store
                .call(Request::SaveEmbeddings(batch))
                .await?
                .as_u64()
                .unwrap_or(0) as usize
        };
        match failure {
            Some(e) => Err(e),
            None => Ok(saved),
        }
    }
}

#[cfg(test)]
mod mobile_backend_tests {
    use super::*;
    use crate::{config::Config, providers::MobileProviders};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn sync_pause_is_shared_and_cancellation_cannot_leave_memory_paused() {
        let dir = tempfile::tempdir().unwrap();
        let native = tempfile::tempdir().unwrap();
        let config = Config::for_android(dir.path(), native.path()).unwrap();
        let store = Store::open_for_config(&config).unwrap();
        let local = LocalModels::new(&config, reqwest::Client::new()).unwrap();
        let (events, _) = broadcast::channel(8);
        let runner = Runner::new(store.clone(), local, Arc::new(Notify::new()), events, false);
        let first = runner.pause_for_sync().await;
        let second = runner.pause_for_sync().await;
        drop(first);
        assert_eq!(runner.state().await["pausedForSync"], true, "another sync request still owns the pause");
        drop(second);
        assert_eq!(*runner.sync_users.lock().unwrap(), 0);
        let gate = runner.cycle_gate.lock().await;
        let waiting_runner = runner.clone();
        let waiting = tokio::spawn(async move { waiting_runner.pause_for_sync().await });
        tokio::task::yield_now().await;
        assert_eq!(runner.state().await["pausedForSync"], true);
        waiting.abort(); let _ = waiting.await;
        assert_eq!(*runner.sync_users.lock().unwrap(), 0, "cancelled gate acquisition must release its pause");
        drop(gate);
        store.call(Request::Close).await.unwrap();
    }

    #[tokio::test]
    async fn synchronization_preempts_memory_and_resumes_same_job_without_spending_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let native = tempfile::tempdir().unwrap();
        let config = Config::for_android(dir.path(), native.path()).unwrap();
        let store = Store::open_for_config(&config).unwrap();
        let client = reqwest::Client::new();
        let local = LocalModels::new(&config, client.clone()).unwrap();
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let app_started = started.clone();
        let app_release = release.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = axum::Router::new().fallback(move |request: axum::extract::Request| {
            let started = app_started.clone();
            let release = app_release.clone();
            async move {
                if request.uri().path().ends_with("/models") {
                    return axum::Json(json!({"data":[{"id":"exact"}]}));
                }
                started.notify_one();
                release.notified().await;
                axum::Json(json!({}))
            }
        });
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let conv = store.call(Request::CreateConversation(json!({"routeId":"cbai/glm-5.2","presetId":"glm-52","prompt":"Uma viajante curiosa."}))).await.unwrap();
        let job = store.call(Request::EnqueueJob(json!({"convId":conv["id"],"kind":"cast"}))).await.unwrap();
        let (events, _) = broadcast::channel(8);
        let mut runner = Runner::new(store.clone(), local.clone(), Arc::new(Notify::new()), events, false);
        let runtime = RuntimeProviders::new(MobileProviders::parse(&json!({"providers":[{"id":"fixture","baseUrl":url,"models":["exact"],"allowLoopback":true}]})).unwrap().runtime().unwrap());
        let policy = Arc::new(std::sync::RwLock::new(MemoryPolicy::parse(&json!({"mode":"api","api":{"provider":"fixture","model":"exact","historyConsent":true}})).unwrap()));
        runner.set_backends(runtime, policy, client);
        local.activate().await.unwrap();
        let worker = runner.start();
        tokio::time::timeout(Duration::from_secs(5), started.notified()).await.expect("fixture memory request started");
        let before = store.call(Request::JobInput(s(&job,"id"))).await.unwrap();
        assert_eq!(before["job"]["status"], "running");
        assert_eq!(before["job"]["attempts"], 1);
        let pause = tokio::time::timeout(Duration::from_secs(1), runner.pause_for_sync_for(Duration::from_millis(300))).await.expect("sync pause must not wait for inference");
        let paused = store.call(Request::JobInput(s(&job,"id"))).await.unwrap();
        assert_eq!(paused["job"]["status"], "pending");
        assert_eq!(paused["job"]["attempts"], 0);
        let status = store.call(Request::SyncStatus(json!({}))).await.unwrap();
        assert!(status["manifests"].as_array().unwrap().iter().all(|entry| entry["busy"] == false));
        assert_eq!(store.call(Request::GetMeta(s(&conv,"id"))).await.unwrap()["prompt"], "Uma viajante curiosa.");
        tokio::time::timeout(Duration::from_secs(5), started.notified()).await.expect("lease expiry must resume queued job automatically");
        let resumed = store.call(Request::JobInput(s(&job,"id"))).await.unwrap();
        assert_eq!(resumed["job"]["id"], job["id"]);
        assert_eq!(resumed["job"]["status"], "running");
        assert_eq!(resumed["job"]["attempts"], 1);
        drop(pause);
        let paused_again = runner.pause_for_sync().await;
        assert_eq!(runner.state().await["pausedForSync"], true);
        let first_page = store.call(Request::SyncStatus(json!({}))).await.unwrap();
        drop(paused_again);
        let next_page_pause = runner.pause_for_sync().await;
        let next_page = store.call(Request::SyncStatus(json!({}))).await.unwrap();
        assert_eq!(first_page, next_page, "adjacent status/delta requests must not churn job timestamps or snapshot hashes");
        drop(next_page_pause);
        tokio::time::timeout(Duration::from_secs(3), started.notified()).await.expect("completed sync must resume after the short request-burst grace period");
        let _final_pause = runner.pause_for_sync().await;
        runner.stop();
        tokio::time::timeout(Duration::from_secs(1), worker).await.unwrap().unwrap();
        release.notify_waiters(); server.abort();
        local.stop().await.unwrap(); store.call(Request::Close).await.unwrap();
    }

    #[tokio::test]
    async fn desktop_unavailable_local_and_deferred_do_not_claim_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            data_dir: dir.path().to_path_buf(), dist: dir.path().join("dist"),
            address: "127.0.0.1:0".parse().unwrap(), require_pairing: false,
            providers: None, max_generations: 1, production: false, native_lib_dir: None, review: false,
        };
        let store = Store::open_for_config(&config).unwrap();
        let local = LocalModels::new(&config, reqwest::Client::new()).unwrap();
        assert!(!local.chat_available());
        let conv = store.call(Request::CreateConversation(json!({"routeId":"fixture","presetId":"generic"}))).await.unwrap();
        let job = store.call(Request::EnqueueJob(json!({"convId":conv["id"],"kind":"cast"}))).await.unwrap();
        let (events, _) = broadcast::channel(8);
        let runner = Runner::new(store.clone(), local, Arc::new(Notify::new()), events, false);
        for mode in [MemoryMode::Local, MemoryMode::Deferred] {
            *runner.policy.write().unwrap() = MemoryPolicy { mode, api: None };
            assert!(!runner.cycle().await.unwrap());
            let input = store.call(Request::JobInput(s(&job, "id"))).await.unwrap();
            assert_eq!(input["job"]["status"], "pending");
            assert_eq!(input["job"]["attempts"], 0);
        }
        store.call(Request::Close).await.unwrap();
    }

    #[tokio::test]
    async fn deferred_and_standby_preserve_pending_job_attempts_without_requests() {
        let dir = tempfile::tempdir().unwrap();
        let native = tempfile::tempdir().unwrap();
        let config = Config::for_android(dir.path(), native.path()).unwrap();
        let store = Store::open_for_config(&config).unwrap();
        let client = reqwest::Client::new();
        let local = LocalModels::new(&config, client.clone()).unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = axum::Router::new().fallback(move || {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                axum::Json(json!({"data":[]}))
            }
        });
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let conv = store
            .call(Request::CreateConversation(
                json!({"routeId":"cbai/glm-5.2","presetId":"glm-52"}),
            ))
            .await
            .unwrap();
        let job = store
            .call(Request::EnqueueJob(
                json!({"convId":conv["id"],"kind":"curate"}),
            ))
            .await
            .unwrap();
        let (events, _) = broadcast::channel(8);
        let mut runner = Runner::new(
            store.clone(),
            local.clone(),
            Arc::new(Notify::new()),
            events,
            false,
        );
        let runtime = RuntimeProviders::new(MobileProviders::parse(&json!({"providers":[{"id":"fixture","baseUrl":url,"models":["exact"],"allowLoopback":true}]})).unwrap().runtime().unwrap());
        let policy = Arc::new(std::sync::RwLock::new(MemoryPolicy::default()));
        runner.set_backends(runtime, policy.clone(), client);
        assert!(!runner.cycle().await.unwrap());
        local.activate().await.unwrap(); // Missing native runtime: no downloads or subprocesses.
        assert!(
            !runner.cycle().await.unwrap(),
            "deferred must gate before claiming even when activated"
        );
        local.standby().await.unwrap();
        for value in [
            json!({"mode":"local"}),
            json!({"mode":"api","api":{"provider":"fixture","model":"exact","historyConsent":true}}),
        ] {
            *policy.write().unwrap() = MemoryPolicy::parse(&value).unwrap();
            assert!(!runner.cycle().await.unwrap());
        }
        let claimed = store.call(Request::ClaimJob).await.unwrap();
        assert_eq!(claimed["id"], job["id"]);
        assert_eq!(
            claimed["attempts"], 1,
            "no earlier cycle may spend an attempt"
        );
        assert_eq!(requests.load(Ordering::SeqCst), 0);
        local.stop().await.unwrap();
        store.call(Request::Close).await.unwrap();
        server.abort();
    }
}

fn compact_excerpt(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        format!("[contexto parcial] {}", rules::clip(text, limit))
    }
}
fn compact_extraction_prompt(persona: &str) -> String {
    format!(
        "Extraia memória narrativa em português do fragmento integral. Responda somente JSON conforme o schema, com todos os campos obrigatórios e listas vazias quando ausentes. No máximo 5 fatos essenciais. Não invente acontecimentos, relações ou conhecimento. A persona {persona} é controlada pelo usuário. Em relationships, forneça objetos com a, b e description para vínculos estabelecidos/alterados. Em open_threads/resolved_threads, liste pendências narrativas curtas. Preserve autor original de citações/imitações: attribution.quote deve ser trecho exato e originalSpeaker/sourceId devem corresponder à proveniência fornecida; comentários fora das aspas são separados. Resuma apenas fatos ocorridos; cenas, personagens, segredos, relações e pendências devem ser suportados pelo texto. Não exponha segredos a quem não sabe."
    )
}

pub fn episode_prompt(persona: &str, compact: bool) -> String {
    if compact {
        format!(
            "Comprima o episódio em um resumo narrativo coeso em português (3 a 4 frases em parágrafo único), preservando o tom dramático e a continuidade.\n\
             Regras obrigatórias:\n\
             1. Ponto de vista: use 3ª pessoa nominativa para todos os personagens. A persona {persona} é o usuário; use o nome dela, nunca 'usuário', 'você', 'eu' ou 'narrador'.\n\
             2. Atmosfera emocional: preserve o clima (tensão, ternura, desconfiança, cumplicidade, medo) e o tom literário da cena, evitando relatórios clínicos ou listas secas.\n\
             3. Dinâmica interpessoal: registre como a relação e a postura psicológica dos personagens mudaram (quem confrontou, cedeu, se aproximou ou desconfiou).\n\
             4. Fatos e revelações: destaque decisões cruciais, segredos descobertos e fatos determinantes com nomes explícitos.\n\
             5. Compromissos e âncoras: explicite promessas feitas, pendências não resolvidas e a postura com que os personagens terminaram a cena, servindo de elo para a próxima.\n\
             Responda SOMENTE em JSON no formato: {{\"summary\": \"...\"}}"
        )
    } else {
        format!(
            "Você é o sintetizador de memória de longo prazo e continuidade narrativa do chat.\n\
             Sua missão é comprimir o episódio em um resumo narrativo coeso e vivo em português (3 a 5 frases densas em parágrafo único), preservando a atmosfera dramática e prevenindo achatamento clínico.\n\n\
             Diretrizes obrigatórias:\n\
             1. Ponto de Vista e Nomes Nominais (3ª Pessoa Estrita):\n\
                - O usuário interpreta a persona {persona}. Refira-se a {persona} sempre pelo nome próprio, NUNCA como 'o usuário', 'você', 'eu' ou 'narrador'.\n\
                - Identifique todos os personagens pelos seus nomes próprios explícitos (ex.: 'Alex', 'Lia'). Nunca use pronomes isolados sem antecedente inequívoco.\n\
             2. Atmosfera Emocional e Tom Dramático:\n\
                - Capture o clima emocional predominante (tensão, desconfiança mútua, vulnerabilidade íntima, urgência, cumplicidade ou melancolia) e preserve a cadência literária original.\n\
                - Evite linguagem burocrática, resumos policiais ou bullet points assépticos.\n\
             3. Dinâmica Interpessoal e Postura Psicológica:\n\
                - Explicite a evolução relacional: quem ganhou influência, quem se abriu, quem recuou, atritos velados ou vínculos consolidados.\n\
                - Descreva a postura psicológica dos participantes (ex.: defensivo, provocador, relutante, afetuoso).\n\
             4. Revelações, Segredos e Decisões Críticas:\n\
                - Registre decisões determinantes, segredos compartilhados ou revelados e reviravoltas que alteram os rumos da história.\n\
             5. Compromissos em Aberto e Âncora de Transição (Fronteira Narrativa):\n\
                - Explicite promessas firmadas, dívidas morais, planos combinados e questões que ficaram pendentes para o futuro.\n\
                - A última frase DEVE ancorar o estado psicológico e situacional exato em que os personagens terminaram o episódio, fornecendo uma transição fluida para o diálogo que se segue.\n\n\
             Responda SOMENTE em JSON no formato: {{\"summary\": \"...\"}}"
        )
    }
}

pub fn sanitize_episode_summary(raw: &Value) -> Option<String> {
    let text = if let Some(s) = raw.as_str() {
        s
    } else if let Some(s) = raw["summary"]
        .as_str()
        .or_else(|| raw["resumo"].as_str())
        .or_else(|| raw["narrative"].as_str())
        .or_else(|| raw["episode"].as_str())
    {
        s
    } else {
        return None;
    };
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}
pub fn sanitize_extraction(raw: &Value) -> Result<Value> {
    if !raw.is_object() {
        return Err(Error::new(
            502,
            "Extração local incompleta; turno permanece pendente.",
        ));
    }
    let summary = raw["summary"]
        .as_str()
        .or_else(|| raw["resumo"].as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            let facts = raw["facts"].as_array().or_else(|| raw["fatos"].as_array())?;
            let text = facts
                .iter()
                .take(2)
                .filter_map(|f| {
                    f["content"]
                        .as_str()
                        .or_else(|| f["fact"].as_str())
                        .or_else(|| f["fato"].as_str())
                })
                .collect::<Vec<_>>()
                .join(" ");
            if text.trim().is_empty() {
                None
            } else {
                Some(text)
            }
        });

    let Some(summary) = summary else {
        return Err(Error::new(
            502,
            "Extração local incompleta; turno permanece pendente.",
        ));
    };

    let clean = |v: &Value, max: usize| {
        json!(rules::clip(
            &v.as_str()
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            max
        ))
    };
    let mut out = json!({"summary":rules::clip(&summary,500),"cast":[],"scene":null,"facts":[],"relationships":[],"openThreads":[],"resolvedThreads":[]});
    for c in raw["cast"]
        .as_array()
        .or_else(|| raw["personagens"].as_array())
        .into_iter()
        .flatten()
        .take(10)
    {
        if let Some(name) = crate::cast::name(&s(c, "name")) {
            out["cast"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name":name,"description":clean(&c["description"],300)}));
        }
    }
    let scene_val = if raw["scene"].is_object() {
        Some(&raw["scene"])
    } else if raw["cena"].is_object() {
        Some(&raw["cena"])
    } else {
        None
    };
    if let Some(scene) = scene_val {
        let mut present = Vec::<String>::new();
        for v in scene["present"]
            .as_array()
            .or_else(|| scene["presentes"].as_array())
            .into_iter()
            .flatten()
        {
            if let Some(name) = crate::cast::name(v.as_str().unwrap_or_default())
                && !present.contains(&name)
            {
                present.push(name);
            }
        }
        present.truncate(12);
        let mut positions = serde_json::Map::new();
        let mut locations = serde_json::Map::new();
        let mut holding = serde_json::Map::new();
        let mut possessions = serde_json::Map::new();
        for c in scene["characters"]
            .as_array()
            .or_else(|| scene["personagens"].as_array())
            .into_iter()
            .flatten()
            .take(40)
        {
            let Some(name) = crate::cast::name(&s(c, "name")) else {
                continue;
            };
            if let Some(value) = crate::cast::known(&s(c, "position"), 120) {
                positions.insert(name.clone(), json!(value));
            }
            if let Some(value) = crate::cast::known(&s(c, "location"), 80) {
                locations.insert(name.clone(), json!(value));
            }
            holding.insert(
                name.clone(),
                json!(
                    c["holding"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(|s| crate::cast::clean(s, 50))
                        .filter(|s| !s.is_empty())
                        .take(8)
                        .collect::<Vec<_>>()
                ),
            );
            possessions.insert(
                name,
                json!(
                    c["possessions"]
                        .as_array()
                        .or_else(|| c["posses"].as_array())
                        .or_else(|| c["inventory"].as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(|s| crate::cast::clean(s, 50))
                        .filter(|s| !s.is_empty())
                        .take(12)
                        .collect::<Vec<_>>()
                ),
            );
        }
        // Older isolated imports can contain the Node-normalized scene maps.
        for (key, map, max) in [
            ("positions", &mut positions, 120),
            ("locations", &mut locations, 80),
        ] {
            if let Some(input) = scene[key].as_object() {
                for (name, v) in input.iter().take(40) {
                    if let Some(name) = crate::cast::name(name)
                        && let Some(value) = crate::cast::known(v.as_str().unwrap_or_default(), max)
                    {
                        map.entry(name).or_insert(json!(value));
                    }
                }
            }
        }
        if let Some(input) = scene["holding"].as_object() {
            for (name, v) in input.iter().take(40) {
                if let Some(name) = crate::cast::name(name) {
                    holding.entry(name).or_insert(json!(
                        v.as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(|s| crate::cast::clean(s, 50))
                            .filter(|s| !s.is_empty())
                            .take(8)
                            .collect::<Vec<_>>()
                    ));
                }
            }
        }
        if let Some(input) = scene["possessions"].as_object().or_else(|| scene["inventory"].as_object()) {
            for (name, v) in input.iter().take(40) {
                if let Some(name) = crate::cast::name(name) {
                    possessions.entry(name).or_insert(json!(
                        v.as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(|s| crate::cast::clean(s, 50))
                            .filter(|s| !s.is_empty())
                            .take(12)
                            .collect::<Vec<_>>()
                    ));
                }
            }
        }
        let location = crate::cast::known(&s(scene, "location"), 80);
        let time = crate::cast::known(&s(scene, "time"), 60);
        if location.is_some() || time.is_some() || !present.is_empty() || !positions.is_empty() {
            out["scene"] = json!({"location":location,"time":time,"mood":crate::cast::known(&s(scene,"mood"),80),"present":present,"positions":positions,"locations":locations,"holding":holding,"possessions":possessions,"objects":scene["objects"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|s|crate::cast::clean(s,50)).filter(|s|!s.is_empty()).take(12).collect::<Vec<_>>()});
        }
    }
    let facts_arr = raw["facts"]
        .as_array()
        .or_else(|| raw["fatos"].as_array());
    let persona = raw
        .get("persona")
        .or_else(|| raw.get("conv").and_then(|c| c.get("persona")))
        .and_then(Value::as_str)
        .unwrap_or_default();

    for f in facts_arr.into_iter().flatten().take(14) {
        let content_val = if !f["content"].is_null() {
            &f["content"]
        } else if !f["fato"].is_null() {
            &f["fato"]
        } else {
            &f["fact"]
        };
        let speaker = f["attribution"]["originalSpeaker"]
            .as_str()
            .unwrap_or_default();
        let raw_content = content_val.as_str().unwrap_or_default();
        let normalized_content = normalize_fact_text(raw_content, speaker, persona);
        let content = clean(&json!(normalized_content), 300);
        if content.as_str().unwrap().len() < 6 {
            continue;
        }
        let mut fact = f.clone();
        fact["content"] = content;

        let raw_subject = if !f["subject"].is_null() {
            f["subject"].as_str().unwrap_or_default()
        } else {
            f["sujeito"].as_str().unwrap_or_default()
        };
        let normalized_subject = normalize_fact_text(raw_subject, speaker, persona);
        fact["subject"] = clean(&json!(normalized_subject), 60);

        fact["replaces"] = clean(&f["replaces"], 300);
        fact["existingId"] = f["existing_id"].clone();
        fact["kind"] = json!(
            f["kind"]
                .as_str()
                .filter(|k| [
                    "CHARACTER_FACT",
                    "USER_FACT",
                    "RELATIONSHIP",
                    "KINSHIP",
                    "POSSESSION",
                    "INVENTORY",
                    "EVENT",
                    "SECRET",
                    "PROMISE",
                    "PREFERENCE",
                    "WORLD_FACT",
                    "OBJECT",
                    "LOCATION",
                    "OPEN_THREAD"
                ]
                .contains(k))
                .unwrap_or("EVENT")
        );
        if let Some(kinship) = f.get("kinship").filter(|k| !k.is_null()) {
            fact["kinship"] = kinship.clone();
        }
        if let Some(possession) = f.get("possession").filter(|p| !p.is_null()) {
            fact["possession"] = possession.clone();
        }
        for (key, default) in [("importance", 0.5), ("confidence", 0.7)] {
            fact[key] = json!(
                f[key]
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .unwrap_or(default)
                    .clamp(0.0, 1.0)
            );
        }
        fact["knowledge"] = json!(
            f["knowledge"]
                .as_array()
                .into_iter()
                .flatten()
                .take(10)
                .filter_map(|k| {
                    let name = crate::cast::name(&s(k, "who"))?;
                    let mut k = k.clone();
                    k["who"] = json!(name);
                    k["state"] = json!(
                        k["state"]
                            .as_str()
                            .filter(|s| ["knows", "believes", "unaware"].contains(s))
                            .unwrap_or("knows")
                    );
                    k["via"] = json!(
                        k["via"]
                            .as_str()
                            .filter(|s| [
                                "witnessed",
                                "heard",
                                "told",
                                "discovered",
                                "received",
                                "inferred"
                            ]
                            .contains(s))
                            .unwrap_or("witnessed")
                    );
                    Some(k)
                })
                .collect::<Vec<_>>()
        );
        // Evidence strings are copied exactly: they must match the original source.
        for k in fact["knowledge"].as_array_mut().unwrap() {
            k["evidence"] = json!(rules::clip(&s(k, "evidence"), 2000));
        }
        out["facts"].as_array_mut().unwrap().push(fact);
    }
    for r in raw["relationships"]
        .as_array()
        .or_else(|| raw["relacionamentos"].as_array())
        .into_iter()
        .flatten()
        .take(8)
    {
        let mut a = s(r, "a").trim().to_string();
        let mut b = s(r, "b").trim().to_string();
        let desc = s(r, "description").trim().to_string();

        if !persona.is_empty() {
            if rules::key(&a) == "eu" || rules::key(&a) == "me" {
                a = persona.to_string();
            }
            if rules::key(&b) == "eu" || rules::key(&b) == "me" {
                b = persona.to_string();
            }
        }

        let a_norm = crate::cast::name(&a).unwrap_or(a);
        let b_norm = crate::cast::name(&b).unwrap_or(b);

        if !a_norm.is_empty() && !b_norm.is_empty() && rules::key(&a_norm) != rules::key(&b_norm) && !desc.is_empty() {
            let normalized_desc = normalize_fact_text(&desc, &a_norm, persona);
            let mut rel = json!({
                "a": a_norm,
                "b": b_norm,
                "description": clean(&json!(normalized_desc), 300)
            });
            if let Some(kind) = r.get("kind").or_else(|| r.get("relation")).or_else(|| r.get("relation_type")) {
                rel["kind"] = kind.clone();
            }
            if let Some(rel_type) = r.get("relation_type") {
                rel["relation_type"] = rel_type.clone();
            }
            if let Some(directed) = r.get("directed") {
                rel["directed"] = directed.clone();
            }
            out["relationships"].as_array_mut().unwrap().push(rel);
        }
    }
    for (k, source, alt) in [
        ("openThreads", "open_threads", "pendencias"),
        ("resolvedThreads", "resolved_threads", "resolvidas"),
    ] {
        let list = raw[source]
            .as_array()
            .or_else(|| raw[k].as_array())
            .or_else(|| raw[alt].as_array());
        for t in list.into_iter().flatten().take(5) {
            out[k].as_array_mut().unwrap().push(clean(t, 220));
        }
    }
    Ok(out)
}

pub fn normalize_fact_text(text: &str, speaker: &str, persona: &str) -> String {
    let effective_speaker = if !speaker.trim().is_empty() {
        speaker.trim()
    } else if !persona.trim().is_empty() {
        persona.trim()
    } else {
        ""
    };

    if effective_speaker.is_empty() {
        return text.to_string();
    }

    let mut result = text.to_string();

    // 1. Normalize 1st-person possessives (meu/minha/meus/minhas) -> attribution to speaker
    static RE_MINHA: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\bminha\s+((?:melhor\s+)?[a-záéíóúâêîôûãõç]+)").unwrap());
    static RE_MEU: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\bmeu\s+((?:melhor\s+)?[a-záéíóúâêîôûãõç]+)").unwrap());
    static RE_MINHAS: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\bminhas\s+((?:melhores\s+)?[a-záéíóúâêîôûãõç]+)").unwrap());
    static RE_MEUS: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\bmeus\s+((?:melhores\s+)?[a-záéíóúâêîôûãõç]+)").unwrap());

    result = RE_MINHA.replace_all(&result, format!("a $1 de {effective_speaker}")).to_string();
    result = RE_MEU.replace_all(&result, format!("o $1 de {effective_speaker}")).to_string();
    result = RE_MINHAS.replace_all(&result, format!("as $1 de {effective_speaker}")).to_string();
    result = RE_MEUS.replace_all(&result, format!("os $1 de {effective_speaker}")).to_string();

    // 2. Normalize 2nd-person possessives (sua/seu/suas/seus) when addressing persona
    let target = if !persona.trim().is_empty() && effective_speaker != persona.trim() {
        persona.trim()
    } else {
        ""
    };
    if !target.is_empty() {
        static RE_SUA: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\bsua\s+((?:melhor\s+)?[a-záéíóúâêîôûãõç]+)").unwrap());
        static RE_SEU: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\bseu\s+((?:melhor\s+)?[a-záéíóúâêîôûãõç]+)").unwrap());
        static RE_SUAS: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\bsuas\s+((?:melhores\s+)?[a-záéíóúâêîôûãõç]+)").unwrap());
        static RE_SEUS: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\bseus\s+((?:melhores\s+)?[a-záéíóúâêîôûãõç]+)").unwrap());

        result = RE_SUA.replace_all(&result, format!("a $1 de {target}")).to_string();
        result = RE_SEU.replace_all(&result, format!("o $1 de {target}")).to_string();
        result = RE_SUAS.replace_all(&result, format!("as $1 de {target}")).to_string();
        result = RE_SEUS.replace_all(&result, format!("os $1 de {target}")).to_string();
    }

    // 3. Normalize raw first-person pronoun as subject (e.g. "eu" -> effective_speaker)
    static RE_EU: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)^\s*eu\b").unwrap());
    result = RE_EU.replace_all(&result, effective_speaker).to_string();

    result
}

// Same speaker despite case/accents/spacing, or first name vs. full name ("Mira" ~ "Mira Rose").
// Only a CLEAR incompatibility is a conflict; when in doubt no rejection is invented.
fn speaker_tokens(v: &Value) -> Vec<String> {
    v.as_str()
        .map(|n| {
            rules::key(n)
                .split_whitespace()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
fn same_speaker(documented: &Value, claimed: &Value) -> bool {
    let (a, b) = (speaker_tokens(documented), speaker_tokens(claimed));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let (small, big) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    small.iter().all(|t| big.contains(t))
}

/// Facts whose `attribution` contradicts the documented origin of the quote are dropped. When that happens
/// the result also carries `attributionRejected` (the confirmed conflicts): the summary generated together
/// may carry the same false authorship, so the runner must not promote the batch. The key exists only in
/// that case; a fact without attribution never triggers it.
pub fn sanitize_with_sources(raw: &Value, input: &Value) -> Result<Value> {
    let mut raw_with_persona;
    let effective_raw = if raw.get("persona").is_none()
        && input.get("conv").and_then(|c| c.get("persona")).is_some()
    {
        raw_with_persona = raw.clone();
        raw_with_persona["persona"] = input["conv"]["persona"].clone();
        &raw_with_persona
    } else {
        raw
    };
    let mut out = sanitize_extraction(effective_raw)?;
    let sources = input["recentSources"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut rejected = Vec::new();
    out["facts"].as_array_mut().unwrap().retain(|f| {
        let quote = s(&f["attribution"], "quote");
        if quote.is_empty() {
            return true;
        }
        let origins = crate::provenance::quotes(&format!("\"{quote}\""), &sources);
        let Some(origin) = origins.first() else {
            return true;
        };
        let claimed = &f["attribution"]["originalSpeaker"];
        if same_speaker(&origin["originalSpeaker"], claimed) {
            return true;
        }
        // Missing/empty declared speaker = incomplete metadata (fact dropped, no signal);
        // a declared speaker different from the documented one = confirmed conflict.
        if !speaker_tokens(claimed).is_empty() {
            rejected.push(json!({"content":f["content"],"quote":quote,"claimedSpeaker":rules::clip(claimed.as_str().unwrap_or_default(),60),"documentedSpeaker":origin["originalSpeaker"],"sourceId":origin["sourceId"]}));
        }
        false
    });
    if !rejected.is_empty() {
        out["attributionRejected"] = json!(rejected);
    }
    Ok(out)
}

fn attribution_conflict(rejected: &[Value]) -> Error {
    let first = &rejected[0];
    let quote = s(first, "quote");
    let quote = if quote.chars().count() > 60 {
        format!("{}...", quote.chars().take(57).collect::<String>())
    } else {
        quote
    };
    Error::new(
        502,
        format!(
            "Extração descartada por conflito de autoria: {} fato(s) atribuem \"{quote}\" a {}, mas a fonte documenta {}. Nada deste turno foi gravado; o turno permanece recuperável.",
            rejected.len(),
            s(first, "claimedSpeaker"),
            s(first, "documentedSpeaker")
        ),
    )
    .extra("code", "ATTRIBUTION_CONFLICT")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_episode_summary_resilience() {
        // Standard summary key
        let v1 = json!({"summary": "Em meio à névoa, Alex e Lia firmaram um pacto secreto."});
        assert_eq!(
            sanitize_episode_summary(&v1).unwrap(),
            "Em meio à névoa, Alex e Lia firmaram um pacto secreto."
        );

        // Portuguese key fallback
        let v2 = json!({"resumo": "Sob tensão crescente, Elena revelou o mapa roubado."});
        assert_eq!(
            sanitize_episode_summary(&v2).unwrap(),
            "Sob tensão crescente, Elena revelou o mapa roubado."
        );

        // Alternative key fallbacks
        let v_narrative = json!({"narrative": "A revelação mudou o destino do reino."});
        assert_eq!(
            sanitize_episode_summary(&v_narrative).unwrap(),
            "A revelação mudou o destino do reino."
        );
        let v_episode = json!({"episode": "O pacto foi selado."});
        assert_eq!(
            sanitize_episode_summary(&v_episode).unwrap(),
            "O pacto foi selado."
        );

        // Whitespace and newline collapsing into single paragraph
        let v3 = json!({"summary": "Linha um com tensão.\n\nLinha dois com   espaços\n  e promessas."});
        assert_eq!(
            sanitize_episode_summary(&v3).unwrap(),
            "Linha um com tensão. Linha dois com espaços e promessas."
        );

        // Rejection of empty or invalid
        assert!(sanitize_episode_summary(&json!({"summary": "   "})).is_none());
        assert!(sanitize_episode_summary(&json!({})).is_none());
        assert!(sanitize_episode_summary(&json!(null)).is_none());
    }

    #[test]
    fn test_episode_prompt_invariants_and_persona() {
        let p_desktop = episode_prompt("Kael", false);
        assert!(p_desktop.contains("Kael"));
        assert!(p_desktop.contains("NUNCA como 'o usuário'"));
        assert!(p_desktop.contains("Atmosfera Emocional"));
        assert!(p_desktop.contains("Dinâmica Interpessoal"));
        assert!(p_desktop.contains("Compromissos em Aberto"));
        assert!(p_desktop.contains("{\"summary\": \"...\"}"));

        let p_compact = episode_prompt("Mira", true);
        assert!(p_compact.contains("Mira"));
        assert!(p_compact.contains("Atmosfera emocional"));
        assert!(p_compact.contains("Compromissos e âncoras"));
    }
}

#[cfg(test)]
mod m2_cooldown_tests {
    use super::*;
    use crate::{config::Config, providers::Providers};
    use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::{get, post}};
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn wait_for_real<F, Fut>(mut cond: F)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        for _ in 0..400 {
            if cond().await {
                return;
            }
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("condition did not become true in time");
    }

    fn test_config(dir: &std::path::Path) -> Config {
        Config {
            data_dir: dir.to_path_buf(),
            dist: dir.join("dist"),
            address: "127.0.0.1:0".parse().unwrap(),
            require_pairing: false,
            providers: None,
            max_generations: 4,
            production: false,
            native_lib_dir: None,
            review: false,
        }
    }

    fn setup_runtime(url: &str) -> RuntimeProviders {
        let providers = Providers::from_json(&json!({
            "default": "mock",
            "providers": [{
                "id": "mock",
                "baseUrl": url,
                "apiKey": "test-key",
                "models": ["custom/memory-model"]
            }]
        }))
        .unwrap();
        RuntimeProviders::with_default_concurrency(Some(Arc::new(providers)), 1)
    }

    #[tokio::test]
    async fn runner_cooldown_respects_retry_after_suppresses_kick_and_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let store = Store::open_for_config(&config).unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let local = LocalModels::new(&config, client.clone()).unwrap();

        let posts = Arc::new(AtomicUsize::new(0));
        let posts_srv = posts.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"custom/memory-model"}]})) }),
            )
            .route(
                "/v1/chat/completions",
                post(move || {
                    let posts_srv = posts_srv.clone();
                    async move {
                        let attempt = posts_srv.fetch_add(1, Ordering::SeqCst) + 1;
                        if attempt == 1 {
                            (
                                StatusCode::TOO_MANY_REQUESTS,
                                [("retry-after", "2")],
                                r#"{"error":{"message":"rate limit exceeded"}}"#,
                            )
                                .into_response()
                        } else {
                            Json(json!({
                                "choices": [{
                                    "finish_reason": "stop",
                                    "message": { "content": "{\"cast\":[]}" }
                                }]
                            }))
                            .into_response()
                        }
                    }
                }),
            );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let conv = store
            .call(Request::CreateConversation(json!({
                "routeId": "custom/memory-model",
                "presetId": "generic",
                "prompt": "Uma exploradora silenciosa."
            })))
            .await
            .unwrap();
        let job = store
            .call(Request::EnqueueJob(json!({
                "convId": conv["id"],
                "kind": "cast"
            })))
            .await
            .unwrap();

        let (events, _) = broadcast::channel(8);
        let mut runner = Runner::new(store.clone(), local, Arc::new(Notify::new()), events, false);
        let policy = Arc::new(std::sync::RwLock::new(
            MemoryPolicy::parse(&json!({
                "mode": "api",
                "api": {
                    "provider": "mock",
                    "model": "custom/memory-model",
                    "historyConsent": true
                }
            }))
            .unwrap(),
        ));
        runner.set_backends(setup_runtime(&url), policy, client);
        let worker = runner.start();
        runner.kick();

        wait_for_real(|| async {
            let input = store.call(Request::JobInput(s(&job, "id"))).await.unwrap();
            posts.load(Ordering::SeqCst) == 1 && input["job"]["status"] == "pending"
        })
        .await;

        let input = store.call(Request::JobInput(s(&job, "id"))).await.unwrap();
        assert_eq!(input["job"]["attempts"], 0, "transient deferral must refund attempts");
        assert_eq!(input["job"]["deferrals"], 1);
        let st = runner.state().await;
        assert_eq!(st["unavailableStreak"], 1);
        assert!(st["cooldownRemainingSeconds"].as_u64().unwrap() >= 1);
        assert!(st["lastError"].is_string());

        // Normal kick() and cycle() during cooldown must NOT trigger upstream POST
        runner.kick();
        runner.kick();
        assert!(!runner.cycle().await.unwrap());
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(posts.load(Ordering::SeqCst), 1, "kick() and cycle() must be suppressed during cooldown");

        // Wait past 2s Retry-After -> attempt 2 runs automatically and recovers
        tokio::time::sleep(Duration::from_secs(2)).await;
        wait_for_real(|| async {
            let input = store.call(Request::JobInput(s(&job, "id"))).await.unwrap();
            input["job"]["status"] == "done"
        })
        .await;

        assert_eq!(posts.load(Ordering::SeqCst), 2);
        let st_after = runner.state().await;
        assert_eq!(st_after["unavailableStreak"], 0);
        assert_eq!(st_after["cooldownRemainingSeconds"], 0);
        assert_eq!(st_after["lastError"], Value::Null);

        runner.stop();
        let _ = tokio::time::timeout(Duration::from_secs(1), worker).await;
        server.abort();
        store.call(Request::Close).await.unwrap();
    }

    #[tokio::test]
    async fn runner_auth_config_fails_in_3_attempts_and_reset_cooldown_wakes_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let store = Store::open_for_config(&config).unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let local = LocalModels::new(&config, client.clone()).unwrap();

        let posts = Arc::new(AtomicUsize::new(0));
        let posts_srv = posts.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"custom/memory-model"}]})) }),
            )
            .route(
                "/v1/chat/completions",
                post(move || {
                    let posts_srv = posts_srv.clone();
                    async move {
                        posts_srv.fetch_add(1, Ordering::SeqCst);
                        (
                            StatusCode::UNAUTHORIZED,
                            r#"{"error":{"message":"invalid api key"}}"#,
                        )
                            .into_response()
                    }
                }),
            );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let conv = store
            .call(Request::CreateConversation(json!({
                "routeId": "custom/memory-model",
                "presetId": "generic",
                "prompt": "Personagem de teste."
            })))
            .await
            .unwrap();
        let job_auth = store
            .call(Request::EnqueueJob(json!({
                "convId": conv["id"],
                "kind": "cast"
            })))
            .await
            .unwrap();

        let (events, _) = broadcast::channel(8);
        let mut runner = Runner::new(store.clone(), local, Arc::new(Notify::new()), events, false);
        let policy = Arc::new(std::sync::RwLock::new(
            MemoryPolicy::parse(&json!({
                "mode": "api",
                "api": {
                    "provider": "mock",
                    "model": "custom/memory-model",
                    "historyConsent": true
                }
            }))
            .unwrap(),
        ));
        runner.set_backends(setup_runtime(&url), policy, client);
        let worker = runner.start();
        runner.kick();

        for expected_attempts in 1..=3i64 {
            let expected_status = if expected_attempts < 3 {
                "pending"
            } else {
                "failed"
            };
            wait_for_real(|| async {
                let input = store.call(Request::JobInput(s(&job_auth, "id"))).await.unwrap();
                input["job"]["attempts"].as_i64() == Some(expected_attempts)
                    && input["job"]["status"] == expected_status
            })
            .await;
            if expected_attempts < 3 {
                let st = runner.state().await;
                assert_eq!(st["unavailableKind"], "auth_config");
                assert!(st["cooldownRemainingSeconds"].as_u64().unwrap() >= 295);
                // Normal kick() is suppressed during 300s auth cooldown
                runner.kick();
                tokio::time::sleep(Duration::from_millis(80)).await;
                assert_eq!(posts.load(Ordering::SeqCst), expected_attempts as usize);
                // Explicit remediation reset_cooldown() wakes immediately
                runner.reset_cooldown();
            }
        }
        let auth_done = store.call(Request::JobInput(s(&job_auth, "id"))).await.unwrap();
        assert_eq!(auth_done["job"]["status"], "failed");
        assert_eq!(auth_done["job"]["attempts"], 3);
        assert_eq!(posts.load(Ordering::SeqCst), 3);

        runner.stop();
        let _ = tokio::time::timeout(Duration::from_secs(1), worker).await;
        server.abort();
        store.call(Request::Close).await.unwrap();
    }

    #[tokio::test]
    async fn cooldown_schedule_and_bounded_transient_deferrals() {
        // Verify exponential backoff curves and Retry-After clamping
        for streak in 1..=6 {
            let d = compute_cooldown(streak, "transient", None).as_secs_f64();
            let base = match streak {
                1 => 30.0,
                2 => 60.0,
                3 => 120.0,
                4 => 300.0,
                _ => 900.0,
            };
            assert!(
                d >= base * 0.85 - 0.1 && d <= (base * 1.15).min(900.0) + 0.1,
                "transient streak {streak}: got {d}s"
            );

            let a = compute_cooldown(streak, "auth_config", None).as_secs_f64();
            let auth_base = match streak {
                1 => 300.0,
                2 => 600.0,
                _ => 900.0,
            };
            assert!(
                a >= auth_base && a <= (auth_base * 1.10).min(900.0) + 1.0,
                "auth_config streak {streak}: got {a}s"
            );
        }
        assert_eq!(compute_cooldown(1, "transient", Some(0)), Duration::from_secs(1));
        assert_eq!(compute_cooldown(1, "transient", Some(75)), Duration::from_secs(75));
        assert_eq!(compute_cooldown(1, "transient", Some(5000)), Duration::from_secs(900));

        // Verify DeferUnavailableJob refunds attempts and fails cleanly at max_deferrals=8
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let store = Store::open_for_config(&config).unwrap();
        let conv = store
            .call(Request::CreateConversation(json!({
                "routeId": "custom/memory-model",
                "presetId": "generic"
            })))
            .await
            .unwrap();
        let job = store
            .call(Request::EnqueueJob(json!({
                "convId": conv["id"],
                "kind": "cast"
            })))
            .await
            .unwrap();
        let jid = s(&job, "id");
        for def in 1..=8i64 {
            let claimed = store.call(Request::ClaimJob).await.unwrap();
            assert_eq!(claimed["id"], jid);
            store
                .call(Request::DeferUnavailableJob {
                    id: jid.clone(),
                    error: format!("Backend de memória indisponível #{def}"),
                    max_deferrals: 8,
                })
                .await
                .unwrap();
            let row = store.call(Request::JobInput(jid.clone())).await.unwrap();
            assert_eq!(row["job"]["deferrals"], def);
            if def < 8 {
                assert_eq!(row["job"]["status"], "pending");
                assert_eq!(row["job"]["attempts"], 0);
            } else {
                assert_eq!(row["job"]["status"], "failed");
            }
        }
        // RetryJobs resets attempts and deferrals back to 0
        store
            .call(Request::RetryJobs(s(&conv, "id")))
            .await
            .unwrap();
        let reset = store.call(Request::JobInput(jid)).await.unwrap();
        assert_eq!(reset["job"]["status"], "pending");
        assert_eq!(reset["job"]["attempts"], 0);
        assert_eq!(reset["job"]["deferrals"], 0);
        store.call(Request::Close).await.unwrap();
    }

    #[tokio::test]
    async fn runner_last_error_clears_on_reset_and_ignores_stale_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let store = Store::open_for_config(&config).unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let local = LocalModels::new(&config, client.clone()).unwrap();

        let (bad_started_tx, mut bad_started_rx) = tokio::sync::watch::channel(false);
        let (bad_release_tx, bad_release_rx) = tokio::sync::watch::channel(false);
        let bad_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bad_url = format!("http://{}/v1", bad_listener.local_addr().unwrap());
        let bad_app = Router::new().route(
            "/v1/models",
            get(move || {
                let bad_started_tx = bad_started_tx.clone();
                let mut bad_release_rx = bad_release_rx.clone();
                async move {
                    let _ = bad_started_tx.send(true);
                    let _ = bad_release_rx.wait_for(|v| *v).await;
                    (StatusCode::SERVICE_UNAVAILABLE, "booting")
                }
            }),
        );
        let bad_srv = tokio::spawn(async move { axum::serve(bad_listener, bad_app).await.unwrap() });

        let good_posts = Arc::new(AtomicUsize::new(0));
        let good_posts_srv = good_posts.clone();
        let good_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let good_url = format!("http://{}/v1", good_listener.local_addr().unwrap());
        let good_app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"custom/memory-model"}]})) }),
            )
            .route(
                "/v1/chat/completions",
                post(move || {
                    let good_posts_srv = good_posts_srv.clone();
                    async move {
                        good_posts_srv.fetch_add(1, Ordering::SeqCst);
                        Json(json!({
                            "choices": [{
                                "finish_reason": "stop",
                                "message": { "content": "{\"cast\":[]}" }
                            }]
                        }))
                    }
                }),
            );
        let good_srv = tokio::spawn(async move { axum::serve(good_listener, good_app).await.unwrap() });

        let conv = store
            .call(Request::CreateConversation(json!({
                "routeId": "custom/memory-model",
                "presetId": "generic",
                "prompt": "Uma exploradora silenciosa."
            })))
            .await
            .unwrap();
        let job = store
            .call(Request::EnqueueJob(json!({
                "convId": conv["id"],
                "kind": "cast"
            })))
            .await
            .unwrap();

        let (events, _) = broadcast::channel(8);
        let mut runner = Runner::new(store.clone(), local, Arc::new(Notify::new()), events, false);
        let runtime = setup_runtime(&bad_url);
        let policy = Arc::new(std::sync::RwLock::new(
            MemoryPolicy::parse(&json!({
                "mode": "api",
                "api": {
                    "provider": "mock",
                    "model": "custom/memory-model",
                    "historyConsent": true
                }
            }))
            .unwrap(),
        ));
        runner.set_backends(runtime.clone(), policy, client);

        // Synchronous reset_cooldown() must clear any previously recorded lastError immediately.
        runner.state.lock().unwrap().last_error = Some("stale error".into());
        assert!(runner.state().await["lastError"].is_string());
        runner.reset_cooldown();
        assert_eq!(runner.state().await["lastError"], Value::Null);

        let worker = runner.start();
        runner.kick();

        tokio::time::timeout(Duration::from_secs(3), bad_started_rx.wait_for(|v| *v))
            .await
            .unwrap()
            .unwrap();

        // Switch to good provider while MemoryBackend::resolve is blocked on bad_url's /v1/models.
        let good_providers = Providers::from_json(&json!({
            "default": "mock",
            "providers": [{
                "id": "mock",
                "baseUrl": good_url,
                "apiKey": "test-key",
                "models": ["custom/memory-model"]
            }]
        }))
        .unwrap();
        runtime.set(Some(Arc::new(good_providers)));
        runner.reset_cooldown();
        let _ = bad_release_tx.send(true);

        wait_for_real(|| async {
            let input = store.call(Request::JobInput(s(&job, "id"))).await.unwrap();
            input["job"]["status"] == "done"
        })
        .await;

        assert_eq!(good_posts.load(Ordering::SeqCst), 1);
        let st = runner.state().await;
        assert_eq!(st["unavailableStreak"], 0);
        assert_eq!(st["cooldownRemainingSeconds"], 0);
        assert_eq!(st["lastError"], Value::Null);

        runner.stop();
        let _ = tokio::time::timeout(Duration::from_secs(1), worker).await;
        bad_srv.abort();
        good_srv.abort();
        store.call(Request::Close).await.unwrap();
    }
}
