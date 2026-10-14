use super::{Database, PATH_CTE};
use crate::{Error, Result, id, n, now, parsed, s};
use serde_json::{Value, json};
use std::collections::HashSet;

impl Database {
    pub(super) fn enqueue_job(&self, p: &Value) -> Result<Value> {
        let c = s(p, "convId");
        self.conv(&c, false)?;
        let kind = p["kind"].as_str().unwrap_or("turn");
        if !["turn", "episode", "cast", "curate"].contains(&kind) {
            return Err(Error::new(400, "Tipo de trabalho inválido."));
        }
        let jid = id();
        self.exec("INSERT INTO jobs(id,conv_id,interaction_id,version_id,kind,status,payload_json,created_at,updated_at) VALUES(?,?,?,?,?,'pending',?,?,?)",&[json!(jid),json!(c),p["interactionId"].clone(),p["versionId"].clone(),json!(kind),json!(p["payload"].as_object().map(|_|p["payload"].to_string()).unwrap_or("{}".into())),json!(now()),json!(now())])?;
        Ok(json!({"id":jid}))
    }
    // ---- memory curation (memory × context management) ----
    fn active_memories(&self, c: &str) -> Result<i64> {
        Ok(n(
            &self.one("SELECT COUNT(*) n FROM memories WHERE conv_id=? AND status='current' AND validity='ok' AND review_state='ok'", &[json!(c)])?,
            "n",
        ))
    }
    fn curation_key(c: &str) -> String {
        format!("curation:{c}")
    }
    /// Schedules ONE curation when there are too many memories AND enough new ones since the last; otherwise nothing.
    pub(super) fn maybe_queue_curation(&self, c: &str) -> Result<Value> {
        let total = self.active_memories(c)?;
        let last = self.one(
            "SELECT value FROM meta WHERE key=?",
            &[json!(Self::curation_key(c))],
        )?["value"]
            .as_str()
            .and_then(|t| serde_json::from_str::<Value>(t).ok())
            .and_then(|v| v["count"].as_i64())
            .unwrap_or(0);
        if total < crate::curation::MIN_ACTIVE || total - last < crate::curation::MIN_NEW {
            return Ok(json!(false));
        }
        let pending = self.one(
            "SELECT 1 x FROM jobs WHERE conv_id=? AND kind='curate' AND status IN ('pending','running')",
            &[json!(c)],
        )?;
        if !pending.is_null() {
            return Ok(json!(false));
        }
        self.enqueue_job(&json!({"convId":c,"kind":"curate"}))?;
        Ok(json!(true))
    }
    /// Groups of near-duplicate memories (public, not pinned and without per-character knowledge).
    pub(super) fn curation_candidates(&self, c: &str) -> Result<Value> {
        let rows = self.rows(
            "SELECT m.id,m.kind,m.subject,m.content,m.importance FROM memories m WHERE m.conv_id=? AND m.status='current' AND m.validity='ok' AND m.review_state='ok' AND m.pinned=0 AND m.visibility='public' AND NOT EXISTS (SELECT 1 FROM memory_knowledge k WHERE k.memory_id=m.id) ORDER BY m.created_at ASC,m.rowid ASC LIMIT 400",
            &[json!(c)],
        )?;
        let clusters = crate::curation::build_clusters(&rows);
        Ok(json!({"total":self.active_memories(c)?,"clusters":clusters}))
    }
    /// Applies already sanitized decisions: the kept memory gets the summary text; the others become "historical"
    /// (superseded by it, recoverable). Nothing is deleted.
    pub(super) fn apply_curation(&self, p: &Value) -> Result<Value> {
        let c = s(p, "convId");
        self.conv(&c, false)?;
        let t = now();
        let pick = |id: &str| {
            self.one(
                "SELECT * FROM memories WHERE id=? AND conv_id=? AND status='current' AND pinned=0 AND visibility='public'",
                &[json!(id), json!(c)],
            )
        };
        let (mut merged, mut retired) = (0u64, 0u64);
        for d in p["decisions"].as_array().into_iter().flatten() {
            let keep = pick(d["keepId"].as_str().unwrap_or_default())?;
            if keep.is_null() {
                continue;
            }
            let mut drops = Vec::new();
            for id in d["dropIds"].as_array().into_iter().flatten() {
                let o = pick(id.as_str().unwrap_or_default())?;
                if !o.is_null() {
                    drops.push(o);
                }
            }
            if drops.is_empty() {
                continue;
            }
            let content = crate::rules::clip(
                d["merged"]
                    .as_str()
                    .filter(|x| !x.is_empty())
                    .unwrap_or_else(|| keep["content"].as_str().unwrap_or_default()),
                500,
            )
            .to_owned();
            let importance = drops
                .iter()
                .map(|o| o["importance"].as_f64().unwrap_or(0.0))
                .fold(keep["importance"].as_f64().unwrap_or(0.0), f64::max);
            let kid = s(&keep, "id");
            self.exec(
                "UPDATE memories SET content=?,importance=?,last_confirmed_at=? WHERE id=?",
                &[json!(content), json!(importance), json!(t), json!(kid)],
            )?;
            self.unindex("memory", &kid)?;
            self.index(
                &c,
                "memory",
                &kid,
                &format!("{} {content}", s(&keep, "subject")),
            )?;
            self.exec(
                "DELETE FROM embeddings WHERE owner_type='memory' AND owner_id=?",
                &[json!(kid)],
            )?;
            for o in &drops {
                let oid = s(o, "id");
                self.exec(
                    "INSERT OR IGNORE INTO memory_replacements(conv_id,old_id,new_id) VALUES(?,?,?)",
                    &[json!(c), json!(oid), json!(kid)],
                )?;
                self.exec(
                    "UPDATE memories SET status='historical',superseded_by=? WHERE id=?",
                    &[json!(kid), json!(oid)],
                )?;
                self.unindex("memory", &oid)?;
                retired += 1;
            }
            merged += 1;
        }
        self.exec(
            "INSERT OR REPLACE INTO meta(key,value) VALUES(?,?)",
            &[
                json!(Self::curation_key(&c)),
                json!(json!({"count":self.active_memories(&c)?,"at":t}).to_string()),
            ],
        )?;
        let revision = if merged > 0 {
            json!(self.bump(&c)?)
        } else {
            Value::Null
        };
        Ok(json!({"merged":merged,"retired":retired,"revision":revision}))
    }
    pub(super) fn claim_job(&self) -> Result<Value> {
        let last = self.one("SELECT value FROM meta WHERE key='last_job_conv'", &[])?;
        let r=self.one("SELECT j.* FROM jobs j JOIN conversations c ON c.id=j.conv_id WHERE j.status='pending' AND c.deleted_at IS NULL ORDER BY (j.conv_id=?),CASE j.kind WHEN 'turn' THEN 0 WHEN 'cast' THEN 1 ELSE 2 END,j.created_at,j.rowid LIMIT 1",&[last["value"].clone()])?;
        if r.is_null() {
            return Ok(Value::Null);
        }
        self.exec(
            "UPDATE jobs SET status='running',attempts=attempts+1,updated_at=? WHERE id=?",
            &[json!(now()), r["id"].clone()],
        )?;
        self.exec(
            "INSERT OR REPLACE INTO meta(key,value) VALUES('last_job_conv',?)",
            &[r["conv_id"].clone()],
        )?;
        let mut r = r;
        r["attempts"] = json!(n(&r, "attempts") + 1);
        r["payload"] = parsed(&r, "payload_json", json!({}));
        Ok(r)
    }
    pub(super) fn finish_job(&self, jid: &str, status: &str, error: Option<&str>) -> Result<Value> {
        if !["pending", "running", "done", "failed", "blocked", "skipped"].contains(&status) {
            return Err(Error::new(400, "Estado de trabalho inválido."));
        }
        self.exec(
            "UPDATE jobs SET status=?,error=?,updated_at=? WHERE id=? AND status!='skipped'",
            &[
                json!(status),
                json!(error.map(|e| crate::rules::clip(e, 500))),
                json!(now()),
                json!(jid),
            ],
        )?;
        Ok(json!(true))
    }
    /// The claim already counted an attempt; an unavailable model is not the input's fault, so the attempt is
    /// refunded. Otherwise each wait for the model to load pushed the job to the reduced/minimal context and to "failed".
    pub(super) fn defer_job(&self, jid: &str, error: &str) -> Result<Value> {
        self.exec(
            "UPDATE jobs SET status='pending',attempts=MAX(attempts-1,0),error=?,updated_at=? WHERE id=? AND status='running'",
            &[
                json!(crate::rules::clip(error, 500)),
                json!(now()),
                json!(jid),
            ],
        )?;
        Ok(json!(true))
    }
    pub(super) fn job_input(&self, jid: &str) -> Result<Value> {
        let job = self.one("SELECT * FROM jobs WHERE id=?", &[json!(jid)])?;
        if job.is_null() {
            return Ok(Value::Null);
        }
        let c = s(&job, "conv_id");
        let iid = s(&job, "interaction_id");
        if self.conv(&c, false).is_err() {
            return Ok(json!({"job": job, "gone": true}));
        }
        let inter = self.one(
            "SELECT * FROM interactions WHERE id=? AND conv_id=? AND deleted_at IS NULL",
            &[json!(iid), json!(c)],
        )?;
        let eligible = self.eligibility(&c, i64::MAX)?;
        if inter.is_null() || !eligible.seq.contains_key(&iid) {
            return Ok(json!({"job":job,"gone":true}));
        }
        let parts=self.rows("SELECT v.id,v.content,p.idx FROM parts p JOIN versions v ON v.id=p.selected_version_id WHERE p.interaction_id=? AND p.deleted_at IS NULL AND v.status!='generating' ORDER BY p.idx",&[json!(iid)])?;
        let idx = if let Some(v) = job["version_id"].as_str() {
            parts.iter().position(|r| r["id"] == v)
        } else {
            parts.len().checked_sub(1)
        };
        let Some(idx) = idx else {
            return Ok(json!({"job":job,"gone":true}));
        };
        let parts = &parts[..=idx];
        let conv = self.conversation(&c, false)?;
        let texts = parts.iter().map(|p| s(p, "content")).collect::<Vec<_>>();
        let mut sources = Vec::new();
        if let Some(t) = inter["user_text"].as_str() {
            sources.push(crate::provenance::source(
                &iid,
                "user",
                t,
                Some(&s(&conv, "persona")),
            ));
        }
        for part in parts {
            sources.push(crate::provenance::source(
                &s(part, "id"),
                "assistant",
                &s(part, "content"),
                None,
            ));
        }
        let text = sources.join("\n\n");
        let data = self.context_data(&json!({"convId":c,"interactionId":iid,"maxPairs":3}))?;
        let retrieval=self.retrieve(&json!({"convId":c,"queryText":text,"beforeSeq":inter["seq"],"windowStartSeq":inter["seq"],"limit":8}))?;
        let previous = data["pairs"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|p| p["sources"].as_array().into_iter().flatten())
            .map(|source| {
                crate::provenance::source(
                    &s(source, "id"),
                    &s(source, "role"),
                    &s(source, "text"),
                    None,
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let version = &parts.last().unwrap()["id"];
        let flags = self.one(
            "SELECT count(*) n FROM review_flags WHERE version_id=? AND status='open'",
            std::slice::from_ref(version),
        )?;
        Ok(
            json!({"job":job,"gone":false,"conv":conv,"interaction":{"id":iid,"seq":inter["seq"]},"userText":inter["user_text"],"assistantText":texts.last(),"prelude":texts[..texts.len()-1].join("\n\n"),"previous":previous,"timeline":data["timeline"],"recentSources":crate::provenance::sources(data["pairs"].as_array().unwrap()),"text":text,"versionId":version,"versionIds":parts.iter().map(|p|p["id"].clone()).collect::<Vec<_>>(),"scene":data["scene"],"cast":data["cast"],"related":retrieval["candidates"].as_array().unwrap().iter().filter(|m|m["type"]=="memory").cloned().collect::<Vec<_>>(),"threads":data["threads"].as_array().unwrap().iter().map(|t|t["text"].clone()).collect::<Vec<_>>(),"openFlags":flags["n"]}),
        )
    }
    pub(super) fn add_flags(&self, p: &Value) -> Result<Value> {
        let c = s(p, "convId");
        let version = s(p, "versionId");
        let row=self.one("SELECT i.id FROM versions v JOIN parts p ON p.id=v.part_id JOIN interactions i ON i.id=p.interaction_id WHERE v.id=? AND i.conv_id=?",&[json!(version),json!(c)])?;
        if row.is_null() {
            return Err(Error::new(404, "Versão da sinalização não encontrada."));
        }
        if p["jobId"].is_string() {
            let input = self.job_input(&s(p, "jobId"))?;
            if input["gone"] == true
                || input["versionId"] != p["versionId"]
                || input["job"]["status"] != "running"
            {
                return Ok(json!({"skipped":true}));
            }
        }
        for flag in p["flags"].as_array().into_iter().flatten() {
            let detail = s(flag, "detail");
            let kind = s(flag, "kind");
            if detail.is_empty() || !["agency", "knowledge", "continuity"].contains(&kind.as_str())
            {
                continue;
            }
            if !self.one("SELECT id FROM review_flags WHERE version_id=? AND kind=? AND evidence=? AND status='open'",&[json!(version),json!(kind),flag["evidence"].clone()])?.is_null(){continue;}
            self.exec("INSERT INTO review_flags(id,conv_id,version_id,interaction_id,kind,detail,evidence,source,created_at) VALUES(?,?,?,?,?,?,?,?,?)",&[json!(id()),json!(c),json!(version),row["id"].clone(),json!(kind),json!(crate::rules::clip(&detail,400)),flag["evidence"].clone(),json!(p["source"].as_str().unwrap_or("llm")),json!(now())])?;
        }
        self.exec("UPDATE jobs SET status='blocked',updated_at=? WHERE version_id=? AND status IN ('pending','running') AND EXISTS(SELECT 1 FROM review_flags WHERE version_id=? AND status='open')",&[json!(now()),json!(version),json!(version)])?;
        Ok(json!({"revision":self.bump(&c)?}))
    }
    pub(super) fn resolve_flag(&self, c: &str, fid: &str, action: &str) -> Result<Value> {
        let flag = self.one(
            "SELECT * FROM review_flags WHERE conv_id=? AND id=?",
            &[json!(c), json!(fid)],
        )?;
        if flag.is_null() {
            return Err(Error::new(404, "Sinalização não encontrada."));
        }
        let status = match action {
            "accept" => "accepted",
            "dismiss" => "dismissed",
            "corrected" => "corrected",
            _ => return Err(Error::new(400, "Ação inválida.")),
        };
        self.exec(
            "UPDATE review_flags SET status=?,resolved_at=? WHERE id=?",
            &[json!(status), json!(now()), json!(fid)],
        )?;
        let left = self.one(
            "SELECT count(*) n FROM review_flags WHERE version_id=? AND status='open'",
            &[flag["version_id"].clone()],
        )?;
        let mut unblocked = false;
        if n(&left, "n") == 0 {
            unblocked=self.exec("UPDATE jobs SET status='pending',payload_json='{\"skipReview\":true}',updated_at=? WHERE version_id=? AND status='blocked'",&[json!(now()),flag["version_id"].clone()])?>0;
            self.exec(
                "UPDATE memories SET review_state='ok' WHERE source_version_id=?",
                &[flag["version_id"].clone()],
            )?;
        }
        Ok(json!({"revision":self.bump(c)?,"unblocked":unblocked}))
    }
    pub(super) fn queue_episodes(&self, c: &str) -> Result<Value> {
        let path=self.rows(&format!("{PATH_CTE} SELECT i.id,i.seq,n.summary,n.version_ids_json FROM p JOIN interactions i ON i.id=p.id LEFT JOIN interaction_notes n ON n.interaction_id=i.id WHERE i.deleted_at IS NULL ORDER BY i.seq"),&[json!(c)])?;
        let cutoff = path
            .get(path.len().saturating_sub(30))
            .map(|i| n(i, "seq"))
            .unwrap_or(-1);
        if path.len() <= 30 {
            return Ok(json!(0));
        }
        let mut covered = HashSet::new();
        for e in self.rows(
            "SELECT interaction_ids_json FROM episodes WHERE conv_id=? AND validity='ok'",
            &[json!(c)],
        )? {
            covered.extend(
                parsed(&e, "interaction_ids_json", json!([]))
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned),
            );
        }
        let scenes=self.rows("SELECT interaction_id,location FROM scene_states WHERE conv_id=? ORDER BY seq,created_at,rowid",&[json!(c)])?.into_iter().map(|v|(s(&v,"interaction_id"),s(&v,"location"))).collect::<std::collections::HashMap<_,_>>();
        let mut groups = Vec::<Vec<Value>>::new();
        let mut group = Vec::<Value>::new();
        let mut previous_scene = String::new();
        for i in path.iter().filter(|i| {
            n(i, "seq") < cutoff && i["summary"].is_string() && !covered.contains(&s(i, "id"))
        }) {
            let location = scenes.get(&s(i, "id")).cloned().unwrap_or_default();
            let scene_changed = !location.is_empty()
                && !previous_scene.is_empty()
                && crate::rules::key(&location) != crate::rules::key(&previous_scene);
            let topic_changed = group.len() >= 4
                && crate::rules::overlap(
                    &crate::rules::tokens(&s(i, "summary")),
                    &crate::rules::tokens(&s(group.last().unwrap(), "summary")),
                ) < 0.12;
            if !group.is_empty() && (group.len() >= 10 || scene_changed || topic_changed) {
                groups.push(std::mem::take(&mut group));
            }
            group.push(i.clone());
            if !location.is_empty() {
                previous_scene = location;
            }
        }
        if group.len() >= 10 {
            groups.push(group);
        }
        let mut queued = 0;
        for group in groups {
            let ids = group.iter().map(|i| i["id"].clone()).collect::<Vec<_>>();
            let key = json!(ids).to_string();
            if self.one("SELECT id FROM jobs WHERE conv_id=? AND kind='episode' AND status IN ('pending','running','failed') AND json_extract(payload_json,'$.key')=?",&[json!(c),json!(key)])?.is_null(){
                let notes=group.iter().map(|i|json!({"interaction_id":i["id"],"summary":i["summary"],"version_ids_json":i["version_ids_json"]})).collect::<Vec<_>>();
                self.enqueue_job(&json!({"convId":c,"kind":"episode","payload":{"key":key,"interactionIds":ids,"fromSeq":group.first().unwrap()["seq"],"toSeq":group.last().unwrap()["seq"],"notes":notes}}))?;queued+=1;
            }
        }
        Ok(json!(queued))
    }

    pub(super) fn create_episode(&self, p: &Value) -> Result<Value> {
        let c = s(p, "convId");
        let eligible = self.eligibility(&c, i64::MAX)?;
        let ids = p["interactionIds"]
            .as_array()
            .ok_or_else(|| Error::new(400, "Fontes do episódio ausentes."))?;
        if !ids
            .iter()
            .all(|i| eligible.seq.contains_key(i.as_str().unwrap_or_default()))
        {
            return Ok(json!({"skipped":true}));
        }
        for note in p["expectedNotes"].as_array().into_iter().flatten() {
            let old = self.one(
                "SELECT summary,version_ids_json FROM interaction_notes WHERE interaction_id=?",
                &[note["interaction_id"].clone()],
            )?;
            if old["summary"] != note["summary"]
                || old["version_ids_json"] != note["version_ids_json"]
            {
                return Ok(json!({"skipped":true}));
            }
        }
        let eid = id();
        let summary = s(p, "summary");
        if summary.trim().is_empty() {
            return Err(Error::new(422, "Resumo episódico vazio."));
        }
        if !self
            .one(
                "SELECT id FROM episodes WHERE conv_id=? AND interaction_ids_json=?",
                &[json!(c), json!(json!(ids).to_string())],
            )?
            .is_null()
        {
            return Ok(json!({"skipped":true}));
        }
        self.exec("INSERT INTO episodes(id,conv_id,from_seq,to_seq,interaction_ids_json,summary,created_at) VALUES(?,?,?,?,?,?,?)",&[json!(eid),json!(c),p["fromSeq"].clone(),p["toSeq"].clone(),json!(json!(ids).to_string()),json!(summary),json!(now())])?;
        self.index(&c, "episode", &eid, &summary)?;
        Ok(json!({"id":eid}))
    }
    pub(super) fn source(&self, c: &str, iid: &str) -> Result<Value> {
        let inter = self.one(
            "SELECT * FROM interactions WHERE conv_id=? AND id=?",
            &[json!(c), json!(iid)],
        )?;
        if inter.is_null() {
            return Err(Error::new(404, "Trecho de origem não encontrado."));
        }
        let parts = self.part_messages(&inter)?;
        let mut sources = Vec::new();
        if inter["user_text"].is_string() {
            sources.push(json!({"id":iid,"role":"user","text":inter["user_text"]}));
        }
        sources.extend(
            parts
                .iter()
                .map(|p| json!({"id":p["versionId"],"role":"assistant","text":p["content"]})),
        );
        Ok(
            json!({"interactionId":iid,"seq":inter["seq"],"deleted":!inter["deleted_at"].is_null(),"user":inter["user_text"],"assistant":parts.iter().map(|p|s(p,"content")).collect::<Vec<_>>().join("\n\n"),"sources":sources}),
        )
    }
}
