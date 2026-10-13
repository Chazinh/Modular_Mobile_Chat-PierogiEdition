//! Detached generation supervision. Slow/disconnected consumers cannot cancel a turn.
use crate::{
    Error, Result,
    config::Config,
    context,
    local::LocalModels,
    presets, review, rules, s,
    store::{FinishRequest, Request, Store, TurnMode, TurnRequest},
    style_policy,
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Notify, OwnedSemaphorePermit, Semaphore, broadcast, watch};

#[derive(Clone, Default)]
pub struct Locks(Arc<Mutex<HashMap<String, std::sync::Weak<Mutex<()>>>>>);
impl Locks {
    pub async fn get(&self, id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.0.lock().await;
        locks.retain(|_, v| v.strong_count() > 0);
        if let Some(lock) = locks.get(id).and_then(std::sync::Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(id.to_owned(), Arc::downgrade(&lock));
        lock
    }
}

#[derive(Default)]
pub struct Output {
    pub buffer: String,
    pub context: Value,
    pub final_event: Option<Value>,
}
pub struct Generation {
    pub id: String,
    pub conv: String,
    pub version: String,
    pub part: String,
    pub interaction: String,
    pub mode: String,
    pub ack: Value,
    pub output: Mutex<Output>,
    pub wake: broadcast::Sender<()>,
    cancel: watch::Sender<bool>,
    pub ended: Notify,
    pub created: Instant,
    /// Hard cut (UTF-16) of the generated text: the configured size, or 135% of it in "target" mode.
    pub bot_limit: usize,
}
impl Generation {
    pub fn info(&self) -> Value {
        json!({"id":self.id,"generationId":self.id,"versionId":self.version,"partId":self.part,"interactionId":self.interaction,"mode":self.mode,"status":"running"})
    }
    fn cancelled(&self) -> impl std::future::Future<Output = ()> + Send {
        let mut rx = self.cancel.subscribe();
        async move {
            if *rx.borrow() {
                return;
            }
            let _ = rx.wait_for(|v| *v).await;
        }
    }
    pub async fn snapshot(&self, offset: usize, ack: bool) -> Vec<Value> {
        let state = self.output.lock().await;
        let mut events = Vec::new();
        if ack {
            events.push(json!({"ack":self.ack}));
        }
        let length = rules::utf16_len(&state.buffer);
        if length > offset {
            let delta = state
                .buffer
                .chars()
                .scan(0usize, |n, c| {
                    let start = *n;
                    *n += c.len_utf16();
                    Some((start, c))
                })
                .filter(|(start, _)| *start >= offset)
                .map(|(_, c)| c)
                .collect::<String>();
            events.push(json!({"delta":delta,"offset":length,"resumed":ack}));
        }
        if !state.context.is_null() {
            events.push(json!({"context":state.context}));
        }
        if let Some(final_event) = &state.final_event {
            events.push(final_event.clone());
        }
        events
    }
}
#[derive(Clone)]
pub struct Manager {
    pub store: Store,
    pub local: LocalModels,
    pub client: reqwest::Client,
    pub config: Config,
    pub events: broadcast::Sender<Value>,
    pub jobs: Arc<Notify>,
    pub locks: Locks,
    active: Arc<Mutex<HashMap<String, Arc<Generation>>>>,
    recent: Arc<Mutex<HashMap<String, Arc<Generation>>>>,
    start_gate: Arc<Mutex<()>>,
    slots: Arc<Semaphore>,
}
impl Manager {
    pub fn new(
        store: Store,
        local: LocalModels,
        client: reqwest::Client,
        config: Config,
        events: broadcast::Sender<Value>,
        jobs: Arc<Notify>,
    ) -> Self {
        let slots = Arc::new(Semaphore::new(config.max_generations));
        Self {
            store,
            local,
            client,
            config,
            events,
            jobs,
            locks: Locks::default(),
            active: Arc::default(),
            recent: Arc::default(),
            start_gate: Arc::default(),
            slots,
        }
    }
    pub fn validation(&self) -> Value {
        std::fs::read(self.config.data_dir.join("route-validation.json"))
            .ok()
            .filter(|v| v.len() < 2 * 1024 * 1024)
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or(json!({"routes":{}}))
    }
    pub async fn busy(&self) -> Vec<String> {
        self.active.lock().await.keys().cloned().collect()
    }
    pub async fn busy_ids(&self) -> Vec<String> {
        self.busy().await
    }
    pub async fn get(&self, conv: &str, gid: Option<&str>) -> Option<Arc<Generation>> {
        if let Some(g) = self
            .active
            .lock()
            .await
            .get(conv)
            .filter(|g| gid.is_none_or(|id| g.id == id))
        {
            return Some(g.clone());
        }
        self.recent
            .lock()
            .await
            .values()
            .find(|g| {
                g.conv == conv
                    && gid.is_some_and(|id| g.id == id)
                    && g.created.elapsed() < Duration::from_secs(600)
            })
            .cloned()
    }
    pub async fn sync_character(&self, conv: &str) -> Result<()> {
        if crate::api::sync_character(&self.store, &self.config, conv).await? {
            self.jobs.notify_one();
        }
        Ok(())
    }
    pub async fn start(&self, mut req: TurnRequest) -> Result<(Arc<Generation>, bool)> {
        let _gate = self.start_gate.lock().await;
        let conv_lock = self.locks.get(&req.conv_id).await;
        let _conv_guard = conv_lock.lock().await;
        if let Some(op) = req.op_id.as_ref().filter(|s| !s.is_empty()) {
            let old = self
                .store
                .call(Request::GenerationByOp {
                    conv_id: req.conv_id.clone(),
                    op_id: op.clone(),
                })
                .await?;
            if !old.is_null() {
                if let Some(g) = self.get(&req.conv_id, old["generationId"].as_str()).await {
                    return Ok((g, true));
                }
                let (wake, _) = broadcast::channel(32);
                let (cancel, _) = watch::channel(false);
                return Ok((
                    Arc::new(Generation {
                        id: s(&old, "generationId"),
                        conv: req.conv_id.clone(),
                        version: s(&old, "versionId"),
                        part: s(&old, "partId"),
                        interaction: s(&old, "interactionId"),
                        mode: s(&old, "mode"),
                        ack: {
                            let mut ack = old.clone();
                            ack["replay"] = json!(true);
                            ack
                        },
                        output: Mutex::new(Output {
                            buffer: s(&old, "content"),
                            context: Value::Null,
                            final_event: Some(
                                json!({"done":true,"status":old["status"],"replay":true}),
                            ),
                        }),
                        wake,
                        cancel,
                        ended: Notify::new(),
                        created: Instant::now(),
                        bot_limit: rules::REPLY_STORE_LIMIT,
                    }),
                    true,
                ));
            }
        }
        if let Some(g) = self.active.lock().await.get(&req.conv_id) {
            return Err(Error::new(409, "Já existe uma resposta em andamento.")
                .extra("generationId", json!(g.id)));
        }
        if self.config.providers.is_none() {
            return Err(Error::new(
                503,
                "Geração experimental desabilitada; habilite apenas um upstream explícito.",
            ));
        }
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            Error::new(503, "Limite global de gerações atingido.")
                .extra("code", "GENERATION_OVERLOADED")
        })?;
        if matches!(req.mode, TurnMode::Send | TurnMode::Edit)
            && rules::utf16_len(&req.content) > rules::USER_LIMIT
        {
            return Err(Error::new(
                422,
                "Sua mensagem pode ter no máximo 6.000 caracteres.",
            ));
        }
        self.sync_character(&req.conv_id).await?;
        let conv = self
            .store
            .call(Request::GetMeta(req.conv_id.clone()))
            .await?;
        let mode = match req.mode {
            TurnMode::Send => "send",
            TurnMode::Edit => "edit",
            TurnMode::Continue => "continue",
            TurnMode::Regenerate => "regenerate",
        }
        .to_string();
        // only the pairs the history ladder can use (30 by default), not the maximum of 50
        let mut input = json!({"convId":req.conv_id,"maxPairs":context::pairs_to_load(&conv)});
        match req.mode {
            TurnMode::Send => input["pendingUserText"] = json!(req.content.trim()),
            TurnMode::Edit => {
                input["interactionId"] = json!(req.target_id);
                input["overrideUserText"] = json!(req.content.trim());
            }
            TurnMode::Regenerate => input["dropLastPart"] = json!(true),
            _ => {}
        }
        let data = self.store.call(Request::Context(input)).await?;
        if data["current"].is_null() {
            return Err(Error::new(400, "Nada para responder."));
        }
        let validation = self.validation();
        let mut plan = presets::request(&conv, vec![], &validation);
        let length = presets::apply_length(&mut plan, &data["globalSettings"]);
        let calibration = self
            .store
            .call(Request::Calibration(s(&conv, "routeId")))
            .await?["ratio"]
            .as_f64();
        let ctx = context::assemble(&data, &json!({}), &mode, &plan, calibration)?;
        // hard cut of the generated text: the chosen size, or 135% of it in "approximate target" mode
        let bot_limit = rules::reply_bounds(&data["globalSettings"]).2;
        req.route_id = Some(s(&conv, "routeId"));
        req.preset_id = Some(s(&conv, "presetId"));
        req.effort = conv["effort"].as_str().map(str::to_owned);
        req.request = json!({"controls":plan.controls,"capacity":plan.capacity,"length":length,"presetVersion":presets::PRESET_VERSION,"systemMode":plan.system_mode});
        let begin = self.store.call(Request::BeginTurn(req)).await?;
        self.store
            .call(Request::SetContext {
                generation_id: s(&begin, "generationId"),
                value: ctx.report.clone(),
            })
            .await?;
        let (wake, _) = broadcast::channel(32);
        let (cancel, _) = watch::channel(false);
        let ack = json!({"opId":begin["opId"],"generationId":begin["generationId"],"mode":mode,"interactionId":begin["interactionId"],"partId":begin["partId"],"userMessage":begin["userMessage"],"revision":begin["revision"],
            "preset":{"id":conv["presetId"],"label":presets::by_id(&s(&conv,"presetId"))["label"],"routeId":conv["routeId"],"effort":conv["effort"]},"controls":plan.controls,"capacity":plan.capacity,"context":ctx.report,"length":length});
        let g = Arc::new(Generation {
            id: s(&begin, "generationId"),
            conv: s(&conv, "id"),
            version: s(&begin, "versionId"),
            part: s(&begin, "partId"),
            interaction: s(&begin, "interactionId"),
            mode,
            ack,
            output: Mutex::new(Output {
                context: ctx.report.clone(),
                ..Output::default()
            }),
            wake,
            cancel,
            ended: Notify::new(),
            created: Instant::now(),
            bot_limit,
        });
        self.active.lock().await.insert(g.conv.clone(), g.clone());
        let _=self.events.send(json!({"convId":g.conv,"type":"generation","generating":true,"revision":begin["revision"]}));
        plan.body["messages"] = json!(ctx.messages);
        let this = self.clone();
        let task = g.clone();
        tokio::spawn(async move {
            this.run(task, data, plan, calibration, ctx, permit).await;
        });
        Ok((g, false))
    }
    async fn run(
        &self,
        g: Arc<Generation>,
        data: Value,
        mut plan: presets::Plan,
        calibration: Option<f64>,
        mut ctx: context::Context,
        _permit: OwnedSemaphorePermit,
    ) {
        let conv = data["conv"].clone();
        let query = s(&data["current"], "userText");
        let enrich = async {
            let vector = if self.local.embed_available() {
                tokio::time::timeout(Duration::from_secs(4), self.local.embed_text(&query, true))
                    .await
                    .ok()
                    .and_then(std::result::Result::ok)
            } else {
                None
            };
            let retrieval=self.store.call(Request::Retrieve(json!({"convId":g.conv,"queryText":query,"queryVec":vector,"embeddingModel":self.local.fingerprint(),"beforeSeq":data["beforeSeq"],"windowStartSeq":data["pairs"].as_array().and_then(|a|a.get(a.len().saturating_sub(30))).map(|p|p["seq"].clone()).unwrap_or(data["current"]["seq"].clone()),"limit":24}))).await?;
            context::assemble(&data, &retrieval, &g.mode, &plan, calibration)
        };
        let enriched = tokio::select! {_=g.cancelled()=>None,v=tokio::time::timeout(Duration::from_secs(6),enrich)=>Some(v)};
        if let Some(Ok(Ok(full))) = enriched {
            ctx = full;
            plan.body["messages"] = json!(ctx.messages);
        }
        let _ = self
            .store
            .call(Request::SetContext {
                generation_id: g.id.clone(),
                value: ctx.report.clone(),
            })
            .await;
        g.output.lock().await.context = ctx.report.clone();
        let _ = g.wake.send(());
        let (
            mut filter,
            mut finish,
            mut measured,
            mut error,
            mut saw_done,
            mut capped,
            mut reasoning,
        ) = (
            rules::ThinkFilter::default(),
            None,
            Value::Null,
            None,
            false,
            false,
            0usize,
        );
        let since = crate::now();
        let consume = async {
            let request = crate::providers::chat_request(
                &self.client,
                self.config.providers.as_ref().unwrap(),
                &plan.body,
                Duration::from_secs(900),
            );
            let response = request
                .send()
                .await
                .map_err(|e| Error::new(502, format!("Upstream: {e}")))?;
            if !response.status().is_success() {
                return Err(Error::new(
                    502,
                    format!(
                        "Provedor {} HTTP {}",
                        self.config.providers.as_ref().unwrap().resolve(plan.body["model"].as_str().unwrap_or_default()).0.id,
                        response.status()
                    ),
                ));
            }
            let mut stream = response.bytes_stream();
            let mut buffer = Vec::<u8>::new();
            let mut event = String::new();
            let mut checkpoint = tokio::time::interval(Duration::from_secs(2));
            checkpoint.tick().await;
            let mut idle_deadline = tokio::time::Instant::now() + Duration::from_secs(180);
            loop {
                let chunk = tokio::select! {
                    _=g.cancelled()=>break,
                    _=checkpoint.tick()=>{self.store.call(Request::FlushVersion{version_id:g.version.clone(),text:g.output.lock().await.buffer.clone()}).await?;continue;},
                    _=tokio::time::sleep_until(idle_deadline)=>return Err(Error::new(504,"O provedor ficou sem responder.")),
                    value=stream.next()=>value,
                };
                let Some(chunk) = chunk else {
                    break;
                };
                let chunk =
                    chunk.map_err(|e| Error::new(502, format!("Fluxo interrompido: {e}")))?;
                idle_deadline = tokio::time::Instant::now() + Duration::from_secs(180);
                buffer.extend_from_slice(&chunk);
                // Every individual event/line is bounded, including hidden reasoning.
                while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
                    if end > 256 * 1024 {
                        return Err(Error::new(502, "Linha SSE excedeu o limite."));
                    }
                    let line = String::from_utf8(buffer.drain(..=end).collect())
                        .map_err(|_| Error::new(502, "UTF-8 upstream inválido."))?;
                    let line = line.trim_end_matches(['\n', '\r']);
                    if let Some(payload) = line.strip_prefix("data:") {
                        if !event.is_empty() {
                            event.push('\n');
                        }
                        event.push_str(payload.trim_start());
                        if event.len() > 256 * 1024 {
                            return Err(Error::new(502, "Evento SSE excedeu o limite."));
                        }
                    }
                    if !line.is_empty() {
                        continue;
                    }
                    if event.is_empty() {
                        continue;
                    }
                    let payload = std::mem::take(&mut event);
                    if payload == "[DONE]" {
                        saw_done = true;
                        continue;
                    }
                    let value: Value = serde_json::from_str(&payload)
                        .map_err(|_| Error::new(502, "JSON SSE upstream inválido."))?;
                    if !value["error"].is_null() {
                        return Err(Error::new(
                            502,
                            value["error"]["message"]
                                .as_str()
                                .unwrap_or("Erro do provedor."),
                        ));
                    }
                    if !value["usage"].is_null() {
                        measured = value["usage"].clone();
                    }
                    let choice = &value["choices"][0];
                    let d = &choice["delta"];
                    for field in ["reasoning_content", "reasoning", "thinking"] {
                        if let Some(text) = d[field].as_str() {
                            reasoning = reasoning.saturating_add(rules::utf16_len(text));
                            break;
                        }
                    }
                    if let Some(text) = d["content"].as_str() {
                        let visible = filter.push(text);
                        if self.delta(&g, &visible).await {
                            capped = true;
                            break;
                        }
                    }
                    if let Some(reason) = choice["finish_reason"].as_str() {
                        finish = Some(reason.to_owned());
                    }
                }
                if buffer.len() > 256 * 1024 {
                    return Err(Error::new(502, "Linha SSE excedeu o limite."));
                }
                if capped {
                    break;
                }
            }
            Ok::<(), Error>(())
        };
        let result = tokio::select! {_=g.cancelled()=>Ok(()),v=consume=>v};
        if let Err(e) = result {
            error = Some(e.message);
        }
        if self.delta(&g, &filter.flush()).await {
            capped = true;
        }
        let mut stopped = *g.cancel.borrow();
        let mut status = if stopped {
            "interrupted"
        } else if capped
            || finish.as_deref() == Some("length")
            || finish.as_deref() == Some("max_tokens")
        {
            "length"
        } else if error.is_some() {
            "error"
        } else if finish.is_some() || saw_done {
            "complete"
        } else {
            "interrupted"
        };
        let mut visible = g.output.lock().await.buffer.clone();
        // A reply that ran into the hard cut ends at its last complete sentence instead of mid-word.
        if capped && !stopped {
            let trimmed = rules::trim_to_sentence(&visible);
            if trimmed.len() < visible.len() {
                visible = trimmed.to_owned();
                g.output.lock().await.buffer = visible.clone();
            }
        }
        let size = rules::reply_size(&data["globalSettings"]);
        if visible.trim().is_empty() && error.is_none() && !stopped {
            error = Some("O modelo retornou uma resposta vazia.".into());
        }
        let pct = data["globalSettings"]["dialoguePct"]
            .as_f64()
            .unwrap_or(65.0);
        let before =
            style_policy::assess_settings(&visible, size, pct, &data["globalSettings"]);
        let mut style_report = json!({"attempted":false,"before":before,"after":before});
        if !visible.trim().is_empty()
            && !stopped
            && error.is_none()
            // a reply cut by the limit (status length) is never rewritten: continuing it completes it
            && status == "complete"
            && before["ok"] != true
        {
            let mut repair_body = plan.body.clone();
            repair_body["stream"] = json!(false);
            repair_body
                .as_object_mut()
                .unwrap()
                .remove("stream_options");
            let messages = repair_body["messages"].as_array_mut().unwrap();
            messages.push(json!({"role":"assistant","content":visible}));
            messages.push(json!({"role":"user","content":style_policy::instruction_settings(&before,pct,&data["globalSettings"])}));
            let repair = async {
                let req = crate::providers::chat_request(
                    &self.client,
                    self.config.providers.as_ref().unwrap(),
                    &repair_body,
                    Duration::from_secs(180),
                );
                req.send().await?.error_for_status()?.json::<Value>().await
            };
            let repaired = tokio::select! { _=g.cancelled()=>None, result=repair=>Some(result) };
            style_report["attempted"] = json!(true);
            style_report["accepted"] = json!(false);
            if let Some(Ok(result)) = repaired {
                let raw = result["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or_default();
                let mut filter = rules::ThinkFilter::default();
                let candidate = (filter.push(raw) + &filter.flush()).trim().to_owned();
                let after = style_policy::assess_settings(
                    &candidate,
                    size,
                    pct,
                    &data["globalSettings"],
                );
                let accept = !candidate.is_empty()
                    && after["sizeOk"] == true
                    && after["monologueOk"] == true
                    && after["languageOk"] == true
                    && result["choices"][0]["finish_reason"]
                        .as_str()
                        .is_none_or(|r| !matches!(r, "length" | "max_tokens"))
                    && style_policy::score(&after, size, pct)
                        < style_policy::score(&before, size, pct);
                style_report["usage"] = result["usage"].clone();
                if accept {
                    visible = candidate;
                    g.output.lock().await.buffer = visible.clone();
                    style_report["accepted"] = json!(true);
                    style_report["after"] = after;
                } else {
                    // "after" stays the saved text; the rejected candidate is recorded separately.
                    style_report["candidate"] = after;
                }
            } else if let Some(Err(err)) = repaired {
                style_report["error"] = json!(err.to_string());
            }
            stopped = *g.cancel.borrow();
            if stopped {
                status = "interrupted";
            }
        }
        let usage = json!({"input":measured["prompt_tokens"].as_u64().unwrap_or(ctx.report["tokens"]["total"].as_u64().unwrap_or(0)),"output":measured["completion_tokens"].as_u64().unwrap_or(rules::estimate(&visible) as u64),
            "reasoning":measured.pointer("/completion_tokens_details/reasoning_tokens").cloned().unwrap_or(if reasoning>0{json!((reasoning as f64/3.6).ceil())}else{Value::Null}),
            "method":{"input":if measured["prompt_tokens"].is_number(){"measured"}else{"estimated"},"output":if measured["completion_tokens"].is_number(){"measured"}else{"estimated"},"reasoning":if reasoning>0{"estimated"}else{"unknown"}},
            "visibleEstimate":rules::estimate(&visible),"finish":finish,"reasoningHidden":reasoning>0||filter.hidden>0,"style":style_report,"length":plan.controls["length"]});
        if let Some(input) = measured["prompt_tokens"].as_u64() {
            let _ = self
                .store
                .call(Request::RecordUsage {
                    route_id: s(&conv, "routeId"),
                    raw: ctx.raw_estimate,
                    measured: input as usize,
                })
                .await;
        }
        let finalized = self
            .store
            .call(Request::FinishGeneration(FinishRequest {
                generation_id: g.id.clone(),
                status: status.into(),
                text: Some(visible.clone()),
                usage: usage.clone(),
                error: error.clone(),
            }))
            .await;
        let final_event = match finalized {
            Ok(fin) => {
                if self.config.review
                    && matches!(status, "complete" | "length" | "interrupted")
                    && !visible.trim().is_empty()
                {
                    let flags = review::heuristic(&visible, &s(&conv, "persona"));
                    if !flags.is_empty() {
                        let _=self.store.call(Request::AddFlags(json!({"convId":g.conv,"versionId":g.version,"flags":flags,"source":"heuristic"}))).await;
                    }
                }
                json!({"done":true,"status":fin["status"],"error":error,"message":fin["message"],"revision":fin["revision"],"usage":usage})
            }
            Err(e) => {
                json!({"done":true,"status":"error","error":format!("Falha na persistência final; checkpoint preservado: {}",e.message)})
            }
        };
        self.active.lock().await.remove(&g.conv);
        // Release capacity before publishing done; its consumer may send immediately.
        drop(_permit);
        g.output.lock().await.final_event = Some(final_event.clone());
        let _ = g.wake.send(());
        g.ended.notify_waiters();
        let _=self.events.send(json!({"convId":g.conv,"type":"message","revision":final_event["revision"],"generating":false}));
        let mut recent = self.recent.lock().await;
        recent.retain(|_, v| v.created.elapsed() < Duration::from_secs(600));
        if recent.len() >= 128
            && let Some(old) = recent
                .iter()
                .min_by_key(|(_, v)| v.created)
                .map(|(k, _)| k.clone())
        {
            recent.remove(&old);
        }
        recent.insert(g.id.clone(), g.clone());
        drop(recent);
        self.jobs.notify_one();
        let request_log = self.config.providers.as_ref().and_then(|p| {
            p.resolve(plan.body["model"].as_str().unwrap_or_default())
                .0
                .request_log
                .clone()
        });
        if let Some(file) = request_log {
            let last = plan.body["messages"]
                .as_array()
                .and_then(|m| m.iter().rev().find(|m| m["role"] == "user"))
                .map(|m| s(m, "content"))
                .unwrap_or_default();
            let preset = presets::by_id(&s(&conv, "presetId"));
            let family = s(&preset, "family");
            let omit_off = preset["thinkingOffShape"] == "omit";
            let mut check = json!({"found":false,"verification":"unverified"});
            // The router writes/fills its log asynchronously. A missing/busy row
            // is provisional; bounded retries never write to or assume its DB.
            for delay in [0, 100, 400, 1500, 4000] {
                if delay > 0 {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
                let (file, since, last, family) =
                    (file.clone(), since.clone(), last.clone(), family.clone());
                let evidence = tokio::task::spawn_blocking(move || {
                    crate::router_log::find(std::path::Path::new(&file), &since, &last, &family)
                })
                .await
                .unwrap_or(json!({"found":false,"reason":"leitor indisponível"}));
                if evidence["found"] == true {
                    check = evidence;
                    if check["usage"]["input"].as_u64().unwrap_or(0) > 0 {
                        break;
                    }
                } else if check["found"] != true {
                    check = evidence;
                }
            }
            let value = if check["found"] == true {
                let thinking_off = plan.controls["thinkingEnabled"] == false;
                let off_confirmed = matches!(
                    check["thinking"]["type"].as_str(),
                    Some("disabled" | "between_tools")
) || check["effort"] == "none"
                    // GLM 5.2 (cbai): thinking is only off WITHOUT any control in the request
                    || omit_off && check["thinking"].is_null() && check["effort"].is_null();
                let off_check = if off_confirmed {
                    json!(true)
                } else if !check["thinking"].is_null() || !check["effort"].is_null() {
                    json!(false)
                } else {
                    Value::Null
                };
                json!({"checkedAt":crate::now(),"provider":check["provider"],"model":check["model"],"effortExpected":conv["effort"],"effortSent":check["effort"],"effortOk":if thinking_off {off_check.clone()}else{json!(conv["effort"]==check["effort"])},"thinkingEnabled":!thinking_off,"thinkingOk":if thinking_off {off_check}else{Value::Null},"thinkingMode":check["thinking"],"maxTokens":check["maxTokens"],"usage":check["usage"]})
            } else {
                json!({"checkedAt":crate::now(),"notFound":true,"verification":"unverified","reason":check["reason"]})
            };
            let _ = self
                .store
                .call(Request::ProviderCheck {
                    generation_id: g.id.clone(),
                    value,
                })
                .await;
        }
    }
    async fn delta(&self, g: &Generation, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        let mut state = g.output.lock().await;
        let left = g.bot_limit.saturating_sub(rules::utf16_len(&state.buffer));
        let accepted = rules::clip(text, left);
        let capped = accepted.len() < text.len()
            || rules::utf16_len(&state.buffer) + rules::utf16_len(accepted) >= g.bot_limit;
        state.buffer.push_str(accepted);
        drop(state);
        let _ = g.wake.send(());
        capped
    }
    pub async fn stop(&self, conv: &str, id: &str) -> Result<Value> {
        let g = self
            .get(conv, Some(id))
            .await
            .ok_or_else(|| Error::new(404, "Nenhuma geração em andamento."))?;
        let _ = g.cancel.send(true);
        let deadline = Instant::now() + Duration::from_secs(35);
        loop {
            let notified = g.ended.notified();
            if g.output.lock().await.final_event.is_some() {
                break;
            }
            if tokio::time::timeout_at(deadline.into(), notified)
                .await
                .is_err()
            {
                return Err(Error::new(
                    504,
                    "Parada solicitada; finalização ainda pendente.",
                ));
            }
        }
        Ok(json!({"ok":true}))
    }
    pub async fn shutdown(&self) {
        let jobs = self
            .active
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for g in jobs {
            let _ = self.stop(&g.conv, &g.id).await;
        }
    }
    pub async fn persisted(&self, conv: &str, id: &str) -> Result<Arc<Generation>> {
        let old = self.store.call(Request::GetGeneration(id.into())).await?;
        if old.is_null() || old["convId"] != conv {
            return Err(Error::new(404, "Geração não encontrada."));
        }
        let (wake, _) = broadcast::channel(32);
        let (cancel, _) = watch::channel(false);
        Ok(Arc::new(Generation {
            id: id.into(),
            conv: conv.into(),
            version: s(&old, "versionId"),
            part: s(&old, "partId"),
            interaction: s(&old, "interactionId"),
            mode: s(&old, "mode"),
            ack: {
                let mut ack = old.clone();
                ack["replay"] = json!(true);
                ack
            },
            output: Mutex::new(Output {
                buffer: s(&old, "content"),
                context: old["context"].clone(),
                final_event: Some(json!({"done":true,"status":old["status"],"replay":true})),
            }),
            wake,
            cancel,
            ended: Notify::new(),
            created: Instant::now(),
            bot_limit: rules::REPLY_STORE_LIMIT,
        }))
    }
    pub async fn stream(
        &self,
        g: Arc<Generation>,
        from: usize,
        replay: bool,
    ) -> Result<
        axum::response::sse::Sse<
            impl futures_util::Stream<
                Item = std::result::Result<axum::response::sse::Event, std::convert::Infallible>,
            > + Send
            + 'static,
        >,
    > {
        {
            let state = g.output.lock().await;
            if rules::suffix(&state.buffer, from).is_none() {
                return Err(Error::new(400, "Offset UTF-16 inválido."));
            }
        }
        let wake = g.wake.subscribe();
        let stream = futures_util::stream::unfold(
            (g, wake, from, true, false, Vec::<Value>::new()),
            move |(g, mut wake, mut offset, mut ack, mut done, mut events)| async move {
                loop {
                    if !events.is_empty() {
                        let event = events.remove(0);
                        if event["offset"].is_number() {
                            offset = event["offset"].as_u64().unwrap() as usize;
                        }
                        if event["done"] == true {
                            done = true;
                        }
                        return Some((
                            Ok(axum::response::sse::Event::default().data(event.to_string())),
                            (g, wake, offset, ack, done, events),
                        ));
                    }
                    if done {
                        return None;
                    }
                    if !ack {
                        let _ = wake.recv().await;
                    }
                    events = g.snapshot(offset, ack).await;
                    if replay {
                        for event in &mut events {
                            if event["ack"].is_object() {
                                event["ack"]["replay"] = json!(true);
                            }
                            if event["done"] == true {
                                event["replay"] = json!(true);
                            }
                        }
                    }
                    ack = false;
                }
            },
        );
        Ok(axum::response::sse::Sse::new(stream)
            .keep_alive(axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15))))
    }
}
pub async fn write_character(file: &std::path::Path, text: &str) -> Result<()> {
    tokio::fs::create_dir_all(file.parent().unwrap()).await?;
    let tmp = file.with_extension(format!("{}.tmp", crate::id()));
    tokio::fs::write(&tmp, text).await?;
    tokio::fs::rename(&tmp, file).await?;
    Ok(())
}
