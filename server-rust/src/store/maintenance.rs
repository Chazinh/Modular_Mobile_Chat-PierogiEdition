use super::{Database, PATH_CTE};
use crate::{Error, Result, n, parsed, rules, s};
use serde_json::{Value, json};
/// Retention of the automatic pruning of operational records.
const LOG_RETENTION_DAYS: i64 = 14;
impl Database {
    pub(super) fn seed_personas(&self) -> Result<Value> {
        let rows = self.rows(
            "SELECT * FROM conversations WHERE deleted_at IS NULL ORDER BY created_at",
            &[],
        )?;
        let mut created = 0;
        for row in rows {
            let name = s(&row, "persona");
            let settings = parsed(&row, "settings_json", json!({}));
            if settings["personaId"].is_string() {
                continue;
            }
            let existing = self
                .personas()?
                .as_array()
                .unwrap()
                .iter()
                .find(|p| rules::key(&s(p, "name")) == rules::key(&name))
                .cloned();
            let p = if let Some(p) = existing {
                p
            } else {
                let c = s(&row, "id");
                let cast=self.one("SELECT description,avatar FROM characters WHERE conv_id=? AND kind='user' ORDER BY active DESC LIMIT 1",&[json!(c)])?;
                created += 1;
                self.save_persona(&json!({"name":name,"description":cast["description"].as_str().or(settings["personaNotes"].as_str()).unwrap_or_default(),"avatar":cast["avatar"]}))?
            };
            self.patch(
                &s(&row, "id"),
                &json!({"settings":{"personaId":p["id"]}}),
                None,
            )?;
        }
        Ok(json!(created))
    }
    pub(super) fn replace_cast(
        &self,
        c: &str,
        list: &[Value],
        persona: Option<&Value>,
    ) -> Result<Value> {
        self.conv(c, false)?;
        let keys = list
            .iter()
            .map(|p| rules::key(&s(p, "name")))
            .collect::<std::collections::HashSet<_>>();
        let mut removed = 0;
        for row in self.rows("SELECT id,name_key FROM characters WHERE conv_id=? AND source='card' AND confirmed=0 AND kind!='user'",&[json!(c)])?{
            if !keys.contains(&s(&row,"name_key")){removed+=self.exec("DELETE FROM characters WHERE id=?",&[row["id"].clone()])?;}
        }
        for value in list {
            let mut value = value.clone();
            value["source"] = json!("card");
            self.save_cast(c, &value)?;
        }
        if let Some(persona) = persona {
            self.sync_user_cast(c, &s(persona, "name"), &s(persona, "description"))?;
        }
        Ok(json!({"removed":removed}))
    }
    pub(super) fn scene_history(
        &self,
        c: &str,
        entity: Option<&str>,
        limit: usize,
    ) -> Result<Value> {
        self.conv(c, false)?;
        let rows = if let Some(name) = entity {
            self.rows(&format!("{PATH_CTE} SELECT s.* FROM character_states s JOIN p ON p.id=s.interaction_id WHERE s.conv_id=?1 AND s.entity_key=?2 ORDER BY seq DESC,created_at DESC LIMIT ?3"),&[json!(c),json!(rules::key(name)),json!(limit)])?
        } else {
            self.rows(&format!("{PATH_CTE} SELECT s.* FROM scene_states s JOIN p ON p.id=s.interaction_id ORDER BY seq DESC,created_at DESC LIMIT ?2"),&[json!(c),json!(limit)])?
        };
        Ok(json!(rows))
    }
    /// Automatic pruning (on open and every 24 h of process uptime). 14-day retention, only for operational
    /// records that are rebuilt or have no reader after that: finished jobs, idempotent operations,
    /// `request_json`/`context_json` of finished generations and resolved flags.
    /// Never touches messages, versions, memories, notes, episodes or sources. Idempotent.
    pub(super) fn auto_prune(&self) -> Result<Value> {
        self.prune_logs(&json!({"days":LOG_RETENTION_DAYS}))
    }
    pub(super) fn prune_logs(&self, p: &Value) -> Result<Value> {
        let current = p["now"]
            .as_i64()
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
        let days = p["days"].as_i64().unwrap_or(7).clamp(1, 3650);
        let cutoff = chrono::DateTime::from_timestamp_millis(current - days * 86400000)
            .ok_or_else(|| Error::new(400, "Data inválida."))?
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let jobs = self.exec(
            "DELETE FROM jobs WHERE status IN ('done','skipped') AND updated_at<?",
            &[json!(cutoff)],
        )?;
        let operations = self.exec(
            "DELETE FROM operations WHERE created_at<?",
            &[json!(cutoff)],
        )?;
        let generations=self.exec("UPDATE generations SET request_json=NULL,context_json=NULL WHERE started_at<? AND status!='running' AND (request_json IS NOT NULL OR context_json IS NOT NULL)",&[json!(cutoff)])?;
        let flags = self.exec(
            "DELETE FROM review_flags WHERE status!='open' AND resolved_at<?",
            &[json!(cutoff)],
        )?;
        Ok(
            json!({"jobs":jobs,"operations":operations,"generationsTrimmed":generations,"flags":flags}),
        )
    }
    fn purge_part(&self, id: &str) -> Result<()> {
        for v in self.rows("SELECT id FROM versions WHERE part_id=?", &[json!(id)])? {
            let vid = s(&v, "id");
            self.unindex("asst", &vid)?;
            self.exec("DELETE FROM review_flags WHERE version_id=?", &[json!(vid)])?;
        }
        self.exec("DELETE FROM versions WHERE part_id=?", &[json!(id)])?;
        self.exec("DELETE FROM parts WHERE id=?", &[json!(id)])?;
        Ok(())
    }
    fn purge_interaction(&self, id: &str) -> Result<()> {
        for part in self.rows("SELECT id FROM parts WHERE interaction_id=?", &[json!(id)])? {
            self.purge_part(&s(&part, "id"))?;
        }
        self.delete_derived(id)?;
        self.unindex("user", id)?;
        self.exec("DELETE FROM interactions WHERE id=?", &[json!(id)])?;
        Ok(())
    }
    /// GLOBAL settings (apply to every conversation): botChars 500–8000, dialoguePct 0–100, content ≤1500.
    pub fn normalize_global_settings(raw: &Value) -> Value {
        let int = |v: &Value, min: i64, max: i64, default: i64| -> i64 {
            let n = v
                .as_f64()
                .or_else(|| v.as_str().and_then(|t| t.trim().parse::<f64>().ok()));
            match n {
                Some(x) if x.is_finite() => (x.round() as i64).clamp(min, max),
                _ => default,
            }
        };
        let content = raw["content"]
            .as_str()
            .map(|t| rules::clip(t.replace("\r\n", "\n").trim(), 1500).to_owned())
            .unwrap_or_default();
        json!({
            "botChars": int(&raw["botChars"], 500, rules::BOT_LIMIT as i64, rules::BOT_LIMIT as i64),
            "dialoguePct": int(&raw["dialoguePct"], 0, 100, 65),
            "lengthMode": if raw["lengthMode"] == "target" { "target" } else { "maximum" },
            "monologueChars": match int(&raw["monologueChars"], -1, 2000, 300) { n if n < 0 => 300, 0 => 0, n => n.max(100) },
            "turnPacing": match raw["turnPacing"].as_str() { Some("balanced") => "balanced", Some("free") => "free", _ => "interactive" },
            "content": content,
        })
    }
    pub(super) fn global_settings(&self) -> Result<Value> {
        let row = self.one("SELECT value FROM meta WHERE key='global_settings'", &[])?;
        let raw = row["value"]
            .as_str()
            .and_then(|t| serde_json::from_str::<Value>(t).ok())
            .unwrap_or_else(|| json!({}));
        Ok(Self::normalize_global_settings(&raw))
    }
    pub(super) fn set_global_settings(&self, patch: &Value) -> Result<Value> {
        let mut merged = self.global_settings()?;
        if let Some(obj) = patch.as_object() {
            for (k, v) in obj {
                merged[k.as_str()] = v.clone();
            }
        }
        let next = Self::normalize_global_settings(&merged);
        self.exec(
            "INSERT OR REPLACE INTO meta(key,value) VALUES('global_settings',?)",
            &[json!(next.to_string())],
        )?;
        Ok(next)
    }
    /// Permanently deletes every row of a conversation (already in the trash).
    fn purge_conversation_rows(&self, c: &str) -> Result<()> {
        self.ensure_idle(c)?;
        for inter in self.rows("SELECT id FROM interactions WHERE conv_id=?", &[json!(c)])? {
            self.purge_interaction(&s(&inter, "id"))?;
        }
        for table in [
            "character_revisions",
            "generations",
            "operations",
            "characters",
            "memories",
            "scene_states",
            "character_states",
            "relationships",
            "threads",
            "interaction_notes",
            "episodes",
            "embeddings",
            "jobs",
            "review_flags",
            "knowledge_events",
            "memory_replacements",
            "text_chunks",
            "narrative_snapshots",
        ] {
            self.exec(&format!("DELETE FROM {table} WHERE conv_id=?"), &[json!(c)])?;
        }
        self.exec("DELETE FROM search_fts WHERE conv_id=?", &[json!(c)])?;
        self.exec("DELETE FROM conversations WHERE id=?", &[json!(c)])?;
        Ok(())
    }
    fn purge_interaction_detached(&self, i: &Value) -> Result<()> {
        self.exec(
            "UPDATE interactions SET parent_id=? WHERE parent_id=?",
            &[i["parent_id"].clone(), i["id"].clone()],
        )?;
        self.exec(
            "UPDATE conversations SET head_interaction_id=? WHERE head_interaction_id=?",
            &[i["parent_id"].clone(), i["id"].clone()],
        )?;
        self.purge_interaction(&s(i, "id"))
    }
    /// PERMANENTLY deletes trash items (one item or everything). Only touches rows that are already deleted.
    pub(super) fn purge_trash(&self, p: &Value) -> Result<Value> {
        let (mut conversations, mut interactions, mut parts) = (0, 0, 0);
        if p["all"] == true {
            for conv in self.rows(
                "SELECT id FROM conversations WHERE deleted_at IS NOT NULL",
                &[],
            )? {
                self.purge_conversation_rows(&s(&conv, "id"))?;
                conversations += 1;
            }
            for i in self.rows(
                "SELECT id,conv_id,parent_id FROM interactions WHERE deleted_at IS NOT NULL",
                &[],
            )? {
                self.ensure_idle(&s(&i, "conv_id"))?;
                self.purge_interaction_detached(&i)?;
                interactions += 1;
            }
            for part in self.rows(
                "SELECT p.id,i.conv_id FROM parts p JOIN interactions i ON i.id=p.interaction_id WHERE p.deleted_at IS NOT NULL",
                &[],
            )? {
                self.ensure_idle(&s(&part, "conv_id"))?;
                self.purge_part(&s(&part, "id"))?;
                parts += 1;
            }
        } else {
            let (kind, id, conv, batch) = (s(p, "type"), s(p, "id"), s(p, "convId"), s(p, "batch"));
            match kind.as_str() {
                "conversation" => {
                    let row = self.conv(&id, true)?;
                    if row["deleted_at"].is_null() {
                        return Err(Error::new(400, "Esta conversa não está na lixeira."));
                    }
                    self.purge_conversation_rows(&id)?;
                    conversations += 1;
                }
                "interactions" | "responses" => {
                    self.conv(&conv, false)?;
                    if batch.is_empty() {
                        return Err(Error::new(400, "Lote inválido."));
                    }
                    self.ensure_idle(&conv)?;
                    if kind == "interactions" {
                        for i in self.rows(
                            "SELECT id,conv_id,parent_id FROM interactions WHERE conv_id=? AND deleted_batch=? AND deleted_at IS NOT NULL",
                            &[json!(conv), json!(batch)],
                        )? {
                            self.purge_interaction_detached(&i)?;
                            interactions += 1;
                        }
                    } else {
                        for part in self.rows(
                            "SELECT p.id FROM parts p JOIN interactions i ON i.id=p.interaction_id WHERE i.conv_id=? AND p.deleted_batch=? AND p.deleted_at IS NOT NULL",
                            &[json!(conv), json!(batch)],
                        )? {
                            self.purge_part(&s(&part, "id"))?;
                            parts += 1;
                        }
                    }
                }
                _ => return Err(Error::new(400, "Item da lixeira inválido.")),
            }
        }
        Ok(json!({"conversations":conversations,"interactions":interactions,"parts":parts}))
    }
    pub(super) fn purge_expired(&self, p: &Value) -> Result<Value> {
        let current = p["now"]
            .as_i64()
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
        let days = p["days"].as_i64().unwrap_or(30).clamp(1, 3650);
        let cutoff = chrono::DateTime::from_timestamp_millis(current - days * 86400000)
            .ok_or_else(|| Error::new(400, "Data inválida."))?
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let mut conversations = 0;
        for conv in self.rows(
            "SELECT id FROM conversations WHERE deleted_at IS NOT NULL AND deleted_at<?",
            &[json!(cutoff)],
        )? {
            self.purge_conversation_rows(&s(&conv, "id"))?;
            conversations += 1;
        }
        let expired=self.rows("SELECT id,conv_id,parent_id FROM interactions WHERE deleted_at IS NOT NULL AND deleted_at<?",&[json!(cutoff)])?;
        for i in &expired {
            self.ensure_idle(&s(i, "conv_id"))?;
            self.exec(
                "UPDATE interactions SET parent_id=? WHERE parent_id=?",
                &[i["parent_id"].clone(), i["id"].clone()],
            )?;
            self.exec(
                "UPDATE conversations SET head_interaction_id=? WHERE head_interaction_id=?",
                &[i["parent_id"].clone(), i["id"].clone()],
            )?;
            self.purge_interaction(&s(i, "id"))?;
        }
        let parts=self.rows("SELECT p.id,i.conv_id FROM parts p JOIN interactions i ON i.id=p.interaction_id WHERE p.deleted_at IS NOT NULL AND p.deleted_at<?",&[json!(cutoff)])?;
        for p in &parts {
            self.ensure_idle(&s(p, "conv_id"))?;
            self.purge_part(&s(p, "id"))?;
        }
        Ok(json!({"conversations":conversations,"interactions":expired.len(),"parts":parts.len()}))
    }
    pub(super) fn summary(&self, c: &str) -> Result<Value> {
        let rows = self.rows(
            "SELECT status,count(*) n FROM jobs WHERE conv_id=? GROUP BY status",
            &[json!(c)],
        )?;
        let mut by = std::collections::HashMap::new();
        for row in rows {
            by.insert(s(&row, "status"), n(&row, "n"));
        }
        Ok(
            json!({"pending":by.get("pending").unwrap_or(&0)+by.get("running").unwrap_or(&0),"failed":by.get("failed").unwrap_or(&0),"blocked":by.get("blocked").unwrap_or(&0),
            "flags":n(&self.one("SELECT count(*) n FROM review_flags WHERE conv_id=? AND status='open'",&[json!(c)])?,"n"),"sync":self.sync_lag(c)?}),
        )
    }
}
