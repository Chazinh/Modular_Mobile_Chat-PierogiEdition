//! Authenticated sync v1: bounded, full conversation snapshots, not a row journal.
//! All SQL identifiers below come from this module/schema, never from the client.
use super::{Database, SCHEMA_VERSION, transfer};
use crate::{Error, Result, n, parsed, rules, s};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_CONVERSATIONS: usize = 4;
const REMOVED_TABLES: &[&str] = &["interactions", "parts", "versions", "memories"];
const MANIFEST_PAGE: usize = 128;
const MAX_KNOWN: usize = 256;
const MAX_TEXT: usize = 4 * 1024 * 1024;
const TABLES: &[(&str, &[&str])] = &[
    ("character_revisions", &["conv_id", "rev"]),
    ("interactions", &["id"]),
    ("parts", &["id"]),
    ("versions", &["id"]),
    ("characters", &["id"]),
    ("memories", &["id"]),
    ("memory_knowledge", &["memory_id", "entity_key"]),
    ("scene_states", &["id"]),
    ("character_states", &["id"]),
    ("relationships", &["id"]),
    ("threads", &["id"]),
    ("interaction_notes", &["interaction_id"]),
    ("episodes", &["id"]),
    ("review_flags", &["id"]),
    ("knowledge_events", &["id"]),
    ("memory_replacements", &["old_id", "new_id"]),
    ("jobs", &["id"]),
    ("operations", &["conv_id", "op_id"]),
    ("generations", &["id"]),
    ("personas", &["id"]),
    ("embeddings", &["owner_type", "owner_id"]),
];
const SETTINGS: &[&str] = &[
    "personaId",
    "personaNotes",
    "thinkingEnabled",
    "botChars",
    "dialoguePct",
    "lengthMode",
    "monologueChars",
    "turnPacing",
    "content",
    "minPairs",
    "maxPairs",
    "preferredPairs",
    "hardLimit",
    "softLimit",
    "targetTokens",
];
const PUBLIC_SETTINGS: &[&str] = &[
    "botChars",
    "dialoguePct",
    "lengthMode",
    "monologueChars",
    "turnPacing",
    "content",
];
const JOB_PAYLOAD: &[&str] = &[
    "skipReview",
    "key",
    "interactionIds",
    "fromSeq",
    "toSeq",
    "notes",
];
const MESSAGE_FIELDS: &[&str] = &[
    "id",
    "role",
    "content",
    "ts",
    "interactionId",
    "partIndex",
    "versionId",
    "versionCount",
    "status",
    "origin",
    "edited",
    "model",
    "presetId",
    "effort",
];

fn invalid(message: &str) -> Error {
    Error::new(400, message).extra("code", "INVALID_SYNC")
}
fn sized(conv: Option<&str>, bytes: usize) -> Error {
    Error::new(
        413,
        "Delta excede 8 MiB; reduza o lote. Uma conversa nunca é truncada.",
    )
    .extra("code", "SYNC_TOO_LARGE")
    .extra("maxBytes", MAX_BYTES)
    .extra("requiredBytes", bytes)
    .extra("convId", conv.map(json_string).unwrap_or(Value::Null))
}
fn json_string(s: &str) -> Value {
    json!(s)
}
fn object_keys(v: &Value, allowed: &[&str]) -> Result<()> {
    if !v
        .as_object()
        .is_some_and(|o| o.keys().all(|k| allowed.contains(&k.as_str())))
    {
        return Err(invalid("Campos/tabelas não permitidos no sync."));
    }
    Ok(())
}
fn identifier(v: &Value) -> Result<&str> {
    v.as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        })
        .ok_or_else(|| invalid("ID de sync inválido."))
}
fn uuid(v: &Value) -> Result<&str> {
    let s = v
        .as_str()
        .ok_or_else(|| invalid("peerId/batchId UUID obrigatório."))?;
    if uuid::Uuid::parse_str(s)
        .ok()
        .is_none_or(|id| id.is_nil() || id.to_string() != s)
    {
        return Err(invalid("peerId/batchId UUID canônico obrigatório."));
    }
    Ok(s)
}
fn hash(v: &Value) -> Result<&str> {
    v.as_str()
        .filter(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .ok_or_else(|| invalid("Hash SHA-256 inválido."))
}
fn pick(v: &Value, fields: &[&str]) -> Value {
    Value::Object(
        fields
            .iter()
            .filter_map(|k| v.get(*k).map(|v| ((*k).into(), v.clone())))
            .collect(),
    )
}
fn key(row: &Value, keys: &[&str]) -> String {
    Value::Array(keys.iter().map(|k| row[*k].clone()).collect()).to_string()
}
fn normalize(v: &mut Value) {
    match v {
        Value::Object(o) => o.values_mut().for_each(normalize),
        Value::Array(a) => a.iter_mut().for_each(normalize),
        Value::Number(num) => {
            if let Some(f) = num.as_f64()
                && f.fract() == 0.0
                && f.abs() < 9_007_199_254_740_992.0
            {
                *v = json!(f as i64);
            }
        }
        _ => {}
    }
}
fn canonical(v: &Value) -> Value {
    let mut v = v.clone();
    normalize(&mut v);
    for &(table, keys) in TABLES {
        if let Some(rows) = v.get_mut(table).and_then(Value::as_array_mut) {
            rows.sort_by_cached_key(|row| key(row, keys));
        }
    }
    if let Some(rows) = v.get_mut("removed").and_then(Value::as_array_mut) {
        rows.sort_by_cached_key(|r| key(r, &["table", "id"]));
    }
    v
}
/// Canonical SHA-256: object keys sorted, table rows sorted by PK, integral numbers normalized.
pub fn snapshot_hash(snapshot: &Value) -> String {
    rules::hash(&canonical(snapshot).to_string())
}
fn content_hash(snapshot: &Value) -> String {
    let mut core = snapshot.clone();
    if core["purged"] != true {
        core["embeddings"] = json!([]);
    }
    snapshot_hash(&core)
}
fn value_hash(v: &Value) -> String {
    let mut v = v.clone();
    normalize(&mut v);
    rules::hash(&v.to_string())
}
// A baseline needs row/field hashes, not another copy of every message and card.
// Checkpoints remain encrypted and retain the original snapshot hash for CAS.
fn checkpoint(snapshot: &Value) -> Value {
    if snapshot["purged"] == true {
        return snapshot.clone();
    }
    let mut out = pick(
        snapshot,
        &["full", "purged", "convId", "publicSettings", "removed"],
    );
    out["checkpointVersion"] = json!(1);
    out["snapshotHash"] = json!(snapshot_hash(snapshot));
    out["conversation"] = pick(
        &snapshot["conversation"],
        &["id", "deleted_at", "deleted_batch"],
    );
    out["conversation"]["_fieldHashes"] = Value::Object(
        snapshot["conversation"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), json!(value_hash(v))))
            .collect(),
    );
    for &(table, pk) in TABLES {
        out[table] = json!(
            snapshot[table]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    let mut row = pick(r, pk);
                    row["_rowHash"] = json!(value_hash(r));
                    row
                })
                .collect::<Vec<_>>()
        );
    }
    out
}
fn base_same(base: Option<&Value>, data: &Value) -> bool {
    base.is_some_and(|b| {
        if b["checkpointVersion"] == 1 {
            b["snapshotHash"] == snapshot_hash(data)
        } else {
            b == data
        }
    })
}
fn base_row_same(base: Option<&Value>, row: &Value) -> bool {
    base.is_some_and(|b| {
        if b.get("_rowHash").is_some() {
            b["_rowHash"] == value_hash(row)
        } else {
            b == row
        }
    })
}
fn base_field_same(base: Option<&Value>, k: &str, value: &Value) -> bool {
    base.is_some_and(|b| {
        if b.get("_fieldHashes").is_some() {
            b["_fieldHashes"][k] == value_hash(value)
        } else {
            b[k] == *value
        }
    })
}

/// GET adapters use exactly the same worker contract as POST (without pushes/batchId).
pub fn query(query: &HashMap<String, String>, delta: bool) -> Result<Value> {
    let allowed = if delta {
        &["peerId", "known", "convIds", "after", "limit"][..]
    } else {
        &["after", "limit"][..]
    };
    if query.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(invalid("Parâmetro de sync desconhecido."));
    }
    let mut out = json!({});
    for k in ["after", "limit"] {
        if let Some(v) = query.get(k) {
            out[k] = json!(
                v.parse::<u64>()
                    .map_err(|_| invalid("Cursor/limite inválido."))?
            );
        }
    }
    if delta {
        out["format"] = json!("modularchat-sync");
        out["version"] = json!(1);
        out["peerId"] = query.get("peerId").map(|s| json!(s)).unwrap_or(Value::Null);
        out["known"] = match query.get("known") {
            Some(v) => {
                serde_json::from_str(v).map_err(|_| invalid("known deve ser um array JSON."))?
            }
            None => json!([]),
        };
        if let Some(v) = query.get("convIds") {
            out["convIds"] =
                serde_json::from_str(v).map_err(|_| invalid("convIds deve ser array JSON."))?;
        }
    }
    Ok(out)
}

impl Database {
    pub(super) fn sync_audit(&self) -> Value {
        let rows = match self.rows("SELECT id FROM conversations ORDER BY id", &[]) {
            Ok(rows) => rows,
            Err(error) => return json!({"ok":false,"error":error.message}),
        };
        let mut checks = Vec::new();
        for row in rows {
            let c = s(&row, "id");
            let result = self.sync_snapshot(&c).and_then(|mut snapshot| {
                // Read-only audit simulates the runtime's memory pause. Never
                // rewrite the vault or wait for an active inference here.
                for row in snapshot["jobs"].as_array_mut().into_iter().flatten() {
                    if row["status"] == "running" { row["status"] = json!("pending"); }
                }
                self.sync_validate(&snapshot)
            });
            checks.push(match result {
                Ok(_) => json!({"convId":c,"ok":true}),
                Err(error) => {
                    let mut details = error.extra;
                    if details["targetTable"] == "relationships" {
                        let target = self.one("SELECT id,conv_id,source_interaction_id,superseded_by FROM relationships WHERE id=?", &[details["referenceId"].clone()]).unwrap_or(Value::Null);
                        let source = self.one("SELECT id,conv_id FROM interactions WHERE id=?", &[target["source_interaction_id"].clone()]).unwrap_or(Value::Null);
                        details["targetMetadata"] = target;
                        details["sourceMetadata"] = source;
                    }
                    json!({"convId":c,"ok":false,"error":error.message,"details":details})
                },
            });
        }
        json!({"ok":checks.iter().all(|row|row["ok"]==true),"conversations":checks})
    }
    pub(super) fn init_sync(&self) -> Result<()> {
        self.db.execute_batch("
            CREATE TABLE IF NOT EXISTS sync_clock(id INTEGER PRIMARY KEY CHECK(id=1),cursor INTEGER NOT NULL);
            INSERT OR IGNORE INTO sync_clock VALUES(1,0);
            CREATE TABLE IF NOT EXISTS sync_dirty(conv_id TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS sync_manifests(
                conv_id TEXT PRIMARY KEY,hash TEXT NOT NULL,revision INTEGER NOT NULL,
                cursor INTEGER NOT NULL,deleted INTEGER NOT NULL,purged INTEGER NOT NULL,content_hash TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS sync_cursor ON sync_manifests(cursor);
            CREATE TABLE IF NOT EXISTS sync_checkpoints(
                peer_id TEXT NOT NULL,conv_id TEXT NOT NULL,hash TEXT NOT NULL,snapshot_json TEXT NOT NULL,
                PRIMARY KEY(peer_id,conv_id,hash));
            CREATE TABLE IF NOT EXISTS sync_batches(
                peer_id TEXT NOT NULL,batch_id TEXT NOT NULL,request_hash TEXT NOT NULL,response_json TEXT NOT NULL,
                PRIMARY KEY(peer_id,batch_id));
            CREATE TABLE IF NOT EXISTS sync_tombstones(
                conv_id TEXT PRIMARY KEY,deleted_at TEXT NOT NULL,deleted_batch TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS sync_character_authority(conv_id TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS sync_removed(
                conv_id TEXT NOT NULL,table_name TEXT NOT NULL,row_id TEXT NOT NULL,
                deleted_at TEXT NOT NULL,deleted_batch TEXT NOT NULL,PRIMARY KEY(table_name,row_id));
            CREATE TABLE IF NOT EXISTS sync_retained(
                table_name TEXT NOT NULL,row_key TEXT NOT NULL,conv_id TEXT NOT NULL,row_json TEXT NOT NULL,
                PRIMARY KEY(table_name,row_key));
            CREATE TRIGGER IF NOT EXISTS sync_purge AFTER DELETE ON conversations BEGIN
                INSERT OR IGNORE INTO sync_tombstones VALUES(
                    OLD.id,coalesce(OLD.deleted_at,OLD.updated_at),coalesce(OLD.deleted_batch,OLD.id));
                DELETE FROM sync_retained WHERE conv_id=OLD.id;
                DELETE FROM sync_character_authority WHERE conv_id=OLD.id;
                DELETE FROM sync_removed WHERE conv_id=OLD.id;
                INSERT OR IGNORE INTO sync_dirty VALUES(OLD.id);
            END;
        ")?;
        if !self
            .rows("PRAGMA table_info(sync_manifests)", &[])?
            .iter()
            .any(|r| r["name"] == "content_hash")
        {
            self.db.execute_batch("ALTER TABLE sync_manifests ADD COLUMN content_hash TEXT NOT NULL DEFAULT '';
                INSERT INTO sync_dirty SELECT conv_id FROM sync_manifests WHERE true ON CONFLICT(conv_id) DO NOTHING;")?;
        }
        for (table, conv, when, batch) in [
            (
                "interactions",
                "OLD.conv_id",
                "coalesce(OLD.deleted_at,OLD.created_at)",
                "coalesce(OLD.deleted_batch,OLD.id)",
            ),
            (
                "parts",
                "(SELECT conv_id FROM interactions WHERE id=OLD.interaction_id)",
                "coalesce(OLD.deleted_at,OLD.created_at)",
                "coalesce(OLD.deleted_batch,OLD.id)",
            ),
            (
                "versions",
                "(SELECT i.conv_id FROM parts p JOIN interactions i ON i.id=p.interaction_id WHERE p.id=OLD.part_id)",
                "coalesce((SELECT deleted_at FROM parts WHERE id=OLD.part_id),OLD.finished_at,OLD.created_at)",
                "coalesce((SELECT deleted_batch FROM parts WHERE id=OLD.part_id),OLD.id)",
            ),
            (
                "memories",
                "OLD.conv_id",
                "coalesce(OLD.last_confirmed_at,OLD.created_at)",
                "OLD.id",
            ),
        ] {
            self.db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS sync_removed_{table} BEFORE DELETE ON {table} BEGIN
                INSERT INTO sync_removed SELECT {conv},'{table}',OLD.id,{when},{batch} WHERE {conv} IS NOT NULL
                ON CONFLICT(table_name,row_id) DO NOTHING; END;"))?;
        }
        self.exec(
            "INSERT OR IGNORE INTO meta(key,value) VALUES('sync_server_id',?)",
            &[json!(crate::id())],
        )?;
        // Triggers only mark affected conversations; unchanged status/delta never re-hash history.
        for table in std::iter::once("conversations").chain(TABLES.iter().map(|t| t.0)) {
            let watched = self.sync_columns(table)?;
            for (event, row) in [("INSERT", "NEW"), ("UPDATE", "NEW"), ("DELETE", "OLD")] {
                let when = if event == "UPDATE" {
                    let terms=watched.iter().filter(|k| table!="personas" || *k!="is_default").flat_map(|k|{
                        if table=="conversations" && k=="settings_json" {
                            SETTINGS.iter().map(|k|format!("json_extract(OLD.settings_json,'$.{k}') IS NOT json_extract(NEW.settings_json,'$.{k}')")).collect::<Vec<_>>()
                        } else {vec![format!("OLD.{k} IS NOT NEW.{k}")]}
                    }).collect::<Vec<_>>().join(" OR ");
                    format!("WHEN {terms}")
                } else {
                    String::new()
                };
                let conv = match table {
                    "conversations" => format!("{row}.id"),
                    "parts" => {
                        format!("(SELECT conv_id FROM interactions WHERE id={row}.interaction_id)")
                    }
                    "versions" => format!(
                        "(SELECT i.conv_id FROM parts p JOIN interactions i ON i.id=p.interaction_id WHERE p.id={row}.part_id)"
                    ),
                    "memory_knowledge" => {
                        format!("(SELECT conv_id FROM memories WHERE id={row}.memory_id)")
                    }
                    "personas" => {
                        self.db.execute_batch(&format!("DROP TRIGGER IF EXISTS sync_{table}_{event};
                            CREATE TRIGGER sync_{table}_{event} AFTER {event} ON {table} {when} BEGIN
                            INSERT INTO sync_dirty SELECT id FROM conversations WHERE json_extract(settings_json,'$.personaId')={row}.id
                            ON CONFLICT(conv_id) DO NOTHING; END;"))?;
                        continue;
                    }
                    _ => format!("{row}.conv_id"),
                };
                // OR IGNORE inherits the outer UPSERT's conflict policy inside a trigger;
                // an explicit UPSERT DO NOTHING keeps an already-dirty conversation idempotent.
                self.db.execute_batch(&format!("DROP TRIGGER IF EXISTS sync_{table}_{event};
                    CREATE TRIGGER sync_{table}_{event} AFTER {event} ON {table} {when} BEGIN
                    INSERT INTO sync_dirty SELECT {conv} WHERE {conv} IS NOT NULL ON CONFLICT(conv_id) DO NOTHING; END;"))?;
            }
        }
        for event in ["INSERT", "UPDATE", "DELETE"] {
            let row = if event == "DELETE" { "OLD" } else { "NEW" };
            self.db.execute_batch(&format!("DROP TRIGGER IF EXISTS sync_settings_{event};
                CREATE TRIGGER sync_settings_{event} AFTER {event} ON meta
                WHEN {row}.key='global_settings' BEGIN INSERT INTO sync_dirty SELECT id FROM conversations WHERE true ON CONFLICT(conv_id) DO NOTHING; END;"))?;
        }
        // Compact terminal receipts survive the existing 14-day operational pruning.
        // A conversation purge removes them; only its non-revivable tombstone survives.
        for (table, keys) in [
            ("jobs", &["id"][..]),
            ("operations", &["conv_id", "op_id"][..]),
            ("review_flags", &["id"][..]),
        ] {
            let cols = self.sync_columns(table)?;
            let object = cols
                .iter()
                .map(|c| format!("'{c}',OLD.{c}"))
                .collect::<Vec<_>>()
                .join(",");
            let pk = keys
                .iter()
                .map(|c| format!("OLD.{c}"))
                .collect::<Vec<_>>()
                .join(",");
            let when = match table {
                "jobs" => "WHEN OLD.status IN ('done','skipped')",
                "review_flags" => "WHEN OLD.status!='open'",
                _ => "",
            };
            self.db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS sync_retain_{table} BEFORE DELETE ON {table} {when} BEGIN
                INSERT OR REPLACE INTO sync_retained VALUES('{table}',json_array({pk}),OLD.conv_id,json_object({object})); END;"))?;
        }
        self.exec("INSERT OR IGNORE INTO sync_dirty SELECT id FROM conversations UNION SELECT conv_id FROM sync_tombstones", &[])?;
        self.sync_refresh()?;
        Ok(())
    }

    fn sync_columns(&self, table: &str) -> Result<Vec<String>> {
        Ok(self
            .rows(&format!("PRAGMA table_info({table})"), &[])?
            .iter()
            .map(|r| s(r, "name"))
            .filter(|k| match table {
                "generations" => {
                    !["request_json", "context_json", "usage_json", "error"].contains(&k.as_str())
                }
                "jobs" => k != "error",
                _ => true,
            })
            .collect())
    }
    fn sync_project(&self, table: &str, raw: &Value) -> Result<Value> {
        let mut row = raw.clone();
        for k in match table {
            "generations" => &["request_json", "context_json", "usage_json", "error"][..],
            "jobs" => &["error"][..],
            _ => &[],
        } {
            row.as_object_mut().unwrap().remove(*k);
        }
        if table == "conversations" {
            row["settings_json"] =
                json!(pick(&parsed(raw, "settings_json", json!({})), SETTINGS).to_string());
        }
        if table == "personas" {
            row["is_default"] = json!(0);
        }
        if table == "versions" {
            // No provider request/diagnostic JSON enters the wire format.
            let usage = parsed(raw, "usage_json", Value::Null);
            row["usage_json"] = if usage.is_null() {
                Value::Null
            } else {
                let mut picked = serde_json::Map::new();
                if let Some(obj) = usage.as_object() {
                    for &k in &["input", "output", "reasoning", "visibleEstimate", "reasoningHidden"] {
                        if let Some(v) = obj.get(k) {
                            if v.is_number() || v.is_boolean() || v.is_null() {
                                picked.insert(k.to_string(), v.clone());
                            }
                        }
                    }
                    if let Some(v) = obj.get("length") {
                        if v.is_number() || v.is_boolean() || v.is_null() {
                            picked.insert("length".to_string(), v.clone());
                        }
                    }
                }
                json!(Value::Object(picked).to_string())
            };
        }
        if table == "jobs" {
            let mut payload = pick(&parsed(raw, "payload_json", json!({})), JOB_PAYLOAD);
            if let Some(notes) = payload.get_mut("notes").and_then(Value::as_array_mut) {
                for note in notes {
                    *note = pick(note, &["interaction_id", "summary", "version_ids_json"]);
                }
            }
            row["payload_json"] = json!(payload.to_string());
        }
        if table == "operations" {
            let old = parsed(raw, "result_json", json!({}));
            let mut message = pick(&old["message"], MESSAGE_FIELDS);
            message["usage"] = Value::Null;
            message["flags"] = json!(
                old["message"]["flags"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|f| pick(f, &["id", "kind", "detail", "evidence", "source"]))
                    .collect::<Vec<_>>()
            );
            row["result_json"] = json!(
                json!({
                    "revision":old["revision"],
                    "message":message,
                })
                .to_string()
            );
        }
        Ok(row)
    }
    fn sync_snapshot(&self, c: &str) -> Result<Value> {
        let tomb = self.one("SELECT * FROM sync_tombstones WHERE conv_id=?", &[json!(c)])?;
        if !tomb.is_null() {
            return Ok(
                json!({"full":true,"purged":true,"convId":c,"deletedAt":tomb["deleted_at"],"deletedBatch":tomb["deleted_batch"]}),
            );
        }
        let conv = self.conv(c, true)?;
        let removed=self.rows("SELECT table_name AS 'table',row_id AS id,deleted_at AS deletedAt,deleted_batch AS deletedBatch FROM sync_removed WHERE conv_id=? ORDER BY table_name,row_id",&[json!(c)])?;
        let removed_ids = removed
            .iter()
            .map(|r| (s(r, "table"), s(r, "id")))
            .collect::<HashSet<_>>();
        let mut out = json!({"full":true,"purged":false,"convId":c,"removed":removed,
            "conversation":self.sync_project("conversations",&conv)?,
            "publicSettings":self.global_settings()?});
        for &(table, keys) in TABLES {
            let sql = match table {
                "parts" => "SELECT p.* FROM parts p JOIN interactions i ON i.id=p.interaction_id WHERE i.conv_id=?".into(),
                "versions" => "SELECT v.* FROM versions v JOIN parts p ON p.id=v.part_id JOIN interactions i ON i.id=p.interaction_id WHERE i.conv_id=?".into(),
                "memory_knowledge" => "SELECT k.* FROM memory_knowledge k JOIN memories m ON m.id=k.memory_id WHERE m.conv_id=?".into(),
                "personas" => "SELECT * FROM personas WHERE id=(SELECT json_extract(settings_json,'$.personaId') FROM conversations WHERE id=?)".into(),
                _ => format!("SELECT * FROM {table} WHERE conv_id=?"),
            };
            let mut rows = BTreeMap::new();
            if matches!(table, "jobs" | "operations" | "review_flags") {
                for r in self.rows(
                    "SELECT row_json FROM sync_retained WHERE table_name=? AND conv_id=?",
                    &[json!(table), json!(c)],
                )? {
                    let r = parsed(&r, "row_json", Value::Null);
                    rows.insert(key(&r, keys), self.sync_project(table, &r)?);
                }
            }
            for r in self.rows(&sql, &[json!(c)])? {
                let mut r = self.sync_project(table, &r)?;
                if removed_ids.contains(&(table.to_owned(), s(&r, "id"))) {
                    continue;
                }
                if table == "embeddings" {
                    let bytes = r["vec"]
                        .as_array()
                        .ok_or_else(|| invalid("BLOB de vetor inválido."))?
                        .iter()
                        .map(|b| {
                            b.as_u64()
                                .and_then(|b| u8::try_from(b).ok())
                                .ok_or_else(|| invalid("BLOB de vetor inválido."))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    r["vec"] = json!(STANDARD.encode(bytes));
                }
                rows.insert(key(&r, keys), r);
            }
            out[table] = json!(rows.into_values().collect::<Vec<_>>());
        }
        filter_removed_dependents(&mut out);
        let relationship_ids = out["relationships"].as_array().unwrap().iter().map(|r| s(r,"id")).collect::<HashSet<_>>();
        for row in out["relationships"].as_array_mut().unwrap() {
            if let Some(target) = row["superseded_by"].as_str().filter(|id| !relationship_ids.contains(*id)) {
                let target_conv = self.one("SELECT conv_id FROM relationships WHERE id=?", &[json!(target)])?;
                // Compatibility for links left by earlier re-extractions, or
                // a target omitted because its source was permanently removed.
                // A real cross-conversation reference remains invalid.
                if target_conv.is_null() || target_conv["conv_id"] == c {
                    row["superseded_by"] = Value::Null;
                    row["status"] = json!("current");
                }
            }
        }
        for row in out["jobs"].as_array_mut().unwrap() {
            let gone = [
                ("interactions", "interaction_id"),
                ("versions", "version_id"),
            ]
            .iter()
            .any(|(table, col)| removed_ids.contains(&(table.to_string(), s(row, col))));
            if gone && !matches!(row["status"].as_str(), Some("done" | "skipped")) {
                row["status"] = json!("skipped");
            }
        }
        for row in out["operations"].as_array_mut().unwrap() {
            let mut result = parsed(row, "result_json", json!({}));
            if removed_ids.contains(&("parts".into(), s(&result["message"], "id")))
                || removed_ids.contains(&("versions".into(), s(&result["message"], "versionId")))
            {
                result["message"]["content"] = Value::Null;
                result["message"]["flags"] = json!([]);
                row["result_json"] = json!(result.to_string());
            }
        }
        let sources = embedding_sources(&out);
        for row in out["embeddings"].as_array_mut().unwrap() {
            row["source_hash"] = json!(
                sources
                    .get(&key(row, &["owner_type", "owner_id"]))
                    .map(|text| rules::hash(text))
                    .unwrap_or_default()
            );
        }
        Ok(canonical(&out))
    }
    fn sync_busy(&self, c: &str) -> Result<bool> {
        Ok(!self.one("SELECT 1 busy FROM generations WHERE conv_id=? AND status='running' UNION ALL SELECT 1 FROM jobs WHERE conv_id=? AND status='running' LIMIT 1",
            &[json!(c),json!(c)])?.is_null())
    }
    fn sync_idle(&self, c: &str) -> Result<()> {
        if self.sync_busy(c)? {
            return Err(Error::new(
                409,
                "Chat ocupado por geração/job; tente novamente após standby.",
            )
            .extra("code", "BUSY")
            .extra("convId", c)
            .extra("retryable", true));
        }
        Ok(())
    }
    fn sync_refresh(&self) -> Result<()> {
        for row in self.rows("SELECT conv_id FROM sync_dirty ORDER BY conv_id", &[])? {
            let c = s(&row, "conv_id");
            if self.one("SELECT id FROM conversations WHERE id=? UNION ALL SELECT conv_id FROM sync_tombstones WHERE conv_id=?",&[json!(c),json!(c)])?.is_null() {
                self.exec("DELETE FROM sync_dirty WHERE conv_id=?",&[json!(c)])?;
                continue;
            }
            let snapshot = self.sync_snapshot(&c)?;
            let digest = snapshot_hash(&snapshot);
            let old = self.one("SELECT * FROM sync_manifests WHERE conv_id=?", &[json!(c)])?;
            if old["hash"] != digest || old["content_hash"] == "" {
                self.exec("UPDATE sync_clock SET cursor=cursor+1 WHERE id=1", &[])?;
                let cursor =
                    self.one("SELECT cursor FROM sync_clock WHERE id=1", &[])?["cursor"].clone();
                self.exec(
                    "INSERT OR REPLACE INTO sync_manifests VALUES(?,?,?,?,?,?,?)",
                    &[
                        json!(c),
                        json!(digest),
                        json!(n(&old, "revision") + 1),
                        cursor,
                        json!(
                            snapshot["purged"] == true
                                || !snapshot["conversation"]["deleted_at"].is_null()
                        ),
                        snapshot["purged"].clone(),
                        json!(content_hash(&snapshot)),
                    ],
                )?;
            }
            self.exec("DELETE FROM sync_dirty WHERE conv_id=?", &[json!(c)])?;
        }
        Ok(())
    }
    fn sync_server_id(&self) -> Result<Value> {
        Ok(self.one("SELECT value FROM meta WHERE key='sync_server_id'", &[])?["value"].clone())
    }
    fn sync_header(&self) -> Result<Value> {
        Ok(
            json!({"format":"modularchat-sync","version":1,"serverId":self.sync_server_id()?,
            "schemaVersion":SCHEMA_VERSION,"cursor":self.one("SELECT cursor FROM sync_clock WHERE id=1",&[])?["cursor"],
            "maxBytes":MAX_BYTES,"maxConversations":MAX_CONVERSATIONS}),
        )
    }
    fn sync_manifest(&self, row: &Value) -> Result<Value> {
        Ok(
            json!({"convId":row["conv_id"],"hash":row["hash"],"contentHash":row["content_hash"],"revision":row["revision"],"cursor":row["cursor"],
            "deleted":n(row,"deleted")!=0,"purged":n(row,"purged")!=0,"busy":self.sync_busy(&s(row,"conv_id"))?}),
        )
    }
    pub(super) fn sync_status(&self, p: &Value) -> Result<Value> {
        object_keys(p, &["after", "limit"])?;
        let (after, limit) = page(p, MANIFEST_PAGE, MANIFEST_PAGE)?;
        self.sync_refresh()?;
        let rows = self.rows(
            "SELECT * FROM sync_manifests WHERE cursor>? ORDER BY cursor LIMIT ?",
            &[json!(after), json!(limit + 1)],
        )?;
        let more = rows.len() > limit;
        let manifests = rows
            .iter()
            .take(limit)
            .map(|r| self.sync_manifest(r))
            .collect::<Result<Vec<_>>>()?;
        let mut out = self.sync_header()?;
        out["nextCursor"] = manifests
            .last()
            .map(|r| r["cursor"].clone())
            .unwrap_or(json!(after));
        out["hasMore"] = json!(more);
        out["manifests"] = json!(manifests);
        Ok(out)
    }
    fn sync_checkpoint(&self, peer: &str, c: &str, snapshot: &Value) -> Result<()> {
        self.exec(
            "INSERT OR IGNORE INTO sync_checkpoints VALUES(?,?,?,?)",
            &[
                json!(peer),
                json!(c),
                json!(snapshot_hash(snapshot)),
                json!(checkpoint(snapshot).to_string()),
            ],
        )?;
        Ok(())
    }
    pub(super) fn sync_character_source(&self, c: &str) -> Result<Value> {
        let row = self.one("SELECT text FROM character_revisions WHERE conv_id=? AND EXISTS(SELECT 1 FROM sync_character_authority WHERE conv_id=?) ORDER BY rev DESC LIMIT 1",
            &[json!(c),json!(c)])?;
        Ok(row["text"].clone())
    }
}

impl Database {
    pub(super) fn sync_failed_batch(
        &self,
        peer: &str,
        batch: &str,
        request_hash: &str,
        after: u64,
        model: &str,
        error: &Error,
    ) -> Result<Value> {
        let mut out = self.sync_header()?;
        out["embeddingFingerprint"] = json!(model);
        out["code"] = json!("CONFLICT");
        out["error"] = json!(error.message);
        out["ackBatch"] = Value::Null;
        out["accepted"] = json!([]);
        out["manifests"] = json!([]);
        out["snapshots"] = json!([]);
        out["ignoredEmbeddings"] = json!([]);
        out["hasMore"] = json!(false);
        out["nextCursor"] = json!(after);
        out["conflicts"] = json!([{"convId":error.extra["convId"],"reason":"CONSTRAINT_OR_OWNERSHIP",
            "table":error.extra["table"],"constraint":error.extra["constraint"]}]);
        self.exec(
            "INSERT INTO sync_batches VALUES(?,?,?,?)",
            &[
                json!(peer),
                json!(batch),
                json!(request_hash),
                json!(out.to_string()),
            ],
        )?;
        Ok(out)
    }
    fn sync_validate(&self, data: &Value) -> Result<()> {
        if data.to_string().len() > MAX_BYTES {
            return Err(sized(data["convId"].as_str(), data.to_string().len()));
        }
        let c = identifier(&data["convId"])?;
        if data["full"] != true || !data["purged"].is_boolean() {
            return Err(invalid("Snapshot completo (full:true) obrigatório."));
        }
        if data["purged"] == true {
            object_keys(
                data,
                &["full", "purged", "convId", "deletedAt", "deletedBatch"],
            )?;
            identifier(&data["deletedBatch"])?;
            timestamp(&data["deletedAt"])?;
            return Ok(());
        }
        let mut keys = vec![
            "full",
            "purged",
            "convId",
            "conversation",
            "publicSettings",
            "removed",
        ];
        keys.extend(TABLES.iter().map(|t| t.0));
        object_keys(data, &keys)?;
        let mut removed = HashMap::<&str, HashSet<&str>>::new();
        for row in data["removed"]
            .as_array()
            .ok_or_else(|| invalid("removed deve ser array completo, inclusive vazio."))?
        {
            object_keys(row, &["table", "id", "deletedAt", "deletedBatch"])?;
            let table = row["table"]
                .as_str()
                .filter(|t| REMOVED_TABLES.contains(t))
                .ok_or_else(|| invalid("Tombstone de tabela não permitido."))?;
            let id = identifier(&row["id"])?;
            identifier(&row["deletedBatch"])?;
            timestamp(&row["deletedAt"])?;
            if !removed.entry(table).or_default().insert(id) {
                return Err(invalid("Tombstone duplicado."));
            }
            if data[table]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|r| r["id"] == id))
            {
                return Err(invalid("Registro removido e vivo com o mesmo ID."));
            }
        }
        if data["conversation"]["id"] != c {
            return Err(invalid("convId divergente."));
        }
        object_keys(&data["publicSettings"], PUBLIC_SETTINGS)?;
        validate_settings(&data["publicSettings"], true)?;
        if Self::normalize_global_settings(&data["publicSettings"]) != data["publicSettings"] {
            return Err(invalid("Settings públicos fora dos limites do Store."));
        }
        for table in std::iter::once("conversations").chain(TABLES.iter().map(|t| t.0)) {
            let columns = self.rows(&format!("PRAGMA table_info({table})"), &[])?;
            let mut allowed = self.sync_columns(table)?;
            if table == "embeddings" {
                allowed.push("source_hash".into());
            }
            let allowed = allowed.iter().map(String::as_str).collect::<Vec<_>>();
            let singleton = vec![data["conversation"].clone()];
            let rows = if table == "conversations" {
                &singleton
            } else {
                data[table].as_array().ok_or_else(|| {
                    invalid("Todas as tabelas allowlist devem ser arrays, inclusive vazios.")
                })?
            };
            let pk = if table == "conversations" {
                &["id"][..]
            } else {
                TABLES.iter().find(|t| t.0 == table).unwrap().1
            };
            let mut seen = HashSet::new();
            for row in rows {
                object_keys(row, &allowed)?;
                if row.as_object().unwrap().len() != allowed.len() {
                    return Err(invalid("Snapshot parcial de registro não é permitido."));
                }
                if !seen.insert(key(row, pk)) {
                    return Err(invalid("Chave primária duplicada."));
                }
                for col in &columns {
                    let name = s(col, "name");
                    if !allowed.contains(&name.as_str()) {
                        continue;
                    }
                    let v = &row[&name];
                    if v.is_null() {
                        if n(col, "notnull") != 0 || pk.contains(&name.as_str()) {
                            return Err(invalid("Campo obrigatório nulo."));
                        }
                        continue;
                    }
                    let valid = match s(col, "type").as_str() {
                        "TEXT" => v
                            .as_str()
                            .is_some_and(|s| s.len() <= MAX_TEXT && !s.contains('\0')),
                        "INTEGER" => v.as_i64().is_some(),
                        "REAL" => v.as_f64().is_some_and(f64::is_finite),
                        "BLOB" => table == "embeddings" && v.is_string(),
                        _ => false,
                    };
                    if !valid {
                        return Err(invalid("Tipo/tamanho de campo inválido."));
                    }
                    if name.ends_with("_at") || name == "user_ts" {
                        timestamp(v)?;
                    }
                    if name.ends_with("_json") {
                        let text = v.as_str().ok_or_else(|| invalid("Campo JSON inválido."))?;
                        if text.len() > 256 * 1024 {
                            return Err(invalid("Campo JSON excede 256 KiB."));
                        }
                        let inner: Value = serde_json::from_str(text)
                            .map_err(|_| invalid("Campo JSON inválido."))?;
                        let array = [
                            "aliases_json",
                            "present_json",
                            "objects_json",
                            "holding_json",
                            "version_ids_json",
                            "interaction_ids_json",
                        ]
                        .contains(&name.as_str());
                        if array {
                            if !inner.as_array().is_some_and(|a| {
                                a.iter()
                                    .all(|v| v.as_str().is_some_and(|s| s.len() <= MAX_TEXT))
                            }) {
                                return Err(invalid("Array JSON inválido."));
                            }
                        } else if !inner.is_object() {
                            return Err(invalid("Objeto JSON inválido."));
                        }
                    }
                }
                if row.get("conv_id").is_some() && row["conv_id"] != c {
                    return Err(invalid("Referência entre conversas não permitida."));
                }
                if let Some(id) = row.get("id") {
                    identifier(id)?;
                }
                if let Some(op) = row.get("op_id").filter(|v| !v.is_null()) {
                    identifier(op)?;
                }
                for flag in [
                    "pinned",
                    "archived",
                    "user_edited",
                    "confirmed",
                    "active",
                    "is_default",
                ] {
                    if row
                        .get(flag)
                        .is_some_and(|v| !matches!(v.as_i64(), Some(0 | 1)))
                    {
                        return Err(invalid("Flag booleana inválida."));
                    }
                }
                for num in ["rev", "revision", "seq", "from_seq", "to_seq"] {
                    if row
                        .get(num)
                        .is_some_and(|v| v.as_i64().is_none_or(|n| n < 1))
                    {
                        return Err(invalid("Revisão/sequência inválida."));
                    }
                }
                for num in ["idx", "attempts", "character_rev"] {
                    if row
                        .get(num)
                        .is_some_and(|v| v.as_i64().is_none_or(|n| n < 0))
                    {
                        return Err(invalid("Índice/contador inválido."));
                    }
                }
                for num in ["confidence", "importance"] {
                    if row
                        .get(num)
                        .is_some_and(|v| v.as_f64().is_none_or(|n| !(0.0..=1.0).contains(&n)))
                    {
                        return Err(invalid("Confiança/importância inválida."));
                    }
                }
                if row.get("deleted_at").is_some()
                    && row["deleted_at"].is_null() != row["deleted_batch"].is_null()
                {
                    return Err(invalid("Tombstone incompleto."));
                }
                if let Some(batch) = row.get("deleted_batch").filter(|v| !v.is_null()) {
                    identifier(batch)?;
                }
                if table == "conversations" {
                    validate_settings(&parsed(row, "settings_json", json!({})), false)?;
                }
                if table == "personas" && row["is_default"] != 0 {
                    return Err(invalid(
                        "A seleção da persona padrão é local ao dispositivo.",
                    ));
                }
                if table == "character_revisions" && row["hash"] != rules::hash(&s(row, "text")) {
                    return Err(invalid("Hash/proveniência da ficha inválido."));
                }
                if table == "jobs"
                    && !["pending", "done", "failed", "blocked", "skipped"]
                        .contains(&s(row, "status").as_str())
                {
                    return Err(invalid(
                        "Jobs running não são transferíveis; conclua/defer na origem primeiro.",
                    ));
                }
                if table == "jobs"
                    && !["turn", "episode", "cast", "curate"].contains(&s(row, "kind").as_str())
                {
                    return Err(invalid("Tipo de job inválido."));
                }
                if table == "versions"
                    && !["complete", "length", "interrupted", "error", "empty"]
                        .contains(&s(row, "status").as_str())
                {
                    return Err(invalid("Versão não terminal."));
                }
                if table == "generations"
                    && !["complete", "length", "interrupted", "empty", "error"]
                        .contains(&s(row, "status").as_str())
                {
                    return Err(invalid("Geração não terminal."));
                }
                if table == "generations"
                    && !["send", "edit", "regenerate", "continue"]
                        .contains(&s(row, "mode").as_str())
                {
                    return Err(invalid("Modo de geração inválido."));
                }
                if table == "operations" && row["kind"] != "edit" {
                    return Err(invalid("Operação desconhecida."));
                }
                if matches!(table, "jobs" | "operations" | "versions")
                    && self.sync_project(table, row)? != *row
                {
                    return Err(invalid("JSON contém campos privados/não canônicos."));
                }
                if table == "versions" {
                    let usage = parsed(row, "usage_json", json!({}));
                    if usage.as_object().is_some_and(|o| {
                        o.values()
                            .any(|v| !v.is_number() && !v.is_boolean() && !v.is_null())
                    }) {
                        return Err(invalid("Usage privado/não numérico."));
                    }
                }
                if table == "embeddings" {
                    hash(&row["model"])?;
                    hash(&row["source_hash"])?;
                    vector(row)?;
                }
            }
        }
        transfer::validate_sync(data, &removed)?;
        validate_refs(data)?;
        Ok(())
    }

    pub(super) fn sync_delta(&self, p: &Value, model: &str) -> Result<Value> {
        if self.vault.is_none() {
            return Err(Error::new(
                409,
                "Sync exige Store criptografado para checkpoints e replay.",
            )
            .extra("code", "SYNC_ENCRYPTION_REQUIRED"));
        }
        let bytes = p.to_string().len();
        if bytes > MAX_BYTES {
            return Err(sized(None, bytes));
        }
        object_keys(
            p,
            &[
                "format", "version", "peerId", "batchId", "known", "convIds", "after", "limit",
                "pushes",
            ],
        )?;
        if p["format"] != "modularchat-sync" || p["version"] != 1 {
            return Err(invalid("Protocolo de sync inválido."));
        }
        let peer = uuid(&p["peerId"])?;
        if self.sync_server_id()? == peer {
            return Err(invalid("peerId deve identificar o outro Store."));
        }
        let (after, limit) = page(p, MAX_CONVERSATIONS, MAX_CONVERSATIONS)?;
        let batch = p.get("batchId").map(uuid).transpose()?;
        let empty = Vec::new();
        let selected = match p.get("convIds") {
            Some(v) => v
                .as_array()
                .ok_or_else(|| invalid("convIds deve ser array."))?,
            None => &empty,
        };
        if selected.len() > MAX_CONVERSATIONS {
            return Err(invalid("Até 4 convIds por página."));
        }
        let mut selection = HashSet::new();
        for c in selected {
            if !selection.insert(identifier(c)?) {
                return Err(invalid("convIds duplicados."));
            }
        }
        let pushes = match p.get("pushes") {
            Some(v) => v
                .as_array()
                .ok_or_else(|| invalid("pushes deve ser array."))?,
            None => &empty,
        };
        if pushes.len() > MAX_CONVERSATIONS || !pushes.is_empty() && batch.is_none() {
            return Err(invalid("Até 4 pushes com batchId obrigatório."));
        }
        let known = match p.get("known") {
            Some(v) => v
                .as_array()
                .ok_or_else(|| invalid("known deve ser array."))?,
            None => &empty,
        };
        if known.len() > MAX_KNOWN {
            return Err(invalid("known excede 256 entradas; use cursor/página."));
        }
        let mut hashes = HashMap::new();
        for entry in known {
            object_keys(entry, &["convId", "hash"])?;
            let c = identifier(&entry["convId"])?;
            let h = hash(&entry["hash"])?;
            if hashes.insert(c, h).is_some() {
                return Err(invalid("convId duplicado em known."));
            }
        }
        let request_hash = rules::hash(&p.to_string());
        if let Some(batch) = batch {
            let old = self.one(
                "SELECT * FROM sync_batches WHERE peer_id=? AND batch_id=?",
                &[json!(peer), json!(batch)],
            )?;
            if !old.is_null() {
                if old["request_hash"] != request_hash {
                    return Err(Error::new(409, "batchId reutilizado com outro conteúdo.")
                        .extra("code", "BATCH_REUSE"));
                }
                return Ok(parsed(&old, "response_json", Value::Null));
            }
        }
        let mut seen = HashSet::new();
        for push in pushes {
            object_keys(push, &["convId", "baseHash", "hash", "snapshot"])?;
            let c = identifier(&push["convId"])?;
            if !selection.is_empty() && !selection.contains(c) {
                return Err(invalid("Push fora de convIds."));
            }
            hash(&push["hash"])?;
            if !push["baseHash"].is_null() {
                hash(&push["baseHash"])?;
            }
            if push.get("baseHash").is_none() || !seen.insert(c) || push["snapshot"]["convId"] != c
            {
                return Err(invalid("Push duplicado/convId/baseHash inválido."));
            }
            self.sync_validate(&push["snapshot"])?;
            if snapshot_hash(&push["snapshot"]) != push["hash"] {
                return Err(invalid("Hash do snapshot não confere."));
            }
            self.sync_idle(c)?;
        }
        self.sync_refresh()?;
        let mut plans = Vec::new();
        let mut conflicts = Vec::new();
        let mut ignored = Vec::new();
        for push in pushes {
            let c = s(push, "convId");
            let local = self.one(
                "SELECT hash FROM sync_manifests WHERE conv_id=?",
                &[json!(c)],
            )?;
            let ours = if local.is_null() {
                None
            } else {
                Some(self.sync_snapshot(&c)?)
            };
            let base = if push["baseHash"].is_null() {
                None
            } else {
                let stored=self.one("SELECT snapshot_json FROM sync_checkpoints WHERE peer_id=? AND conv_id=? AND hash=?",
                    &[json!(peer),json!(c),push["baseHash"].clone()])?;
                if !stored.is_null() {
                    Some(parsed(&stored, "snapshot_json", Value::Null))
                } else if push["baseHash"] == local["hash"] {
                    ours.clone()
                } else {
                    conflicts.push(conflict(&c, "BASE_UNKNOWN", None, push, &local));
                    continue;
                }
            };
            let incoming = canonical(&push["snapshot"]);
            if incoming["purged"] != true {
                for row in incoming["embeddings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|r| r["model"] != model)
                {
                    ignored.push(json!({"convId":c,"ownerType":row["owner_type"],"ownerId":row["owner_id"],"reason":"fingerprint"}));
                }
            }
            let mut merge = match merge_snapshot(base.as_ref(), ours.as_ref(), &incoming, model) {
                Ok(v) => v,
                Err(reason) => {
                    conflicts.push(conflict(&c, &reason, None, push, &local));
                    continue;
                }
            };
            if merge["purged"] != true {
                let current = self.global_settings()?;
                let base_settings = base.as_ref().map(|b| &b["publicSettings"]);
                let settings = if ours.is_none()
                    && base_settings.is_none()
                    && self
                        .one("SELECT value FROM meta WHERE key='global_settings'", &[])?
                        .is_null()
                {
                    Ok(merge["publicSettings"].clone())
                } else if ours.is_none() && base_settings.is_none() {
                    Ok(current.clone())
                } else {
                    merge_object(
                        base_settings,
                        &current,
                        &incoming["publicSettings"],
                        &[],
                        None,
                    )
                };
                match settings {
                    Ok(settings) => merge["publicSettings"] = settings,
                    Err(_) => {
                        conflicts.push(conflict(&c, "PUBLIC_SETTINGS", None, push, &local));
                        continue;
                    }
                }
                let sources = embedding_sources(&merge);
                merge["embeddings"].as_array_mut().unwrap().retain(|row|{
                    let reason=if row["model"]!=model { Some("fingerprint") }
                        else if sources.get(&key(row,&["owner_type","owner_id"])).map(|t|rules::hash(t)).as_deref()!=row["source_hash"].as_str() { Some("source_changed") }
                        else { None };
                    if let Some(reason)=reason { ignored.push(json!({"convId":c,"ownerType":row["owner_type"],"ownerId":row["owner_id"],"reason":reason}));false } else {true}
                });
                if self.sync_validate(&merge).is_err() {
                    conflicts.push(conflict(
                        &c,
                        "MERGED_REFERENCES_OR_UNIQUE_KEYS",
                        None,
                        push,
                        &local,
                    ));
                    continue;
                }
            }
            plans.push((c, incoming, merge, ours));
        }
        let mut public_settings = None;
        let mut personas = HashMap::new();
        for (c, _, merged, _) in &plans {
            if merged["purged"] == true {
                continue;
            }
            if public_settings
                .as_ref()
                .is_some_and(|s| s != &merged["publicSettings"])
            {
                conflicts.push(json!({"convId":c,"reason":"PUBLIC_SETTINGS_BATCH"}));
            }
            public_settings = Some(merged["publicSettings"].clone());
            for row in merged["personas"].as_array().unwrap() {
                if personas
                    .insert(s(row, "id"), row.clone())
                    .is_some_and(|prev| prev != *row)
                {
                    conflicts.push(json!({"convId":c,"reason":"PERSONA_BATCH","row":row["id"]}));
                }
            }
        }
        let mut out = self.sync_header()?;
        out["embeddingFingerprint"] = json!(model);
        out["ackBatch"] = Value::Null;
        out["accepted"] = json!([]);
        out["conflicts"] = json!(conflicts);
        let mut seen_ignored = HashSet::new();
        ignored.retain(|e| seen_ignored.insert(e.to_string()));
        out["ignoredEmbeddings"] = json!(ignored);
        out["snapshots"] = json!([]);
        out["manifests"] = json!([]);
        out["hasMore"] = json!(false);
        out["nextCursor"] = json!(after);
        if !out["conflicts"].as_array().unwrap().is_empty() {
            out["code"] = json!("CONFLICT");
        } else {
            for (c, incoming, merged, ours) in &plans {
                if ours.as_ref() != Some(merged) {
                    self.sync_apply(c, merged, ours.as_ref())?;
                }
                self.sync_checkpoint(peer, c, incoming)?;
                self.sync_checkpoint(peer, c, merged)?;
            }
            self.sync_refresh()?;
            out["cursor"] = self.sync_header()?["cursor"].clone();
            let mut accepted = Vec::new();
            for (c, _, _, _) in &plans {
                accepted.push(self.sync_manifest(
                    &self.one("SELECT * FROM sync_manifests WHERE conv_id=?", &[json!(c)])?,
                )?);
            }
            out["accepted"] = json!(accepted);
            out["ackBatch"] = batch.map(json_string).unwrap_or(Value::Null);
            let rows=self.rows("SELECT * FROM sync_manifests WHERE cursor>?1 AND (json_array_length(?2)=0 OR conv_id IN(SELECT value FROM json_each(?2))) ORDER BY cursor LIMIT 257",
                &[json!(after),json!(json!(selected).to_string())])?;
            let mut offers = Vec::new();
            let mut scanned = 0;
            for row in &rows {
                if offers.len() == limit {
                    break;
                }
                let c = s(row, "conv_id");
                if hashes.get(c.as_str()).is_some_and(|h| *h == s(row, "hash")) {
                    out["nextCursor"] = row["cursor"].clone();
                    scanned += 1;
                    continue;
                }
                self.sync_idle(&c)?;
                let snapshot = self.sync_snapshot(&c)?;
                self.sync_validate(&snapshot)?;
                let entry = json!({"convId":c,"hash":row["hash"],"contentHash":row["content_hash"],"snapshot":snapshot});
                out["snapshots"].as_array_mut().unwrap().push(entry);
                out["manifests"]
                    .as_array_mut()
                    .unwrap()
                    .push(self.sync_manifest(row)?);
                let size = out.to_string().len() + 128;
                if size > MAX_BYTES {
                    out["snapshots"].as_array_mut().unwrap().pop();
                    out["manifests"].as_array_mut().unwrap().pop();
                    if offers.is_empty() {
                        return Err(sized(Some(&c), size));
                    }
                    break;
                }
                offers.push((c, snapshot));
                out["nextCursor"] = row["cursor"].clone();
                scanned += 1;
            }
            out["hasMore"] = json!(scanned < rows.len() || rows.len() == 257);
            for (c, snapshot) in offers {
                self.sync_checkpoint(peer, &c, &snapshot)?;
            }
        }
        if let Some(batch) = batch {
            if out.to_string().len() > MAX_BYTES {
                return Err(sized(None, out.to_string().len()));
            }
            self.exec(
                "INSERT INTO sync_batches VALUES(?,?,?,?)",
                &[
                    json!(peer),
                    json!(batch),
                    json!(request_hash),
                    json!(out.to_string()),
                ],
            )?;
        }
        Ok(out)
    }

    fn sync_apply(&self, c: &str, data: &Value, ours: Option<&Value>) -> Result<()> {
        if data["purged"] == true {
            self.exec(
                "INSERT OR IGNORE INTO sync_tombstones VALUES(?,?,?)",
                &[
                    json!(c),
                    data["deletedAt"].clone(),
                    data["deletedBatch"].clone(),
                ],
            )?;
            // Purge replication hides the local conversation but never physically deletes its history.
            self.exec("UPDATE conversations SET deleted_at=?,deleted_batch=? WHERE id=? AND deleted_at IS NULL",
                &[data["deletedAt"].clone(),data["deletedBatch"].clone(),json!(c)])?;
            self.exec("UPDATE jobs SET status='skipped' WHERE conv_id=? AND status IN ('pending','blocked','failed')",&[json!(c)])?;
            self.exec("INSERT OR IGNORE INTO sync_dirty VALUES(?)", &[json!(c)])?;
            return Ok(());
        }
        if self.global_settings()? != data["publicSettings"] {
            self.set_global_settings(&data["publicSettings"])?;
        }
        for tomb in data["removed"].as_array().unwrap() {
            let table = s(tomb, "table");
            let id = s(tomb, "id");
            let existing = self.one(&format!("SELECT * FROM {table} WHERE id=?"), &[json!(id)])?;
            let owner=match table.as_str() {
                "parts"=>self.one("SELECT conv_id FROM interactions WHERE id=?",&[existing["interaction_id"].clone()])?["conv_id"].clone(),
                "versions"=>self.one("SELECT i.conv_id FROM parts p JOIN interactions i ON i.id=p.interaction_id WHERE p.id=?",&[existing["part_id"].clone()])?["conv_id"].clone(),
                _=>existing["conv_id"].clone(),
            };
            let previous = self.one(
                "SELECT conv_id FROM sync_removed WHERE table_name=? AND row_id=?",
                &[json!(table), json!(id)],
            )?;
            if !owner.is_null() && owner != c || !previous.is_null() && previous["conv_id"] != c {
                return Err(Error::new(409, "Tombstone pertence a outra conversa.")
                    .extra("code", "CONFLICT")
                    .extra("convId", c));
            }
            self.exec("INSERT INTO sync_removed VALUES(?,?,?,?,?) ON CONFLICT(table_name,row_id) DO NOTHING",
                &[json!(c),json!(table),json!(id),tomb["deletedAt"].clone(),tomb["deletedBatch"].clone()])?;
            if table == "memories" {
                self.exec(
                    "UPDATE memories SET status='rejected' WHERE id=?",
                    &[json!(id)],
                )?;
            } else if table != "versions" {
                self.exec(&format!("UPDATE {table} SET deleted_at=?,deleted_batch=? WHERE id=? AND deleted_at IS NULL"),
                    &[tomb["deletedAt"].clone(),tomb["deletedBatch"].clone(),json!(id)])?;
            }
        }
        for (table, pk) in std::iter::once(("conversations", &["id"][..]))
            .chain(TABLES.iter().copied())
            .filter(|t| t.0 != "embeddings")
        {
            let singleton = vec![data["conversation"].clone()];
            let rows = if table == "conversations" {
                &singleton
            } else {
                data[table].as_array().unwrap()
            };
            let cols = self.sync_columns(table)?;
            for row in rows {
                let args = pk.iter().map(|k| row[*k].clone()).collect::<Vec<_>>();
                let where_pk = pk
                    .iter()
                    .map(|k| format!("{k}=?"))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                let existing =
                    self.one(&format!("SELECT * FROM {table} WHERE {where_pk}"), &args)?;
                if !existing.is_null() {
                    let owner=match table {
                        "parts"=>self.one("SELECT conv_id FROM interactions WHERE id=?",&[existing["interaction_id"].clone()])?["conv_id"].clone(),
                        "versions"=>self.one("SELECT i.conv_id FROM parts p JOIN interactions i ON i.id=p.interaction_id WHERE p.id=?",&[existing["part_id"].clone()])?["conv_id"].clone(),
                        "memory_knowledge"=>self.one("SELECT conv_id FROM memories WHERE id=?",&[existing["memory_id"].clone()])?["conv_id"].clone(),
                        _=>existing["conv_id"].clone(),
                    };
                    let persona_collision = table == "personas"
                        && self.sync_project(table, &existing)? != *row
                        && !ours.is_some_and(|o| {
                            o["personas"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .any(|r| r["id"] == row["id"])
                        });
                    if !owner.is_null() && owner != c || persona_collision {
                        return Err(Error::new(
                            409,
                            "ID existente pertence a outra origem/conversa.",
                        )
                        .extra("code", "CONFLICT")
                        .extra("convId", c));
                    }
                }
                let mut row = row.clone();
                if table == "personas" && !existing.is_null() {
                    row["is_default"] = existing["is_default"].clone();
                }
                if table == "conversations" && !existing.is_null() {
                    let mut settings = parsed(&existing, "settings_json", json!({}));
                    let incoming = parsed(&row, "settings_json", json!({}));
                    for k in SETTINGS {
                        settings.as_object_mut().unwrap().remove(*k);
                    }
                    settings
                        .as_object_mut()
                        .unwrap()
                        .extend(incoming.as_object().unwrap().clone());
                    row["settings_json"] = json!(settings.to_string());
                }
                if !existing.is_null() && cols.iter().all(|k| existing[k] == row[k]) {
                    continue;
                }
                let updates = cols
                    .iter()
                    .filter(|k| !pk.contains(&k.as_str()))
                    .map(|k| format!("{k}=excluded.{k}"))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "INSERT INTO {table}({}) VALUES({}) ON CONFLICT({}) DO UPDATE SET {updates}",
                    cols.join(","),
                    vec!["?"; cols.len()].join(","),
                    pk.join(",")
                );
                self.exec(
                    &sql,
                    &cols.iter().map(|k| row[k].clone()).collect::<Vec<_>>(),
                )
                .map_err(|e| {
                    if e.message.contains("UNIQUE constraint") {
                        Error::new(409, "Colisão de IDs/chaves únicas.")
                            .extra("code", "CONFLICT")
                            .extra("convId", c)
                            .extra("table", table)
                            .extra("constraint", e.message)
                    } else {
                        e
                    }
                })?;
            }
        }
        self.exec(
            "INSERT OR IGNORE INTO sync_character_authority VALUES(?)",
            &[json!(c)],
        )?;
        // Rebuild only this conversation's derived indexes. Do not call invalidate(): it can
        // enqueue a completed turn again when an operational job/note has already been pruned.
        self.timeline_cache.borrow_mut().retain(|t| !t.is_for(c));
        self.exec(
            "DELETE FROM narrative_snapshots WHERE conv_id=?",
            &[json!(c)],
        )?;
        self.exec("DELETE FROM search_fts WHERE conv_id=?", &[json!(c)])?;
        self.exec("DELETE FROM text_chunks WHERE conv_id=?", &[json!(c)])?;
        self.exec("DELETE FROM embeddings WHERE conv_id=?", &[json!(c)])?;
        for row in data["interactions"].as_array().unwrap() {
            if let Some(text) = row["user_text"].as_str() {
                self.index(c, "user", &s(row, "id"), text)?;
            }
        }
        for row in data["versions"].as_array().unwrap() {
            self.index(c, "asst", &s(row, "id"), &s(row, "content"))?;
        }
        for row in data["memories"].as_array().unwrap() {
            self.index(
                c,
                "memory",
                &s(row, "id"),
                &format!("{} {}", s(row, "subject"), s(row, "content")),
            )?;
        }
        for row in data["episodes"].as_array().unwrap() {
            self.index(c, "episode", &s(row, "id"), &s(row, "summary"))?;
        }
        for row in data["embeddings"].as_array().unwrap() {
            let blob = vector(row)?;
            self.db.execute("INSERT OR REPLACE INTO embeddings(owner_type,owner_id,conv_id,model,dim,vec) VALUES(?,?,?,?,?,?)",
                rusqlite::params![s(row,"owner_type"),s(row,"owner_id"),c,s(row,"model"),n(row,"dim"),blob])?;
        }
        self.exec(
            "INSERT INTO sync_dirty VALUES(?) ON CONFLICT(conv_id) DO NOTHING",
            &[json!(c)],
        )?;
        Ok(())
    }
}

fn timestamp(v: &Value) -> Result<()> {
    if !v
        .as_str()
        .is_some_and(|s| s.len() <= 64 && chrono::DateTime::parse_from_rfc3339(s).is_ok())
    {
        return Err(invalid("Timestamp/proveniência inválido."));
    }
    Ok(())
}
fn validate_settings(v: &Value, global: bool) -> Result<()> {
    object_keys(v, if global { PUBLIC_SETTINGS } else { SETTINGS })?;
    for (k, v) in v.as_object().unwrap() {
        if !global && v.is_null() {
            continue;
        }
        let ok = match k.as_str() {
            "personaId" => identifier(v).is_ok(),
            "personaNotes" | "content" => v.as_str().is_some_and(|s| s.len() <= 16 * 1024),
            "lengthMode" => matches!(v.as_str(), Some("target" | "maximum")),
            "turnPacing" => matches!(v.as_str(), Some("interactive" | "balanced" | "free")),
            "thinkingEnabled" => v.is_boolean(),
            _ => v.as_u64().is_some_and(|n| n <= 2_000_000),
        };
        if !ok {
            return Err(invalid("Setting público inválido."));
        }
    }
    Ok(())
}
fn vector(row: &Value) -> Result<Vec<u8>> {
    let encoded = row["vec"]
        .as_str()
        .ok_or_else(|| invalid("Vetor deve ser base64."))?;
    if encoded.len() > 44 * 1024 {
        return Err(invalid("Vetor excede dimensão 8192."));
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| invalid("Base64 de vetor inválido."))?;
    let dim = row["dim"]
        .as_u64()
        .filter(|d| *d <= 8192)
        .ok_or_else(|| invalid("Dimensão de vetor inválida."))?;
    if rules::decode_vector(&bytes, dim as usize).is_none() {
        return Err(invalid("Vetor inválido: dimensão/bytes/finitude."));
    }
    Ok(bytes)
}
fn filter_removed_dependents(data: &mut Value) {
    let removed = data["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (s(r, "table"), s(r, "id")))
        .collect::<HashSet<_>>();
    let interactions = data["interactions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| s(r, "id"))
        .collect::<HashSet<_>>();
    let versions = data["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| s(r, "id"))
        .collect::<HashSet<_>>();
    let memories = data["memories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| s(r, "id"))
        .collect::<HashSet<_>>();

    let parts = data["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| s(r, "id"))
        .collect::<HashSet<_>>();

    let has_interaction = |id: &str| id.is_empty() || interactions.contains(id) || removed.contains(&("interactions".into(), id.to_string()));
    let has_part = |id: &str| id.is_empty() || parts.contains(id) || removed.contains(&("parts".into(), id.to_string()));
    let has_version = |id: &str| id.is_empty() || versions.contains(id) || removed.contains(&("versions".into(), id.to_string()));
    let has_memory = |id: &str| id.is_empty() || memories.contains(id) || removed.contains(&("memories".into(), id.to_string()));

    for table in [
        "memory_knowledge",
        "knowledge_events",
        "memory_replacements",
        "review_flags",
    ] {
        data[table].as_array_mut().unwrap().retain(|r| match table {
            "memory_knowledge" | "knowledge_events" => {
                has_memory(&s(r, "memory_id"))
            }
            "memory_replacements" => ["old_id", "new_id"]
                .iter()
                .all(|k| has_memory(&s(r, k))),
            _ => has_version(&s(r, "version_id")) && has_interaction(&s(r, "interaction_id")),
        });
    }

    data["jobs"].as_array_mut().unwrap().retain(|r| {
        if !has_interaction(&s(r, "interaction_id")) || !has_version(&s(r, "version_id")) {
            return false;
        }
        let payload = parsed(r, "payload_json", json!({}));
        if payload["interactionIds"].as_array().into_iter().flatten().any(|id| !has_interaction(id.as_str().unwrap_or_default())) {
            return false;
        }
        if payload["notes"].as_array().into_iter().flatten().any(|note| {
            !has_interaction(&s(note, "interaction_id")) || parsed(note, "version_ids_json", json!([])).as_array().into_iter().flatten().any(|v| !has_version(v.as_str().unwrap_or_default()))
        }) {
            return false;
        }
        true
    });

    data["generations"].as_array_mut().unwrap().retain(|r| {
        has_interaction(&s(r, "interaction_id")) && has_part(&s(r, "part_id")) && has_version(&s(r, "version_id"))
    });

    for table in ["scene_states", "character_states", "interaction_notes"] {
        data[table].as_array_mut().unwrap().retain(|r| {
            has_interaction(&s(r, "interaction_id"))
        });
    }

    data["threads"].as_array_mut().unwrap().retain(|r| {
        has_interaction(&s(r, "opened_interaction_id")) && has_interaction(&s(r, "resolved_interaction_id"))
    });

    data["operations"].as_array_mut().unwrap().retain(|r| {
        if !has_interaction(&s(r, "interaction_id")) || !has_interaction(&s(r, "edit_of")) {
            return false;
        }
        let result = parsed(r, "result_json", json!({}));
        let m = &result["message"];
        has_part(&s(m, "id")) && has_version(&s(m, "versionId")) && has_interaction(&s(m, "interactionId"))
    });

    data["episodes"].as_array_mut().unwrap().retain(|r| {
        has_interaction(&s(r, "first_interaction_id")) && has_interaction(&s(r, "interaction_id"))
    });

    data["memories"].as_array_mut().unwrap().retain(|r| {
        has_interaction(&s(r, "source_interaction_id")) && has_version(&s(r, "source_version_id"))
    });

    data["relationships"].as_array_mut().unwrap().retain(|r| {
        has_interaction(&s(r, "source_interaction_id")) && has_version(&s(r, "source_version_id"))
    });

    let sources = embedding_sources(data);
    data["embeddings"]
        .as_array_mut()
        .unwrap()
        .retain(|r| sources.contains_key(&key(r, &["owner_type", "owner_id"])));
}
fn embedding_sources(data: &Value) -> HashMap<String, String> {
    let mut sources = HashMap::new();
    for (table, kind, textkey) in [
        ("memories", "memory", "content"),
        ("episodes", "episode", "summary"),
    ] {
        for row in data[table].as_array().into_iter().flatten() {
            let text = if table == "memories" {
                format!("{}: {}", s(row, "subject"), s(row, textkey))
            } else {
                s(row, textkey)
            };
            sources.insert(
                key(
                    &json!({"owner_type":kind,"owner_id":row["id"]}),
                    &["owner_type", "owner_id"],
                ),
                text,
            );
        }
    }
    for (table, kind, textkey) in [
        ("interactions", "user", "user_text"),
        ("versions", "asst", "content"),
    ] {
        for row in data[table].as_array().into_iter().flatten() {
            if let Some(text) = row[textkey].as_str() {
                for (start, _, text) in rules::chunks(text, 1600, 160) {
                    let id = format!("{kind}:{}:{start}", s(row, "id"));
                    sources.insert(
                        key(
                            &json!({"owner_type":"chunk","owner_id":id}),
                            &["owner_type", "owner_id"],
                        ),
                        text,
                    );
                }
            }
        }
    }
    sources
}
fn conflict(c: &str, reason: &str, row: Option<&str>, push: &Value, local: &Value) -> Value {
    json!({"convId":c,"reason":reason,"row":row,"baseHash":push["baseHash"],"localHash":local["hash"],"incomingHash":push["hash"]})
}

fn validate_refs(data: &Value) -> Result<()> {
    let arrays = |t: &str| data[t].as_array().unwrap();
    let ids = |t: &str| {
        arrays(t)
            .iter()
            .map(|r| (s(r, "id"), r))
            .collect::<HashMap<_, _>>()
    };
    let interactions = ids("interactions");
    let parts = ids("parts");
    let versions = ids("versions");
    let memories = ids("memories");
    let relationships = ids("relationships");
    let generations = ids("generations");
    let tombstones = data["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (s(r, "table"), s(r, "id")))
        .collect::<HashSet<_>>();
    let check = |value: &Value, set: &HashMap<String, &Value>, table: &str| -> Result<()> {
        if !value.is_null()
            && !value.as_str().is_some_and(|s| {
                set.contains_key(s) || tombstones.contains(&(table.to_owned(), s.to_owned()))
            })
        {
            return Err(invalid("Referência de sync ausente/fora da conversa.")
                .extra("targetTable", table).extra("referenceId", value.clone()));
        }
        Ok(())
    };
    let mut global_ids = HashSet::from([s(&data["conversation"], "id")]);
    for (_, id) in &tombstones {
        if !global_ids.insert(id.clone()) {
            return Err(invalid("Tombstone com ID duplicado entre tipos."));
        }
    }
    for &(table, pk) in TABLES {
        if pk == ["id"] {
            for row in arrays(table) {
                if !global_ids.insert(s(row, "id")) {
                    return Err(invalid("Mesmo ID usado por registros de tipos diferentes."));
                }
            }
        }
        for row in arrays(table) {
            for k in [
                "interaction_id",
                "source_interaction_id",
                "first_interaction_id",
                "opened_interaction_id",
                "resolved_interaction_id",
                "edit_of",
            ] {
                check(&row[k], &interactions, "interactions")?;
            }
            for k in ["version_id", "source_version_id"] {
                check(&row[k], &versions, "versions")?;
            }
            for k in ["memory_id", "old_id", "new_id"] {
                check(&row[k], &memories, "memories")?;
            }
            if table == "memories" {
                check(&row["superseded_by"], &memories, "memories")?;
            }
            if table == "relationships" {
                check(&row["superseded_by"], &relationships, "relationships")?;
            }
            if table == "versions" {
                check(&row["generation_id"], &generations, "generations")?;
                if !parts.contains_key(&s(row, "part_id")) {
                    return Err(invalid("Versão viva sem part viva."));
                }
            }
            if table == "parts" && !interactions.contains_key(&s(row, "interaction_id")) {
                return Err(invalid("Part viva sem interação viva."));
            }
            if let Some(i) = row["source_interaction_id"]
                .as_str()
                .or(row["interaction_id"].as_str())
                && let Some(v) = row["source_version_id"]
                    .as_str()
                    .or(row["version_id"].as_str())
                && versions.get(v).is_some_and(|v| {
                    parts
                        .get(&s(v, "part_id"))
                        .is_none_or(|p| p["interaction_id"] != i)
                })
            {
                return Err(invalid("Versão não pertence à interação de origem."));
            }
            if matches!(
                table,
                "scene_states" | "character_states" | "interaction_notes"
            ) && interactions
                .get(&s(row, "interaction_id"))
                .is_some_and(|i| row["seq"] != i["seq"])
            {
                return Err(invalid("Seq de origem divergente."));
            }
        }
    }
    let max_rev = arrays("character_revisions")
        .iter()
        .map(|r| n(r, "rev"))
        .max()
        .unwrap_or(0);
    if max_rev != n(&data["conversation"], "character_rev") {
        return Err(invalid("character_rev não aponta à última revisão."));
    }
    let settings = parsed(&data["conversation"], "settings_json", json!({}));
    let personas = ids("personas");
    if let Some(pid) = settings["personaId"].as_str() {
        if !personas.contains_key(pid) || personas.len() != 1 {
            return Err(invalid("Referência de persona inválida."));
        }
    } else if !personas.is_empty() {
        return Err(invalid("Persona sem referência não é replicável."));
    }
    let mut positions = HashSet::new();
    for row in arrays("parts") {
        if !positions.insert(key(row, &["interaction_id", "idx"])) {
            return Err(invalid("Índice de part duplicado."));
        }
    }
    let mut ops = HashSet::new();
    for row in arrays("generations") {
        for (k, set, table) in [
            ("interaction_id", &interactions, "interactions"),
            ("part_id", &parts, "parts"),
            ("version_id", &versions, "versions"),
        ] {
            if row[k].is_null() {
                return Err(invalid("Geração terminal sem referência."));
            }
            check(&row[k], set, table)?;
        }
        let p = parts.get(&s(row, "part_id"));
        let v = versions.get(&s(row, "version_id"));
        if p.is_some_and(|p| p["interaction_id"] != row["interaction_id"])
            || v.is_some_and(|v| v["part_id"] != row["part_id"] || v["generation_id"] != row["id"])
        {
            return Err(invalid("Proveniência de geração/versão inválida."));
        }
        if let Some(op) = row["op_id"].as_str()
            && !ops.insert(op.to_owned())
        {
            return Err(invalid("opId de geração duplicado."));
        }
    }
    for row in arrays("operations") {
        if !ops.insert(s(row, "op_id")) {
            return Err(invalid("opId de operação duplicado."));
        }
        let result = parsed(row, "result_json", Value::Null);
        if result["revision"].as_i64().is_none_or(|n| n < 1) {
            return Err(invalid("Resultado da operação inválido."));
        }
        let m = &result["message"];
        let mut message_keys = MESSAGE_FIELDS.to_vec();
        message_keys.extend(["usage", "flags"]);
        object_keys(m, &message_keys)?;
        if !m["usage"].is_null()
            || m["role"] != "assistant"
            || !m["edited"].is_boolean()
            || m["partIndex"].as_u64().is_none()
            || m["versionCount"].as_u64().is_none_or(|n| n < 1)
        {
            return Err(invalid("Formato do resultado da operação inválido."));
        }
        timestamp(&m["ts"])?;
        for k in ["status", "origin", "model", "presetId", "effort"] {
            if !m[k].is_null() && !m[k].as_str().is_some_and(|s| s.len() <= 4096) {
                return Err(invalid("Campo de operação não é texto público."));
            }
        }
        for f in m["flags"]
            .as_array()
            .ok_or_else(|| invalid("Flags de operação inválidas."))?
        {
            object_keys(f, &["id", "kind", "detail", "evidence", "source"])?;
            identifier(&f["id"])?;
            for k in ["kind", "detail", "evidence", "source"] {
                if !f[k].is_null() && !f[k].is_string() {
                    return Err(invalid("Flag de operação contém campo não textual."));
                }
            }
        }
        check(&m["id"], &parts, "parts")?;
        check(&m["versionId"], &versions, "versions")?;
        check(&m["interactionId"], &interactions, "interactions")?;
        if m["id"].is_null()
            || m["versionId"].is_null()
            || m["interactionId"].is_null()
            || versions
                .get(&s(m, "versionId"))
                .is_some_and(|v| v["part_id"] != m["id"] || v["content"] != m["content"])
            || parts
                .get(&s(m, "id"))
                .is_some_and(|p| p["interaction_id"] != m["interactionId"])
            || !versions.contains_key(&s(m, "versionId")) && !m["content"].is_null()
        {
            return Err(invalid("Resultado/referências da operação inválidos."));
        }
    }
    for row in arrays("jobs") {
        let payload = parsed(row, "payload_json", json!({}));
        if payload.get("skipReview").is_some_and(|v| !v.is_boolean()) {
            return Err(invalid("skipReview inválido."));
        }
        for id in payload["interactionIds"].as_array().into_iter().flatten() {
            check(id, &interactions, "interactions")?;
        }
        for note in payload["notes"].as_array().into_iter().flatten() {
            object_keys(note, &["interaction_id", "summary", "version_ids_json"])?;
            check(&note["interaction_id"], &interactions, "interactions")?;
            if !note["summary"].is_string() || !note["version_ids_json"].is_string() {
                return Err(invalid("Texto/versões de note inválidos."));
            }
            let vs = parsed(note, "version_ids_json", json!([]));
            for v in vs
                .as_array()
                .ok_or_else(|| invalid("Versões de note inválidas."))?
            {
                check(v, &versions, "versions")?;
            }
        }
        for k in ["fromSeq", "toSeq"] {
            if payload
                .get(k)
                .is_some_and(|v| v.as_u64().is_none_or(|n| n < 1))
            {
                return Err(invalid("Seq de job inválida."));
            }
        }
        if payload.get("interactionIds").is_some_and(|v| !v.is_array())
            || payload.get("notes").is_some_and(|v| !v.is_array())
            || payload.get("key").is_some_and(|v| !v.is_string())
        {
            return Err(invalid("Payload do job inválido."));
        }
    }
    let sources = embedding_sources(data);
    for row in arrays("embeddings") {
        if !sources.contains_key(&key(row, &["owner_type", "owner_id"])) {
            return Err(invalid("Owner de embedding inválido."));
        }
    }
    Ok(())
}

fn row_map(data: Option<&Value>, table: &str, pk: &[&str]) -> BTreeMap<String, Value> {
    data.and_then(|d| d[table].as_array())
        .into_iter()
        .flatten()
        .map(|r| (key(r, pk), r.clone()))
        .collect()
}
fn merge_snapshot(
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: &Value,
    model: &str,
) -> std::result::Result<Value, String> {
    let Some(ours) = ours else {
        return Ok(theirs.clone());
    };
    if ours == theirs {
        return Ok(ours.clone());
    }
    if ours["purged"] == true {
        return Err("PURGED_TOMBSTONE".into());
    }
    if theirs["purged"] == true {
        return if base_same(base, ours) {
            Ok(theirs.clone())
        } else {
            Err("PURGE_DIVERGED".into())
        };
    }
    if base.is_none_or(|b| b["purged"] == true) {
        return Err("BASE_REQUIRED".into());
    }
    if ours["conversation"]["deleted_at"].is_null()
        != theirs["conversation"]["deleted_at"].is_null()
        && base.unwrap()["conversation"]["deleted_at"].is_null()
        && !base_same(base, ours)
        && !base_same(base, theirs)
    {
        return Err("DELETE_DIVERGED".into());
    }
    let mut merged = ours.clone();
    let local_tombs = ours["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (key(r, &["table", "id"]), r.clone()))
        .collect::<BTreeMap<_, _>>();
    for &(table, pk) in TABLES {
        let old = row_map(base, table, pk);
        let local = row_map(Some(ours), table, pk);
        let remote = row_map(Some(theirs), table, pk);
        let mut rows = local.clone();
        for (id, remote) in remote {
            if table == "embeddings" && remote["model"] != model {
                continue;
            }
            let local = local.get(&id);
            let old = old.get(&id);
            if local == Some(&remote) {
                continue;
            }
            if local.is_none() {
                if base_row_same(old, &remote) {
                    continue;
                } // Locally purged/pruned: do not resurrect an old row.
                if old.is_some() {
                    return Err(format!("ROW_REMOVED:{table}:{id}"));
                }
                if local_tombs
                    .values()
                    .any(|r| r["table"] == table && r["id"] == remote["id"])
                {
                    return Err(format!("PURGED_ROW_TOMBSTONE:{table}:{id}"));
                }
                rows.insert(id, remote);
                continue;
            }
            if base_row_same(old, &remote) {
                continue;
            }
            if !base_row_same(old, local.unwrap()) {
                return Err(format!("ROW_DIVERGED:{table}:{id}"));
            }
            // Completed messages/recovery identities are immutable; edits must have new IDs.
            if matches!(table, "versions" | "character_revisions" | "generations") {
                return Err(format!("IMMUTABLE_ROW:{table}:{id}"));
            }
            if table == "operations" {
                let mut redacted = local.unwrap().clone();
                let mut result = parsed(&redacted, "result_json", json!({}));
                result["message"]["content"] = Value::Null;
                result["message"]["flags"] = json!([]);
                redacted["result_json"] = json!(result.to_string());
                let gone = theirs["removed"].as_array().unwrap().iter().any(|r| {
                    r["table"] == "parts" && r["id"] == result["message"]["id"]
                        || r["table"] == "versions" && r["id"] == result["message"]["versionId"]
                });
                if !gone || redacted != remote {
                    return Err(format!("IMMUTABLE_ROW:{table}:{id}"));
                }
            }
            if table == "interactions" {
                let mut a = local.unwrap().clone();
                let mut b = remote.clone();
                for k in ["deleted_at", "deleted_batch"] {
                    a.as_object_mut().unwrap().remove(k);
                    b.as_object_mut().unwrap().remove(k);
                }
                if a != b {
                    return Err(format!("IMMUTABLE_ROW:{table}:{id}"));
                }
            }
            if table == "jobs"
                && matches!(local.unwrap()["status"].as_str(), Some("done" | "skipped"))
                && local.unwrap()["status"] != remote["status"]
            {
                return Err(format!("TERMINAL_JOB:{id}"));
            }
            rows.insert(id, remote);
        }
        merged[table] = json!(rows.into_values().collect::<Vec<_>>());
    }
    let mut removed = local_tombs.clone();
    for tomb in theirs["removed"].as_array().unwrap() {
        let id = key(tomb, &["table", "id"]);
        if local_tombs.get(&id).is_some_and(|t| t != tomb) {
            return Err(format!("TOMBSTONE_DIVERGED:{id}"));
        }
        let table = s(tomb, "table");
        let row_key = key(tomb, &["id"]);
        let local = row_map(Some(ours), &table, &["id"]);
        if let Some(row) = local.get(&row_key)
            && !base_row_same(row_map(base, &table, &["id"]).get(&row_key), row)
        {
            return Err(format!("PURGE_ROW_DIVERGED:{table}:{row_key}"));
        }
        removed.insert(id, tomb.clone());
    }
    for tomb in removed.values() {
        let table = s(tomb, "table");
        merged[&table]
            .as_array_mut()
            .unwrap()
            .retain(|r| r["id"] != tomb["id"]);
    }
    merged["removed"] = json!(removed.into_values().collect::<Vec<_>>());
    filter_removed_dependents(&mut merged);
    merged["conversation"] = merge_object(
        base.map(|b| &b["conversation"]),
        &ours["conversation"],
        &theirs["conversation"],
        &["revision", "updated_at"],
        Some(&merged),
    )
    .map_err(|k| format!("CONVERSATION_DIVERGED:{k}"))?;
    if ours["conversation"]["created_at"] != theirs["conversation"]["created_at"] {
        return Err("CONVERSATION_PROVENANCE".into());
    }
    merged["conversation"]["revision"] =
        json!(n(&ours["conversation"], "revision").max(n(&theirs["conversation"], "revision")));
    merged["conversation"]["updated_at"] =
        json!(s(&ours["conversation"], "updated_at").max(s(&theirs["conversation"], "updated_at")));
    Ok(canonical(&merged))
}
fn merge_object(
    base: Option<&Value>,
    ours: &Value,
    theirs: &Value,
    ignored: &[&str],
    graph: Option<&Value>,
) -> std::result::Result<Value, String> {
    let mut out = ours.clone();
    let mut keys = ours
        .as_object()
        .unwrap()
        .keys()
        .chain(theirs.as_object().unwrap().keys())
        .collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    for k in keys {
        if ignored.contains(&k.as_str()) {
            continue;
        }
        let local = &ours[k];
        let remote = &theirs[k];
        if local == remote || base_field_same(base, k, remote) {
            continue;
        }
        if base_field_same(base, k, local) {
            out[k] = remote.clone();
            continue;
        }
        if k == "head_interaction_id"
            && let Some(graph) = graph
        {
            if descendant(graph, remote, local) {
                out[k] = remote.clone();
                continue;
            }
            if descendant(graph, local, remote) {
                continue;
            }
        }
        return Err(k.clone());
    }
    Ok(out)
}
fn descendant(data: &Value, child: &Value, ancestor: &Value) -> bool {
    let rows = row_map(Some(data), "interactions", &["id"]);
    let mut at = child.clone();
    let mut seen = HashSet::new();
    loop {
        if &at == ancestor {
            return true;
        }
        let Some(id) = at.as_str() else {
            return false;
        };
        if !seen.insert(id.to_owned()) {
            return false;
        }
        let Some(row) = rows.get(&key(&json!({"id":id}), &["id"])) else {
            return false;
        };
        at = row["parent_id"].clone();
    }
}

fn page(p: &Value, default: usize, max: usize) -> Result<(u64, usize)> {
    let after = match p.get("after") {
        Some(v) => v
            .as_u64()
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or_else(|| invalid("Cursor inválido."))?,
        None => 0,
    };
    let limit = match p.get("limit") {
        Some(v) => v
            .as_u64()
            .filter(|n| *n >= 1 && *n <= max as u64)
            .ok_or_else(|| invalid("Limite inválido."))? as usize,
        None => default,
    };
    Ok((after, limit))
}
