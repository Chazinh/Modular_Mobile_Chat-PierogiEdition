use super::{Database, PATH_CTE};
use crate::{Error, Result, id, n, now, parsed, rules, s};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

pub(super) struct Eligibility {
    pub seq: HashMap<String, i64>,
    pub selected: HashSet<String>,
    pub max_seq: i64,
}
impl Eligibility {
    /// The rows of `active_path` with seq below `before` (one path query serves several cuts).
    pub(super) fn from_path(rows: &[Value], before: i64) -> Self {
        let mut seq = HashMap::new();
        let mut selected = HashSet::new();
        let mut max_seq = 0;
        for r in rows.iter().filter(|r| n(r, "seq") < before) {
            seq.insert(s(r, "id"), n(r, "seq"));
            max_seq = max_seq.max(n(r, "seq"));
            if r["version_id"].is_string() {
                selected.insert(s(r, "version_id"));
            }
        }
        Self {
            seq,
            selected,
            max_seq,
        }
    }
}
impl Database {
    pub(super) fn eligibility(&self, c: &str, before: i64) -> Result<Eligibility> {
        Ok(Eligibility::from_path(&self.active_path(c, before)?, before))
    }
    /// Active path rows (interaction, seq, selected version) before `before`, for one or more `Eligibility` cuts.
    pub(super) fn active_path(&self, c: &str, before: i64) -> Result<Vec<Value>> {
        self.rows(&format!("{PATH_CTE} SELECT i.id,i.seq,v.id version_id FROM p active JOIN interactions i ON i.id=active.id LEFT JOIN parts p ON p.interaction_id=i.id AND p.deleted_at IS NULL LEFT JOIN versions v ON v.id=p.selected_version_id AND v.status!='generating' WHERE i.deleted_at IS NULL AND i.seq<?2"),&[json!(c),json!(before)])
    }
    pub(super) fn memories_at(&self, c: &str, eligible: &Eligibility) -> Result<Vec<Value>> {
        let rows = self
            .rows(
                "SELECT * FROM memories WHERE conv_id=? AND status!='rejected'",
                &[json!(c)],
            )?
            .into_iter()
            .filter(|m| {
                m["source_interaction_id"].is_null()
                    || eligible.seq.contains_key(&s(m, "source_interaction_id"))
                        && (m["source_version_id"].is_null()
                            || eligible.selected.contains(&s(m, "source_version_id")))
            })
            .collect::<Vec<_>>();
        let ids: HashSet<_> = rows.iter().map(|m| s(m, "id")).collect();
        let replaced: HashSet<_> = self
            .rows(
                "SELECT old_id,new_id FROM memory_replacements WHERE conv_id=?",
                &[json!(c)],
            )?
            .iter()
            .filter(|m| ids.contains(&s(m, "new_id")))
            .map(|m| s(m, "old_id"))
            .collect();
        Ok(rows
            .into_iter()
            .filter(|m| !replaced.contains(&s(m, "id")))
            .collect())
    }
    pub(super) fn knowledge_at(
        &self,
        c: &str,
        eligible: &Eligibility,
    ) -> Result<HashMap<String, Vec<Value>>> {
        let rows=self.rows("SELECT k.memory_id,k.entity_key,k.entity_name,k.state,k.via,k.belief,k.evidence,k.source_interaction_id,NULL source_version_id,m.created_at FROM memory_knowledge k JOIN memories m ON m.id=k.memory_id WHERE m.conv_id=? UNION ALL SELECT memory_id,entity_key,entity_name,state,via,belief,evidence,source_interaction_id,source_version_id,created_at FROM knowledge_events WHERE conv_id=? ORDER BY created_at",&[json!(c),json!(c)])?;
        let mut rows = rows
            .into_iter()
            .filter(|k| {
                k["source_interaction_id"].is_null()
                    || eligible.seq.contains_key(&s(k, "source_interaction_id"))
                        && (k["source_version_id"].is_null()
                            || eligible.selected.contains(&s(k, "source_version_id")))
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|r| {
            eligible
                .seq
                .get(&s(r, "source_interaction_id"))
                .copied()
                .unwrap_or(i64::MAX)
        });
        let mut out = HashMap::<String, HashMap<String, Value>>::new();
        for r in rows {
            out.entry(s(&r,"memory_id")).or_default().insert(s(&r,"entity_key"),json!({"key":r["entity_key"],"name":r["entity_name"],"state":r["state"],"via":r["via"],"belief":r["belief"]}));
        }
        Ok(out
            .into_iter()
            .map(|(id, rows)| (id, rows.into_values().collect()))
            .collect())
    }
    /// Last state of each character before `before` (the conversation's active path).
    fn latest_characters(&self, c: &str, before: i64) -> Result<Vec<Value>> {
        self.rows(&format!("{PATH_CTE}, ranked AS(SELECT cs.*,row_number() OVER(PARTITION BY cs.entity_key ORDER BY cs.seq DESC,cs.created_at DESC,cs.rowid DESC) rn FROM character_states cs JOIN p ON p.id=cs.interaction_id JOIN interactions i ON i.id=cs.interaction_id WHERE i.deleted_at IS NULL AND cs.seq<?2) SELECT * FROM ranked WHERE rn=1"),&[json!(c),json!(before)])
    }
    pub(super) fn scene_at(&self, c: &str, before: i64) -> Result<Value> {
        let scene=self.one(&format!("{PATH_CTE} SELECT s.* FROM scene_states s JOIN p ON p.id=s.interaction_id JOIN interactions i ON i.id=s.interaction_id WHERE i.deleted_at IS NULL AND s.seq<?2 ORDER BY s.seq DESC,s.created_at DESC,s.rowid DESC LIMIT 1"),&[json!(c),json!(before)])?;
        if scene.is_null() {
            return Ok(Value::Null);
        }
        let chars = self.latest_characters(c, before)?;
        let present = parsed(&scene, "present_json", json!([]));
        let keys: HashSet<_> = present
            .as_array()
            .unwrap()
            .iter()
            .map(|n| rules::key(n.as_str().unwrap_or_default()))
            .collect();
        Ok(
            json!({"interactionId":scene["interaction_id"],"seq":scene["seq"],"location":scene["location"],"time":scene["time_text"],"mood":scene["mood"],"present":present,"objects":parsed(&scene,"objects_json",json!([])),"characters":chars.iter().filter(|c|keys.contains(&s(c,"entity_key"))).map(|c|json!({"key":c["entity_key"],"name":c["entity_name"],"location":c["location"],"position":c["position"],"holding":parsed(c,"holding_json",json!([])),"possessions":c.get("possessions").cloned().unwrap_or_else(|| parsed(c,"possessions_json",json!([])))})).collect::<Vec<_>>()}),
        )
    }
    pub(super) fn sync_lag(&self, c: &str) -> Result<Value> {
        let r=self.one(&format!("{PATH_CTE} SELECT coalesce(sum(n.interaction_id IS NULL),0) lag,coalesce(max(CASE WHEN n.interaction_id IS NOT NULL THEN i.seq ELSE 0 END),0) lastSyncedSeq FROM interactions i JOIN p active ON active.id=i.id LEFT JOIN interaction_notes n ON n.interaction_id=i.id WHERE i.deleted_at IS NULL AND EXISTS(SELECT 1 FROM parts p JOIN versions v ON v.id=p.selected_version_id WHERE p.interaction_id=i.id AND p.deleted_at IS NULL AND v.status IN ('complete','length','interrupted') AND length(v.content)>0)"),&[json!(c)])?;
        Ok(json!({"lag":r["lag"],"lastSyncedSeq":r["lastSyncedSeq"],"synced":n(&r,"lag")==0}))
    }
    pub(super) fn memories(&self, c: &str) -> Result<Value> {
        self.conv(c, false)?;
        let eligible = self.eligibility(c, i64::MAX)?;
        let knowledge = self.knowledge_at(c, &eligible)?;
        let mut memories = self.memories_at(c, &eligible)?;
        memories.sort_by(|a, b| {
            b["importance"]
                .as_f64()
                .unwrap_or(0.0)
                .total_cmp(&a["importance"].as_f64().unwrap_or(0.0))
        });
        let flags=self.rows("SELECT * FROM review_flags WHERE conv_id=? AND status='open' ORDER BY created_at DESC LIMIT 100",&[json!(c)])?;
        let jobs: serde_json::Map<String, Value> = self
            .rows(
                "SELECT status,count(*) n FROM jobs WHERE conv_id=? GROUP BY status",
                &[json!(c)],
            )?
            .iter()
            .map(|j| (s(j, "status"), j["n"].clone()))
            .collect();
        let (relationships, threads) = self.story_at(c, &eligible)?;
        Ok(
            json!({"sync":self.sync_lag(c)?,"scene":self.scene_at(c,i64::MAX)?,"timeline":self.timeline(c,i64::MAX,None)?.view(),"memories":memories.iter().take(200).map(|m|memory_view(m,knowledge.get(&s(m,"id")))).collect::<Vec<_>>(),"relationships":relationships,"threads":threads,"episodes":self.rows("SELECT id,from_seq,to_seq,summary,interaction_ids_json FROM episodes WHERE conv_id=? AND validity='ok' ORDER BY from_seq",&[json!(c)])?,"flags":flags,"jobs":jobs}),
        )
    }
    pub(super) fn story_at(
        &self,
        c: &str,
        eligible: &Eligibility,
    ) -> Result<(Vec<Value>, Vec<Value>)> {
        let mut seen = HashSet::new();
        let relationships = self.rows(
            "SELECT * FROM relationships WHERE conv_id=? AND (status IS NULL OR status='current') AND (validity IS NULL OR validity='ok') ORDER BY created_at DESC, rowid DESC",
            &[json!(c)],
        )?
        .into_iter()
        .filter(|r| {
            r["source_interaction_id"].is_null()
                || eligible.seq.contains_key(&s(r, "source_interaction_id"))
        })
        .filter(|r| {
            // Directed relationships: preserve (a_key -> b_key) uniquely, do NOT sort keys!
            let key = (s(r, "a_key"), s(r, "b_key"));
            seen.insert(key)
        })
        .map(|r| {
            let mut rel = json!({
                "id": r["id"],
                "a": r["a_name"],
                "b": r["b_name"],
                "a_key": r["a_key"],
                "b_key": r["b_key"],
                "description": r["description"]
            });
            if let Some(d) = r.get("directed") {
                rel["directed"] = d.clone();
            }
            if let Some(k) = r.get("kind") {
                rel["kind"] = k.clone();
            }
            if let Some(rt) = r.get("relation_type") {
                rel["relation_type"] = rt.clone();
            }
            rel
        })
        .collect();
        let threads = self
            .rows(
                "SELECT * FROM threads WHERE conv_id=? ORDER BY created_at",
                &[json!(c)],
            )?
            .iter()
            .filter(|t| {
                (t["opened_interaction_id"].is_null()
                    || eligible.seq.contains_key(&s(t, "opened_interaction_id")))
                    && !eligible.seq.contains_key(&s(t, "resolved_interaction_id"))
            })
            .take(12)
            .map(|t| json!({"id":t["id"],"text":t["text"]}))
            .collect();
        Ok((relationships, threads))
    }
    pub(super) fn add_memory(&self, c: &str, p: &Value) -> Result<Value> {
        self.conv(c, false)?;
        let content = rules::clip(p["content"].as_str().unwrap_or_default().trim(), 500);
        if content.is_empty() {
            return Err(Error::new(400, "Memória vazia."));
        }
        let kind = p["kind"]
            .as_str()
            .filter(|s| KINDS.contains(s))
            .unwrap_or("EVENT");
        let mid = id();
        let visibility = p["visibility"]
            .as_str()
            .filter(|v| ["public", "private"].contains(v))
            .unwrap_or(if PRIVATE.contains(&kind) {
                "private"
            } else {
                "public"
            });
        self.exec("INSERT INTO memories(id,conv_id,kind,subject,content,importance,confidence,visibility,pinned,created_at,last_confirmed_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)",&[json!(mid),json!(c),json!(kind),p["subject"].clone(),json!(content),json!(number(p,"importance",0.5)),json!(number(p,"confidence",1.0)),json!(visibility),json!(p["pinned"].as_bool().unwrap_or(false)),json!(now()),json!(now())])?;
        self.manual_knowledge(c, &mid, &p["knowledge"])?;
        self.index(c, "memory", &mid, &format!("{} {content}", s(p, "subject")))?;
        self.bump(c)?;
        Ok(json!({"id":mid}))
    }
    pub(super) fn patch_memory(&self, c: &str, mid: &str, p: &Value) -> Result<Value> {
        let old = self.one(
            "SELECT * FROM memories WHERE conv_id=? AND id=?",
            &[json!(c), json!(mid)],
        )?;
        if old.is_null() {
            return Err(Error::new(404, "Memória não encontrada."));
        }
        for key in [
            "content",
            "subject",
            "visibility",
            "importance",
            "confidence",
            "pinned",
        ] {
            if let Some(mut value) = p.get(key).cloned() {
                if key == "content" {
                    let text = value.as_str().unwrap_or_default().trim();
                    if text.is_empty() {
                        return Err(Error::new(400, "Memória vazia."));
                    }
                    value = json!(rules::clip(text, 500));
                }
                if key == "visibility"
                    && !value
                        .as_str()
                        .is_some_and(|s| ["public", "private"].contains(&s))
                {
                    return Err(Error::new(400, "Visibilidade inválida."));
                }
                if key == "importance" || key == "confidence" {
                    value = json!(number(p, key, 0.5));
                }
                self.exec(
                    &format!("UPDATE memories SET {key}=? WHERE id=?"),
                    &[value, json!(mid)],
                )?;
            }
        }
        if p["knowledge"].is_array() {
            self.exec(
                "DELETE FROM memory_knowledge WHERE memory_id=?",
                &[json!(mid)],
            )?;
            self.exec(
                "DELETE FROM knowledge_events WHERE memory_id=?",
                &[json!(mid)],
            )?;
            self.manual_knowledge(c, mid, &p["knowledge"])?;
        }
        let r = self.one("SELECT * FROM memories WHERE id=?", &[json!(mid)])?;
        self.index(
            c,
            "memory",
            mid,
            &format!("{} {}", s(&r, "subject"), s(&r, "content")),
        )?;
        self.exec(
            "DELETE FROM embeddings WHERE owner_type='memory' AND owner_id=?",
            &[json!(mid)],
        )?;
        Ok(json!({"revision":self.bump(c)?}))
    }
    fn manual_knowledge(&self, c: &str, mid: &str, list: &Value) -> Result<()> {
        for k in list.as_array().into_iter().flatten() {
            let name = k["who"].as_str().or(k["name"].as_str()).unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let row = self.resolve_cast(c, name)?;
            let key = if row.is_null() {
                rules::key(name)
            } else {
                s(&row, "name_key")
            };
            let state = k["state"]
                .as_str()
                .filter(|s| ["knows", "believes", "unaware"].contains(s))
                .unwrap_or("knows");
            self.exec("INSERT OR REPLACE INTO memory_knowledge(memory_id,entity_key,entity_name,state,via,belief,evidence,source_interaction_id) VALUES(?,?,?,?,?,?,?,NULL)",&[json!(mid),json!(key),json!(name),json!(state),k["via"].clone(),k["belief"].clone(),k["evidence"].clone()])?;
        }
        Ok(())
    }
    pub(super) fn delete_memory_internal(&self, mid: &str) -> Result<()> {
        self.unindex("memory", mid)?;
        for (table, col) in [
            ("memory_knowledge", "memory_id"),
            ("knowledge_events", "memory_id"),
            ("embeddings", "owner_id"),
        ] {
            self.exec(&format!("DELETE FROM {table} WHERE {col}=?"), &[json!(mid)])?;
        }
        self.exec(
            "DELETE FROM memory_replacements WHERE old_id=? OR new_id=?",
            &[json!(mid), json!(mid)],
        )?;
        self.exec("DELETE FROM memories WHERE id=?", &[json!(mid)])?;
        Ok(())
    }
    pub(super) fn delete_memory(&self, c: &str, mid: &str) -> Result<Value> {
        if self
            .one(
                "SELECT id FROM memories WHERE conv_id=? AND id=?",
                &[json!(c), json!(mid)],
            )?
            .is_null()
        {
            return Err(Error::new(404, "Memória não encontrada."));
        }
        self.delete_memory_internal(mid)?;
        self.invalidate(c, None)?;
        Ok(json!({"revision":self.bump(c)?}))
    }
    pub(super) fn save_embedding(
        &self,
        c: &str,
        owner_type: &str,
        owner: &str,
        model: &str,
        text: Option<&str>,
        vector: &[f32],
    ) -> Result<Value> {
        let sql = match owner_type {
            "memory" => {
                "SELECT coalesce(subject,'')||': '||content text FROM memories WHERE id=? AND conv_id=?"
            }
            "episode" => "SELECT summary text FROM episodes WHERE id=? AND conv_id=?",
            "chunk" => "SELECT text FROM text_chunks WHERE id=? AND conv_id=?",
            _ => return Err(Error::new(400, "Tipo de vetor inválido.")),
        };
        let current = self.one(sql, &[json!(owner), json!(c)])?;
        if current.is_null() || text.is_some_and(|text| s(&current, "text") != text) {
            return Ok(json!(false));
        }
        let blob = rules::vector_blob(vector)?;
        self.db.execute("INSERT OR REPLACE INTO embeddings(owner_type,owner_id,conv_id,model,dim,vec) VALUES(?,?,?,?,?,?)",rusqlite::params![owner_type,owner,c,model,vector.len(),blob])?;
        Ok(json!(true))
    }
    pub(super) fn pending_embeddings(&self, limit: usize, model: &str) -> Result<Value> {
        // Stable round-robin order across chats; completed vectors leave the eligible set.
        Ok(json!(self.rows("WITH candidates AS(SELECT 'memory' t,m.id,m.conv_id,coalesce(subject,'')||': '||content text FROM memories m JOIN conversations c ON c.id=m.conv_id WHERE c.deleted_at IS NULL AND m.status='current' AND m.validity='ok' AND m.review_state='ok' UNION ALL SELECT 'episode',e.id,e.conv_id,e.summary FROM episodes e JOIN conversations c ON c.id=e.conv_id WHERE c.deleted_at IS NULL AND e.validity='ok' UNION ALL SELECT 'chunk',t.id,t.conv_id,t.text FROM text_chunks t JOIN conversations c ON c.id=t.conv_id WHERE c.deleted_at IS NULL), eligible AS(SELECT *,row_number() OVER(PARTITION BY conv_id ORDER BY t,id) rn FROM candidates WHERE NOT EXISTS(SELECT 1 FROM embeddings e WHERE e.owner_type=candidates.t AND e.owner_id=candidates.id AND e.model=?)) SELECT t,id,conv_id,text FROM eligible ORDER BY rn,conv_id LIMIT ?",&[json!(model),json!(limit.clamp(1,64))])?))
    }
    pub(super) fn apply_extraction(&self, p: &Value) -> Result<Value> {
        let c = s(p, "convId");
        let iid = s(p, "interactionId");
        // Interactions/parts/versions do not change inside this request: one path query serves all three cuts.
        let path = self.active_path(&c, i64::MAX)?;
        let eligible = Eligibility::from_path(&path, i64::MAX);
        if !eligible.seq.contains_key(&iid) {
            return Ok(json!({"skipped":true,"reason":"interaction-missing"}));
        }
        if let Some(j) = p["jobId"].as_str()
            && self.one("SELECT status FROM jobs WHERE id=?", &[json!(j)])?["status"] != "running"
        {
            return Ok(json!({"skipped":true,"reason":"job-invalidated"}));
        }
        let selected=self.rows("SELECT selected_version_id id FROM parts WHERE interaction_id=? AND deleted_at IS NULL ORDER BY idx",&[json!(iid)])?.iter().map(|v|v["id"].clone()).collect::<Vec<_>>();
        if p["versionIds"]
            .as_array()
            .is_some_and(|ids| !ids.is_empty() && ids != &selected)
            || p["versionId"]
                .as_str()
                .is_some_and(|v| !selected.contains(&json!(v)))
        {
            return Ok(json!({"skipped":true,"reason":"versions-changed"}));
        }
        let conv = self.conv(&c, false)?;
        let seq = eligible.seq[&iid];
        let source = s(p, "sourceText");
        let ext = &p["extraction"];
        self.delete_derived(&iid)?;
        for cast in ext["cast"].as_array().into_iter().flatten() {
            let mut cast = cast.clone();
            cast["source"] = json!("narrative");
            cast["firstInteractionId"] = json!(iid);
            self.save_cast(&c, &cast)?;
        }
        let mut present = HashSet::from([rules::key(&s(&conv, "persona"))]);
        let scenes = ext["scenes"].as_array().cloned().unwrap_or_else(|| {
            if ext["scene"].is_object() {
                vec![ext["scene"].clone()]
            } else {
                Vec::new()
            }
        });
        // What the model omitted (scene place/time/mood; character place/position) inherits the last non-empty
        // value from the previous scene; null fields only, so an outdated place is not kept when there is an explicit change.
        let before_scene = self.scene_at(&c, seq)?;
        let before_loc = before_scene["location"].as_str().unwrap_or_default().trim();
        let mut carried = json!({"location":before_scene["location"],"time":before_scene["time"],"mood":before_scene["mood"]});
        let mut last_chars = self
            .latest_characters(&c, seq)?
            .into_iter()
            .map(|ch| {
                (
                    s(&ch, "entity_key"),
                    (ch["location"].clone(), ch["position"].clone()),
                )
            })
            .collect::<HashMap<_, _>>();
        for scene in &scenes {
            let explicit_location = !blank(&scene["location"]);
            let new_loc = scene["location"].as_str().unwrap_or_default().trim();
            let transition_detected = !before_loc.is_empty()
                && explicit_location
                && rules::key(before_loc) != rules::key(new_loc);

            let scene_present_arr = scene["present"].as_array();
            let is_explicit = scene_present_arr.is_some_and(|a| !a.is_empty());
            present = scene_present_arr
                .into_iter()
                .flatten()
                .map(|n| rules::key(n.as_str().unwrap_or_default()))
                .collect();
            if !is_explicit || present.contains(&rules::key(&s(&conv, "persona"))) {
                present.insert(rules::key(&s(&conv, "persona")));
            }
            let mut filled = scene.clone();
            if filled.is_object() {
                for field in ["location", "time", "mood"] {
                    if blank(&filled[field]) {
                        filled[field] = carried[field].clone();
                    } else {
                        carried[field] = filled[field].clone();
                    }
                }
            }
            self.exec("INSERT INTO scene_states(id,conv_id,interaction_id,seq,location,time_text,mood,present_json,objects_json,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)",&[json!(id()),json!(c),json!(iid),json!(seq),filled["location"].clone(),filled["time"].clone(),filled["mood"].clone(),json!(scene["present"].as_array().cloned().unwrap_or_default().into_iter().take(12).collect::<Vec<_>>()).to_string().into(),json!(scene["objects"].as_array().cloned().unwrap_or_default().into_iter().take(12).collect::<Vec<_>>()).to_string().into(),json!(now())])?;
            let mut chars = scene["characters"].as_array().cloned().unwrap_or_default();
            if chars.is_empty() {
                let mut names = HashSet::new();
                for key in ["positions", "locations", "holding"] {
                    if let Some(map) = scene[key].as_object() {
                        names.extend(map.keys().cloned());
                    }
                }
                chars=names.into_iter().map(|name|json!({"name":name,"location":scene["locations"][&name].as_str().or(scene["location"].as_str()),"position":scene["positions"][&name],"holding":scene["holding"][&name].as_array().cloned().unwrap_or_default()})).collect();
            }
            for mut ch in chars {
                let name = s(&ch, "name");
                if name.is_empty() {
                    continue;
                }
                let key = rules::key(&name);
                if let Some((last_location, last_position)) = last_chars.get(&key) {
                    if blank(&ch["location"]) && !explicit_location && !blank(last_location) {
                        ch["location"] = last_location.clone();
                    }
                    if !transition_detected && blank(&ch["position"]) && ch["location"] == *last_location {
                        ch["position"] = last_position.clone();
                    }
                }
                if blank(&ch["location"]) && !explicit_location && !blank(&filled["location"]) {
                    ch["location"] = filled["location"].clone();
                }
                last_chars.insert(key, (ch["location"].clone(), ch["position"].clone()));
                self.save_cast(
                    &c,
                    &json!({"name":name,"source":"narrative","firstInteractionId":iid}),
                )?;
                self.exec("INSERT INTO character_states(id,conv_id,interaction_id,seq,entity_key,entity_name,location,position,holding_json,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)",&[json!(id()),json!(c),json!(iid),json!(seq),json!(rules::key(&name)),json!(name),ch["location"].clone(),ch["position"].clone(),json!(ch["holding"].as_array().cloned().unwrap_or_default()).to_string().into(),json!(now())])?;
            }
        }
        let past = Eligibility::from_path(&path, seq + 1);
        let mut existing = self.memories_at(&c, &past)?;
        let mut created = Vec::new();
        for f in ext["facts"].as_array().into_iter().flatten() {
            let content = rules::clip(f["content"].as_str().unwrap_or_default().trim(), 500);
            if rules::utf16_len(content) < 6 {
                continue;
            }
            let kind = f["kind"]
                .as_str()
                .filter(|s| KINDS.contains(s))
                .unwrap_or("EVENT");
            let subject = f["subject"].as_str().filter(|s| !s.trim().is_empty());
            let importance = number(f, "importance", 0.5);
            let confidence = number(f, "confidence", 0.8);
            if importance < 0.2 && confidence < 0.6 {
                continue;
            }
            let explicit = f["existingId"]
                .as_str()
                .filter(|_| s(f, "replaces").is_empty())
                .and_then(|id| existing.iter().find(|m| m["id"] == id && m["kind"] == kind));
            let duplicate = explicit
                .or_else(|| {
                    existing.iter().find(|m| {
                        m["kind"] == kind
                            && rules::key(&s(m, "subject"))
                                == rules::key(subject.unwrap_or_default())
                            && rules::fact_key(&s(m, "content")) == rules::fact_key(content)
                    })
                })
                .cloned();
            let mid = duplicate.as_ref().map(|m| s(m, "id")).unwrap_or_else(id);
            let visibility = f["visibility"]
                .as_str()
                .filter(|s| ["public", "private"].contains(s))
                .unwrap_or(if PRIVATE.contains(&kind) {
                    "private"
                } else {
                    "public"
                });
            if duplicate.is_none() {
                self.exec("INSERT INTO memories(id,conv_id,kind,subject,content,source_interaction_id,source_version_id,importance,confidence,visibility,review_state,conv_revision,created_at,last_confirmed_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",&[json!(mid),json!(c),json!(kind),json!(subject),json!(content),json!(iid),p["versionId"].clone(),json!(importance),json!(confidence),json!(visibility),json!(if p["reviewPending"]==true{"pending_review"}else{"ok"}),conv["revision"].clone(),json!(now()),json!(now())])?;
                self.index(
                    &c,
                    "memory",
                    &mid,
                    &format!("{} {content}", subject.unwrap_or_default()),
                )?;
            } else {
                self.exec("UPDATE memories SET importance=max(importance,?),last_confirmed_at=? WHERE id=?",&[json!(importance),json!(now()),json!(mid)])?;
            }
            if let Some(replaces) = f["replaces"].as_str().filter(|s| !s.is_empty())
                && let Some(old) = existing.iter().find(|m| {
                    m["id"] != mid
                        && m["kind"] == kind
                        && rules::key(&s(m, "subject")) == rules::key(subject.unwrap_or_default())
                        && rules::fact_key(&s(m, "content")) == rules::fact_key(replaces)
                })
            {
                self.exec("INSERT OR IGNORE INTO memory_replacements(conv_id,old_id,new_id) VALUES(?,?,?)",&[json!(c),old["id"].clone(),json!(mid)])?;
                self.exec(
                    "UPDATE memories SET status='historical',superseded_by=? WHERE id=?",
                    &[json!(mid), old["id"].clone()],
                )?;
            }
            let knows = f["knowledge"].as_array().cloned().unwrap_or_default();
            let persona_key = rules::key(&s(&conv, "persona"));
            let is_persona_present = present.contains(&persona_key);
            let witnesses = f["witnesses"]
                .as_array()
                .map(|a| {
                    let mut set = a.iter()
                        .map(|n| rules::key(n.as_str().unwrap_or_default()))
                        .collect::<HashSet<_>>();
                    if is_persona_present {
                        set.insert(persona_key.clone());
                    } else {
                        set.remove(&persona_key);
                    }
                    set
                })
                .unwrap_or_else(|| {
                    if is_persona_present {
                        let mut s = present.clone();
                        s.insert(persona_key.clone());
                        s
                    } else {
                        present.iter().filter(|k| **k != persona_key).cloned().collect()
                    }
                });
            let mut seen = HashSet::new();
            for k in &knows {
                let name = s(k, "who");
                let key = rules::key(&name);
                if key.is_empty() || !seen.insert(key.clone()) {
                    continue;
                }
                let state = k["state"].as_str().unwrap_or("knows");
                let via = k["via"].as_str().unwrap_or("witnessed");
                let evidence = s(k, "evidence");
                if state != "unaware" {
                    let exact = evidence.len() >= 8 && source.contains(&evidence);
                    if via == "witnessed" {
                        if !witnesses.contains(&key)
                            && !(exact && rules::key(&evidence).contains(&key))
                        {
                            continue;
                        }
                    } else if !exact {
                        continue;
                    }
                }
                self.exec("INSERT INTO knowledge_events(id,conv_id,memory_id,entity_key,entity_name,state,via,belief,evidence,source_interaction_id,source_version_id,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",&[json!(id()),json!(c),json!(mid),json!(key),json!(name),json!(if ["knows","believes","unaware"].contains(&state){state}else{"knows"}),json!(via),k["belief"].clone(),json!(evidence),json!(iid),p["versionId"].clone(),json!(now())])?;
            }
            if visibility == "private" && kind != "SECRET" && knows.is_empty() {
                for key in &witnesses {
                    if !present.contains(key) {
                        continue;
                    }
                    let row = self.resolve_cast(&c, key)?;
                    let name = row["name"].as_str().unwrap_or(key);
                    self.exec("INSERT INTO knowledge_events(id,conv_id,memory_id,entity_key,entity_name,state,via,source_interaction_id,source_version_id,created_at) VALUES(?,?,?,?,?,'knows','witnessed',?,?,?)",&[json!(id()),json!(c),json!(mid),json!(key),json!(name),json!(iid),p["versionId"].clone(),json!(now())])?;
                }
            }
            created.push(json!(mid));
            if duplicate.is_none() {
                existing.push(json!({"id":mid,"kind":kind,"subject":subject,"content":content}));
            }
        }
        let before = Eligibility::from_path(&path, seq);
        let mut rel_list = ext["relationships"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        let mut derived_inverses = Vec::new();
        for r in &rel_list {
            let (a, b) = (s(r, "a"), s(r, "b"));
            let desc = s(r, "description");
            if a.is_empty() || b.is_empty() || a == b || desc.is_empty() {
                continue;
            }
            let a_key = rules::key(&a);
            let b_key = rules::key(&b);
            let has_reverse = rel_list.iter().any(|other| {
                rules::key(&s(other, "a")) == b_key && rules::key(&s(other, "b")) == a_key
            });
            if !has_reverse {
                if let Some(inv_desc) = derive_inverse_kinship(&a, &b, &desc) {
                    derived_inverses.push(json!({
                        "a": b,
                        "b": a,
                        "description": inv_desc,
                        "directed": true,
                        "kind": "kinship"
                    }));
                }
            }
        }
        rel_list.extend(derived_inverses);

        for r in rel_list.into_iter().take(12) {
            let (a, b) = (s(&r, "a"), s(&r, "b"));
            let description = s(&r, "description");
            if a.is_empty() || b.is_empty() || a == b || description.is_empty() {
                continue;
            }
            let a_key = rules::key(&a);
            let b_key = rules::key(&b);
            let previous = self.rows(
                "SELECT * FROM relationships WHERE conv_id=? AND a_key=? AND b_key=? AND (status IS NULL OR status='current') ORDER BY created_at DESC",
                &[json!(c), json!(a_key), json!(b_key)],
            )?
            .into_iter()
            .find(|r| {
                r["source_interaction_id"].is_null()
                    || before.seq.contains_key(&s(r, "source_interaction_id"))
            });

            if previous
                .as_ref()
                .is_some_and(|r| rules::jaccard(&s(r, "description"), &description) >= 0.7)
            {
                continue;
            }
            let rid = id();
            self.exec(
                "INSERT INTO relationships(id,conv_id,a_key,a_name,b_key,b_name,description,source_interaction_id,created_at) VALUES(?,?,?,?,?,?,?,?,?)",
                &[
                    json!(rid),
                    json!(c),
                    json!(a_key),
                    json!(a),
                    json!(b_key),
                    json!(b),
                    json!(rules::clip(&description, 300)),
                    json!(iid),
                    json!(now()),
                ],
            )?;
            if let Some(old) = previous {
                self.exec(
                    "UPDATE relationships SET status='historical',superseded_by=? WHERE id=?",
                    &[json!(rid), old["id"].clone()],
                )?;
            }
        }
        let (_, mut threads) = self.story_at(&c, &before)?;
        for t in ext["openThreads"]
            .as_array()
            .or(ext["open_threads"].as_array())
            .into_iter()
            .flatten()
            .take(5)
        {
            let text = t.as_str().unwrap_or_default();
            if text.len() < 6
                || threads
                    .iter()
                    .any(|o| rules::jaccard(&s(o, "text"), text) >= 0.6)
            {
                continue;
            }
            let tid = id();
            self.exec("INSERT INTO threads(id,conv_id,text,opened_interaction_id,created_at) VALUES(?,?,?,?,?)",&[json!(tid),json!(c),json!(rules::clip(text,240)),json!(iid),json!(now())])?;
            threads.push(json!({"id":tid,"text":text}));
        }
        for t in ext["resolvedThreads"]
            .as_array()
            .or(ext["resolved_threads"].as_array())
            .into_iter()
            .flatten()
            .take(5)
        {
            if let Some(hit) = threads
                .iter()
                .find(|o| rules::jaccard(&s(o, "text"), t.as_str().unwrap_or_default()) >= 0.4)
            {
                self.exec(
                    "UPDATE threads SET resolved_interaction_id=?,status='resolved' WHERE id=?",
                    &[json!(iid), hit["id"].clone()],
                )?;
            }
        }
        if let Some(summary) = ext["summary"].as_str().filter(|s| !s.trim().is_empty()) {
            self.exec("INSERT OR REPLACE INTO interaction_notes(interaction_id,conv_id,seq,summary,version_ids_json,created_at) VALUES(?,?,?,?,?,?)",&[json!(iid),json!(c),json!(seq),json!(summary),json!(json!(selected).to_string()),json!(now())])?;
        }
        self.invalidate(&c, None)?;
        self.timeline(&c, seq + 1, None)?;
        Ok(json!({"skipped":false,"createdMemoryIds":created}))
    }
}
pub(super) const KINDS: &[&str] = &[
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
    "OPEN_THREAD",
];
const PRIVATE: &[&str] = &[
    "EVENT",
    "SECRET",
    "PROMISE",
    "OBJECT",
    "OPEN_THREAD",
    "USER_FACT",
    "RELATIONSHIP",
    "KINSHIP",
    "POSSESSION",
    "INVENTORY",
];

pub(super) fn derive_inverse_kinship(a_name: &str, _b_name: &str, desc: &str) -> Option<String> {
    let lower = desc.to_lowercase();
    if lower.contains("mãe de") || lower.contains("pai de") || lower.contains("progenitor") {
        Some(format!("filho(a) de {a_name}"))
    } else if lower.contains("filho de") || lower.contains("filha de") {
        Some(format!("pai/mãe de {a_name}"))
    } else if lower.contains("irmão de") || lower.contains("irmã de") {
        Some(format!("irmão/irmã de {a_name}"))
    } else if lower.contains("esposa de") || lower.contains("marido de") || lower.contains("esposo de") {
        Some(format!("cônjuge de {a_name}"))
    } else if lower.contains("avó de") || lower.contains("avô de") {
        Some(format!("neto(a) de {a_name}"))
    } else if lower.contains("neto de") || lower.contains("neta de") {
        Some(format!("avô/avó de {a_name}"))
    } else if lower.contains("tio de") || lower.contains("tia de") {
        Some(format!("sobrinho(a) de {a_name}"))
    } else if lower.contains("sobrinho de") || lower.contains("sobrinha de") {
        Some(format!("tio/tia de {a_name}"))
    } else {
        None
    }
}
/// Field omitted by the model: null or empty text.
fn blank(v: &Value) -> bool {
    v.is_null() || v.as_str().is_some_and(|t| t.trim().is_empty())
}
fn number(p: &Value, key: &str, default: f64) -> f64 {
    p[key]
        .as_f64()
        .filter(|v| v.is_finite())
        .unwrap_or(default)
        .clamp(0.0, 1.0)
}
pub(super) fn memory_view(m: &Value, knowledge: Option<&Vec<Value>>) -> Value {
    json!({"id":m["id"],"kind":m["kind"],"subject":m["subject"],"content":m["content"],"importance":m["importance"],"confidence":m["confidence"],"visibility":m["visibility"],"status":m["status"],"reviewState":m["review_state"],"sourceInteractionId":m["source_interaction_id"],"sourceVersionId":m["source_version_id"],"pinned":n(m,"pinned")!=0,"knowledge":knowledge.cloned().unwrap_or_default(),"createdAt":m["created_at"]})
}
