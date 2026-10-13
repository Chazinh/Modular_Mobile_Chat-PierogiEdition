//! Persisted local-only jobs; one supervisor, bounded work, source/version checks.
use crate::{
    Error, Result,
    local::LocalModels,
    review, rules, s,
    store::{EmbeddingItem, Request, Store},
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, Notify, broadcast, watch};
#[derive(Default)]
struct State {
    running: bool,
    last_error: Option<String>,
}
/// Parsed once; every reviewed chunk reuses it.
fn review_schema() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::from_str(include_str!("review-schema.json"))
            .expect("bundled review schema must be valid JSON")
    })
}
#[derive(Clone)]
pub struct Runner {
    store: Store,
    local: LocalModels,
    wake: Arc<Notify>,
    events: broadcast::Sender<Value>,
    state: Arc<Mutex<State>>,
    stop: watch::Sender<bool>,
    review: bool,
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
        Self {
            store,
            local,
            wake,
            events,
            state: Arc::default(),
            stop,
            review,
        }
    }
    pub async fn state(&self) -> Value {
        let s = self.state.lock().await;
        json!({"running":s.running,"lastError":s.last_error})
    }
    pub fn start(&self) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this.store.call(Request::RequeueJobs).await {
                this.state.lock().await.last_error = Some(e.message);
            }
            let mut stop = this.stop.subscribe();
            let mut timer = tokio::time::interval(Duration::from_secs(30));
            loop {
                tokio::select! {_=stop.wait_for(|s|*s)=>break,_=this.wake.notified()=>{},_=timer.tick()=>{}}
                this.state.lock().await.running = true;
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
                            this.state.lock().await.last_error = Some(e.message);
                            break;
                        }
                    }
                }
                this.state.lock().await.running = false;
            }
            this.state.lock().await.running = false;
        })
    }
    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }
    pub fn kick(&self) {
        self.wake.notify_one();
    }
    async fn cycle(&self) -> Result<bool> {
        let job = self.store.call(Request::ClaimJob).await?;
        if job.is_null() {
            return Ok(self.embeddings().await? > 0);
        }
        let outcome = self.handle(&job).await;
        let (status, error, continue_work) = match outcome {
            Ok(status) => (status, None, true),
            Err(e) if e.extra["code"] == "LOCAL_UNAVAILABLE" => {
                self.state.lock().await.last_error = Some(e.message.clone());
                self.store
                    .call(Request::DeferJob {
                        id: s(&job, "id"),
                        error: e.message,
                    })
                    .await?;
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
                self.state.lock().await.last_error = Some(e.message.clone());
                (status.into(), Some(e.message), true)
            }
        };
        let done_turn = job["kind"] == "turn" && status == "done";
        self.store
            .call(Request::FinishJob {
                id: s(&job, "id"),
                status,
                error,
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
        if let Err(e) = self.embeddings().await {
            self.state.lock().await.last_error = Some(e.message);
        }
        Ok(continue_work)
    }
    async fn handle(&self, job: &Value) -> Result<String> {
        match job["kind"].as_str() {
            Some("turn") => self.turn(job).await,
            Some("episode") => self.episode(job).await,
            Some("cast") => self.cast(job).await,
            Some("curate") => self.curate(job).await,
            _ => Ok("skipped".into()),
        }
    }
    /// Memory curation: groups near-duplicate memories (deterministic decision) and asks the local model,
    /// in a single call, only which one to keep and the summary sentence. Without groups, the model is not called.
    async fn curate(&self, job: &Value) -> Result<String> {
        if !self.local.chat_available() {
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
        let raw = self
            .local
            .chat_json(
                crate::curation::messages(&clusters),
                crate::curation::schema(),
                900,
                0.1,
            )
            .await?;
        let decisions = crate::curation::sanitize(&raw, &clusters);
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
    async fn turn(&self, job: &Value) -> Result<String> {
        let input = self.store.call(Request::JobInput(s(job, "id"))).await?;
        if input["gone"] == true || s(&input, "text").trim().is_empty() {
            return Ok("skipped".into());
        }
        if !self.local.chat_available() {
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
            for (start, _, text) in rules::chunks(&assistant, 8000, 240) {
                let prefix = format!(
                    "{}\n{}",
                    s(&input, "prelude"),
                    &assistant[..rules::byte_at(&assistant, start)]
                );
                let raw = self
                    .local
                    .chat_json(
                        review::messages(&input, &text, &prefix, &secrets),
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
            if level == 0 || chars <= n {
                t.to_string()
            } else {
                t.chars().skip(chars - n).collect()
            }
        };
        for (_, _, text) in rules::chunks(&full, if level == 0 { 5000 } else { 3000 }, 160) {
            let retrieval = if level == 0 {
                self.store.call(Request::Retrieve(json!({"convId":job["conv_id"],"queryText":text,"beforeSeq":input["interaction"]["seq"],"limit":8}))).await?
            } else {
                json!({"candidates":[]})
            };
            let sources = if level >= 2 {
                Vec::new()
            } else {
                input["recentSources"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            };
            let prompt = format!(
                "PERSONA: {}\nCENA ANTERIOR: {scene}\nTEMPO: {}\nFATOS ANTERIORES: {related}\nRECUPERAÇÃO: {}\nTHREADS: {}\nHISTÓRICO ANTERIOR RELEVANTE (fontes identificadas):\n{}\n{}\nTEXTO DO TURNO (fragmento integral):\n{text}",
                input["conv"]["persona"],
                input["timeline"]["current"],
                retrieval["candidates"],
                input["threads"],
                tail(&s(&input, "previous"), 1500),
                crate::provenance::block(&s(&input, "userText"), &sources)
            );
            let messages = vec![
                json!({"role":"system","content":include_str!("extraction-prompt.txt").replace("{{persona}}", input["conv"]["persona"].as_str().unwrap_or("o usuário"))}),
                json!({"role":"user","content":prompt}),
            ];
            let raw = self
                .local
                .chat_json(
                    messages,
                    serde_json::from_str(include_str!("extraction-schema.json")).unwrap(),
                    if level == 0 { 2200 } else { 3000 },
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
                if f.get("witnesses").is_none() {
                    f["witnesses"] = json!(present);
                }
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
    async fn episode(&self, job: &Value) -> Result<String> {
        let payload = &job["payload"];
        let ids = payload["interactionIds"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if ids.is_empty() {
            return Ok("skipped".into());
        }
        let notes = payload["notes"].as_array().cloned().unwrap_or_default();
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
        if self.local.chat_available() {
            for (_, _, text) in rules::chunks(&original, 5000, 0) {
                let raw=self.local.chat_json(vec![json!({"role":"system","content":"Resuma em até 4 frases o episódio em português, preservando nomes, decisões, segredos revelados e promessas. Responda somente JSON."}),json!({"role":"user","content":text})],json!({"type":"object","properties":{"summary":{"type":"string"}},"required":["summary"]}),500,0.1).await?;
                let summary = s(&raw, "summary");
                if summary.trim().is_empty() {
                    return Err(Error::new(502, "Resumo episódico vazio."));
                }
                summaries.push(summary);
            }
        } else {
            summaries.push(original);
        }
        let result=self.store.call(Request::CreateEpisode(json!({"convId":job["conv_id"],"interactionIds":ids,"fromSeq":payload["fromSeq"],"toSeq":payload["toSeq"],"summary":summaries.join("\n"),"expectedNotes":notes}))).await?;
        Ok(if result["skipped"] == true {
            "skipped"
        } else {
            "done"
        }
        .into())
    }
    async fn cast(&self, job: &Value) -> Result<String> {
        let conv = self.store.call(Request::GetMeta(s(job, "conv_id"))).await?;
        let text = s(&conv, "prompt");
        let heuristic = crate::local::cast_from_card(&text, &s(&conv, "persona"));
        self.store
            .call(Request::ReanalyzeCast(s(job, "conv_id")))
            .await?;
        if heuristic.len() < 2 && !text.trim().is_empty() && self.local.chat_available() {
            for (_, _, text) in rules::chunks(&text, 5000, 160) {
                let raw=self.local.chat_json(vec![json!({"role":"system","content":include_str!("cast-prompt.txt").replace("{{persona}}", conv["persona"].as_str().unwrap_or("o usuário"))}),json!({"role":"user","content":format!("PERSONA: {}\nFICHA:\n{text}",conv["persona"])})],serde_json::from_str(include_str!("cast-schema.json")).unwrap(),1500,0.1).await?;
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
        if !self.local.embed_available() {
            return Ok(0);
        }
        let model = self.local.fingerprint();
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
pub fn sanitize_extraction(raw: &Value) -> Result<Value> {
    if !raw.is_object()
        || raw["summary"].as_str().is_none_or(|s| s.trim().is_empty())
        || !raw["facts"].is_array()
    {
        return Err(Error::new(
            502,
            "Extração local incompleta; turno permanece pendente.",
        ));
    }
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
    let mut out = json!({"summary":clean(&raw["summary"],500),"cast":[],"scene":null,"facts":[],"relationships":[],"openThreads":[],"resolvedThreads":[]});
    for c in raw["cast"].as_array().into_iter().flatten().take(10) {
        if let Some(name) = crate::cast::name(&s(c, "name")) {
            out["cast"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name":name,"description":clean(&c["description"],300)}));
        }
    }
    if raw["scene"].is_object() {
        let scene = &raw["scene"];
        let mut present = Vec::<String>::new();
        for v in scene["present"].as_array().into_iter().flatten() {
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
        for c in scene["characters"]
            .as_array()
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
                name,
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
        let location = crate::cast::known(&s(scene, "location"), 80);
        let time = crate::cast::known(&s(scene, "time"), 60);
        if location.is_some() || time.is_some() || !present.is_empty() || !positions.is_empty() {
            out["scene"] = json!({"location":location,"time":time,"mood":crate::cast::known(&s(scene,"mood"),80),"present":present,"positions":positions,"locations":locations,"holding":holding,"objects":scene["objects"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|s|crate::cast::clean(s,50)).filter(|s|!s.is_empty()).take(12).collect::<Vec<_>>()});
        }
    }
    for f in raw["facts"].as_array().into_iter().flatten().take(14) {
        let content = clean(&f["content"], 300);
        if content.as_str().unwrap().len() < 6 {
            continue;
        }
        let mut fact = f.clone();
        fact["content"] = content;
        fact["subject"] = clean(&f["subject"], 60);
        fact["replaces"] = clean(&f["replaces"], 300);
        fact["existingId"] = f["existing_id"].clone();
        fact["kind"] = json!(
            f["kind"]
                .as_str()
                .filter(|k| [
                    "CHARACTER_FACT",
                    "USER_FACT",
                    "RELATIONSHIP",
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
        .into_iter()
        .flatten()
        .take(8)
    {
        if !s(r, "a").is_empty() && !s(r, "b").is_empty() && !s(r, "description").is_empty() {
            out["relationships"].as_array_mut().unwrap().push(r.clone());
        }
    }
    for (k, source) in [
        ("openThreads", "open_threads"),
        ("resolvedThreads", "resolved_threads"),
    ] {
        for t in raw[source].as_array().into_iter().flatten().take(5) {
            out[k].as_array_mut().unwrap().push(clean(t, 220));
        }
    }
    Ok(out)
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
    let mut out = sanitize_extraction(raw)?;
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
