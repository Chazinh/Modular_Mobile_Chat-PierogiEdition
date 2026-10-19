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
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::{Mutex, Notify, OwnedSemaphorePermit, Semaphore, broadcast, watch};
use tracing::{info, warn};

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
    /// UTF-16 length of `buffer`, kept in step by `append`/`replace` (never rescanned per wake).
    pub units: usize,
    /// Bumped when the text is replaced instead of appended: cursors that cached a byte position must rescan.
    pub epoch: u64,
    pub context: Value,
    /// Bumped on every change of `context`; each consumer sends the report only when it moved.
    pub context_rev: u64,
    pub final_event: Option<Value>,
}
impl Output {
    fn new(buffer: String, context: Value, final_event: Option<Value>) -> Self {
        Self {
            units: rules::utf16_len(&buffer),
            buffer,
            epoch: 0,
            context_rev: u64::from(!context.is_null()),
            context,
            final_event,
        }
    }
    fn append(&mut self, text: &str) {
        self.buffer.push_str(text);
        self.units += rules::utf16_len(text);
    }
    fn replace(&mut self, text: String) {
        self.units = rules::utf16_len(&text);
        self.buffer = text;
        self.epoch += 1;
    }
    fn set_context(&mut self, value: Value) {
        self.context = value;
        self.context_rev += 1;
    }
}
/// Where one consumer (an SSE stream) is in the output: the UTF-16 offset it already received, the matching byte
/// position (valid while the text is only appended to) and the last context report it was sent.
#[derive(Clone, Copy, Default)]
pub struct Cursor {
    offset: usize,
    byte: usize,
    epoch: u64,
    context_rev: u64,
}
/// Byte index of the first char that starts at or after the UTF-16 `offset`.
fn start_byte(text: &str, offset: usize) -> usize {
    let at = rules::byte_at(text, offset);
    if rules::utf16_len(&text[..at]) == offset {
        at
    } else {
        // the offset splits a surrogate pair: that char was already (partly) delivered
        at + text[at..].chars().next().map_or(0, char::len_utf8)
    }
}
/// Context report as stored and streamed: counters only. The ID lists of the memories/episodes that went into the
/// prompt are diagnostics (nothing in the client reads them) and stay out unless `MC_DEBUG_CONTEXT=1`.
fn public_report(report: &Value) -> Value {
    let mut out = report.clone();
    if !crate::config::debug_context()
        && let Some(map) = out.as_object_mut()
    {
        for (list, count) in [
            ("memoriesIncluded", "memoriesCount"),
            ("episodesIncluded", "episodesCount"),
        ] {
            if let Some(items) = map.remove(list) {
                map.insert(count.into(), json!(items.as_array().map_or(0, Vec::len)));
            }
        }
    }
    out
}
/// Ack of a generation that is already persisted: identifiers, mode and status. The text itself travels as the
/// `delta` of the same stream, so `content`/`usage` are not repeated here.
fn replay_ack(old: &Value) -> Value {
    let mut ack = serde_json::Map::new();
    for key in [
        "generationId",
        "convId",
        "opId",
        "mode",
        "status",
        "interactionId",
        "partId",
        "versionId",
        "error",
        "userMessage",
        "revision",
    ] {
        if let Some(value) = old.get(key) {
            ack.insert(key.into(), value.clone());
        }
    }
    ack.insert("replay".into(), json!(true));
    Value::Object(ack)
}
/// Upstream error text is cut before it is stored or sent to the client.
fn clipped(text: &str) -> &str {
    rules::clip(text, 300)
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
    pub async fn snapshot(&self, cursor: &mut Cursor, ack: bool) -> Vec<Value> {
        let state = self.output.lock().await;
        let mut events = Vec::new();
        if ack {
            events.push(json!({"ack":self.ack}));
            // the ack already carries the first report; later ones go out as `context` events only when they change
            cursor.context_rev = u64::from(!self.ack["context"].is_null());
        }
        if state.units > cursor.offset {
            let from = if cursor.epoch == state.epoch && cursor.byte <= state.buffer.len() {
                cursor.byte
            } else {
                start_byte(&state.buffer, cursor.offset)
            };
            events.push(json!({"delta":&state.buffer[from..],"offset":state.units,"resumed":ack}));
            *cursor = Cursor {
                offset: state.units,
                byte: state.buffer.len(),
                epoch: state.epoch,
                context_rev: cursor.context_rev,
            };
        }
        if !state.context.is_null() && state.context_rev > cursor.context_rev {
            events.push(json!({"context":state.context}));
            cursor.context_rev = state.context_rev;
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
    pub providers: crate::providers::RuntimeProviders,
    pub events: broadcast::Sender<Value>,
    pub jobs: Arc<Notify>,
    pub locks: Locks,
    active: Arc<Mutex<HashMap<String, Arc<Generation>>>>,
    start_gate: Arc<Mutex<()>>,
    slots: Arc<Semaphore>,
    validation: Arc<Mutex<ValidationCache>>,
}
/// `route-validation.json` as last read: reloaded only when the file's modification time changes.
#[derive(Default)]
struct ValidationCache {
    loaded: bool,
    modified: Option<SystemTime>,
    value: Arc<Value>,
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
        let providers = crate::providers::RuntimeProviders::new(config.providers.clone());
        Self {
            store,
            local,
            client,
            config,
            providers,
            events,
            jobs,
            locks: Locks::default(),
            active: Arc::default(),
            start_gate: Arc::default(),
            slots,
            validation: Arc::default(),
        }
    }
    /// Route validation data. A turn only stats the file (async); it is read and parsed again when the file changed.
    pub async fn validation(&self) -> Arc<Value> {
        let file = self.config.data_dir.join("route-validation.json");
        let modified = tokio::fs::metadata(&file)
            .await
            .ok()
            .and_then(|m| m.modified().ok());
        let mut cache = self.validation.lock().await;
        if !cache.loaded || cache.modified != modified {
            cache.value = Arc::new(
                read_validation(&file).await
                    .unwrap_or(json!({"routes":{}})),
            );
            cache.modified = modified;
            cache.loaded = true;
        }
        cache.value.clone()
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
        None
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
                        ack: replay_ack(&old),
                        output: Mutex::new(Output::new(
                            s(&old, "content"),
                            Value::Null,
                            Some(json!({"done":true,"status":old["status"],"replay":true})),
                        )),
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
        let provider_snapshot = self.providers.snapshot();
        if provider_snapshot.is_none() && !self.config.is_android() {
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
        let validation = self.validation().await;
        let mut plan = presets::request(&conv, vec![], &validation);
        let requested_route = s(&conv, "routeId");
        let backend = if self.config.is_android() && requested_route == "mobile-local" {
            crate::providers::SelectedBackend::Local
        } else if let Some(providers) = &provider_snapshot {
            providers
                .select(if self.config.is_android() {
                    &requested_route
                } else {
                    plan.body["model"].as_str().unwrap_or_default()
                })
                .await
        } else {
            crate::providers::SelectedBackend::Local
        };
        if matches!(backend, crate::providers::SelectedBackend::Local) && !self.config.is_android()
        {
            return Err(Error::new(503, "Destino de fallback indisponível."));
        }
        if self.config.is_android() && requested_route != "mobile-local"
            && matches!(backend, crate::providers::SelectedBackend::Local)
        {
            return Err(Error::new(409,
                "Este modelo não está configurado no Android. Conecte ao PC para usar o modelo escolhido ou selecione explicitamente o modelo local nas opções do chat.")
                .extra("code", "REMOTE_MODEL_UNAVAILABLE"));
        }
        if self.config.is_android() {
            match &backend {
                crate::providers::SelectedBackend::Local => {
                    plan = presets::mobile_local_plan(&conv, &data["globalSettings"])
                }
                crate::providers::SelectedBackend::Cloud { provider, .. } => {
                    provider.prepare_mobile_plan(&mut plan)
                }
            }
        }
        if !presets::unrestricted_safety_enabled(&conv) {
            plan.controls["unrestrictedSafety"] = json!(false);
            if let Some(obj) = plan.body.as_object_mut() {
                obj.remove("safetySettings");
                obj.remove("safety_settings");
                obj.insert("unrestrictedSafety".into(), json!(false));
            }
        }
        let length = presets::apply_length(&mut plan, &data["globalSettings"]);
        let calibration = self
            .store
            .call(Request::Calibration(s(&conv, "routeId")))
            .await?["ratio"]
            .as_f64();
        let mut ctx = context::assemble(&data, &json!({}), &mode, &plan, calibration)?;
        if self.config.is_android() {
            ctx.report["routing"] = backend.diagnostic(&requested_route);
        }
        // hard cut of the generated text: the chosen size, or 135% of it in "approximate target" mode
        let bot_limit = rules::reply_bounds(&data["globalSettings"]).2;
        req.route_id = Some(s(&conv, "routeId"));
        req.preset_id = Some(s(&conv, "presetId"));
        req.effort = conv["effort"].as_str().map(str::to_owned);
        req.request = json!({"controls":plan.controls,"capacity":plan.capacity,"length":length,"presetVersion":presets::PRESET_VERSION,"systemMode":plan.system_mode});
        let begin = self.store.call(Request::BeginTurn(req)).await?;
        // No SetContext here: `run` always stores the (possibly enriched) report moments later, and nothing reads
        // `context_json` back (it is diagnostic only), so a crash in between loses nothing recoverable.
        let report = public_report(&ctx.report);
        let (wake, _) = broadcast::channel(32);
        let (cancel, _) = watch::channel(false);
        let ack = json!({"opId":begin["opId"],"generationId":begin["generationId"],"mode":mode,"interactionId":begin["interactionId"],"partId":begin["partId"],"userMessage":begin["userMessage"],"revision":begin["revision"],
            "preset":{"id":conv["presetId"],"label":presets::by_id(&s(&conv,"presetId"))["label"],"routeId":conv["routeId"],"effort":conv["effort"]},"controls":plan.controls,"capacity":plan.capacity,"context":report,"length":length});
        let g = Arc::new(Generation {
            id: s(&begin, "generationId"),
            conv: s(&conv, "id"),
            version: s(&begin, "versionId"),
            part: s(&begin, "partId"),
            interaction: s(&begin, "interactionId"),
            mode,
            ack,
            output: Mutex::new(Output::new(String::new(), report, None)),
            wake,
            cancel,
            ended: Notify::new(),
            created: Instant::now(),
            bot_limit,
        });
        self.active.lock().await.insert(g.conv.clone(), g.clone());
        let _=self.events.send(json!({"convId":g.conv,"type":"generation","generating":true,"revision":begin["revision"]}));
        plan.body["messages"] = Value::Array(std::mem::take(&mut ctx.messages));
        let this = self.clone();
        let task = g.clone();
        tokio::spawn(async move {
            this.run(task, data, plan, calibration, ctx, permit, backend)
                .await;
        });
        Ok((g, false))
    }
    async fn run(
        &self,
        g: Arc<Generation>,
        mut data: Value,
        mut plan: presets::Plan,
        calibration: Option<f64>,
        mut ctx: context::Context,
        _permit: OwnedSemaphorePermit,
        backend: crate::providers::SelectedBackend,
    ) {
        let query = s(&data["current"], "userText");
        let enrich = async {
            let vector = if !matches!(backend, crate::providers::SelectedBackend::Local)
                && self.local.embed_available()
            {
                tokio::time::timeout(Duration::from_secs(4), self.local.embed_text(&query, true))
                    .await
                    .ok()
                    .and_then(std::result::Result::ok)
            } else {
                None
            };
            let retrieval=self.store.call(Request::Retrieve(json!({"convId":g.conv,"queryText":query,"queryVec":vector,"embeddingModel":self.local.fingerprint().await,"beforeSeq":data["beforeSeq"],"windowStartSeq":context::retrieval_window_start(&data),"limit":24}))).await?;
            context::assemble(&data, &retrieval, &g.mode, &plan, calibration)
        };
        let enriched = tokio::select! {_=g.cancelled()=>None,v=tokio::time::timeout(Duration::from_secs(6),enrich)=>Some(v)};
        if let Some(Ok(Ok(full))) = enriched {
            ctx = full;
            plan.body["messages"] = Value::Array(std::mem::take(&mut ctx.messages));
        }
        let conv = data["conv"].take();
        let global_settings = data["globalSettings"].take();
        drop(data);
        drop(query);
        if self.config.is_android() {
            ctx.report["routing"] = backend.diagnostic(&s(&conv, "routeId"));
        }
        let report = public_report(&ctx.report);
        let _ = self
            .store
            .call(Request::SetContext {
                generation_id: g.id.clone(),
                value: report.clone(),
            })
            .await;
        // Consumers already got the first report in the ack: only a different one is published.
        let changed = {
            let mut output = g.output.lock().await;
            let changed = output.context != report;
            if changed {
                output.set_context(report);
            }
            changed
        };
        if changed {
            let _ = g.wake.send(());
        }
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
        // This guard survives the SSE consumer and style repair. Standby waits for its Drop.
        let session = if matches!(backend, crate::providers::SelectedBackend::Local) {
            Some(
                tokio::select! { _=g.cancelled()=>Err(Error::new(503, "Geração cancelada.")), session=self.local.chat_session()=>session },
            )
        } else {
            None
        };
        let mut error_code = Value::Null;
        let consume = async {
            let response = match &backend {
                crate::providers::SelectedBackend::Local => match session.as_ref().unwrap() {
                    Ok(session) => session.send(&plan.body, Duration::from_secs(900)).await?,
                    Err(e) => return Err(e.clone()),
                },
                crate::providers::SelectedBackend::Cloud { provider, model, .. } => provider.chat_request(&self.client, model, &plan.body, Duration::from_secs(900)).send().await
                    .map_err(|_| Error::new(502, "Requisição ao provedor falhou; nenhum destino alternativo foi enviado.").extra("code", "PROVIDER_UNAVAILABLE"))?,
            };
            if !response.status().is_success() {
                let status = response.status();
                let body_text = response.text().await.unwrap_or_default();
                warn!(generation = %g.id, http = status.as_u16(), body = %body_text, "provedor respondeu com erro");
                let detail = serde_json::from_str::<Value>(&body_text)
                    .ok()
                    .and_then(|v| {
                        v["error"]["message"].as_str()
                            .or_else(|| v["message"].as_str())
                            .or_else(|| v["msg"].as_str())
                            .or_else(|| v["displayTips"]["en"].as_str())
                            .map(str::to_owned)
                    });
                let provider_name = match &backend {
                    crate::providers::SelectedBackend::Cloud { provider, .. } =>
                        provider.id.as_str(),
                    _ => "mobile-local",
                };
                let err_msg = match detail {
                    Some(msg) => format!("Provedor {provider_name} HTTP {status}: {msg}"),
                    None => format!("Provedor {provider_name} HTTP {status}"),
                };
                return Err(Error::new(502, err_msg).extra("body", json!(body_text)));
            }
            let mut stream = response.bytes_stream();
            let mut buffer = Vec::<u8>::new();
            let mut event = String::new();
            let mut checkpoint = tokio::time::interval(Duration::from_secs(2));
            checkpoint.tick().await;
            // UTF-16 length of the text at the last checkpoint: a tick without new text neither clones nor stores it
            let mut flushed = 0usize;
            let mut idle_deadline = tokio::time::Instant::now() + Duration::from_secs(180);
            loop {
                let chunk = tokio::select! {
                    _=g.cancelled()=>break,
                    _=checkpoint.tick()=>{
                        let grown = {
                            let output = g.output.lock().await;
                            (output.units != flushed).then(|| (output.units, output.buffer.clone()))
                        };
                        if let Some((units, text)) = grown {
                            self.store.call(Request::FlushVersion{version_id:g.version.clone(),text}).await?;
                            flushed = units;
                        }
                        continue;
                    },
                    _=tokio::time::sleep_until(idle_deadline)=>return Err(Error::new(504,"O provedor ficou sem responder.")),
                    value=stream.next()=>value,
                };
                let Some(chunk) = chunk else {
                    break;
                };
                let chunk = chunk.map_err(|_| Error::new(502, "Fluxo do provedor interrompido."))?;
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
                        return Err(Error::new(502, "Erro no fluxo do provedor."));
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
            warn!(generation = %g.id, code = e.status, "geração falhou no provedor");
            error_code = e.extra["code"].clone();
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
                g.output.lock().await.replace(visible.clone());
            }
        }
        let size = rules::reply_size(&global_settings);
        if visible.trim().is_empty() && error.is_none() && !stopped {
            error = Some("O modelo retornou uma resposta vazia.".into());
        }
        let pct = global_settings["dialoguePct"]
            .as_f64()
            .unwrap_or(65.0);
        let before = style_policy::assess_settings(&visible, size, pct, &global_settings);
        let mut style_report = json!({"attempted":false,"before":before,"after":before});
        if !visible.trim().is_empty()
            && !stopped
            && error.is_none()
            // a reply cut by the limit (status length) is never rewritten: continuing it completes it
            && status == "complete"
            && before["ok"] != true
        {
            let repair_body = style_policy::repair_body(&plan.body, &visible,
                &style_policy::instruction_settings(&before,pct,&global_settings));
            style_report["inputEstimate"] = json!(rules::estimate_messages(repair_body["messages"].as_array().unwrap()));
            let repair = async {
                let response = match &backend {
                    crate::providers::SelectedBackend::Local => match session.as_ref().unwrap() {
                        Ok(session) => session.send(&repair_body, Duration::from_secs(180)).await?,
                        Err(e) => return Err(e.clone()),
                    },
                    crate::providers::SelectedBackend::Cloud {
                        provider, model, ..
                    } => provider
                        .chat_request(&self.client, model, &repair_body, Duration::from_secs(180))
                        .send()
                        .await
                        .map_err(|_| Error::new(502, "Revisão de estilo indisponível."))?,
                };
                let response = response
                    .error_for_status()
                    .map_err(|_| Error::new(502, "Revisão de estilo recusada."))?;
                crate::bounded_http::json(response, crate::bounded_http::STYLE_LIMIT)
                    .await
                    .map_err(|_| Error::new(502, "Revisão de estilo inválida."))
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
                let after =
                    style_policy::assess_settings(&candidate, size, pct, &global_settings);
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
                    g.output.lock().await.replace(visible.clone());
                    style_report["accepted"] = json!(true);
                    style_report["after"] = after;
                } else {
                    // "after" stays the saved text; the rejected candidate is recorded separately.
                    style_report["candidate"] = after;
                }
            } else if let Some(Err(err)) = repaired {
                style_report["error"] = json!(clipped(&err.to_string()));
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
                json!({"done":true,"status":fin["status"],"error":error,"code":error_code,"message":fin["message"],"revision":fin["revision"],"usage":usage})
            }
            Err(e) => {
                json!({"done":true,"status":"error","error":format!("Falha na persistência final; checkpoint preservado: {}",e.message)})
            }
        };
        // Metadata only: never the text, the prompt or the memories.
        info!(
            generation = %g.id,
            status = final_event["status"].as_str().unwrap_or("error"),
            ms = g.created.elapsed().as_millis() as u64,
            input = usage["input"].as_u64().unwrap_or(0),
            output = usage["output"].as_u64().unwrap_or(0),
            "geração encerrada"
        );
        self.active.lock().await.remove(&g.conv);
        // Release capacity before publishing done; its consumer may send immediately.
        drop(session);
        drop(_permit);
        g.output.lock().await.final_event = Some(final_event.clone());
        let _ = g.wake.send(());
        g.ended.notify_waiters();
        let _=self.events.send(json!({"convId":g.conv,"type":"message","revision":final_event["revision"],"generating":false}));
        self.jobs.notify_one();
        let request_log = match &backend {
            crate::providers::SelectedBackend::Cloud { provider, .. } => {
                provider.request_log.clone()
            }
            _ => None,
        };
        let last = if request_log.is_some() {
            plan.body["messages"]
                .as_array()
                .and_then(|m| m.iter().rev().find(|m| m["role"] == "user"))
                .map(|m| s(m, "content"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let (family, omit_off) = {
            let preset = presets::by_id(&s(&conv, "presetId"));
            (s(&preset, "family"), preset["thinkingOffShape"] == "omit")
        };
        let thinking_off = plan.controls["thinkingEnabled"] == false;
        let expected_effort = conv["effort"].clone();
        let generation_id = g.id.clone();
        // Read-only log retries need only this match key and these controls. Release
        // prompt/history/memory payloads and the task's output owner before waiting.
        drop(plan);
        drop(conv);
        drop(global_settings);
        drop(ctx);
        drop(visible);
        drop(measured);
        drop(style_report);
        drop(usage);
        drop(final_event);
        drop(filter);
        drop(backend);
        drop(g);
        if let Some(file) = request_log {
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
                json!({"checkedAt":crate::now(),"provider":check["provider"],"model":check["model"],"effortExpected":expected_effort,"effortSent":check["effort"],"effortOk":if thinking_off {off_check.clone()}else{json!(expected_effort==check["effort"])},"thinkingEnabled":!thinking_off,"thinkingOk":if thinking_off {off_check}else{Value::Null},"thinkingMode":check["thinking"],"maxTokens":check["maxTokens"],"usage":check["usage"]})
            } else {
                json!({"checkedAt":crate::now(),"notFound":true,"verification":"unverified","reason":check["reason"]})
            };
            let _ = self
                .store
                .call(Request::ProviderCheck {
                    generation_id: generation_id.clone(),
                    value,
                })
                .await;
        } else if self.config.is_android() {
            let _ = self.store.call(Request::ProviderCheck { generation_id,
                value: json!({"verification":"unverified","availability":"not_available","effortOk":null,"thinkingOk":null,"reason":"requestLog não disponível no mobile"}) }).await;
        }
    }
    async fn delta(&self, g: &Generation, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        let mut state = g.output.lock().await;
        let left = g.bot_limit.saturating_sub(state.units);
        let accepted = rules::clip(text, left);
        let capped =
            accepted.len() < text.len() || state.units + rules::utf16_len(accepted) >= g.bot_limit;
        state.append(accepted);
        drop(state);
        let _ = g.wake.send(());
        capped
    }
    pub async fn stop(&self, conv: &str, id: &str) -> Result<Value> {
        let g = if let Some(g) = self.get(conv, Some(id)).await {
            g
        } else {
            // A finalized generation is no longer retained by the supervisor. Stop
            // remains idempotent for persisted turns, as it was for the recent cache.
            self.persisted(conv, id).await.map_err(|e| {
                if e.status == 404 {
                    Error::new(404, "Nenhuma geração em andamento.")
                } else {
                    e
                }
            })?
        };
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
            ack: replay_ack(&old),
            output: Mutex::new(Output::new(
                s(&old, "content"),
                old["context"].clone(),
                Some(json!({"done":true,"status":old["status"],"replay":true})),
            )),
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
        let cursor = {
            let state = g.output.lock().await;
            let Some(rest) = rules::suffix(&state.buffer, from) else {
                return Err(Error::new(400, "Offset UTF-16 inválido."));
            };
            Cursor {
                offset: from,
                byte: state.buffer.len() - rest.len(),
                epoch: state.epoch,
                context_rev: 0,
            }
        };
        let wake = g.wake.subscribe();
        let stream = futures_util::stream::unfold(
            (g, wake, cursor, true, false, Vec::<Value>::new()),
            move |(g, mut wake, mut cursor, mut ack, mut done, mut events)| async move {
                loop {
                    if !events.is_empty() {
                        let event = events.remove(0);
                        if event["done"] == true {
                            done = true;
                        }
                        return Some((
                            Ok(axum::response::sse::Event::default().data(event.to_string())),
                            (g, wake, cursor, ack, done, events),
                        ));
                    }
                    if done {
                        return None;
                    }
                    if !ack {
                        let _ = wake.recv().await;
                    }
                    events = g.snapshot(&mut cursor, ack).await;
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
async fn read_validation(path: &std::path::Path) -> Option<Value> {
    use tokio::io::AsyncReadExt;
    const LIMIT: usize = 2 * 1024 * 1024;
    let mut file = tokio::fs::File::open(path).await.ok()?;
    if file.metadata().await.ok()?.len() >= LIMIT as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        // At the boundary, read at most one byte to detect a concurrently grown file.
        let left = LIMIT - bytes.len();
        let n = file.read(&mut chunk[..left.min(8192)]).await.ok()?;
        if n == 0 {
            return serde_json::from_slice(&bytes).ok();
        }
        let len = bytes.len().checked_add(n)?;
        if len >= LIMIT {
            return None;
        }
        if len > bytes.capacity() {
            let capacity = bytes.capacity().saturating_mul(2).max(len).min(LIMIT);
            bytes.reserve_exact(capacity - bytes.len());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
}

pub async fn write_character(file: &std::path::Path, text: &str) -> Result<()> {
    tokio::fs::create_dir_all(file.parent().unwrap()).await?;
    let tmp = file.with_extension(format!("{}.tmp", crate::id()));
    tokio::fs::write(&tmp, text).await?;
    tokio::fs::rename(&tmp, file).await?;
    Ok(())
}
