use super::Database;
use crate::{Error, Result, id, n, now, parsed, rules, s};
use serde_json::{Value, json};
use std::collections::HashSet;

impl Database {
    pub(super) fn personas(&self) -> Result<Value> {
        let rows = self.rows("SELECT * FROM personas ORDER BY is_default DESC,name", &[])?;
        Ok(json!(rows.iter().map(|r|json!({"id":r["id"],"name":r["name"],"description":r["description"],"avatar":r["avatar"],"isDefault":n(r,"is_default")!=0,"createdAt":r["created_at"],"updatedAt":r["updated_at"]})).collect::<Vec<_>>()))
    }
    pub(super) fn save_persona(&self, p: &Value) -> Result<Value> {
        let name = rules::clip(p["name"].as_str().unwrap_or_default().trim(), 60);
        if name.is_empty() {
            return Err(Error::new(400, "Informe o nome da persona."));
        }
        let pid = p["id"].as_str().map(str::to_owned).unwrap_or_else(id);
        let old = self.one("SELECT * FROM personas WHERE id=?", &[json!(pid)])?;
        if p["id"].is_string() && old.is_null() {
            return Err(Error::new(404, "Persona não encontrada."));
        }
        let description = p["description"]
            .as_str()
            .unwrap_or(old["description"].as_str().unwrap_or_default());
        let avatar = p.get("avatar").unwrap_or(&old["avatar"]);
        if avatar
            .as_str()
            .is_some_and(|s| rules::utf16_len(s) > 400000)
        {
            return Err(Error::new(422, "Avatar grande demais."));
        }
        let default = p["isDefault"]
            .as_bool()
            .unwrap_or(n(&old, "is_default") != 0)
            || self.one("SELECT id FROM personas LIMIT 1", &[])?.is_null();
        if default {
            self.exec("UPDATE personas SET is_default=0", &[])?;
        }
        self.exec("INSERT INTO personas(id,name,description,avatar,is_default,created_at,updated_at) VALUES(?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,description=excluded.description,avatar=excluded.avatar,is_default=excluded.is_default,updated_at=excluded.updated_at",&[json!(pid),json!(name),json!(rules::clip(description,20000)),avatar.clone(),json!(default),json!(old["created_at"].as_str().unwrap_or(&now())),json!(now())])?;
        for conv in self.rows("SELECT id FROM conversations WHERE deleted_at IS NULL AND json_extract(settings_json,'$.personaId')=?",&[json!(pid)])?{
            let c=s(&conv,"id");self.exec("UPDATE conversations SET persona=? WHERE id=?",&[json!(name),json!(c)])?;
            self.sync_user_cast(&c,name,description)?;self.bump(&c)?;
        }
        Ok(self
            .personas()?
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == pid)
            .unwrap()
            .clone())
    }
    pub(super) fn delete_persona(&self, pid: &str) -> Result<Value> {
        if self
            .one("SELECT id FROM personas WHERE id=?", &[json!(pid)])?
            .is_null()
        {
            return Err(Error::new(404, "Persona não encontrada."));
        }
        if !self.one("SELECT id FROM conversations WHERE deleted_at IS NULL AND json_extract(settings_json,'$.personaId')=?",&[json!(pid)])?.is_null(){return Err(Error::new(409,"Persona ainda usada em uma conversa."));}
        self.exec("DELETE FROM personas WHERE id=?", &[json!(pid)])?;
        if self
            .one("SELECT id FROM personas WHERE is_default=1", &[])?
            .is_null()
        {
            self.exec("UPDATE personas SET is_default=1 WHERE id=(SELECT id FROM personas ORDER BY created_at LIMIT 1)",&[])?;
        }
        Ok(json!({"ok":true}))
    }
    pub(super) fn cast(&self, c: &str) -> Result<Value> {
        self.conv(c, false)?;
        Ok(json!(self.rows("SELECT * FROM characters WHERE conv_id=? ORDER BY (kind='user') DESC,confirmed DESC,name",&[json!(c)])?.iter().map(cast_view).collect::<Vec<_>>()))
    }
    pub(super) fn resolve_cast(&self, c: &str, name: &str) -> Result<Value> {
        let key = rules::key(name);
        if key.is_empty() {
            return Ok(Value::Null);
        }
        let rows = self.rows("SELECT * FROM characters WHERE conv_id=?", &[json!(c)])?;
        Ok(rows
            .iter()
            .find(|r| {
                r["name_key"] == key
                    || parsed(r, "aliases_json", json!([]))
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|a| rules::key(a.as_str().unwrap_or_default()) == key)
            })
            .or_else(|| {
                rows.iter().find(|r| {
                    s(r, "name_key").starts_with(&format!("{key} "))
                        || key.starts_with(&format!("{} ", s(r, "name_key")))
                })
            })
            .cloned()
            .unwrap_or(Value::Null))
    }
    pub(super) fn save_cast(&self, c: &str, p: &Value) -> Result<Value> {
        self.conv(c, false)?;
        let fallback = if p["id"].is_string() {
            self.one(
                "SELECT name FROM characters WHERE conv_id=? AND id=?",
                &[json!(c), p["id"].clone()],
            )?
        } else {
            Value::Null
        };
        let name = rules::clip(
            p["name"]
                .as_str()
                .or(fallback["name"].as_str())
                .unwrap_or_default()
                .trim(),
            60,
        );
        if name.is_empty() {
            return Err(Error::new(400, "Informe o nome."));
        }
        let old = if let Some(cid) = p["id"].as_str() {
            let r = self.one(
                "SELECT * FROM characters WHERE conv_id=? AND id=?",
                &[json!(c), json!(cid)],
            )?;
            if r.is_null() {
                return Err(Error::new(404, "Personagem não encontrado."));
            }
            r
        } else {
            self.resolve_cast(c, name)?
        };
        let cid = old["id"].as_str().map(str::to_owned).unwrap_or_else(id);
        let key = rules::key(name);
        let clash = self.one(
            "SELECT id FROM characters WHERE conv_id=? AND name_key=? AND id!=?",
            &[json!(c), json!(key), json!(cid)],
        )?;
        if !clash.is_null() {
            return Err(Error::new(409, "Já existe personagem com esse nome."));
        }
        let kind = if old["kind"] == "user" {
            "user"
        } else {
            p["kind"]
                .as_str()
                .filter(|k| ["main", "npc", "user"].contains(k))
                .unwrap_or(old["kind"].as_str().unwrap_or("npc"))
        };
        let description =
            if n(&old, "confirmed") != 0 && p["source"].as_str().is_some_and(|s| s != "manual") {
                s(&old, "description")
            } else {
                p["description"]
                    .as_str()
                    .unwrap_or(old["description"].as_str().unwrap_or_default())
                    .to_owned()
            };
        let mut aliases = parsed(&old, "aliases_json", json!([]))
            .as_array()
            .cloned()
            .unwrap_or_default();
        if let Some(a) = p["aliases"].as_array() {
            if p["id"].is_string() {
                aliases = a.clone();
            } else {
                aliases.extend(a.iter().cloned());
            }
        }
        if !old.is_null() && old["name_key"] != key && !p["id"].is_string() {
            aliases.push(json!(name));
        }
        let mut seen = HashSet::new();
        aliases = aliases
            .into_iter()
            .filter_map(|a| a.as_str().map(|a| rules::clip(a.trim(), 60).to_owned()))
            .filter(|a| !a.is_empty() && rules::key(a) != key && seen.insert(rules::key(a)))
            .take(12)
            .map(Value::String)
            .collect();
        let avatar = p.get("avatar").unwrap_or(&old["avatar"]);
        if avatar
            .as_str()
            .is_some_and(|s| rules::utf16_len(s) > 400000)
        {
            return Err(Error::new(422, "Avatar grande demais."));
        }
        self.exec("INSERT INTO characters(id,conv_id,name,name_key,aliases_json,description,kind,avatar,source,confirmed,active,first_interaction_id,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,name_key=excluded.name_key,aliases_json=excluded.aliases_json,description=excluded.description,kind=excluded.kind,avatar=excluded.avatar,confirmed=excluded.confirmed,active=excluded.active,updated_at=excluded.updated_at",&[
            json!(cid),json!(c),json!(if !old.is_null()&&!p["id"].is_string(){s(&old,"name")}else{name.to_owned()}),json!(if !old.is_null()&&!p["id"].is_string(){s(&old,"name_key")}else{key}),json!(json!(aliases).to_string()),json!(rules::clip(&description,600)),json!(kind),avatar.clone(),json!(p["source"].as_str().unwrap_or("manual")),json!(p["confirmed"].as_bool().unwrap_or(n(&old,"confirmed")!=0)),json!(p["active"].as_bool().unwrap_or(old.is_null()||n(&old,"active")!=0)),p["firstInteractionId"].clone(),json!(old["created_at"].as_str().unwrap_or(&now())),json!(now())])?;
        Ok(cast_view(&self.one(
            "SELECT * FROM characters WHERE id=?",
            &[json!(cid)],
        )?))
    }
    pub(super) fn delete_cast(&self, c: &str, cid: &str) -> Result<Value> {
        let r = self.one(
            "SELECT * FROM characters WHERE conv_id=? AND id=?",
            &[json!(c), json!(cid)],
        )?;
        if r.is_null() {
            return Err(Error::new(404, "Personagem não encontrado."));
        }
        if r["kind"] == "user" {
            return Err(Error::new(400, "A persona não pode ser removida."));
        }
        self.exec("DELETE FROM characters WHERE id=?", &[json!(cid)])?;
        Ok(json!({"ok":true}))
    }
    pub(super) fn merge_cast(&self, c: &str, from: &str, into: &str) -> Result<Value> {
        let a = self.one(
            "SELECT * FROM characters WHERE conv_id=? AND id=?",
            &[json!(c), json!(from)],
        )?;
        let b = self.one(
            "SELECT * FROM characters WHERE conv_id=? AND id=?",
            &[json!(c), json!(into)],
        )?;
        if a.is_null() || b.is_null() || from == into {
            return Err(Error::new(404, "Personagem não encontrado."));
        }
        if a["kind"] == "user" || b["kind"] == "user" {
            return Err(Error::new(400, "A persona não pode ser mesclada."));
        }
        let mut aliases = parsed(&b, "aliases_json", json!([]))
            .as_array()
            .cloned()
            .unwrap_or_default();
        aliases.extend(
            parsed(&a, "aliases_json", json!([]))
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
        aliases.push(a["name"].clone());
        self.exec("UPDATE knowledge_events SET entity_key=?,entity_name=? WHERE conv_id=? AND entity_key=?",&[b["name_key"].clone(),b["name"].clone(),json!(c),a["name_key"].clone()])?;
        self.exec("DELETE FROM memory_knowledge WHERE entity_key=? AND memory_id IN (SELECT memory_id FROM memory_knowledge WHERE entity_key=? AND memory_id IN (SELECT id FROM memories WHERE conv_id=?))",&[a["name_key"].clone(),b["name_key"].clone(),json!(c)])?;
        self.exec("UPDATE memory_knowledge SET entity_key=?,entity_name=? WHERE entity_key=? AND memory_id IN (SELECT id FROM memories WHERE conv_id=?)",&[b["name_key"].clone(),b["name"].clone(),a["name_key"].clone(),json!(c)])?;
        self.exec("DELETE FROM characters WHERE id=?", &[json!(from)])?;
        self.save_cast(c,&json!({"id":into,"name":b["name"],"aliases":aliases,"description":if s(&b,"description").is_empty(){a["description"].clone()}else{b["description"].clone()}}))
    }
    pub(super) fn sync_user_cast(&self, c: &str, name: &str, description: &str) -> Result<()> {
        self.exec(
            "UPDATE characters SET active=0 WHERE conv_id=? AND kind='user' AND name_key!=?",
            &[json!(c), json!(rules::key(name))],
        )?;
        self.save_cast(c,&json!({"name":name,"description":description,"kind":"user","source":"manual","confirmed":true,"active":true}))?;
        Ok(())
    }
    pub(super) fn reanalyze_cast(&self, c: &str) -> Result<Value> {
        let conv = self.conversation(c, false)?;
        let persona = s(&conv, "persona");
        self.exec(
            "DELETE FROM characters WHERE conv_id=? AND source='card' AND confirmed=0",
            &[json!(c)],
        )?;
        for mut entry in crate::local::cast_from_card(&s(&conv, "prompt"), &persona) {
            entry["source"] = json!("card");
            self.save_cast(c, &entry)?;
        }
        self.sync_user_cast(
            c,
            &persona,
            conv["personaProfile"]["description"]
                .as_str()
                .unwrap_or_default(),
        )?;
        Ok(json!({"cast":self.cast(c)?}))
    }
    pub(super) fn add_device(&self, name: &str, token_hash: &str) -> Result<Value> {
        let did = id();
        self.exec(
            "INSERT INTO devices(id,name,token_hash,created_at) VALUES(?,?,?,?)",
            &[
                json!(did),
                json!(rules::clip(name, 60)),
                json!(token_hash),
                json!(now()),
            ],
        )?;
        Ok(json!({"id":did}))
    }
    pub(super) fn find_device(&self, hash: &str) -> Result<Value> {
        let device = self.one(
            "SELECT id,name FROM devices WHERE token_hash=? AND revoked_at IS NULL",
            &[json!(hash)],
        )?;
        if !device.is_null() {
            // Only renews after 5 min: every authenticated request must not rewrite the whole vault.
            let stale = (chrono::Utc::now() - chrono::Duration::minutes(5))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            self.exec(
                "UPDATE devices SET last_seen_at=? WHERE id=? AND (last_seen_at IS NULL OR last_seen_at<?)",
                &[json!(now()), device["id"].clone(), json!(stale)],
            )?;
        }
        Ok(device)
    }
    pub(super) fn devices(&self) -> Result<Value> {
        Ok(json!(self.rows("SELECT id,name,created_at,last_seen_at,revoked_at FROM devices ORDER BY created_at DESC",&[])?))
    }
    pub(super) fn provider_check(&self, gid: &str, check: &Value) -> Result<Value> {
        let g = self.one("SELECT * FROM generations WHERE id=?", &[json!(gid)])?;
        if g.is_null() {
            return Ok(Value::Null);
        }
        let mut usage = parsed(&g, "usage_json", json!({}));
        usage["provider"] = check.clone();
        self.exec(
            "UPDATE generations SET usage_json=? WHERE id=?",
            &[json!(usage.to_string()), json!(gid)],
        )?;
        self.exec(
            "UPDATE versions SET usage_json=? WHERE id=?",
            &[json!(usage.to_string()), g["version_id"].clone()],
        )?;
        Ok(json!({"convId":g["conv_id"],"revision":self.bump(&s(&g,"conv_id"))?}))
    }
}
fn cast_view(r: &Value) -> Value {
    json!({"id":r["id"],"name":r["name"],"aliases":parsed(r,"aliases_json",json!([])),"description":r["description"],"kind":r["kind"],"avatar":r["avatar"],"source":r["source"],"confirmed":n(r,"confirmed")!=0,"active":n(r,"active")!=0,"createdAt":r["created_at"],"updatedAt":r["updated_at"]})
}
