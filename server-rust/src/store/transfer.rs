use super::{Database, IMPORT_LIMIT};
use crate::{Error, Result, id, n, now, parsed, rules, s};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::Duration,
};

const TABLES: &[&str] = &[
    "character_revisions",
    "interactions",
    "characters",
    "memories",
    "scene_states",
    "character_states",
    "relationships",
    "threads",
    "interaction_notes",
    "episodes",
    "review_flags",
    "knowledge_events",
    "memory_replacements",
    "parts",
    "versions",
    "memory_knowledge",
    "narrative_snapshots",
];
const ID_TABLES: &[&str] = &[
    "interactions",
    "characters",
    "memories",
    "scene_states",
    "character_states",
    "relationships",
    "threads",
    "episodes",
    "review_flags",
    "knowledge_events",
    "parts",
    "versions",
];
const REFS: &[&str] = &[
    "id",
    "conv_id",
    "parent_id",
    "head_interaction_id",
    "interaction_id",
    "part_id",
    "selected_version_id",
    "source_interaction_id",
    "source_version_id",
    "first_interaction_id",
    "opened_interaction_id",
    "resolved_interaction_id",
    "memory_id",
    "superseded_by",
    "edit_of",
    "version_id",
    "old_id",
    "new_id",
];

impl Database {
    pub(super) fn insert_row(&self, table: &str, row: &Value) -> Result<()> {
        if table != "conversations" && !TABLES.contains(&table) {
            return Err(Error::new(400, "Tabela de importação inválida."));
        }
        let cols = self
            .rows(&format!("PRAGMA table_info({table})"), &[])?
            .into_iter()
            .map(|r| s(&r, "name"))
            .filter(|k| row.get(k).is_some())
            .collect::<Vec<_>>();
        if cols.is_empty() {
            return Err(Error::new(400, "Registro de importação vazio."));
        }
        self.exec(
            &format!(
                "INSERT INTO {table}({}) VALUES({})",
                cols.join(","),
                vec!["?"; cols.len()].join(",")
            ),
            &cols.iter().map(|k| row[k].clone()).collect::<Vec<_>>(),
        )?;
        Ok(())
    }
    pub(super) fn export(&self, c: Option<&str>) -> Result<Value> {
        let ids = if let Some(c) = c {
            self.conv(c, true)?;
            vec![c.to_owned()]
        } else {
            self.rows("SELECT id FROM conversations WHERE deleted_at IS NULL", &[])?
                .iter()
                .map(|r| s(r, "id"))
                .collect()
        };
        let mut conversations = Vec::new();
        for c in ids {
            let mut data = json!({"conversation":self.conv(&c,true)?});
            for &table in TABLES {
                let sql=match table {
                    "parts"=>"SELECT p.* FROM parts p JOIN interactions i ON i.id=p.interaction_id WHERE i.conv_id=?".into(),
                    "versions"=>"SELECT v.* FROM versions v JOIN parts p ON p.id=v.part_id JOIN interactions i ON i.id=p.interaction_id WHERE i.conv_id=?".into(),
                    "memory_knowledge"=>"SELECT k.* FROM memory_knowledge k JOIN memories m ON m.id=k.memory_id WHERE m.conv_id=?".into(),
                    _=>format!("SELECT * FROM {table} WHERE conv_id=?"),
                };
                data[table] = json!(self.rows(&sql, &[json!(c)])?);
            }
            conversations.push(data);
        }
        Ok(
            json!({"format":"modularchat-export","version":1,"exportedAt":now(),"conversations":conversations}),
        )
    }
    pub(super) fn import(&self, p: &Value) -> Result<Value> {
        if p.to_string().len() > IMPORT_LIMIT {
            return Err(Error::new(413, "Importação excede 80 MiB."));
        }
        if p["format"] != "modularchat-export" || !p["conversations"].is_array() {
            return Err(Error::new(400, "Arquivo de importação inválido."));
        }
        let mut imported = Vec::new();
        let mut errors = Vec::new();
        for data in p["conversations"].as_array().unwrap() {
            match self.tx(|| self.import_one(data)) {
                Ok(conv) => imported.push(conv["id"].clone()),
                Err(e) => {
                    errors.push(json!({"name":data["conversation"]["name"],"error":e.message}))
                }
            }
        }
        Ok(json!({"imported":imported,"errors":errors}))
    }
    pub(super) fn import_one(&self, data: &Value) -> Result<Value> {
        validate(data)?;
        let mut ids = HashMap::new();
        ids.insert(s(&data["conversation"], "id"), id());
        for &table in ID_TABLES {
            for r in data[table].as_array().into_iter().flatten() {
                ids.insert(s(r, "id"), id());
            }
        }
        let c = ids[&s(&data["conversation"], "id")].clone();
        let remap = |raw: &Value| {
            let mut r = raw.clone();
            for &key in REFS {
                if let Some(new) = r[key].as_str().and_then(|old| ids.get(old)) {
                    r[key] = json!(new);
                }
            }
            r
        };
        let mut conv = remap(&data["conversation"]);
        conv["id"] = json!(c);
        conv["revision"] = json!(1);
        conv["deleted_at"] = Value::Null;
        conv["deleted_batch"] = Value::Null;
        // Imported persona IDs are external to the transfer. Keep the name and local notes.
        let mut settings = parsed(&conv, "settings_json", json!({}));
        if settings["personaId"].as_str().is_some_and(|pid| {
            self.one("SELECT id FROM personas WHERE id=?", &[json!(pid)])
                .map(|r| r.is_null())
                .unwrap_or(true)
        }) {
            settings.as_object_mut().unwrap().remove("personaId");
        }
        conv["settings_json"] = json!(settings.to_string());
        self.insert_row("conversations", &conv)?;
        for &table in TABLES {
            for raw in data[table].as_array().into_iter().flatten() {
                let mut r = remap(raw);
                if r.get("conv_id").is_some() {
                    r["conv_id"] = json!(c);
                }
                for (key, kind) in [
                    ("version_ids_json", "versions"),
                    ("interaction_ids_json", "interactions"),
                ] {
                    if r.get(key).is_some() {
                        let mapped = parsed(&r, key, json!([]))
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|old| {
                                ids.get(old.as_str().unwrap_or_default())
                                    .map(|id| json!(id))
                                    .unwrap_or_else(|| old.clone())
                            })
                            .collect::<Vec<_>>();
                        r[key] = json!(json!(mapped).to_string());
                        let _ = kind;
                    }
                }
                if table == "versions" {
                    r["generation_id"] = Value::Null;
                    if r["status"] == "generating" {
                        r["status"] = json!("interrupted");
                    }
                }
                if table == "narrative_snapshots" {
                    let mut snapshot = parsed(&r, "snapshot_json", json!({}));
                    remap_snapshot(&mut snapshot, &ids);
                    r["snapshot_json"] = json!(snapshot.to_string());
                    r["fingerprint"] = json!(rules::hash(&format!("{c}:{}", snapshot)));
                }
                self.insert_row(table, &r)?;
                match table {
                    "interactions" => {
                        if let Some(text) = r["user_text"].as_str() {
                            self.index(&c, "user", &s(&r, "id"), text)?;
                        }
                    }
                    "versions" => self.index(
                        &c,
                        "asst",
                        &s(&r, "id"),
                        r["content"].as_str().unwrap_or_default(),
                    )?,
                    "memories" => self.index(
                        &c,
                        "memory",
                        &s(&r, "id"),
                        &format!("{} {}", s(&r, "subject"), s(&r, "content")),
                    )?,
                    "episodes" => self.index(&c, "episode", &s(&r, "id"), &s(&r, "summary"))?,
                    _ => {}
                }
            }
        }
        self.invalidate(&c, None)?;
        self.conversation(&c, false)
    }
    pub(super) fn backup(&self) -> Result<Value> {
        let dir = self.dir.join("backups");
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!(
            "modularchat-{}.db{}",
            id(),
            if self.vault.is_some() { ".vault" } else { "" }
        ));
        if let Some(vault) = &self.vault {
            vault.backup(&self.db, &file)?;
        } else {
            sqlite_snapshot(&self.dir.join("modularchat.db"), &file)?;
        }
        Ok(json!({"file":file}))
    }
    pub(super) fn import_legacy(&self) -> Result<Value> {
        let dir = self.dir.join("chats");
        let backup = self.dir.join("backup").join(format!("legacy-{}", id()));
        std::fs::create_dir_all(&backup)?;
        let mut imported = Vec::new();
        let mut errors = Vec::new();
        let mut skipped = 0;
        if !dir.is_dir() {
            return Ok(
                json!({"imported":imported.len(),"failed":errors,"backup":backup,"skipped":skipped}),
            );
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let old = entry.file_name().to_string_lossy().into_owned();
            if !entry.file_type()?.is_dir() || !entry.path().join("metadata.json").is_file() {
                continue;
            }
            let backup_dir = backup.join(&old);
            std::fs::create_dir_all(&backup_dir)?;
            for name in ["metadata.json", "history.json", "character.txt"] {
                let source = entry.path().join(name);
                if source.is_file() {
                    std::fs::copy(source, backup_dir.join(name))?;
                }
            }
            if !self
                .one(
                    "SELECT legacy_id FROM legacy_imports WHERE legacy_id=?",
                    &[json!(old)],
                )?
                .is_null()
            {
                skipped += 1;
                continue;
            }
            let result=self.tx(||{
                if !self.one("SELECT id FROM conversations WHERE id=?",&[json!(old)])?.is_null(){return Err(Error::new(409,"Já existe conversa com esse ID."));}
                let metadata:Value=serde_json::from_slice(&std::fs::read(entry.path().join("metadata.json"))?).map_err(|_|Error::new(400,"metadata.json inválido."))?;
                let history:Vec<Value>=serde_json::from_slice(&std::fs::read(entry.path().join("history.json"))?).map_err(|_|Error::new(400,"history.json inválido."))?;
                let prompt=std::fs::read_to_string(entry.path().join("character.txt")).unwrap_or_default();
                let route=metadata["model"].as_str().unwrap_or("cbai/deepseek-v4.1-flash");let preset=crate::presets::for_route(route);
                self.create(&json!({"id":old,"name":metadata["name"].as_str().unwrap_or("Chat importado"),"persona":metadata["persona"].as_str().unwrap_or("Alex"),"routeId":route,"presetId":preset["id"],"effort":preset["defaultEffort"],"temperature":metadata["temperature"],"prompt":prompt,"promptSource":"migration","createdAt":metadata["createdAt"].as_str().unwrap_or(&now())}))?;
                let(mut parent,mut current)=(Value::Null,Value::Null);let(mut seq,mut idx)=(0,0);let mut expected=Vec::new();
                for m in history {let role=s(&m,"role");let Some(text)=m["content"].as_str()else{continue;};if !["user","assistant"].contains(&role.as_str()){continue;}
                    let time=m["ts"].as_str().unwrap_or(metadata["createdAt"].as_str().unwrap_or("1970-01-01T00:00:00.000Z"));
                    if role=="user"||current.is_null(){seq+=1;idx=0;let iid=if role=="user"{m["id"].as_str().map(str::to_owned).unwrap_or_else(id)}else{id()};
                        self.exec("INSERT INTO interactions(id,conv_id,parent_id,seq,kind,user_text,user_ts,user_edited,created_at) VALUES(?,?,?,?,?,?,?,?,?)",&[json!(iid),json!(old),parent.clone(),json!(seq),json!(if role=="user"{"turn"}else{"opening"}),if role=="user"{json!(text)}else{Value::Null},json!(time),json!(m["edited"].as_bool().unwrap_or(false)),json!(time)])?;current=json!(iid);parent=current.clone();
                        if role=="user"{self.index(&old,"user",&iid,text)?;expected.push(json!({"id":iid,"role":role,"content":text}));continue;}
                    }
                    let pid=m["id"].as_str().map(str::to_owned).unwrap_or_else(id);let vid=id();
                    self.exec("INSERT INTO parts(id,interaction_id,idx,selected_version_id,created_at) VALUES(?,?,?,?,?)",&[json!(pid),current.clone(),json!(idx),json!(vid),json!(time)])?;idx+=1;
                    self.exec("INSERT INTO versions(id,part_id,content,status,origin,route_id,created_at,finished_at) VALUES(?,?,?,'complete','legacy',?,?,?)",&[json!(vid),json!(pid),json!(text),m["model"].clone(),json!(time),json!(time)])?;self.index(&old,"asst",&vid,text)?;expected.push(json!({"id":pid,"role":role,"content":text}));
                }
                self.exec("UPDATE conversations SET head_interaction_id=? WHERE id=?",&[parent,json!(old)])?;self.invalidate(&old,None)?;
                let actual=self.messages(&old)?;if actual.len()!=expected.len()||actual.iter().zip(&expected).any(|(a,e)|a["content"]!=e["content"]||a["role"]!=e["role"]||a["id"]!=e["id"]){return Err(Error::new(409,"Conferência do legado falhou."));}
                self.exec("INSERT INTO legacy_imports(legacy_id,conv_id,imported_at,report_json) VALUES(?,?,?,?)",&[json!(old),json!(old),json!(now()),json!(json!({"messages":actual.len()}).to_string())])?;Ok(json!(old))
            });
            match result {
                Ok(c) => imported.push(c),
                Err(e) => errors.push(json!({"id":old,"error":e.message})),
            }
        }
        Ok(json!({"imported":imported.len(),"failed":errors,"backup":backup,"skipped":skipped}))
    }
}

pub fn sqlite_snapshot(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        return Err(Error::new(409, "O destino do backup já existe."));
    }
    let source =
        rusqlite::Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(Duration::from_secs(5))?;
    source.execute_batch("BEGIN")?;
    let _: i64 = source.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
    let mut target = rusqlite::Connection::open(destination)?;
    rusqlite::backup::Backup::new(&source, &mut target)?.run_to_completion(
        128,
        Duration::from_millis(10),
        None,
    )?;
    let check: String = target.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if check != "ok" {
        return Err(Error::new(500, "Backup falhou na integridade."));
    }
    for table in std::iter::once(&"conversations")
        .chain(ID_TABLES.iter())
        .chain(
            [
                "personas",
                "generations",
                "jobs",
                "operations",
                "text_chunks",
            ]
            .iter(),
        )
    {
        let exists: i64 = source.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?",
            [table],
            |r| r.get(0),
        )?;
        if exists == 0 {
            continue;
        }
        let key = if *table == "operations" {
            "conv_id||':'||op_id"
        } else {
            "id"
        };
        let sql = format!("SELECT {key} FROM {table} ORDER BY 1");
        let mut a = source.prepare(&sql)?;
        let mut b = target.prepare(&sql)?;
        let a = a
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let b = b
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if a != b {
            return Err(Error::new(
                500,
                format!("IDs do snapshot divergem em {table}."),
            ));
        }
    }
    source.execute_batch("ROLLBACK")?;
    Ok(())
}
fn remap_snapshot(v: &mut Value, ids: &HashMap<String, String>) {
    match v {
        Value::Object(map) => {
            for (k, v) in map {
                if ["id", "sourceId", "interactionId"].contains(&k.as_str()) {
                    if let Some(new) = v.as_str().and_then(|s| ids.get(s)) {
                        *v = json!(new);
                    }
                } else if k == "versionIds" {
                    if let Some(a) = v.as_array_mut() {
                        for v in a {
                            if let Some(new) = v.as_str().and_then(|s| ids.get(s)) {
                                *v = json!(new);
                            }
                        }
                    }
                } else {
                    remap_snapshot(v, ids);
                }
            }
        }
        Value::Array(a) => {
            for v in a {
                remap_snapshot(v, ids)
            }
        }
        _ => {}
    }
}
pub(super) fn validate(data: &Value) -> Result<()> {
    validate_sync(data, &HashMap::new())
}
pub(super) fn validate_sync(data: &Value, removed: &HashMap<&str, HashSet<&str>>) -> Result<()> {
    let fail = || Error::new(400, "Dados/IDs/referências da importação inválidos.");
    let c = data["conversation"]["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(fail)?;
    if !data["conversation"]["route_id"].is_string()
        || !data["conversation"]["preset_id"].is_string()
    {
        return Err(fail());
    }
    let mut ids = HashMap::<&str, HashSet<&str>>::new();
    let mut all = HashSet::from([c]);
    for &table in ID_TABLES {
        let mut set = HashSet::new();
        if !data[table].is_null() && !data[table].is_array() {
            return Err(fail());
        }
        for r in data[table].as_array().into_iter().flatten() {
            let id = r["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(fail)?;
            if !all.insert(id) || r["conv_id"].as_str().is_some_and(|id| id != c) {
                return Err(fail());
            }
            set.insert(id);
        }
        ids.insert(table, set);
    }
    for (&table, tombstones) in removed {
        if let Some(set) = ids.get_mut(table) {
            set.extend(tombstones);
        }
    }
    let check = |v: &Value, table: &str| -> Result<()> {
        if !v.is_null() && !v.as_str().is_some_and(|s| ids[table].contains(s)) {
            return Err(fail());
        }
        Ok(())
    };
    check(&data["conversation"]["head_interaction_id"], "interactions")?;
    let inter: HashMap<_, _> = data["interactions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| (s(r, "id"), r))
        .collect();
    for i in inter.values() {
        check(&i["parent_id"], "interactions")?;
        if n(i, "seq") < 1 {
            return Err(fail());
        }
        let mut at = s(i, "id");
        let mut seen = HashSet::new();
        while !at.is_empty() {
            if !seen.insert(at.clone()) {
                return Err(fail());
            }
            if let Some(row) = inter.get(&at) {
                at = s(row, "parent_id");
            } else if removed.get("interactions").is_some_and(|r| r.contains(at.as_str())) {
                break;
            } else {
                return Err(fail());
            }
        }
        if let Some(parent) = i["parent_id"].as_str()
            && inter.get(parent).is_some_and(|row| n(row, "seq") >= n(i, "seq"))
        {
            return Err(fail());
        }
    }
    for r in data["parts"].as_array().into_iter().flatten() {
        check(&r["interaction_id"], "interactions")?;
        check(&r["selected_version_id"], "versions")?;
        if !r["selected_version_id"].is_null()
            && !data["versions"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|v| v["id"] == r["selected_version_id"] && v["part_id"] == r["id"])
        {
            return Err(fail());
        }
    }
    for r in data["versions"].as_array().into_iter().flatten() {
        check(&r["part_id"], "parts")?;
        if !r["content"].is_string() {
            return Err(fail());
        }
    }
    for &table in TABLES {
        for r in data[table].as_array().into_iter().flatten() {
            if r["conv_id"].as_str().is_some_and(|id| id != c) {
                return Err(fail());
            }
            for key in [
                "source_interaction_id",
                "interaction_id",
                "first_interaction_id",
                "opened_interaction_id",
                "resolved_interaction_id",
            ] {
                check(&r[key], "interactions")?;
            }
            for key in ["source_version_id", "version_id"] {
                check(&r[key], "versions")?;
            }
            for key in ["memory_id", "old_id", "new_id"] {
                check(&r[key], "memories")?;
            }
            for (key, t) in [
                ("version_ids_json", "versions"),
                ("interaction_ids_json", "interactions"),
            ] {
                if let Some(text) = r[key].as_str() {
                    let a: Vec<Value> = serde_json::from_str(text).map_err(|_| fail())?;
                    for v in a {
                        check(&v, t)?;
                    }
                }
            }
        }
    }
    Ok(())
}
