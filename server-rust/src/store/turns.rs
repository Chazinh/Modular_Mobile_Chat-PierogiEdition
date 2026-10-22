use super::{Database, FinishRequest, PATH_CTE, TurnMode, TurnRequest, maintenance::usage_summary};
use crate::{Error, Result, id, n, now, parsed, rules, s};
use serde_json::{Value, json};

impl Database {
    fn info(&self, g: &Value) -> Result<Value> {
        let i = self.one(
            "SELECT * FROM interactions WHERE id=?",
            &[g["interaction_id"].clone()],
        )?;
        Ok(
            json!({"generationId":g["id"],"convId":g["conv_id"],"opId":g["op_id"],"mode":g["mode"],"status":g["status"],"interactionId":g["interaction_id"],"partId":g["part_id"],"versionId":g["version_id"],"error":g["error"],"userMessage":if i["user_text"].is_null(){Value::Null}else{self.user_message(&i)?}}),
        )
    }
    pub(super) fn begin(&self, p: &TurnRequest) -> Result<Value> {
        let c = self.conv(&p.conv_id, false)?;
        if let Some(op) = p.op_id.as_deref().filter(|x| !x.is_empty()) {
            let prev = self.one(
                "SELECT * FROM generations WHERE conv_id=? AND op_id=?",
                &[json!(p.conv_id), json!(op)],
            )?;
            if !prev.is_null() {
                let mut info = self.info(&prev)?;
                info["replay"] = json!(true);
                info["revision"] = c["revision"].clone();
                return Ok(info);
            }
        }
        self.ensure_idle(&p.conv_id)?;
        let route = p
            .route_id
            .as_deref()
            .unwrap_or(c["route_id"].as_str().unwrap_or_default());
        let preset = p
            .preset_id
            .as_deref()
            .unwrap_or(c["preset_id"].as_str().unwrap_or_default());
        let effort = p.effort.as_deref().or(c["effort"].as_str());
        let time = now();
        let generation = id();
        let mut origin = "generate";
        let mut select = true;
        let (interaction, part) = match p.mode {
            TurnMode::Send | TurnMode::Edit => {
                let limit = rules::user_limit_for(Some(preset));
                let mut text = p.content.trim();
                if rules::utf16_len(&p.content) > limit {
                    if preset == "muse-spark-13" || preset.starts_with("muse") {
                        let byte_limit = rules::byte_at(text, limit);
                        text = &text[..byte_limit];
                    } else {
                        return Err(Error::new(
                            422,
                            "Sua mensagem pode ter no máximo 6.000 caracteres.",
                        ));
                    }
                }
                if text.is_empty() {
                    return Err(Error::new(400, "Mensagem vazia."));
                }
                let mut parent = c["head_interaction_id"].clone();
                let mut edit_of = Value::Null;
                let seq = if p.mode == TurnMode::Edit {
                    let old=self.one("SELECT * FROM interactions WHERE id=? AND conv_id=? AND deleted_at IS NULL",&[json!(p.target_id),json!(p.conv_id)])?;
                    if old.is_null() || old["user_text"].is_null() {
                        return Err(Error::new(404, "Mensagem não encontrada."));
                    }
                    parent = old["parent_id"].clone();
                    edit_of = old["id"].clone();
                    n(&old, "seq")
                } else {
                    n(
                        &self.one("SELECT seq FROM interactions WHERE id=?", &[parent.clone()])?,
                        "seq",
                    ) + 1
                };
                let i = id();
                let part = id();
                self.exec("INSERT INTO interactions(id,conv_id,parent_id,seq,kind,user_text,user_ts,user_edited,edit_of,created_at) VALUES(?,?,?,?,'turn',?,?,?,?,?)",&[json!(i),json!(p.conv_id),parent,json!(seq),json!(text),json!(time),json!(!edit_of.is_null()),edit_of,json!(time)])?;
                self.exec(
                    "UPDATE conversations SET head_interaction_id=? WHERE id=?",
                    &[json!(i), json!(p.conv_id)],
                )?;
                self.index(&p.conv_id, "user", &i, text)?;
                self.exec(
                    "INSERT INTO parts(id,interaction_id,idx,created_at) VALUES(?,?,0,?)",
                    &[json!(part), json!(i), json!(time)],
                )?;
                (i, part)
            }
            TurnMode::Regenerate | TurnMode::Continue => {
                let path = self.path(&p.conv_id)?;
                let last = path.last().ok_or_else(|| {
                    Error::new(
                        400,
                        if p.mode == TurnMode::Continue {
                            "Nada para continuar."
                        } else {
                            "Nada para refazer."
                        },
                    )
                })?;
                let i = s(last, "id");
                let parts=self.rows("SELECT * FROM parts WHERE interaction_id=? AND deleted_at IS NULL ORDER BY idx DESC",&[json!(i)])?;
                if p.mode == TurnMode::Regenerate && !parts.is_empty() {
                    let target = &parts[0];
                    if p.target_id.as_ref().is_some_and(|id| target["id"] != *id) {
                        return Err(Error::new(400, "Só a última resposta pode ser refeita."));
                    }
                    origin = "regenerate";
                    select = false;
                    (i, s(target, "id"))
                } else {
                    let part = id();
                    let max = self.one(
                        "SELECT coalesce(max(idx),-1) m FROM parts WHERE interaction_id=?",
                        &[json!(i)],
                    )?;
                    self.exec(
                        "INSERT INTO parts(id,interaction_id,idx,created_at) VALUES(?,?,?,?)",
                        &[json!(part), json!(i), json!(n(&max, "m") + 1), json!(time)],
                    )?;
                    if p.mode == TurnMode::Continue {
                        origin = "continue";
                    }
                    (i, part)
                }
            }
        };
        let version = id();
        self.exec("INSERT INTO versions(id,part_id,content,status,origin,generation_id,route_id,preset_id,effort,created_at) VALUES(?,?,'','generating',?,?,?,?,?,?)",&[json!(version),json!(part),json!(origin),json!(generation),json!(route),json!(preset),json!(effort),json!(time)])?;
        if select {
            self.exec(
                "UPDATE parts SET selected_version_id=? WHERE id=?",
                &[json!(version), json!(part)],
            )?;
        }
        self.exec("INSERT INTO generations(id,conv_id,op_id,interaction_id,part_id,version_id,mode,status,route_id,preset_id,effort,request_json,started_at) VALUES(?,?,?,?,?,?,?,'running',?,?,?,?,?)",&[json!(generation),json!(p.conv_id),json!(p.op_id.as_deref().filter(|x|!x.is_empty())),json!(interaction),json!(part),json!(version),json!(p.mode.as_str()),json!(route),json!(preset),json!(effort),json!(p.request.to_string()),json!(time)])?;
        if p.mode == TurnMode::Edit {
            self.invalidate(&p.conv_id, None)?;
        }
        if p.mode == TurnMode::Regenerate {
            self.exec("UPDATE jobs SET status='skipped',updated_at=? WHERE interaction_id=? AND status IN ('pending','blocked')",&[json!(time),json!(interaction)])?;
        }
        let revision = self.bump(&p.conv_id)?;
        let mut info = self.generation(&generation)?;
        info["replay"] = json!(false);
        info["revision"] = json!(revision);
        Ok(info)
    }
    pub(super) fn generation(&self, g: &str) -> Result<Value> {
        let row = self.one("SELECT * FROM generations WHERE id=?", &[json!(g)])?;
        if row.is_null() {
            return Ok(Value::Null);
        }
        let v = self.one(
            "SELECT content,status FROM versions WHERE id=?",
            &[row["version_id"].clone()],
        )?;
        let mut out = self.info(&row)?;
        out["content"] = json!(s(&v, "content"));
        out["versionStatus"] = v["status"].clone();
        out["usage"] = parsed(&row, "usage_json", Value::Null);
        Ok(out)
    }
    pub(super) fn flush(&self, v: &str, text: &str) -> Result<Value> {
        if rules::utf16_len(text) > rules::REPLY_STORE_LIMIT {
            return Err(Error::new(
                422,
                "A resposta pode ter no máximo 10.800 caracteres.",
            ));
        }
        // Text identical to what is stored changes no row (and does not mark the vault dirty).
        if self.exec(
            "UPDATE versions SET content=? WHERE id=? AND status='generating' AND content IS NOT ?1",
            &[json!(text), json!(v)],
        )? == 0
            && self
                .one(
                    "SELECT 1 FROM versions WHERE id=? AND status='generating'",
                    &[json!(v)],
                )?
                .is_null()
        {
            return Err(Error::new(409, "Versão não está em geração."));
        }
        Ok(json!(true))
    }
    pub(super) fn finish(&self, p: &FinishRequest) -> Result<Value> {
        if !["complete", "length", "interrupted", "error"].contains(&p.status.as_str()) {
            return Err(Error::new(400, "Estado final inválido."));
        }
        let g = self.one(
            "SELECT * FROM generations WHERE id=?",
            &[json!(p.generation_id)],
        )?;
        if g.is_null() {
            return Err(Error::new(404, "Geração não encontrada."));
        }
        if g["status"] != "running" {
            return Ok(json!({"already":true,"status":g["status"]}));
        }
        let v = self.one(
            "SELECT * FROM versions WHERE id=?",
            &[g["version_id"].clone()],
        )?;
        let text = p
            .text
            .as_deref()
            .unwrap_or(v["content"].as_str().unwrap_or_default());
        if rules::utf16_len(text) > rules::REPLY_STORE_LIMIT {
            return Err(Error::new(
                422,
                "A resposta pode ter no máximo 10.800 caracteres.",
            ));
        }
        let has_text = !text.trim().is_empty();
        let status = if !has_text && p.status != "error" {
            "empty"
        } else {
            &p.status
        };
        let c = s(&g, "conv_id");
        if has_text {
            self.exec(
                "UPDATE versions SET content=?,status=?,usage_json=?,finished_at=? WHERE id=?",
                &[
                    json!(text),
                    json!(status),
                    // the message keeps the summary the UI reads; the full diagnostic stays in `generations`
                    if p.usage.is_null() {
                        Value::Null
                    } else {
                        json!(usage_summary(&p.usage).to_string())
                    },
                    json!(now()),
                    g["version_id"].clone(),
                ],
            )?;
            let select = !(g["mode"] == "regenerate" && status == "error");
            if select {
                self.exec(
                    "UPDATE parts SET selected_version_id=? WHERE id=?",
                    &[g["version_id"].clone(), g["part_id"].clone()],
                )?;
            }
            self.index(&c, "asst", &s(&g, "version_id"), text)?;
            if select && matches!(status, "complete" | "length" | "interrupted") {
                self.invalidate(&c, Some(&s(&g, "interaction_id")))?;
                self.enqueue_turn(&c, &s(&g, "interaction_id"), &s(&g, "version_id"))?;
            }
        } else {
            self.unindex("asst", &s(&g, "version_id"))?;
            self.exec(
                "DELETE FROM versions WHERE id=?",
                &[g["version_id"].clone()],
            )?;
            // Clean empty parts for every mode; a retry must not leave a dangling selection.
            let other = self.one(
                "SELECT count(*) n FROM versions WHERE part_id=?",
                &[g["part_id"].clone()],
            )?;
            if n(&other, "n") == 0 {
                self.exec("DELETE FROM parts WHERE id=?", &[g["part_id"].clone()])?;
            }
        }
        self.exec(
            "UPDATE generations SET status=?,usage_json=?,error=?,ended_at=? WHERE id=?",
            &[
                json!(status),
                if p.usage.is_null() {
                    Value::Null
                } else {
                    json!(p.usage.to_string())
                },
                json!(p.error),
                json!(now()),
                g["id"].clone(),
            ],
        )?;
        let revision = self.bump(&c)?;
        let i = self.one(
            "SELECT * FROM interactions WHERE id=?",
            &[g["interaction_id"].clone()],
        )?;
        let message = if has_text {
            self.part_messages(&i)?
                .into_iter()
                .find(|m| m["id"] == g["part_id"])
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        Ok(
            json!({"already":false,"status":status,"revision":revision,"message":message,"versionKept":has_text}),
        )
    }
    pub(super) fn recover(&self) -> Result<Value> {
        let rows = self.rows("SELECT id FROM generations WHERE status='running'", &[])?;
        for g in &rows {
            self.finish(&FinishRequest {
                generation_id: s(g, "id"),
                status: "interrupted".into(),
                text: None,
                usage: Value::Null,
                error: Some("Servidor reiniciado durante a geração.".into()),
            })?;
        }
        Ok(json!(rows.len()))
    }
    pub(super) fn part(&self, c: &str, p: &str) -> Result<Value> {
        let r=self.one("SELECT p.* FROM parts p JOIN interactions i ON i.id=p.interaction_id WHERE p.id=? AND i.conv_id=?",&[json!(p),json!(c)])?;
        if r.is_null() {
            return Err(Error::new(404, "Mensagem não encontrada."));
        }
        Ok(r)
    }
    pub(super) fn edit(
        &self,
        c: &str,
        part: &str,
        content: &str,
        op: Option<&str>,
    ) -> Result<Value> {
        self.conv(c, false)?;
        if let Some(op) = op {
            let mut prev = self.one(
                "SELECT result_json FROM operations WHERE conv_id=? AND op_id=?",
                &[json!(c), json!(op)],
            )?;
            if prev.is_null() {
                prev = self.one("SELECT json_extract(row_json,'$.result_json') result_json FROM sync_retained WHERE table_name='operations' AND conv_id=? AND row_key=json_array(?,?)",
                    &[json!(c),json!(c),json!(op)])?;
            }
            if !prev.is_null() {
                return Ok(parsed(&prev, "result_json", Value::Null));
            }
        }
        self.ensure_idle(c)?;
        if rules::utf16_len(content) > rules::REPLY_STORE_LIMIT {
            return Err(Error::new(
                422,
                "A resposta pode ter no máximo 10.800 caracteres.",
            ));
        }
        let text = content.trim();
        if text.is_empty() {
            return Err(Error::new(400, "Mensagem vazia."));
        }
        let p = self.part(c, part)?;
        let old = self.one(
            "SELECT * FROM versions WHERE id=?",
            &[p["selected_version_id"].clone()],
        )?;
        let version = id();
        let time = now();
        self.exec("INSERT INTO versions(id,part_id,content,status,origin,route_id,preset_id,effort,created_at,finished_at) VALUES(?,?,?,'complete','edit',?,?,?,?,?)",&[json!(version),json!(part),json!(text),old["route_id"].clone(),old["preset_id"].clone(),old["effort"].clone(),json!(time),json!(time)])?;
        self.exec(
            "UPDATE parts SET selected_version_id=? WHERE id=?",
            &[json!(version), json!(part)],
        )?;
        self.index(c, "asst", &version, text)?;
        self.refresh_descendants(c, &s(&p, "interaction_id"))?;
        let out = self.changed_part(c, &p)?;
        if let Some(op) = op {
            self.exec("INSERT INTO operations(conv_id,op_id,kind,result_json,created_at) VALUES(?,?,'edit',?,?)",&[json!(c),json!(op),json!(out.to_string()),json!(now())])?;
        }
        Ok(out)
    }
    pub(super) fn versions(&self, c: &str, part: &str) -> Result<Value> {
        self.conv(c, false)?;
        let p = self.part(c, part)?;
        let rows = self.rows(
            "SELECT * FROM versions WHERE part_id=? ORDER BY created_at,rowid",
            &[json!(part)],
        )?;
        Ok(json!(rows.iter().map(|v|json!({"id":v["id"],"status":v["status"],"origin":v["origin"],"routeId":v["route_id"],"effort":v["effort"],"createdAt":v["created_at"],"selected":v["id"]==p["selected_version_id"],"length":rules::utf16_len(v["content"].as_str().unwrap_or_default()),"preview":rules::clip(v["content"].as_str().unwrap_or_default(),160),"content":v["content"]})).collect::<Vec<_>>()))
    }
    pub(super) fn select_version(&self, c: &str, part: &str, version: &str) -> Result<Value> {
        self.conv(c, false)?;
        self.ensure_idle(c)?;
        let p = self.part(c, part)?;
        let v = self.one(
            "SELECT * FROM versions WHERE id=? AND part_id=?",
            &[json!(version), json!(part)],
        )?;
        if v.is_null() {
            return Err(Error::new(404, "Versão não encontrada."));
        }
        if v["status"] == "generating" {
            return Err(Error::new(409, "Versão ainda em geração."));
        }
        self.exec(
            "UPDATE parts SET selected_version_id=? WHERE id=?",
            &[json!(version), json!(part)],
        )?;
        self.refresh_descendants(c, &s(&p, "interaction_id"))?;
        self.changed_part(c, &p)
    }
    fn changed_part(&self, c: &str, p: &Value) -> Result<Value> {
        let i = self.one(
            "SELECT * FROM interactions WHERE id=?",
            &[p["interaction_id"].clone()],
        )?;
        Ok(
            json!({"revision":self.bump(c)?,"message":self.part_messages(&i)?.into_iter().find(|m|m["id"]==p["id"]).unwrap_or(Value::Null)}),
        )
    }
    pub(super) fn enqueue_turn(&self, c: &str, i: &str, v: &str) -> Result<()> {
        self.exec("UPDATE jobs SET status='skipped',updated_at=? WHERE interaction_id=? AND kind='turn' AND status IN ('pending','blocked')",&[json!(now()),json!(i)])?;
        let flags=self.one("SELECT count(*) n FROM review_flags WHERE interaction_id=? AND status='open' AND version_id IN (SELECT selected_version_id FROM parts WHERE interaction_id=? AND deleted_at IS NULL)",&[json!(i),json!(i)])?;
        self.exec("INSERT INTO jobs(id,conv_id,interaction_id,version_id,kind,status,payload_json,created_at,updated_at) VALUES(?,?,?,?,'turn',?,'{}',?,?)",&[json!(id()),json!(c),json!(i),json!(v),json!(if n(&flags,"n")>0{"blocked"}else{"pending"}),json!(now()),json!(now())])?;
        Ok(())
    }
    pub(super) fn delete_derived(&self, i: &str) -> Result<()> {
        // Reprocessing a later turn removes its derived relationship revision.
        // Restore earlier rows before deleting that revision so no dangling
        // supersession link survives into a subsequent sync snapshot.
        self.exec("UPDATE relationships SET superseded_by=NULL,status='current' WHERE superseded_by IN (SELECT id FROM relationships WHERE source_interaction_id=?)", &[json!(i)])?;
        for m in self.rows(
            "SELECT id FROM memories WHERE source_interaction_id=?",
            &[json!(i)],
        )? {
            self.delete_memory_internal(&s(&m, "id"))?;
        }
        for (table, col) in [
            ("knowledge_events", "source_interaction_id"),
            ("scene_states", "interaction_id"),
            ("character_states", "interaction_id"),
            ("relationships", "source_interaction_id"),
            ("threads", "opened_interaction_id"),
            ("interaction_notes", "interaction_id"),
        ] {
            self.exec(&format!("DELETE FROM {table} WHERE {col}=?"), &[json!(i)])?;
        }
        self.exec("UPDATE threads SET resolved_interaction_id=NULL,status='open' WHERE resolved_interaction_id=?",&[json!(i)])?;
        for e in self.rows("SELECT id FROM episodes WHERE EXISTS(SELECT 1 FROM json_each(episodes.interaction_ids_json) j WHERE j.value=?)",&[json!(i)])?{self.unindex("episode",&s(&e,"id"))?;self.exec("DELETE FROM embeddings WHERE owner_type='episode' AND owner_id=?",&[e["id"].clone()])?;self.exec("DELETE FROM episodes WHERE id=?",&[e["id"].clone()])?;}
        Ok(())
    }
    fn refresh_descendants(&self, c: &str, i: &str) -> Result<()> {
        for r in self.rows("WITH RECURSIVE d(id) AS (SELECT id FROM interactions WHERE id=?1 AND conv_id=?2 UNION SELECT i.id FROM interactions i JOIN d ON i.parent_id=d.id WHERE i.conv_id=?2) SELECT id FROM d",&[json!(i),json!(c)])?{self.delete_derived(&s(&r,"id"))?;self.exec("UPDATE jobs SET status='skipped',updated_at=? WHERE interaction_id=? AND status IN ('pending','running','blocked','failed')",&[json!(now()),r["id"].clone()])?;}
        self.invalidate(c, None)
    }
    pub(super) fn invalidate(&self, c: &str, changed: Option<&str>) -> Result<()> {
        if let Some(i) = changed {
            self.delete_derived(i)?;
        }
        let valid = "(SELECT i.id FROM p JOIN interactions i ON i.id=p.id WHERE i.conv_id=?1 AND i.deleted_at IS NULL)";
        for (table, col) in [
            ("memories", "source_interaction_id"),
            ("relationships", "source_interaction_id"),
            ("threads", "opened_interaction_id"),
        ] {
            // Change clause: only touches (and only counts as a change) the row whose value changes.
            let to = format!(
                "CASE WHEN {col} IS NULL OR {col} IN {valid} THEN 'ok' ELSE 'inactive' END"
            );
            self.exec(&format!("{PATH_CTE} UPDATE {table} SET validity={to} WHERE conv_id=?1 AND validity IS NOT ({to})"),&[json!(c)])?;
        }
        let to = format!(
            "CASE WHEN NOT EXISTS(SELECT 1 FROM json_each(episodes.interaction_ids_json) j WHERE j.value NOT IN {valid}) THEN 'ok' ELSE 'inactive' END"
        );
        self.exec(&format!("{PATH_CTE} UPDATE episodes SET validity={to} WHERE conv_id=?1 AND validity IS NOT ({to})"),&[json!(c)])?;
        let to = "CASE WHEN EXISTS(SELECT 1 FROM memory_replacements r JOIN memories m ON m.id=r.new_id WHERE r.old_id=memories.id AND m.validity='ok' AND m.status!='rejected') THEN 'historical' ELSE 'current' END";
        self.exec(&format!("UPDATE memories SET status={to} WHERE conv_id=? AND status IN ('current','historical') AND status IS NOT ({to})"),&[json!(c)])?;
        let to = "CASE WHEN EXISTS(SELECT 1 FROM relationships s WHERE s.id=relationships.superseded_by AND s.validity='ok') THEN 'historical' ELSE 'current' END";
        self.exec(&format!("UPDATE relationships SET status={to} WHERE conv_id=? AND superseded_by IS NOT NULL AND status IS NOT ({to})"),&[json!(c)])?;
        let to = format!(
            "CASE WHEN resolved_interaction_id IS NOT NULL AND resolved_interaction_id IN {valid} THEN 'resolved' ELSE 'open' END"
        );
        self.exec(&format!("{PATH_CTE} UPDATE threads SET status={to} WHERE conv_id=?1 AND status IS NOT ({to})"),&[json!(c)])?;
        self.exec(&format!("{PATH_CTE} UPDATE jobs SET status='skipped',updated_at=?2 WHERE conv_id=?1 AND interaction_id IS NOT NULL AND interaction_id NOT IN {valid} AND status IN ('pending','running','blocked','failed')"),&[json!(c),json!(now())])?;
        for i in self.path(c)? {
            if !self
                .one(
                    "SELECT 1 FROM interaction_notes WHERE interaction_id=?",
                    &[i["id"].clone()],
                )?
                .is_null()
            {
                continue;
            }
            if !self.one("SELECT 1 FROM jobs WHERE interaction_id=? AND kind='turn' AND status IN ('pending','running','blocked','failed')",&[i["id"].clone()])?.is_null(){continue;}
            let v=self.one("SELECT v.id FROM parts p JOIN versions v ON v.id=p.selected_version_id WHERE p.interaction_id=? AND p.deleted_at IS NULL AND v.status!='generating' AND length(v.content)>0 ORDER BY p.idx DESC LIMIT 1",&[i["id"].clone()])?;
            if !v.is_null() {
                self.enqueue_turn(c, &s(&i, "id"), &s(&v, "id"))?;
            }
        }
        Ok(())
    }
}
