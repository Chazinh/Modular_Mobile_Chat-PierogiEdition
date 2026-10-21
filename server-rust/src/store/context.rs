use super::{
    Database, PATH_CTE,
    memory::{Eligibility, memory_view},
};
use crate::{Result, n, now, parsed, rules, s};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

impl Database {
    pub(super) fn context_data(&self, p: &Value) -> Result<Value> {
        let c = s(p, "convId");
        let conv = self.conversation(&c, false)?;
        let path=self.rows(&format!("{PATH_CTE} SELECT i.id,i.seq,i.user_text IS NOT NULL has_user FROM p JOIN interactions i ON i.id=p.id WHERE i.deleted_at IS NULL ORDER BY i.seq"),&[json!(c)])?;
        let pending = p["pendingUserText"].is_string() && !p["interactionId"].is_string();
        let current_index = if pending {
            path.len()
        } else {
            p["interactionId"]
                .as_str()
                .and_then(|id| path.iter().position(|i| i["id"] == id))
                .unwrap_or(path.len().saturating_sub(1))
        };
        let current_meta = path.get(current_index);
        let current_id = current_meta.map(|i| s(i, "id"));
        let cutoff = if pending {
            path.last().map(|i| n(i, "seq") + 1).unwrap_or(1)
        } else {
            current_meta.map(|i| n(i, "seq")).unwrap_or(i64::MAX)
        };
        let before = &path[..current_index];
        let notes=self.rows(&format!("{PATH_CTE} SELECT n.* FROM interaction_notes n JOIN p ON p.id=n.interaction_id JOIN interactions i ON i.id=n.interaction_id WHERE i.deleted_at IS NULL AND n.seq<?2 ORDER BY n.seq"),&[json!(c),json!(cutoff)])?;
        let metadata=self.rows(&format!("{PATH_CTE} SELECT i.id,i.seq,p.idx,v.id version_id,length(v.content)>0 has_content FROM p active JOIN interactions i ON i.id=active.id LEFT JOIN parts p ON p.interaction_id=i.id AND p.deleted_at IS NULL LEFT JOIN versions v ON v.id=p.selected_version_id AND v.status!='generating' WHERE i.deleted_at IS NULL ORDER BY i.seq,p.idx"),&[json!(c)])?;
        let mut version_ids = HashMap::<String, Vec<Value>>::new();
        let mut complete = HashSet::new();
        for row in &metadata {
            if row["version_id"].is_string() {
                version_ids
                    .entry(s(row, "id"))
                    .or_default()
                    .push(row["version_id"].clone());
                if n(row, "has_content") != 0 {
                    complete.insert(s(row, "id"));
                }
            }
        }
        let notes = notes
            .into_iter()
            .filter(|note| {
                parsed(note, "version_ids_json", json!([]))
                    .as_array()
                    .is_some_and(|ids| {
                        version_ids
                            .get(&s(note, "interaction_id"))
                            .is_some_and(|selected| ids == selected)
                    })
            })
            .collect::<Vec<_>>();
        let noted = notes
            .iter()
            .map(|note| s(note, "interaction_id"))
            .collect::<HashSet<_>>();
        let pending_ids = before
            .iter()
            .filter(|i| complete.contains(&s(i, "id")) && !noted.contains(&s(i, "id")))
            .map(|i| s(i, "id"))
            .collect::<HashSet<_>>();
        let pairs = before
            .iter()
            .filter(|i| n(i, "has_user") != 0 || complete.contains(&s(i, "id")))
            .collect::<Vec<_>>();
        let total = pairs.len();
        let limit = p["maxPairs"].as_u64().unwrap_or(50).clamp(1, 50) as usize;
        let recent_ids = pairs
            .iter()
            .skip(total.saturating_sub(limit))
            .map(|i| s(i, "id"))
            .collect::<HashSet<_>>();
        let mut needed = pending_ids
            .union(&recent_ids)
            .cloned()
            .collect::<HashSet<_>>();
        if let Some(id) = &current_id {
            needed.insert(id.clone());
        }
        // Only recent, mandatory pending, and the current turn load full source text.
        let rows=self.rows("SELECT i.id,i.seq,i.user_text,p.idx,v.id version_id,v.content FROM interactions i LEFT JOIN parts p ON p.interaction_id=i.id AND p.deleted_at IS NULL LEFT JOIN versions v ON v.id=p.selected_version_id AND v.status!='generating' WHERE i.id IN (SELECT value FROM json_each(?)) ORDER BY i.seq,p.idx",&[json!(json!(needed).to_string())])?;
        let scene_rows = self.rows(
            "SELECT interaction_id, present_json FROM scene_states WHERE conv_id=? AND interaction_id IN (SELECT value FROM json_each(?)) ORDER BY seq, created_at, rowid",
            &[json!(c), json!(json!(needed).to_string())],
        )?;
        let mut presence_by_id = HashMap::<String, Vec<String>>::new();
        for row in scene_rows {
            let iid = s(&row, "interaction_id");
            let list = parsed(&row, "present_json", json!([]));
            if let Some(arr) = list.as_array() {
                let entry = presence_by_id.entry(iid).or_default();
                for item in arr {
                    if let Some(name) = item.as_str() {
                        let trimmed = name.trim();
                        if !trimmed.is_empty() && !entry.iter().any(|e| rules::key(e) == rules::key(trimmed)) {
                            entry.push(trimmed.to_string());
                        }
                    }
                }
            }
        }
        let mut by_id = HashMap::<String, Value>::new();
        for row in rows {
            let pair=by_id.entry(s(&row,"id")).or_insert_with(||json!({"id":row["id"],"seq":row["seq"],"user":row["user_text"],"assistant":"","parts":[],"complete":false,"sources":if row["user_text"].is_string(){vec![json!({"id":row["id"],"role":"user","text":row["user_text"]})]}else{vec![]}}));
            if row["content"].as_str().is_some_and(|s| !s.is_empty()) {
                pair["parts"]
                    .as_array_mut()
                    .unwrap()
                    .push(row["content"].clone());
                pair["complete"] = json!(true);
                pair["sources"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"id":row["version_id"],"role":"assistant","text":row["content"]}));
            }
        }
        for pair in by_id.values_mut() {
            let pid = s(pair, "id");
            pair["assistant"] = json!(
                pair["parts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n\n")
            );
            pair["versions"] = json!(version_ids.get(&pid).cloned().unwrap_or_default());
            pair["present_entities"] = json!(presence_by_id.remove(&pid).unwrap_or_default());
        }
        let mut current = if pending {
            json!({"interactionId":null,"seq":cutoff,"userText":p["pendingUserText"],"priorParts":[]})
        } else if let Some(pair) = current_id.as_ref().and_then(|id| by_id.get(id)) {
            let mut parts = pair["parts"].as_array().cloned().unwrap_or_default();
            if p["dropLastPart"] == true {
                parts.pop();
            }
            json!({"interactionId":pair["id"],"seq":pair["seq"],"userText":pair["user"],"priorParts":parts})
        } else {
            Value::Null
        };
        if p["overrideUserText"].is_string() && !current.is_null() {
            current["userText"] = p["overrideUserText"].clone();
        }
        // Each speech goes out once (`user` + `assistant`). `sources` (text per source, with IDs) only goes on the last 3
        // pairs, its only use in provenance::sources and in the extraction job; `parts` only existed for `current`.
        let recent_pairs = pairs
            .iter()
            .filter(|i| recent_ids.contains(&s(i, "id")))
            .filter_map(|i| by_id.get(&s(i, "id")))
            .collect::<Vec<_>>();
        let first_with_sources = recent_pairs.len().saturating_sub(3);
        let recent = recent_pairs
            .iter()
            .enumerate()
            .map(|(at, pair)| slim_pair(pair, at >= first_with_sources))
            .collect::<Vec<_>>();
        let unprocessed = before
            .iter()
            .filter(|i| pending_ids.contains(&s(i, "id")))
            .filter_map(|i| by_id.get(&s(i, "id")))
            .map(|pair| slim_pair(pair, false))
            .collect::<Vec<_>>();
        let seq = before
            .iter()
            .map(|i| (s(i, "id"), n(i, "seq")))
            .collect::<HashMap<_, _>>();
        let selected = metadata
            .iter()
            .filter(|i| n(i, "seq") < cutoff)
            .filter_map(|i| i["version_id"].as_str().map(str::to_owned))
            .collect();
        let eligible = Eligibility {
            max_seq: seq.values().copied().max().unwrap_or(0),
            seq,
            selected,
        };
        let knowledge = self.knowledge_at(&c, &eligible)?;
        let mut pinned = self
            .memories_at(&c, &eligible)?
            .into_iter()
            .filter(|m| n(m, "pinned") != 0)
            .collect::<Vec<_>>();
        pinned.sort_by(|a, b| {
            b["importance"]
                .as_f64()
                .unwrap_or(0.0)
                .total_cmp(&a["importance"].as_f64().unwrap_or(0.0))
        });
        let (relationships, threads) = self.story_at(&c, &eligible)?;
        let episodes=self.rows("SELECT * FROM episodes WHERE conv_id=? AND to_seq<? ORDER BY from_seq",&[json!(c),json!(cutoff)])?.iter().filter(|e|parsed(e,"interaction_ids_json",json!([])).as_array().unwrap().iter().all(|id|eligible.seq.contains_key(id.as_str().unwrap_or_default()))).map(|e|json!({"id":e["id"],"fromSeq":e["from_seq"],"toSeq":e["to_seq"],"summary":e["summary"],"interactionIds":parsed(e,"interaction_ids_json",json!([]))})).collect::<Vec<_>>();
        let timeline = self.timeline(&c, cutoff, Some(&metadata))?;
        let global_settings = self.global_settings()?;
        let last_synced = notes.last().map(|note| n(note, "seq")).unwrap_or(0);
        let (minimum, _, preferred) = crate::context::pair_limits(&conv);
        let notes = notes_for_prompt(notes, &recent, &episodes, minimum, preferred);
        Ok(
            json!({"conv":conv,"cast":self.cast(&c)?,"pairs":recent,"totalPairs":total,"current":current,"scene":self.scene_at(&c,cutoff)?,"beforeSeq":cutoff,"unprocessed":unprocessed,
            "sync":{"lag":unprocessed.len(),"lastSyncedSeq":last_synced,"synced":unprocessed.is_empty()},
            "relationships":relationships,"threads":threads,"pathLength":path.len(),"episodes":episodes,
            "notes":notes.iter().map(|note|json!({"interactionId":note["interaction_id"],"seq":note["seq"],"summary":note["summary"]})).collect::<Vec<_>>(),
            "globalSettings":global_settings,"pinned":pinned.iter().take(12).map(|m|memory_view(m,knowledge.get(&s(m,"id")))).collect::<Vec<_>>(),"timeline":timeline.summary()}),
        )
    }
    pub(super) fn timeline(
        &self,
        c: &str,
        before: i64,
        supplied: Option<&[Value]>,
    ) -> Result<std::sync::Arc<Timeline>> {
        let prompt = s(
            &self.one(
                "SELECT text FROM character_revisions WHERE conv_id=? ORDER BY rev DESC LIMIT 1",
                &[json!(c)],
            )?,
            "text",
        );
        // Same rows (and same order) as the context metadata query, when the caller already has them:
        // avoids walking the active path again.
        let queried;
        let metadata: Vec<&Value> = match supplied {
            Some(rows) => rows.iter().filter(|r| n(r, "seq") < before).collect(),
            None => {
                queried=self.rows(&format!("{PATH_CTE} SELECT i.id,i.seq,p.idx,v.id version_id FROM p active JOIN interactions i ON i.id=active.id LEFT JOIN parts p ON p.interaction_id=i.id AND p.deleted_at IS NULL LEFT JOIN versions v ON v.id=p.selected_version_id AND v.status!='generating' WHERE i.deleted_at IS NULL AND i.seq<?2 ORDER BY i.seq,p.idx"),&[json!(c),json!(before)])?;
                queried.iter().collect()
            }
        };
        let mut sources = Vec::<Value>::new();
        let mut index = HashMap::new();
        for r in metadata {
            let iid = s(r, "id");
            let idx = *index.entry(iid).or_insert_with(|| {
                sources.push(json!({"id":r["id"],"seq":r["seq"],"versions":[],"scenes":[]}));
                sources.len() - 1
            });
            if r["version_id"].is_string() {
                sources[idx]["versions"]
                    .as_array_mut()
                    .unwrap()
                    .push(r["version_id"].clone());
            }
        }
        for scene in self.rows(&format!("{PATH_CTE} SELECT s.* FROM scene_states s JOIN p ON p.id=s.interaction_id JOIN interactions i ON i.id=s.interaction_id WHERE i.deleted_at IS NULL AND s.seq<?2 ORDER BY s.seq,s.created_at,s.rowid"),&[json!(c),json!(before)])?{
            if let Some(idx)=index.get(&s(&scene,"interaction_id")){sources[*idx]["scenes"].as_array_mut().unwrap().push(json!({"id":scene["id"],"location":scene["location"],"time":scene["time_text"],"mood":scene["mood"],"present":parsed(&scene,"present_json",json!([])),"objects":parsed(&scene,"objects_json",json!([]))}));}
        }
        let mut prefix = rules::hash(&format!("rust-timeline-v4\n{prompt}"));
        for source in &mut sources {
            prefix = rules::hash(&format!("{prefix}\n{source}"));
            source["fingerprint"] = json!(prefix);
        }
        let key = format!("{c}:{before}:{prefix}");
        if let Some(hit) = self.timeline_cache.borrow().iter().find(|t| t.key == key) {
            return Ok(hit.clone());
        }
        // Each fingerprint chains the prompt and every earlier source, so the same fingerprint is always the
        // same snapshot: snapshots already held by a cached timeline of this conversation are reused as they are,
        // and only the rest is read from the database (or derived). A new turn costs one snapshot instead of a
        // reread and reparse of the whole path.
        let mut known = HashMap::<String, (std::sync::Arc<Value>, usize)>::new();
        for t in self.timeline_cache.borrow().iter().filter(|t| t.conv == c) {
            for (fingerprint, snapshot, size) in &t.snapshots {
                known
                    .entry(fingerprint.clone())
                    .or_insert_with(|| (snapshot.clone(), *size));
            }
        }
        let unknown = sources
            .iter()
            .map(|v| v["fingerprint"].clone())
            .filter(|f| !known.contains_key(f.as_str().unwrap_or_default()))
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            for row in self.rows("SELECT fingerprint,snapshot_json FROM narrative_snapshots WHERE conv_id=? AND fingerprint IN (SELECT value FROM json_each(?))",&[json!(c),json!(json!(unknown).to_string())])? {
                let size = row["snapshot_json"].as_str().map(str::len).unwrap_or(0);
                known.insert(s(&row, "fingerprint"), (std::sync::Arc::new(parsed(&row, "snapshot_json", Value::Null)), size));
            }
        }
        let missing = sources
            .iter()
            .filter(|v| !known.contains_key(&s(v, "fingerprint")))
            .map(|v| v["id"].clone())
            .collect::<Vec<_>>();
        let mut parts = HashMap::<String, Vec<Value>>::new();
        if !missing.is_empty() {
            for row in self.rows("SELECT i.id,i.user_text,p.idx,v.id version_id,v.content FROM interactions i LEFT JOIN parts p ON p.interaction_id=i.id AND p.deleted_at IS NULL LEFT JOIN versions v ON v.id=p.selected_version_id AND v.status!='generating' WHERE i.id IN (SELECT value FROM json_each(?)) ORDER BY i.seq,p.idx",&[json!(json!(missing).to_string())])?{
                let list=parts.entry(s(&row,"id")).or_insert_with(||if row["user_text"].is_null(){vec![]}else{vec![json!({"id":row["id"],"role":"user","text":row["user_text"]})]});
                if row["version_id"].is_string()&&row["content"].as_str().is_some_and(|s|!s.is_empty()){list.push(json!({"id":row["version_id"],"role":"assistant","text":row["content"]}));}
            }
        }
        let mut snapshots = Vec::<(String, std::sync::Arc<Value>, usize)>::with_capacity(sources.len());
        let mut records = Vec::new();
        for source in &mut sources {
            let fingerprint = s(source, "fingerprint");
            let entry = if let Some((snapshot, size)) = known.get(&fingerprint) {
                (fingerprint, snapshot.clone(), *size)
            } else {
                source["parts"] = json!(parts.remove(&s(source, "id")).unwrap_or_default());
                let snapshot = crate::narrative::derive_snapshots(
                    &prompt,
                    std::slice::from_ref(source),
                    snapshots.last().map(|(_, v, _)| v.as_ref()),
                )
                .remove(0);
                let text = snapshot.to_string();
                let size = text.len();
                records.push(json!({"fingerprint":fingerprint,"id":source["id"],"seq":source["seq"],"snapshot_json":text,"created_at":now()}));
                (fingerprint, std::sync::Arc::new(snapshot), size)
            };
            snapshots.push(entry);
        }
        if !records.is_empty() {
            self.exec("INSERT OR IGNORE INTO narrative_snapshots(fingerprint,conv_id,interaction_id,seq,snapshot_json,created_at) SELECT json_extract(value,'$.fingerprint'),?,json_extract(value,'$.id'),json_extract(value,'$.seq'),json_extract(value,'$.snapshot_json'),json_extract(value,'$.created_at') FROM json_each(?)",&[json!(c),json!(json!(records).to_string())])?;
            self.exec("DELETE FROM narrative_snapshots WHERE conv_id=? AND rowid NOT IN(SELECT rowid FROM narrative_snapshots WHERE conv_id=? ORDER BY rowid DESC LIMIT 4096)",&[json!(c),json!(c)])?;
        }
        let size = snapshots.iter().map(|(_, _, n)| n).sum::<usize>();
        let result = std::sync::Arc::new(Timeline {
            key,
            conv: c.to_owned(),
            anchors: json!(crate::narrative::evidence(&prompt, true)),
            prompt,
            snapshots,
            size,
        });
        let mut cache = self.timeline_cache.borrow_mut();
        cache.push(result.clone());
        // At most 8 entries and ~16 MB: long conversations cannot hold memory without a bound.
        while cache.len() > 8
            || (cache.len() > 1 && cache.iter().map(|e| e.size).sum::<usize>() > TIMELINE_CACHE_BYTES)
        {
            cache.remove(0);
        }
        Ok(result)
    }

    pub(super) fn retrieve(&self, p: &Value) -> Result<Value> {
        let c = s(p, "convId");
        let before = p["beforeSeq"].as_i64().unwrap_or(i64::MAX);
        let start = p["windowStartSeq"].as_i64().unwrap_or(i64::MAX);
        let eligible = self.eligibility(&c, before)?;
        let ids = json!(
            eligible
                .seq
                .iter()
                .map(|(id, seq)| json!({"id":id,"seq":seq}))
                .collect::<Vec<_>>()
        )
        .to_string();
        // MATERIALIZED: `active` is decoded from JSON once and gets an automatic index. Inlined, every join probe
        // re-ran json_each/json_extract over the whole path (720 turns: 25 s in debug, past the store deadline).
        let owners = "WITH active AS MATERIALIZED(SELECT json_extract(value,'$.id') id,json_extract(value,'$.seq') seq FROM json_each(?1)),owners AS(SELECT 'memory' kind,m.id owner_id,coalesce(a.seq,0) seq,m.content text,m.subject,m.importance,m.confidence,m.visibility,m.kind memory_kind,m.source_interaction_id iid,0 start_offset,0 end_offset,NULL source_id,NULL role,0 from_seq FROM memories m LEFT JOIN active a ON a.id=m.source_interaction_id WHERE m.conv_id=?2 AND m.status!='rejected' AND m.review_state='ok' AND (m.source_interaction_id IS NULL OR a.id IS NOT NULL) AND coalesce(a.seq,0)<?3 AND NOT EXISTS(SELECT 1 FROM memory_replacements r JOIN memories replacement ON replacement.id=r.new_id LEFT JOIN active ra ON ra.id=replacement.source_interaction_id WHERE r.old_id=m.id AND replacement.status!='rejected' AND (replacement.source_interaction_id IS NULL OR ra.id IS NOT NULL)) UNION ALL SELECT 'episode',e.id,e.to_seq,e.summary,NULL,0,0,NULL,NULL,NULL,0,0,NULL,NULL,e.from_seq FROM episodes e WHERE e.conv_id=?2 AND e.to_seq<?3 AND e.to_seq<?4 AND NOT EXISTS(SELECT 1 FROM json_each(e.interaction_ids_json) j WHERE j.value NOT IN (SELECT id FROM active)) UNION ALL SELECT 'chunk',t.id,a.seq,t.text,NULL,0,0,NULL,NULL,a.id,t.start_offset,t.end_offset,t.owner_id,CASE WHEN t.kind='user' THEN 'user' ELSE 'assistant' END,0 FROM text_chunks t JOIN active a ON a.id=CASE WHEN t.kind='user' THEN t.owner_id ELSE (SELECT p.interaction_id FROM parts p JOIN versions v ON v.id=p.selected_version_id WHERE v.id=t.owner_id AND p.deleted_at IS NULL AND v.status!='generating') END WHERE t.conv_id=?2 AND a.seq<?3)".to_string();
        let query = s(p, "queryText");
        let qt = rules::tokens(&query);
        let entities = p["entityKeys"].as_array().cloned().unwrap_or_default();
        let match_query = rules::fts(&query, true);
        let mut lexical = HashMap::new();
        let mut metadata = HashMap::new();
        if !match_query.is_empty() {
            for r in self.rows(&format!("{owners} SELECT o.*,bm25(search_fts) rank FROM search_fts f JOIN owners o ON o.kind=f.kind AND o.owner_id=f.owner_id WHERE search_fts MATCH ?5 AND f.conv_id=?2 ORDER BY rank LIMIT 2000"),&[json!(ids),json!(c),json!(start),json!(before),json!(match_query)])? {let key=format!("{}:{}",s(&r,"kind"),s(&r,"owner_id"));let score=1.0/(1.0+lexical.len() as f64*0.015);lexical.insert(key.clone(),score);metadata.insert(key,r);}
        }
        // Entity hits must survive an absent embedding model or no lexical term overlap.
        for entity in &entities {
            if let Some(entity) = entity.as_str().filter(|s| !s.is_empty()) {
                for r in self.rows(&format!("{owners} SELECT * FROM owners WHERE kind='memory' AND lower(coalesce(subject,'')||' '||text) LIKE ?5 LIMIT 256"),&[json!(ids),json!(c),json!(start),json!(before),json!(format!("%{entity}%"))])?{metadata.entry(format!("memory:{}",s(&r,"owner_id"))).or_insert(r);}
            }
        }
        let vector = p["queryVec"].as_array().map(|a| {
            a.iter()
                .filter_map(|v| v.as_f64().map(|v| v as f32))
                .collect::<Vec<_>>()
        });
        let model = p["embeddingModel"].as_str().unwrap_or("bge-m3");
        let mut best = Vec::new();
        let mut seen = HashSet::new();
        let mut hybrid = false;
        let rank = |r: &Value, dense: f64| -> Option<Value> {
            let kind = s(r, "kind");
            let lex = *lexical
                .get(&format!("{kind}:{}", s(r, "owner_id")))
                .unwrap_or(&0.0);
            let sem = lex.max(if dense >= 0.4 { dense } else { 0.0 });
            let text = format!("{} {}", s(r, "subject"), s(r, "text"));
            let ent = if entities
                .iter()
                .any(|e| e.as_str().is_some_and(|e| rules::key(&text).contains(e)))
            {
                1.0
            } else {
                0.0
            };
            if sem == 0.0 && (kind != "memory" || ent == 0.0) {
                return None;
            }
            let overlap = rules::overlap(&qt, &rules::tokens(&text));
            let score = if kind == "memory" {
                memory_score(
                    sem,
                    overlap,
                    ent,
                    r["importance"].as_f64().unwrap_or(0.5),
                    eligible.max_seq - n(r, "seq"),
                )
            } else {
                0.7 * sem + 0.3 * overlap
            };
            Some(
                json!({"type":if kind=="chunk"{"raw"}else{kind.as_str()},"id":r["owner_id"],"kind":r["memory_kind"],"subject":r["subject"],"content":r["text"],"text":r["text"],"importance":r["importance"],"confidence":r["confidence"],"visibility":r["visibility"],"sourceInteractionId":r["iid"],"interactionId":r["iid"],"sourceSeq":r["seq"],"seq":r["seq"],"sourceId":r["source_id"],"role":r["role"],"start":r["start_offset"],"end":r["end_offset"],"fromSeq":r["from_seq"],"toSeq":r["seq"],"score":score,"parts":{"sem":sem,"lexical":lex,"dense":dense,"ent":ent},"eligible":true,"knowledge":[]}),
            )
        };
        if let Some(v) = vector
            .as_ref()
            .filter(|v| !v.is_empty() && v.iter().all(|v| v.is_finite()))
        {
            let mut after = String::new();
            loop {
                // The embedding BLOB is decoded to f32 straight from the SQLite buffer (`rows_with_vector`).
                let page=self.rows_with_vector(&format!("{owners} SELECT o.*,e.vec,e.dim FROM embeddings e JOIN owners o ON o.kind=e.owner_type AND o.owner_id=e.owner_id WHERE e.conv_id=?2 AND e.model=?5 AND e.dim=?6 AND (e.owner_type||':'||e.owner_id)>?7 ORDER BY e.owner_type||':'||e.owner_id LIMIT 256"),&[json!(ids),json!(c),json!(start),json!(before),json!(model),json!(v.len()),json!(after)],"vec",v.len())?;
                if page.is_empty() {
                    break;
                }
                for (row, stored) in &page {
                    let key = format!("{}:{}", s(row, "kind"), s(row, "owner_id"));
                    after = key.clone();
                    let Some(vec) = stored else {
                        continue;
                    };
                    hybrid = true;
                    seen.insert(key);
                    if let Some(candidate) = rank(row, rules::cosine(v, vec)) {
                        best.push(candidate);
                    }
                }
                trim_candidates(&mut best);
                if page.len() < 256 {
                    break;
                }
            }
        }
        for (key, row) in metadata {
            if !seen.contains(&key)
                && let Some(candidate) = rank(&row, 0.0)
            {
                best.push(candidate);
            }
        }
        trim_candidates(&mut best);
        best.sort_by(|a, b| {
            b["score"]
                .as_f64()
                .unwrap_or(0.0)
                .total_cmp(&a["score"].as_f64().unwrap_or(0.0))
        });
        let knowledge = self.knowledge_at(&c, &eligible)?;
        let mut candidates = Vec::new();
        let mut raw = Vec::new();
        let mut counts = HashMap::<String, usize>::new();
        let (mut memories, mut episodes) = (0, 0);
        let limit = p["limit"].as_u64().unwrap_or(24).clamp(1, 64) as usize;
        for mut candidate in best {
            let kind = s(&candidate, "type");
            if kind == "raw" {
                let count = counts.entry(s(&candidate, "interactionId")).or_default();
                if raw.len() < 4 && *count < 2 {
                    *count += 1;
                    raw.push(candidate);
                }
                continue;
            }
            if kind == "memory" {
                if memories >= limit {
                    continue;
                }
                memories += 1;
                candidate["knowledge"] = json!(
                    knowledge
                        .get(&s(&candidate, "id"))
                        .cloned()
                        .unwrap_or_default()
                );
            } else {
                if episodes >= 4 {
                    continue;
                }
                episodes += 1;
            }
            candidates.push(candidate);
        }
        Ok(
            json!({"privateMemories":candidates.iter().filter(|m|m["visibility"]=="private").take(14).cloned().collect::<Vec<_>>(),"candidates":candidates,"raw":raw,"method":if hybrid{"hybrid"}else{"lexical"},"embeddingModel":p["embeddingModel"]}),
        )
    }
}
/// Pair without the redundant text copies (see `context_data`).
fn slim_pair(pair: &Value, with_sources: bool) -> Value {
    let present = pair["present_entities"]
        .as_array()
        .or_else(|| pair["present"].as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = json!({
        "id": pair["id"],
        "seq": pair["seq"],
        "user": pair["user"],
        "assistant": pair["assistant"],
        "complete": pair["complete"],
        "versions": pair["versions"],
        "present_entities": present,
    });
    if with_sources {
        out["sources"] = pair["sources"].clone();
    }
    out
}
/// Notes `context::assemble` can use. The timeline only reads notes BEFORE the history window and not
/// covered by an episode. The window is the first pair of the last `take` (preferred on the ladder, minimum in the worst case):
/// a note from the minimum window on never gets in; a note covered by an episode already summarized before the
/// preferred (smallest) window is covered on every step of the ladder. Everything else stays whole.
fn notes_for_prompt(
    notes: Vec<Value>,
    recent: &[Value],
    episodes: &[Value],
    minimum: usize,
    preferred: usize,
) -> Vec<Value> {
    let window = |take: usize| {
        recent
            .get(recent.len().saturating_sub(take.min(recent.len())))
            .map(|p| n(p, "seq"))
    };
    let (Some(widest), Some(narrowest)) = (window(minimum), window(preferred)) else {
        return notes;
    };
    let covered = episodes
        .iter()
        .filter(|e| n(e, "toSeq") < narrowest)
        .flat_map(|e| e["interactionIds"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect::<HashSet<_>>();
    notes
        .into_iter()
        .filter(|note| {
            n(note, "seq") < widest && !covered.contains(s(note, "interaction_id").as_str())
        })
        .collect()
}
pub fn memory_score(sem: f64, overlap: f64, entity: f64, importance: f64, age: i64) -> f64 {
    0.48 * sem
        + 0.22 * overlap
        + 0.15 * entity
        + 0.1 * importance
        + 0.05 / (1.0 + (age.max(0) as f64) / 60.0)
}
fn trim_candidates(best: &mut Vec<Value>) {
    best.sort_by(|a, b| {
        b["score"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["score"].as_f64().unwrap_or(0.0))
    });
    let mut counts = HashMap::new();
    best.retain(|m| {
        let count = counts.entry(s(m, "type")).or_insert(0);
        *count += 1;
        *count <= if m["type"] == "raw" { 32 } else { 64 }
    });
}

const TIMELINE_CACHE_BYTES: usize = 16 * 1024 * 1024;
/// Narrative timeline up to a point of the path: one snapshot per interaction, keyed by its chained fingerprint.
/// The full view (`narrative::view`) is only built for the memory overview; the context needs just the summary.
pub(crate) struct Timeline {
    key: String,
    conv: String,
    prompt: String,
    anchors: Value,
    /// (fingerprint, snapshot, approximate serialized size)
    snapshots: Vec<(String, std::sync::Arc<Value>, usize)>,
    size: usize,
}
impl Timeline {
    pub(super) fn is_for(&self, conv: &str) -> bool {
        self.conv == conv
    }
    /// What the context assembly reads: the same fields `narrative::view` gives, without the snapshot list.
    pub(super) fn summary(&self) -> Value {
        json!({"version":3,"basis":"narrative","anchors":self.anchors,"current":self.snapshots.last().map(|(_, v, _)| v.as_ref())})
    }
    pub(super) fn view(&self) -> Value {
        let snapshots = self
            .snapshots
            .iter()
            .map(|(_, v, _)| v.as_ref().clone())
            .collect::<Vec<_>>();
        crate::narrative::view(&self.prompt, &snapshots)
    }
}
