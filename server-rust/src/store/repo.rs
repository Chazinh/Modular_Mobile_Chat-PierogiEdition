use super::{Database, PATH_CTE};
use crate::{Error, Result, id, n, now, parsed, rules, s};
use serde_json::{Value, json};

impl Database {
    pub(super) fn character(&self, c: &str, text: &str, source: &str) -> Result<Value> {
        let last = self.one(
            "SELECT rev,hash FROM character_revisions WHERE conv_id=? ORDER BY rev DESC LIMIT 1",
            &[json!(c)],
        )?;
        let hash = rules::hash(text);
        if last["hash"] == hash {
            return Ok(json!({"changed":false,"rev":last["rev"]}));
        }
        let rev = n(&last, "rev") + 1;
        self.exec("INSERT INTO character_revisions(conv_id,rev,text,hash,source,created_at) VALUES(?,?,?,?,?,?)",&[json!(c),json!(rev),json!(text),json!(hash),json!(source),json!(now())])?;
        self.exec(
            "UPDATE conversations SET character_rev=? WHERE id=?",
            &[json!(rev), json!(c)],
        )?;
        Ok(json!({"changed":true,"rev":rev}))
    }
    pub(super) fn create(&self, p: &Value) -> Result<Value> {
        let c = p["id"]
            .as_str()
            .filter(|x| !x.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(id);
        let time = p["createdAt"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(now);
        let route = p["routeId"]
            .as_str()
            .filter(|x| !x.is_empty())
            .ok_or_else(|| Error::new(400, "Informe routeId."))?;
        let preset = p["presetId"]
            .as_str()
            .filter(|x| !x.is_empty())
            .ok_or_else(|| Error::new(400, "Informe presetId."))?;
        if !self
            .one("SELECT id FROM conversations WHERE id=?", &[json!(c)])?
            .is_null()
        {
            return Err(Error::new(409, "ID de conversa já existe."));
        }
        self.exec("INSERT INTO conversations(id,name,persona,route_id,preset_id,effort,temperature,avatar,settings_json,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)",&[
            json!(c),json!(p["name"].as_str().filter(|x|!x.is_empty()).unwrap_or("Novo chat")),json!(p["persona"].as_str().unwrap_or("Alex")),json!(route),json!(preset),p["effort"].clone(),p["temperature"].clone(),p["avatar"].clone(),json!(p["settings"].as_object().map(|_|p["settings"].to_string()).unwrap_or("{}".into())),json!(time),json!(time)])?;
        self.character(
            &c,
            p["prompt"].as_str().unwrap_or_default(),
            p["promptSource"].as_str().unwrap_or("app"),
        )?;
        self.conversation(&c, false)
    }
    pub(super) fn conversation(&self, c: &str, history: bool) -> Result<Value> {
        let r = self.conv(c, false)?;
        let mut v = self.view(&r)?;
        if history {
            v["history"] = json!(self.messages(c)?);
        }
        Ok(v)
    }
    pub(super) fn view(&self, r: &Value) -> Result<Value> {
        let settings = parsed(r, "settings_json", json!({}));
        let persona = if let Some(p) = settings["personaId"].as_str() {
            self.one(
                "SELECT id,name,description,avatar FROM personas WHERE id=?",
                &[json!(p)],
            )?
        } else {
            Value::Null
        };
        let ch = self.one(
            "SELECT text FROM character_revisions WHERE conv_id=? ORDER BY rev DESC LIMIT 1",
            &[r["id"].clone()],
        )?;
        // retired presets/routes and unused effort become what the app accepts today (read only; the record does not change)
        let (route, preset, effort) =
            crate::presets::migrate(&s(r, "preset_id"), &s(r, "route_id"), &r["effort"]);
        let thinking_enabled =
            crate::presets::thinking_enabled(&crate::presets::by_id(&preset), &settings);
        Ok(
            json!({"id":r["id"],"name":r["name"],"persona":r["persona"],"routeId":route,"model":route,"presetId":preset,"effort":effort,"thinkingEnabled":thinking_enabled,"temperature":r["temperature"],"avatar":r["avatar"],"pinned":n(r,"pinned")!=0,"archived":n(r,"archived")!=0,"settings":settings,"revision":r["revision"],"characterRev":r["character_rev"],"prompt":s(&ch,"text"),"createdAt":r["created_at"],"updatedAt":r["updated_at"],"deletedAt":r["deleted_at"],"headInteractionId":r["head_interaction_id"],"personaProfile":persona}),
        )
    }
    pub(super) fn sibling(&self, i: &Value) -> Result<Value> {
        let siblings=self.rows("SELECT id FROM interactions WHERE conv_id=? AND parent_id IS ? AND deleted_at IS NULL ORDER BY created_at,rowid",&[i["conv_id"].clone(),i["parent_id"].clone()])?;
        if siblings.len() < 2 {
            return Ok(Value::Null);
        }
        Ok(
            json!({"index":siblings.iter().position(|r|r["id"]==i["id"]).unwrap_or(0)+1,"count":siblings.len(),"ids":siblings.iter().map(|r|r["id"].clone()).collect::<Vec<_>>()}),
        )
    }
    pub(super) fn user_message(&self, i: &Value) -> Result<Value> {
        Ok(
            json!({"id":i["id"],"role":"user","content":i["user_text"],"ts":i["user_ts"].as_str().unwrap_or(i["created_at"].as_str().unwrap_or_default()),"interactionId":i["id"],"edited":n(i,"user_edited")!=0,"branch":self.sibling(i)?}),
        )
    }
    pub(super) fn part_messages(&self, i: &Value) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for p in self.rows(
            "SELECT * FROM parts WHERE interaction_id=? AND deleted_at IS NULL ORDER BY idx",
            &[i["id"].clone()],
        )? {
            let mut v = self.one(
                "SELECT * FROM versions WHERE id=?",
                &[p["selected_version_id"].clone()],
            )?;
            if v.is_null() {
                v=self.one("SELECT * FROM versions WHERE part_id=? ORDER BY created_at DESC,rowid DESC LIMIT 1",&[p["id"].clone()])?;
            }
            if v.is_null() {
                continue;
            }
            let count = self.one(
                "SELECT count(*) n FROM versions WHERE part_id=?",
                &[p["id"].clone()],
            )?;
            let flags=self.rows("SELECT id,kind,detail,evidence,source FROM review_flags WHERE version_id=? AND status='open'",&[v["id"].clone()])?;
            out.push(json!({"id":p["id"],"role":"assistant","content":v["content"],"ts":v["finished_at"].as_str().unwrap_or(v["created_at"].as_str().unwrap_or_default()),"interactionId":i["id"],"partIndex":p["idx"],"versionId":v["id"],"versionCount":count["n"],"status":v["status"],"origin":v["origin"],"edited":v["origin"]=="edit","model":v["route_id"],"presetId":v["preset_id"],"effort":v["effort"],"usage":parsed(&v,"usage_json",Value::Null),"flags":flags}));
        }
        Ok(out)
    }
    pub(super) fn messages(&self, c: &str) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for i in self.path(c)? {
            if !i["user_text"].is_null() {
                out.push(self.user_message(&i)?);
            }
            out.extend(self.part_messages(&i)?);
        }
        Ok(out)
    }
    pub(super) fn list(&self, archived: bool, query: &str) -> Result<Value> {
        let needle = rules::key(query);
        let mut out = Vec::new();
        for r in self.rows("SELECT * FROM conversations WHERE deleted_at IS NULL AND archived=? ORDER BY pinned DESC,updated_at DESC",&[json!(archived)])?{
            if !needle.is_empty()&&!rules::key(&s(&r,"name")).contains(&needle){continue;}
            let id = s(&r, "id");
            let mut v = self.view(&r)?;
            // A reply only appears when finished: the partial text of a generation in progress never goes to the preview.
            let (mut last, mut count) = (String::new(), 0usize);
            for i in self.path(&id)? {
                let has_user = !i["user_text"].is_null();
                if has_user {
                    count += 1;
                }
                let parts = self
                    .part_messages(&i)?
                    .into_iter()
                    .filter(|m| m["status"] != "generating")
                    .collect::<Vec<_>>();
                count += parts.len();
                if let Some(part) = parts.last() {
                    last = s(part, "content");
                } else if has_user {
                    last = s(&i, "user_text");
                }
            }
            let total = |sql: &str| -> Result<i64> { Ok(n(&self.one(sql, &[json!(id)])?, "n")) };
            v["snippet"] = json!(rules::clip(&last, 160));
            v["lastMessage"] = json!(rules::clip(&last, 240));
            v["messageCount"] = json!(count);
            v["pendingJobs"] = json!(total("SELECT COUNT(*) n FROM jobs WHERE conv_id=? AND status IN ('pending','running')")?);
            v["openFlags"] = json!(total("SELECT COUNT(*) n FROM review_flags WHERE conv_id=? AND status='open'")?);
            v["generating"] = json!(total("SELECT COUNT(*) n FROM generations WHERE conv_id=? AND status='running'")? > 0);
            out.push(v);
        }
        Ok(json!(out))
    }
    pub(super) fn patch(&self, c: &str, p: &Value, base: Option<i64>) -> Result<Value> {
        let row = self.conv(c, false)?;
        let mut changed = false;
        let mut character_changed = false;
        if p["prompt"].is_string() && base.is_some_and(|b| b != n(&row, "character_rev")) {
            return Err(Error::new(409,"A personalização foi alterada em outro lugar.").extra("code","CHARACTER_CONFLICT").extra("current",json!({"characterRev":row["character_rev"],"prompt":self.view(&row)?["prompt"]})));
        }
        for (k, col, limit) in [
            ("name", "name", 80),
            ("persona", "persona", 60),
            ("routeId", "route_id", 200),
            ("presetId", "preset_id", 80),
            ("effort", "effort", 20),
        ] {
            if let Some(raw) = p[k].as_str() {
                let text = rules::clip(raw.trim(), limit);
                if text.is_empty() && matches!(k, "routeId" | "presetId") {
                    continue;
                }
                self.exec(
                    &format!("UPDATE conversations SET {col}=? WHERE id=?"),
                    &[
                        json!(if text.is_empty() && k == "name" {
                            "Sem nome"
                        } else {
                            text
                        }),
                        json!(c),
                    ],
                )?;
                changed = true;
            } else if k == "effort" && p.get(k) == Some(&Value::Null) {
                self.exec(
                    "UPDATE conversations SET effort=NULL WHERE id=?",
                    &[json!(c)],
                )?;
                changed = true;
            }
        }
        for k in ["pinned", "archived"] {
            if let Some(b) = p[k].as_bool() {
                self.exec(
                    &format!("UPDATE conversations SET {k}=? WHERE id=?"),
                    &[json!(b), json!(c)],
                )?;
                changed = true;
            }
        }
        if let Some(temp) = p["temperature"].as_f64() {
            self.exec(
                "UPDATE conversations SET temperature=? WHERE id=?",
                &[
                    json!((temp * 100.0).round().clamp(0.0, 200.0) / 100.0),
                    json!(c),
                ],
            )?;
            changed = true;
        } else if p.get("temperature") == Some(&Value::Null) {
            self.exec(
                "UPDATE conversations SET temperature=NULL WHERE id=?",
                &[json!(c)],
            )?;
            changed = true;
        }
        if p["avatar"]
            .as_str()
            .is_some_and(|t| rules::utf16_len(t) <= 400000)
            || p.get("avatar") == Some(&Value::Null)
        {
            self.exec(
                "UPDATE conversations SET avatar=? WHERE id=?",
                &[p["avatar"].clone(), json!(c)],
            )?;
            changed = true;
        }
        if let Some(settings) = p["settings"].as_object() {
            let mut old = parsed(&row, "settings_json", json!({}))
                .as_object()
                .cloned()
                .unwrap_or_default();
            for (k, v) in settings {
                if v.is_null() {
                    old.remove(k);
                } else {
                    old.insert(k.clone(), v.clone());
                }
            }
            self.exec(
                "UPDATE conversations SET settings_json=? WHERE id=?",
                &[json!(Value::Object(old).to_string()), json!(c)],
            )?;
            changed = true;
        }
        if let Some(prompt) = p["prompt"].as_str() {
            character_changed =
                self.character(c, rules::clip(prompt, 200000), "app")?["changed"] == true;
        }
        if changed || character_changed {
            self.bump(c)?;
        }
        let mut v = self.conversation(c, false)?;
        v["characterChanged"] = json!(character_changed);
        Ok(v)
    }
    pub(super) fn duplicate(&self, c: &str) -> Result<Value> {
        let src = self.conversation(c, false)?;
        let mut p = src.clone();
        p["id"] = json!(id());
        p["name"] = json!(rules::clip(&format!("{} (cópia)", s(&src, "name")), 80));
        p["createdAt"] = json!(now());
        let out = self.create(&p)?;
        for mut row in self.rows("SELECT * FROM characters WHERE conv_id=?", &[json!(c)])? {
            row["id"] = json!(id());
            row["conv_id"] = out["id"].clone();
            row["first_interaction_id"] = Value::Null;
            row["created_at"] = json!(now());
            row["updated_at"] = json!(now());
            self.insert_row("characters", &row)?;
        }
        Ok(out)
    }
    pub(super) fn delete_conv(&self, c: &str) -> Result<Value> {
        self.conv(c, false)?;
        self.ensure_idle(c)?;
        let time = now();
        self.exec(
            "UPDATE conversations SET deleted_at=?,deleted_batch=? WHERE id=?",
            &[json!(time), json!(id()), json!(c)],
        )?;
        self.exec("UPDATE jobs SET status='skipped',updated_at=? WHERE conv_id=? AND status IN ('pending','blocked')",&[json!(time),json!(c)])?;
        Ok(json!({"ok":true,"deletedAt":time}))
    }
    pub(super) fn restore_conv(&self, c: &str) -> Result<Value> {
        self.conv(c, true)?;
        self.exec(
            "UPDATE conversations SET deleted_at=NULL,deleted_batch=NULL WHERE id=?",
            &[json!(c)],
        )?;
        self.bump(c)?;
        self.conversation(c, false)
    }
    pub(super) fn clear(&self, c: &str) -> Result<Value> {
        self.conv(c, false)?;
        self.ensure_idle(c)?;
        let batch = id();
        self.exec("UPDATE interactions SET deleted_at=?,deleted_batch=? WHERE conv_id=? AND deleted_at IS NULL",&[json!(now()),json!(batch),json!(c)])?;
        self.invalidate(c, None)?;
        Ok(json!({"ok":true,"batch":batch,"revision":self.bump(c)?}))
    }
    pub(super) fn trash(&self) -> Result<Value> {
        let mut out = Vec::new();
        // Each item carries a PREVIEW of the deleted content (to decide between restoring and deleting for good).
        let snippet = |t: &str| -> String {
            let flat = t.split_whitespace().collect::<Vec<_>>().join(" ");
            if flat.chars().count() > 140 {
                format!("{}…", flat.chars().take(139).collect::<String>())
            } else {
                flat
            }
        };
        for r in self.rows("SELECT id,name,deleted_at FROM conversations WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC",&[])?{
            let last = self.one("SELECT i.user_text,(SELECT v.content FROM parts p JOIN versions v ON v.id=p.selected_version_id WHERE p.interaction_id=i.id ORDER BY p.idx DESC LIMIT 1) reply FROM interactions i WHERE i.conv_id=? ORDER BY i.created_at DESC,i.rowid DESC LIMIT 1",&[r["id"].clone()])?;
            let count = self.one("SELECT count(*) n FROM interactions WHERE conv_id=?",&[r["id"].clone()])?;
            let text = last["reply"].as_str().filter(|t| !t.is_empty()).or(last["user_text"].as_str()).unwrap_or_default();
            out.push(json!({"type":"conversation","id":r["id"],"name":r["name"],"deletedAt":r["deleted_at"],"count":count["n"],"preview":snippet(text)}));
        }
        for (table, kind) in [("interactions", "interactions"), ("parts", "responses")] {
            let sql = if table == "interactions" {
                "SELECT i.conv_id,i.deleted_batch batch,min(i.deleted_at) deleted_at,count(*) n,c.name FROM interactions i JOIN conversations c ON c.id=i.conv_id WHERE i.deleted_at IS NOT NULL AND c.deleted_at IS NULL GROUP BY i.conv_id,i.deleted_batch"
            } else {
                "SELECT i.conv_id,p.deleted_batch batch,min(p.deleted_at) deleted_at,count(*) n,c.name FROM parts p JOIN interactions i ON i.id=p.interaction_id JOIN conversations c ON c.id=i.conv_id WHERE p.deleted_at IS NOT NULL AND i.deleted_at IS NULL AND c.deleted_at IS NULL GROUP BY i.conv_id,p.deleted_batch"
            };
            for r in self.rows(sql, &[])? {
                let found = if table == "interactions" {
                    self.one("SELECT i.user_text,(SELECT v.content FROM parts p JOIN versions v ON v.id=p.selected_version_id WHERE p.interaction_id=i.id ORDER BY p.idx LIMIT 1) reply FROM interactions i WHERE i.conv_id=? AND i.deleted_batch=? ORDER BY i.created_at ASC,i.rowid ASC LIMIT 1",&[r["conv_id"].clone(),r["batch"].clone()])?
                } else {
                    self.one("SELECT v.content reply FROM parts p JOIN interactions i ON i.id=p.interaction_id JOIN versions v ON v.id=p.selected_version_id WHERE i.conv_id=? AND p.deleted_batch=? ORDER BY p.deleted_at ASC,p.rowid ASC LIMIT 1",&[r["conv_id"].clone(),r["batch"].clone()])?
                };
                let text = if table == "interactions" {
                    found["user_text"]
                        .as_str()
                        .filter(|t| !t.is_empty())
                        .or(found["reply"].as_str())
                        .unwrap_or_default()
                } else {
                    found["reply"].as_str().unwrap_or_default()
                };
                out.push(json!({"type":kind,"convId":r["conv_id"],"batch":r["batch"],"count":r["n"],"name":r["name"],"deletedAt":r["deleted_at"],"preview":snippet(text)}));
            }
        }
        Ok(json!(out))
    }
    pub(super) fn restore_batch(&self, c: &str, batch: &str) -> Result<Value> {
        self.conv(c, false)?;
        self.ensure_idle(c)?;
        let a=self.exec("UPDATE interactions SET deleted_at=NULL,deleted_batch=NULL WHERE conv_id=? AND deleted_batch=?",&[json!(c),json!(batch)])?;
        let b=self.exec("UPDATE parts SET deleted_at=NULL,deleted_batch=NULL WHERE deleted_batch=? AND interaction_id IN (SELECT id FROM interactions WHERE conv_id=?)",&[json!(batch),json!(c)])?;
        if a + b == 0 {
            return Err(Error::new(404, "Item da lixeira não encontrado."));
        }
        self.invalidate(c, None)?;
        Ok(json!({"revision":self.bump(c)?}))
    }
    pub(super) fn delete_message(&self, c: &str, message: &str) -> Result<Value> {
        self.conv(c, false)?;
        self.ensure_idle(c)?;
        let batch = id();
        let i = self.one(
            "SELECT id FROM interactions WHERE id=? AND conv_id=? AND deleted_at IS NULL",
            &[json!(message), json!(c)],
        )?;
        if !i.is_null() {
            self.exec(
                "UPDATE interactions SET deleted_at=?,deleted_batch=? WHERE id=?",
                &[json!(now()), json!(batch), json!(message)],
            )?;
            self.invalidate(c, Some(message))?;
        } else {
            let part = self.part(c, message)?;
            if !part["deleted_at"].is_null() {
                return Err(Error::new(404, "Mensagem não encontrada."));
            }
            self.exec(
                "UPDATE parts SET deleted_at=?,deleted_batch=? WHERE id=?",
                &[json!(now()), json!(batch), json!(message)],
            )?;
            self.invalidate(c, Some(&s(&part, "interaction_id")))?;
        }
        Ok(json!({"revision":self.bump(c)?,"batch":batch}))
    }
    pub(super) fn branches(&self, c: &str) -> Result<Value> {
        self.conv(c, false)?;
        let mut out = Vec::new();
        for i in self.path(c)? {
            let sib=self.rows("SELECT * FROM interactions WHERE conv_id=? AND parent_id IS ? AND deleted_at IS NULL ORDER BY created_at,rowid",&[json!(c),i["parent_id"].clone()])?;
            if sib.len() > 1 {
                out.push(json!({"parentId":i["parent_id"],"seq":i["seq"],"siblings":sib.iter().map(|r|json!({"id":r["id"],"createdAt":r["created_at"],"active":r["id"]==i["id"],"preview":rules::clip(r["user_text"].as_str().unwrap_or_default(),120)})).collect::<Vec<_>>()}));
            }
        }
        Ok(json!(out))
    }
    pub(super) fn activate(&self, c: &str, interaction: &str) -> Result<Value> {
        self.conv(c, false)?;
        self.ensure_idle(c)?;
        let mut cur = self.one(
            "SELECT * FROM interactions WHERE id=? AND conv_id=? AND deleted_at IS NULL",
            &[json!(interaction), json!(c)],
        )?;
        if cur.is_null() {
            return Err(Error::new(404, "Ramo não encontrado."));
        }
        let mut visited = std::collections::HashSet::new();
        loop {
            if !visited.insert(s(&cur, "id")) {
                return Err(Error::new(409, "Ramo com ciclo inválido."));
            }
            let child=self.one("SELECT * FROM interactions WHERE parent_id=? AND conv_id=? AND deleted_at IS NULL ORDER BY created_at DESC,rowid DESC LIMIT 1",&[cur["id"].clone(),json!(c)])?;
            if child.is_null() {
                break;
            }
            cur = child;
        }
        self.exec(
            "UPDATE conversations SET head_interaction_id=? WHERE id=?",
            &[cur["id"].clone(), json!(c)],
        )?;
        self.invalidate(c, None)?;
        Ok(json!({"revision":self.bump(c)?,"headInteractionId":cur["id"]}))
    }
    pub(super) fn search(&self, text: &str, conv: Option<&str>, limit: usize) -> Result<Value> {
        let query = rules::fts(text, false);
        if query.is_empty() {
            return Ok(json!([]));
        }
        // Search over chunks (kind='chunk'); a chunk points to its owning message. A long message may match
        // several chunks, so the 400-row slack covers the cut of 200 distinct messages.
        let rows=self.rows("SELECT t.kind,t.owner_id,f.conv_id,snippet(search_fts,0,'«','»','…',14) snippet,bm25(search_fts) rank FROM search_fts f JOIN text_chunks t ON t.id=f.owner_id WHERE search_fts MATCH ? AND f.kind='chunk' AND t.kind IN ('user','asst') AND (? IS NULL OR f.conv_id=?) ORDER BY rank LIMIT 400",&[json!(query),json!(conv),json!(conv)])?;
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for r in rows {
            if !seen.insert((s(&r, "kind"), s(&r, "owner_id"))) {
                continue;
            }
            let c = s(&r, "conv_id");
            let Ok(cr) = self.conv(&c, false) else {
                continue;
            };
            let visible = if r["kind"] == "user" {
                self.one(&format!("{PATH_CTE} SELECT i.id message,i.user_ts ts FROM interactions i JOIN p ON p.id=i.id WHERE i.id=?2 AND i.deleted_at IS NULL"),&[json!(c),r["owner_id"].clone()])?
            } else {
                self.one(&format!("{PATH_CTE} SELECT p.id message,coalesce(v.finished_at,v.created_at) ts FROM versions v JOIN parts p ON p.id=v.part_id AND p.selected_version_id=v.id JOIN interactions i ON i.id=p.interaction_id JOIN p active ON active.id=i.id WHERE v.id=?2 AND p.deleted_at IS NULL AND i.deleted_at IS NULL"),&[json!(c),r["owner_id"].clone()])?
            };
            if visible.is_null() {
                continue;
            }
            out.push(json!({"convId":c,"convName":cr["name"],"archived":n(&cr,"archived")!=0,"messageId":visible["message"],"role":if r["kind"]=="user"{"user"}else{"assistant"},"snippet":r["snippet"],"ts":visible["ts"]}));
            if out.len() >= limit.clamp(1, 200) {
                break;
            }
        }
        Ok(json!(out))
    }
}
