use axum::{body::Body, extract::ConnectInfo, http::Request as HttpRequest};
use base64::{Engine, engine::general_purpose::STANDARD};
use http_body_util::BodyExt;
use modularchat_server::{
    api::{self, AppState},
    config::Config,
    rules,
    store::{
        FinishRequest, Request, Store, TurnRequest,
        sync::{self, snapshot_hash},
    },
};
use serde_json::{Value, json};
use tower::ServiceExt;

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn model() -> String {
    rules::hash("portable-model-fixture")
}
fn temp() -> tempfile::TempDir {
    let root = std::env::temp_dir().join("Cline/modularchat-autohost-v4-sync");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("fixture-")
        .tempdir_in(root)
        .unwrap()
}
struct Fixture {
    store: Store,
    dir: tempfile::TempDir,
    server: Value,
}
impl Fixture {
    async fn new() -> Self {
        let dir = temp();
        let store = Store::open_encrypted(dir.path()).unwrap();
        let server = store.call(Request::SyncStatus(json!({}))).await.unwrap()["serverId"].clone();
        Self { store, dir, server }
    }
    async fn close(&self) {
        self.store.call(Request::Close).await.unwrap();
    }
    async fn status(&self) -> Value {
        self.store
            .call(Request::SyncStatus(json!({})))
            .await
            .unwrap()
    }
    async fn delta(&self, p: Value) -> modularchat_server::Result<Value> {
        self.store
            .call(Request::SyncDelta {
                value: p,
                embedding_model: model(),
            })
            .await
    }
    async fn create(&self) -> String {
        self.store.call(Request::CreateConversation(json!({
            "name":"Sync fixture","routeId":"fixture-route","presetId":"generic","prompt":"Ficha inicial"
        }))).await.unwrap()["id"].as_str().unwrap().into()
    }
}
fn envelope(peer: &Value) -> Value {
    json!({"format":"modularchat-sync","version":1,"peerId":peer,"known":[],"limit":4})
}
async fn offer(from: &Fixture, peer: &Value, c: &str) -> Value {
    let mut p = envelope(peer);
    p["convIds"] = json!([c]);
    let out = from.delta(p).await.unwrap();
    assert_eq!(out["snapshots"].as_array().unwrap().len(), 1, "{out}");
    out["snapshots"][0].clone()
}
fn push(entry: &Value, base: Value) -> Value {
    json!({"convId":entry["convId"],"baseHash":base,"hash":entry["hash"],"snapshot":entry["snapshot"]})
}
fn batch(peer: &Value, entries: Vec<Value>) -> Value {
    let mut p = envelope(peer);
    p["batchId"] = json!(id());
    p["convIds"] = json!(
        entries
            .iter()
            .map(|e| e["convId"].clone())
            .collect::<Vec<_>>()
    );
    p["pushes"] = json!(entries);
    p
}
async fn mirror(pc: &Fixture, phone: &Fixture, c: &str) -> Value {
    let first = offer(pc, &phone.server, c).await;
    let out = phone
        .delta(batch(&pc.server, vec![push(&first, Value::Null)]))
        .await
        .unwrap();
    assert!(out["conflicts"].as_array().unwrap().is_empty(), "{out}");
    assert_eq!(out["accepted"][0]["hash"], first["hash"]);
    first
}
async fn turn(f: &Fixture, c: &str, text: &str, op: &str) -> Value {
    let request: TurnRequest =
        serde_json::from_value(json!({"convId":c,"mode":"send","content":text,
        "opId":op,"request":{"apiKey":"GENERATION-SECRET","policy":{"token":"PRIVATE"}}}))
        .unwrap();
    let g = f.store.call(Request::BeginTurn(request)).await.unwrap();
    f.store
        .call(Request::FinishGeneration(FinishRequest {
            generation_id: g["generationId"].as_str().unwrap().into(),
            status: "complete".into(),
            text: Some(format!("Mira: {text}")),
            usage: json!({"input":5,"output":8,"apiKey":"USAGE-SECRET"}),
            error: None,
        }))
        .await
        .unwrap();
    g
}
async fn memory(f: &Fixture, c: &str, text: &str) -> Value {
    f.store
        .call(Request::AddMemory {
            conv_id: c.into(),
            value: json!({"kind":"WORLD_FACT","content":text}),
        })
        .await
        .unwrap()
}
async fn finish_jobs(f: &Fixture, peer: &Value, c: &str) {
    let data = offer(f, peer, c).await;
    for row in data["snapshot"]["jobs"].as_array().unwrap() {
        if row["status"] == "pending" {
            f.store
                .call(Request::FinishJob {
                    id: row["id"].as_str().unwrap().into(),
                    status: "done".into(),
                    error: None,
                })
                .await
                .unwrap();
        }
    }
}
fn rehash(entry: &mut Value) {
    entry["hash"] = json!(snapshot_hash(&entry["snapshot"]));
}
fn manifest(status: &Value, c: &str) -> Value {
    status["manifests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["convId"] == c)
        .unwrap()
        .clone()
}

#[tokio::test]
async fn mirror_offline_phone_pc_curation_embedding_pull_and_idempotent_recovery() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    turn(&pc, &c, "Baseline", &id()).await;
    finish_jobs(&pc, &phone.server, &c).await;
    let first = mirror(&pc, &phone, &c).await;
    let op = id();
    let mobile = turn(&phone, &c, "Turno offline", &op).await;
    let note = memory(&phone, &c, "Nota offline estável.").await;
    let job = phone
        .store
        .call(Request::EnqueueJob(json!({"convId":c,"kind":"cast"})))
        .await
        .unwrap();
    let incoming = offer(&phone, &pc.server, &c).await;
    let request = batch(&phone.server, vec![push(&incoming, first["hash"].clone())]);
    let saves = pc.store.call(Request::PersistStats).await.unwrap();
    let ack = pc.delta(request.clone()).await.unwrap();
    assert_eq!(ack["ackBatch"], request["batchId"]);
    assert_eq!(ack["accepted"][0]["hash"], incoming["hash"]);
    assert_eq!(pc.delta(request.clone()).await.unwrap(), ack);
    let persisted = pc.store.call(Request::PersistStats).await.unwrap();
    assert!(
        persisted["immediateSaves"].as_u64().unwrap() > saves["immediateSaves"].as_u64().unwrap()
    );
    assert_eq!(persisted["pending"], false);
    let saved = offer(&pc, &phone.server, &c).await;
    assert_eq!(saved["snapshot"], incoming["snapshot"]);
    assert!(
        saved["snapshot"]["memories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == note["id"])
    );
    assert!(
        saved["snapshot"]["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == job["id"])
    );
    let repeat: TurnRequest = serde_json::from_value(
        json!({"convId":c,"mode":"send","content":"Não duplicar","opId":op}),
    )
    .unwrap();
    let replay = pc.store.call(Request::BeginTurn(repeat)).await.unwrap();
    assert_eq!(replay["generationId"], mobile["generationId"]);
    assert_eq!(offer(&pc, &phone.server, &c).await["hash"], saved["hash"]);
    let a = memory(&pc, &c, "A academia fica numa ilha.").await;
    let b = memory(&pc, &c, "A ilha possui a academia.").await;
    pc.store
        .call(Request::ApplyCuration(json!({"convId":c,"decisions":[{
            "keepId":a["id"],"dropIds":[b["id"]],"merged":"A academia fica numa ilha."
        }]})))
        .await
        .unwrap();
    pc.store
        .call(Request::SaveEmbedding {
            conv_id: c.clone(),
            owner_type: "memory".into(),
            owner_id: a["id"].as_str().unwrap().into(),
            model: model(),
            source_text: None,
            vector: vec![0.25, -0.5, 1.0],
        })
        .await
        .unwrap();
    pc.store
        .call(Request::SetCharacter {
            conv_id: c.clone(),
            text: "Ficha revisada no PC".into(),
            source: "app".into(),
        })
        .await
        .unwrap();
    phone
        .store
        .call(Request::WritePrivateCharacter {
            conv_id: c.clone(),
            text: "Ficha inicial".into(),
        })
        .await
        .unwrap();
    finish_jobs(&pc, &phone.server, &c).await;
    let curated = offer(&pc, &phone.server, &c).await;
    let pulled = phone
        .delta(batch(
            &pc.server,
            vec![push(&curated, incoming["hash"].clone())],
        ))
        .await
        .unwrap();
    assert!(
        pulled["conflicts"].as_array().unwrap().is_empty(),
        "{pulled}"
    );
    let mirrored = offer(&phone, &pc.server, &c).await;
    assert_eq!(mirrored["hash"], curated["hash"]);
    assert_eq!(
        mirrored["snapshot"]["embeddings"],
        curated["snapshot"]["embeddings"]
    );
    assert!(
        !mirrored["snapshot"]["memory_replacements"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        phone
            .store
            .call(Request::ReadPrivateCharacter(c.clone()))
            .await
            .unwrap(),
        "Ficha revisada no PC"
    );
    assert_eq!(
        phone
            .store
            .call(Request::GetConversation(c.clone()))
            .await
            .unwrap()["history"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert!(
        phone
            .store
            .call(Request::Search {
                query: "offline".into(),
                conv_id: Some(c.clone()),
                limit: 20
            })
            .await
            .unwrap()
            .as_array()
            .is_some_and(|a| !a.is_empty())
    );
    assert!(
        pc.store.call(Request::ClaimJob).await.unwrap().is_null(),
        "Completed jobs must not execute again"
    );
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn independent_append_only_memories_union_without_clock_lww() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    let first = mirror(&pc, &phone, &c).await;
    let a = memory(&pc, &c, "Memória PC").await;
    let b = memory(&phone, &c, "Memória mobile").await;
    let incoming = offer(&phone, &pc.server, &c).await;
    let out = pc
        .delta(batch(
            &phone.server,
            vec![push(&incoming, first["hash"].clone())],
        ))
        .await
        .unwrap();
    assert!(out["conflicts"].as_array().unwrap().is_empty(), "{out}");
    let data = offer(&pc, &phone.server, &c).await;
    assert_eq!(data["snapshot"]["memories"].as_array().unwrap().len(), 2);
    for id in [&a["id"], &b["id"]] {
        assert!(
            data["snapshot"]["memories"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| &r["id"] == id)
        );
    }
    let pulled = phone
        .delta(batch(
            &pc.server,
            vec![push(&data, incoming["hash"].clone())],
        ))
        .await
        .unwrap();
    assert!(
        pulled["conflicts"].as_array().unwrap().is_empty(),
        "{pulled}"
    );
    assert_eq!(offer(&phone, &pc.server, &c).await["hash"], data["hash"]);
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn concurrent_divergent_turns_conflict_atomically_and_keep_both_stores() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    let first = mirror(&pc, &phone, &c).await;
    turn(&pc, &c, "PC branch", &id()).await;
    turn(&phone, &c, "Phone branch", &id()).await;
    let before = offer(&pc, &phone.server, &c).await;
    let incoming = offer(&phone, &pc.server, &c).await;
    let other = phone.create().await;
    let other_data = offer(&phone, &pc.server, &other).await;
    let request = batch(
        &phone.server,
        vec![
            push(&other_data, Value::Null),
            push(&incoming, first["hash"].clone()),
        ],
    );
    let out = pc.delta(request.clone()).await.unwrap();
    assert_eq!(out["code"], "CONFLICT");
    assert!(out["ackBatch"].is_null());
    assert!(out["accepted"].as_array().unwrap().is_empty());
    assert_eq!(pc.delta(request).await.unwrap(), out);
    assert_eq!(offer(&pc, &phone.server, &c).await["hash"], before["hash"]);
    assert_eq!(
        offer(&phone, &pc.server, &c).await["hash"],
        incoming["hash"]
    );
    assert_eq!(
        pc.store
            .call(Request::GetConversation(other))
            .await
            .unwrap_err()
            .status,
        404
    );
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn same_memory_uuid_divergence_is_explicit_conflict() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    let m = memory(&pc, &c, "Original").await;
    let first = mirror(&pc, &phone, &c).await;
    for (f, text) in [(&pc, "Curadoria PC"), (&phone, "Curadoria phone")] {
        f.store
            .call(Request::PatchMemory {
                conv_id: c.clone(),
                memory_id: m["id"].as_str().unwrap().into(),
                value: json!({"content":text}),
            })
            .await
            .unwrap();
    }
    let before = offer(&pc, &phone.server, &c).await;
    let incoming = offer(&phone, &pc.server, &c).await;
    let out = pc
        .delta(batch(
            &phone.server,
            vec![push(&incoming, first["hash"].clone())],
        ))
        .await
        .unwrap();
    assert_eq!(out["code"], "CONFLICT");
    assert_eq!(offer(&pc, &phone.server, &c).await["hash"], before["hash"]);
    assert_eq!(
        offer(&phone, &pc.server, &c).await["hash"],
        incoming["hash"]
    );
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn busy_generation_and_running_worker_are_retryable_without_partial_apply() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    let first = mirror(&pc, &phone, &c).await;
    memory(&phone, &c, "Offline").await;
    let incoming = offer(&phone, &pc.server, &c).await;
    let req: TurnRequest = serde_json::from_value(
        json!({"convId":c,"mode":"send","content":"Em andamento","opId":id()}),
    )
    .unwrap();
    let g = pc.store.call(Request::BeginTurn(req)).await.unwrap();
    let request = batch(&phone.server, vec![push(&incoming, first["hash"].clone())]);
    let err = pc.delta(request.clone()).await.unwrap_err();
    assert_eq!(err.status, 409);
    assert_eq!(err.extra["code"], "BUSY");
    assert_eq!(err.extra["retryable"], true);
    assert!(
        pc.store.call(Request::Memories(c.clone())).await.unwrap()["memories"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    pc.store
        .call(Request::FinishGeneration(FinishRequest {
            generation_id: g["generationId"].as_str().unwrap().into(),
            status: "complete".into(),
            text: Some("Mira: Final.".into()),
            usage: json!({}),
            error: None,
        }))
        .await
        .unwrap();
    let claimed = pc.store.call(Request::ClaimJob).await.unwrap();
    assert!(!claimed.is_null());
    let err = pc.delta(request).await.unwrap_err();
    assert_eq!(err.extra["code"], "BUSY");
    let other = phone.create().await;
    let other_data = offer(&phone, &pc.server, &other).await;
    let unaffected = pc
        .delta(batch(&phone.server, vec![push(&other_data, Value::Null)]))
        .await
        .unwrap();
    assert!(!unaffected["ackBatch"].is_null());
    pc.store
        .call(Request::FinishJob {
            id: claimed["id"].as_str().unwrap().into(),
            status: "done".into(),
            error: None,
        })
        .await
        .unwrap();
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn soft_delete_restoration_requires_the_shared_tombstone_and_purge_never_revives() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    let first = mirror(&pc, &phone, &c).await;
    pc.store
        .call(Request::DeleteConversation(c.clone()))
        .await
        .unwrap();
    let deleted = offer(&pc, &phone.server, &c).await;
    assert_eq!(manifest(&pc.status().await, &c)["deleted"], true);
    phone
        .delta(batch(
            &pc.server,
            vec![push(&deleted, first["hash"].clone())],
        ))
        .await
        .unwrap();
    assert_eq!(offer(&phone, &pc.server, &c).await["hash"], deleted["hash"]);
    let stale = pc
        .delta(batch(
            &phone.server,
            vec![push(&first, first["hash"].clone())],
        ))
        .await
        .unwrap();
    assert!(!stale["ackBatch"].is_null());
    assert_eq!(offer(&pc, &phone.server, &c).await["hash"], deleted["hash"]);
    phone
        .store
        .call(Request::RestoreConversation(c.clone()))
        .await
        .unwrap();
    let restored = offer(&phone, &pc.server, &c).await;
    let out = pc
        .delta(batch(
            &phone.server,
            vec![push(&restored, deleted["hash"].clone())],
        ))
        .await
        .unwrap();
    assert!(out["conflicts"].as_array().unwrap().is_empty(), "{out}");
    assert_eq!(manifest(&pc.status().await, &c)["deleted"], false);
    pc.store
        .call(Request::DeleteConversation(c.clone()))
        .await
        .unwrap();
    pc.store
        .call(Request::PurgeTrash(json!({"type":"conversation","id":c})))
        .await
        .unwrap();
    let tomb = offer(&pc, &phone.server, &c).await;
    assert_eq!(tomb["snapshot"]["purged"], true);
    let out = phone
        .delta(batch(
            &pc.server,
            vec![push(&tomb, restored["hash"].clone())],
        ))
        .await
        .unwrap();
    assert!(out["conflicts"].as_array().unwrap().is_empty(), "{out}");
    assert_eq!(manifest(&phone.status().await, &c)["purged"], true);
    let rejected = pc
        .delta(batch(
            &phone.server,
            vec![push(&restored, restored["hash"].clone())],
        ))
        .await
        .unwrap();
    assert_eq!(rejected["code"], "CONFLICT");
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn deletion_concurrent_with_offline_turn_conflicts_preserving_mobile_turn() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    let first = mirror(&pc, &phone, &c).await;
    pc.store
        .call(Request::DeleteConversation(c.clone()))
        .await
        .unwrap();
    turn(&phone, &c, "Não perder este turno", &id()).await;
    let incoming = offer(&phone, &pc.server, &c).await;
    let before = pc.status().await;
    let out = pc
        .delta(batch(
            &phone.server,
            vec![push(&incoming, first["hash"].clone())],
        ))
        .await
        .unwrap();
    assert_eq!(out["conflicts"][0]["reason"], "DELETE_DIVERGED");
    assert_eq!(pc.status().await, before);
    assert_eq!(
        phone.store.call(Request::GetConversation(c)).await.unwrap()["history"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn reextracting_relationship_replacement_keeps_prior_relation_syncable() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    let first = turn(&pc, &c, "Primeira fonte", &id()).await;
    let extract = |generation: &Value, relationships: Value| Request::ApplyExtraction(json!({
        "convId":c,"interactionId":generation["interactionId"],"versionId":generation["versionId"],
        "extraction":{"summary":"fixture","facts":[],"cast":[],"relationships":relationships,"openThreads":[],"resolvedThreads":[]}
    }));
    pc.store.call(extract(&first, json!([{"a":"Mira","b":"Alex","description":"São amigos próximos desde crianças."}]))).await.unwrap();
    let second = turn(&pc, &c, "Segunda fonte", &id()).await;
    pc.store.call(extract(&second, json!([{"a":"Mira","b":"Alex","description":"Tornaram-se adversários por uma disputa política."}]))).await.unwrap();
    let before = offer(&pc, &phone.server, &c).await;
    assert_eq!(before["snapshot"]["relationships"].as_array().unwrap().len(), 2);
    assert!(before["snapshot"]["relationships"].as_array().unwrap().iter().any(|r| !r["superseded_by"].is_null()));
    pc.store.call(extract(&second, json!([]))).await.unwrap();
    let snapshot = mirror(&pc, &phone, &c).await;
    let rows = snapshot["snapshot"]["relationships"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["source_interaction_id"], first["interactionId"]);
    assert_eq!(rows[0]["superseded_by"], Value::Null);
    assert_eq!(rows[0]["status"], "current");
    assert_eq!(offer(&phone, &pc.server, &c).await["snapshot"], snapshot["snapshot"]);
    pc.close().await; phone.close().await;
}

#[tokio::test]
async fn legacy_dangling_relationship_is_canonical_on_wire_without_erasing_vault_data() {
    let mut pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    pc.close().await;
    let missing = id(); let relation = id();
    {
        let (vault, db) = modularchat_server::privacy::Vault::open(pc.dir.path()).unwrap();
        db.execute("INSERT INTO relationships(id,conv_id,a_key,a_name,b_key,b_name,description,status,superseded_by,created_at) VALUES(?1,?2,'mira','Mira','alex','Alex','Relação preservada','historical',?3,?4)", rusqlite::params![relation,c,missing,modularchat_server::now()]).unwrap();
        vault.save(&db).unwrap();
    }
    let audit = Store::audit_sync_snapshot(&pc.dir.path().join("modularchat.db.vault")).unwrap();
    assert_eq!(audit["ok"], true);
    pc.store = Store::open_encrypted(pc.dir.path()).unwrap();
    let snapshot = mirror(&pc, &phone, &c).await;
    assert_eq!(snapshot["snapshot"]["relationships"][0]["id"], relation);
    assert_eq!(snapshot["snapshot"]["relationships"][0]["description"], "Relação preservada");
    assert_eq!(snapshot["snapshot"]["relationships"][0]["superseded_by"], Value::Null);
    let source = modularchat_server::privacy::read_snapshot(&pc.dir.path().join("modularchat.db.vault")).unwrap();
    assert_eq!(source.query_row("SELECT superseded_by FROM relationships WHERE id=?", [&relation], |row| row.get::<_,String>(0)).unwrap(), missing);
    let mut malformed = snapshot.clone();
    malformed["snapshot"]["relationships"][0]["superseded_by"] = json!(id()); rehash(&mut malformed);
    let error = phone.delta(batch(&pc.server, vec![push(&malformed,snapshot["hash"].clone())])).await.unwrap_err();
    assert_eq!(error.extra["code"], "INVALID_SYNC");
    assert_eq!(error.extra["targetTable"], "relationships");
    pc.close().await; phone.close().await;
}

#[tokio::test]
async fn malformed_references_tables_columns_provenance_and_private_json_are_rejected() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    turn(&pc, &c, "Fonte", &id()).await;
    let first = mirror(&pc, &phone, &c).await;
    let before = phone.status().await;
    let mut bad = Vec::new();
    let mut e = first.clone();
    e["snapshot"]["meta"] = json!([{"key":"mobile_providers_v1","value":"TOKEN"}]);
    bad.push(e);
    let mut e = first.clone();
    e["snapshot"]["conversation"]["head_interaction_id"] = json!(id());
    bad.push(e);
    let mut e = first.clone();
    e["snapshot"]["interactions"][0]["parent_id"] = e["snapshot"]["interactions"][0]["id"].clone();
    bad.push(e);
    let mut e = first.clone();
    e["snapshot"]["versions"][0]["part_id"] = json!(id());
    bad.push(e);
    let mut e = first.clone();
    e["snapshot"]["interactions"][0]["conv_id"] = json!(id());
    bad.push(e);
    let mut e = first.clone();
    e["snapshot"]["generations"][0]["request_json"] = json!("{\"apiKey\":\"TOKEN\"}");
    bad.push(e);
    let mut e = first.clone();
    e["snapshot"]["conversation"]["settings_json"] = json!("{\"apiKey\":\"TOKEN\"}");
    bad.push(e);
    let mut e = first.clone();
    e["snapshot"]["jobs"][0]["payload_json"] = json!("{\"apiKey\":\"TOKEN\"}");
    bad.push(e);
    let mut e = first.clone();
    e["snapshot"]["jobs"][0]["status"] = json!("running");
    bad.push(e);
    let mut e = first.clone();
    e["snapshot"]["character_revisions"][0]["hash"] = json!(rules::hash("Wrong card"));
    bad.push(e);
    let mut e = first.clone();
    e["snapshot"]["character_revisions"] = json!([]);
    bad.push(e);
    let mut e = first.clone();
    e["snapshot"]["memories"] = Value::Null;
    bad.push(e);
    let mut e = first.clone();
    e["snapshot"]["full"] = json!(false);
    bad.push(e);
    let mut e = first.clone();
    e["snapshot"]["conversation"]["name"] = json!({"malformed":true});
    bad.push(e);
    let mut e = first.clone();
    e["snapshot"]["conversation"]["deleted_at"] = json!("INVALID");
    bad.push(e);
    for mut entry in bad {
        rehash(&mut entry);
        let err = phone
            .delta(batch(&pc.server, vec![push(&entry, first["hash"].clone())]))
            .await
            .unwrap_err();
        assert_eq!(err.status, 400, "{err}");
    }
    let mut p = envelope(&pc.server);
    p["tables"] = json!(["devices"]);
    assert_eq!(phone.delta(p).await.unwrap_err().status, 400);
    assert_eq!(phone.status().await, before);
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn credentials_devices_policy_diagnostics_and_rebuildable_tables_do_not_cross() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c=pc.store.call(Request::CreateConversation(json!({"name":"Secret fixture","routeId":"fixture","presetId":"generic",
        "prompt":"Somente narrativa","settings":{"apiKey":"SETTINGS-SECRET","deviceToken":"DEVICE-SECRET","thinkingEnabled":false}}))).await.unwrap()["id"].as_str().unwrap().to_owned();
    pc.store.call(Request::SaveMobileProviders(modularchat_server::providers::MobileProviders::parse(&json!({
        "providers":[{"id":"fixture","baseUrl":"https://sync-fixture.invalid/v1","apiKey":"PROVIDER-SECRET","models":["fixture-model"]}]
    })).unwrap())).await.unwrap();
    pc.store
        .call(Request::SaveMobilePolicy(
            modularchat_server::memory_backend::MemoryPolicy::default(),
        ))
        .await
        .unwrap();
    pc.store
        .call(Request::AddDevice {
            name: "PC private device".into(),
            token_hash: "DEVICE-SECRET".into(),
        })
        .await
        .unwrap();
    phone
        .store
        .call(Request::AddDevice {
            name: "Phone private device".into(),
            token_hash: "PHONE-TOKEN".into(),
        })
        .await
        .unwrap();
    pc.store
        .call(Request::WritePrivateCharacter {
            conv_id: c.clone(),
            text: "SIDECAR-SECRET".into(),
        })
        .await
        .unwrap();
    turn(&pc, &c, "Conteúdo autorizado", &id()).await;
    let first = mirror(&pc, &phone, &c).await;
    let wire = first.to_string();
    for secret in [
        "SETTINGS-SECRET",
        "DEVICE-SECRET",
        "GENERATION-SECRET",
        "USAGE-SECRET",
        "PROVIDER-SECRET",
        "SIDECAR-SECRET",
        "mobile_providers_v1",
        "mobile_policy_v1",
        "request_json",
        "context_json",
        "text_chunks",
        "narrative_snapshots",
        "search_fts",
        "token_hash",
    ] {
        assert!(!wire.contains(secret), "Leak: {secret}");
    }
    assert_eq!(
        first["snapshot"]["conversation"]["settings_json"],
        "{\"thinkingEnabled\":false}"
    );
    let devices = phone.store.call(Request::Devices).await.unwrap();
    assert!(
        phone
            .store
            .call(Request::LoadMobileProviders)
            .await
            .unwrap()
            .is_null()
    );
    assert!(
        phone
            .store
            .call(Request::LoadMobilePolicy)
            .await
            .unwrap()
            .is_null()
    );
    assert_eq!(devices.as_array().unwrap().len(), 1);
    assert_eq!(devices[0]["name"], "Phone private device");
    assert_eq!(
        pc.store
            .call(Request::GetConversation(c.clone()))
            .await
            .unwrap()["settings"]["apiKey"],
        "SETTINGS-SECRET"
    );
    assert!(!phone.dir.path().join("modularchat.db").exists());
    let encrypted = std::fs::read(phone.dir.path().join("modularchat.db.vault")).unwrap();
    assert!(
        !encrypted
            .windows("Conteúdo autorizado".len())
            .any(|b| b == "Conteúdo autorizado".as_bytes())
    );
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn incompatible_embeddings_skip_recompute_and_bad_vectors_are_rejected() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    let m = memory(&pc, &c, "Memória vetorizada").await;
    pc.store
        .call(Request::SaveEmbedding {
            conv_id: c.clone(),
            owner_type: "memory".into(),
            owner_id: m["id"].as_str().unwrap().into(),
            model: rules::hash("other-model"),
            source_text: None,
            vector: vec![0.0, 1.0],
        })
        .await
        .unwrap();
    let first = offer(&pc, &phone.server, &c).await;
    let out = phone
        .delta(batch(&pc.server, vec![push(&first, Value::Null)]))
        .await
        .unwrap();
    assert_eq!(out["ignoredEmbeddings"][0]["reason"], "fingerprint");
    assert!(
        offer(&phone, &pc.server, &c).await["snapshot"]["embeddings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !phone
            .store
            .call(Request::PendingEmbeddings {
                limit: 20,
                model: model()
            })
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    for (dim, bytes, owner) in [
        (2, vec![0u8; 4], m["id"].clone()),
        (1, f32::NAN.to_le_bytes().to_vec(), m["id"].clone()),
        (8193, vec![0u8; 4], m["id"].clone()),
        (1, vec![0u8; 4], json!(id())),
    ] {
        let mut bad = first.clone();
        let row = &mut bad["snapshot"]["embeddings"][0];
        row["dim"] = json!(dim);
        row["vec"] = json!(STANDARD.encode(bytes));
        row["owner_id"] = owner;
        rehash(&mut bad);
        assert_eq!(
            phone
                .delta(batch(&pc.server, vec![push(&bad, first["hash"].clone())]))
                .await
                .unwrap_err()
                .status,
            400
        );
    }
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn bounded_snapshot_pages_never_truncate_history_and_oversized_push_rolls_back() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let mut ids = Vec::new();
    for _ in 0..3 {
        let c = pc.create().await;
        pc.store
            .call(Request::SetCharacter {
                conv_id: c.clone(),
                text: format!("{}TAIL", "x".repeat(2_900_000)),
                source: "app".into(),
            })
            .await
            .unwrap();
        ids.push(c);
    }
    let p = envelope(&phone.server);
    let out = pc.delta(p.clone()).await.unwrap();
    assert_eq!(out["snapshots"].as_array().unwrap().len(), 2);
    assert_eq!(out["hasMore"], true);
    assert!(out.to_string().len() <= sync::MAX_BYTES);
    for entry in out["snapshots"].as_array().unwrap() {
        assert_eq!(
            entry["snapshot"]["character_revisions"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["text"]
                .as_str()
                .unwrap()
                .len(),
            2_900_004
        );
        assert!(
            entry["snapshot"]["character_revisions"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["text"]
                .as_str()
                .unwrap()
                .ends_with("TAIL")
        );
    }
    let mut next = p;
    next["after"] = out["nextCursor"].clone();
    let next = pc.delta(next).await.unwrap();
    assert_eq!(next["snapshots"].as_array().unwrap().len(), 1);
    assert_eq!(next["hasMore"], false);
    let before = phone.status().await;
    let combined = batch(
        &pc.server,
        out["snapshots"]
            .as_array()
            .unwrap()
            .iter()
            .chain(next["snapshots"].as_array().unwrap())
            .map(|e| push(e, Value::Null))
            .collect(),
    );
    assert_eq!(phone.delta(combined).await.unwrap_err().status, 413);
    assert_eq!(phone.status().await, before);
    for _ in 0..2 {
        pc.store
            .call(Request::SetCharacter {
                conv_id: ids[0].clone(),
                text: format!("{}{}", id(), "z".repeat(2_900_000)),
                source: "app".into(),
            })
            .await
            .unwrap();
    }
    let mut p = envelope(&phone.server);
    p["convIds"] = json!([ids[0]]);
    let err = pc.delta(p).await.unwrap_err();
    assert_eq!(err.status, 413);
    assert_eq!(err.extra["convId"], ids[0]);
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn persisted_server_id_checkpoints_and_exact_batch_replay_survive_reopen() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    let first = mirror(&pc, &phone, &c).await;
    memory(&phone, &c, "Após checkpoint").await;
    let incoming = offer(&phone, &pc.server, &c).await;
    pc.close().await;
    let reopened = Store::open_encrypted(pc.dir.path()).unwrap();
    assert_eq!(
        reopened.call(Request::SyncStatus(json!({}))).await.unwrap()["serverId"],
        pc.server
    );
    let p = batch(&phone.server, vec![push(&incoming, first["hash"].clone())]);
    let ack = reopened
        .call(Request::SyncDelta {
            value: p.clone(),
            embedding_model: model(),
        })
        .await
        .unwrap();
    assert_eq!(ack["ackBatch"], p["batchId"]);
    reopened.call(Request::Close).await.unwrap();
    let reopened = Store::open_encrypted(pc.dir.path()).unwrap();
    assert_eq!(
        reopened
            .call(Request::SyncDelta {
                value: p.clone(),
                embedding_model: model()
            })
            .await
            .unwrap(),
        ack
    );
    let mut reused = p;
    reused["limit"] = json!(1);
    let err = reopened
        .call(Request::SyncDelta {
            value: reused,
            embedding_model: model(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.status, 409);
    assert_eq!(err.extra["code"], "BATCH_REUSE");
    reopened.call(Request::Close).await.unwrap();
    phone.close().await;
}

#[tokio::test]
async fn missing_rows_do_not_delete_existing_history_and_completed_jobs_survive_pruning() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    let t = turn(&pc, &c, "Imutável", &id()).await;
    finish_jobs(&pc, &phone.server, &c).await;
    let edited = pc
        .store
        .call(Request::EditMessage {
            conv_id: c.clone(),
            part_id: t["partId"].as_str().unwrap().into(),
            content: "Mira: Editada.".into(),
            op_id: Some("stable-edit-operation".into()),
        })
        .await
        .unwrap();
    finish_jobs(&pc, &phone.server, &c).await;
    let m = memory(&pc, &c, "Nunca excluir por ausência").await;
    let first = mirror(&pc, &phone, &c).await;
    pc.store
        .call(Request::Prune(
            json!({"days":1,"now":chrono::Utc::now().timestamp_millis()+365*86400000i64}),
        ))
        .await
        .unwrap();
    let preserved = offer(&pc, &phone.server, &c).await;
    assert_eq!(preserved["hash"], first["hash"]);
    let mut absent = first.clone();
    absent["snapshot"]["memories"] = json!([]);
    rehash(&mut absent);
    let out = pc
        .delta(batch(
            &phone.server,
            vec![push(&absent, first["hash"].clone())],
        ))
        .await
        .unwrap();
    assert!(out["conflicts"].as_array().unwrap().is_empty(), "{out}");
    assert!(
        offer(&pc, &phone.server, &c).await["snapshot"]["memories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == m["id"])
    );
    assert!(pc.store.call(Request::ClaimJob).await.unwrap().is_null());
    let replay = pc
        .store
        .call(Request::EditMessage {
            conv_id: c.clone(),
            part_id: t["partId"].as_str().unwrap().into(),
            content: "Não reexecutar.".into(),
            op_id: Some("stable-edit-operation".into()),
        })
        .await
        .unwrap();
    assert_eq!(
        replay["message"]["versionId"],
        edited["message"]["versionId"]
    );
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn basic_import_keeps_its_distinct_uuid_remap_contract() {
    let f = Fixture::new().await;
    let c = f.create().await;
    turn(&f, &c, "Import básico", &id()).await;
    let exported = f.store.call(Request::Export(None)).await.unwrap();
    let out = f.store.call(Request::Import(exported)).await.unwrap();
    assert_eq!(out["imported"].as_array().unwrap().len(), 1);
    assert_ne!(out["imported"][0], c);
    let copy = f
        .store
        .call(Request::GetConversation(
            out["imported"][0].as_str().unwrap().into(),
        ))
        .await
        .unwrap();
    assert_eq!(copy["history"].as_array().unwrap().len(), 2);
    assert_eq!(copy["prompt"], "Ficha inicial");
    assert_eq!(
        f.store.call(Request::GetConversation(c)).await.unwrap()["history"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    f.close().await;
}

#[tokio::test]
async fn basic_import_invalid_references_roll_back_original_unchanged() {
    let f = Fixture::new().await;
    f.create().await;
    let before = f.status().await;
    let mut exported = f.store.call(Request::Export(None)).await.unwrap();
    exported["conversations"][0]["conversation"]["head_interaction_id"] = json!(id());
    let out = f.store.call(Request::Import(exported)).await.unwrap();
    assert!(out["imported"].as_array().unwrap().is_empty());
    assert_eq!(out["errors"].as_array().unwrap().len(), 1);
    assert_eq!(f.status().await, before);
    f.close().await;
}

async fn http(
    router: axum::Router,
    method: &str,
    url: &str,
    body: String,
    token: Option<&str>,
) -> (u16, Value) {
    let mut req = HttpRequest::builder()
        .method(method)
        .uri(url)
        .extension(ConnectInfo(
            "127.0.0.1:12000".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .header("content-type", "application/json")
        .header("origin", "http://localhost");
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let response = router
        .oneshot(req.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn http_state() -> (tempfile::TempDir, AppState, String) {
    let dir = temp();
    let state = AppState::new(Config {
        data_dir: dir.path().into(),
        dist: dir.path().join("absent"),
        address: "127.0.0.1:0".parse().unwrap(),
        require_pairing: true,
        providers: None,
        max_generations: 1,
        production: true,
        native_lib_dir: None,
        review: true,
    })
    .await
    .unwrap();
    let code = state.auth.start().await["code"].clone();
    let paired = state
        .auth
        .pair(
            &json!({"code":code,"name":"sync-fixture"}),
            "127.0.0.1".parse().unwrap(),
        )
        .await
        .unwrap();
    (dir, state, paired["token"].as_str().unwrap().into())
}

#[tokio::test]
async fn http_sync_pauses_only_background_memory_after_authentication() {
    let (_dir, state, token) = http_state().await;
    let router = api::router(state.clone());
    assert_eq!(state.runner.state().await["pausedForSync"], false);
    assert_eq!(http(router.clone(), "GET", "/api/sync/status", "".into(), None).await.0, 401);
    assert_eq!(state.runner.state().await["pausedForSync"], false);
    assert_eq!(http(router.clone(), "GET", "/api/sync/status?observe=1", "".into(), Some(&token)).await.0, 200);
    assert_eq!(state.runner.state().await["pausedForSync"], false, "periodic observation must let background inference finish");
    let conv = state.store.call(Request::CreateConversation(json!({"routeId":"cbai/glm-5.2","presetId":"glm-52"}))).await.unwrap();
    let generation = state.store.call(Request::BeginTurn(serde_json::from_value::<TurnRequest>(json!({"convId":conv["id"],"mode":"send","content":"fixture","opId":"sync-priority"})).unwrap())).await.unwrap();
    let status = http(router.clone(), "GET", "/api/sync/status", "".into(), Some(&token)).await;
    assert_eq!(status.0, 200);
    assert_eq!(state.runner.state().await["pausedForSync"], true, "adjacent snapshot requests share a short grace period");
    assert!(status.1["manifests"].as_array().unwrap().iter().any(|entry| entry["convId"] == conv["id"] && entry["busy"] == true), "user generation must still block unsafe synchronization");
    state.store.call(Request::FinishGeneration(serde_json::from_value::<FinishRequest>(json!({"generationId":generation["generationId"],"status":"interrupted","text":""})).unwrap())).await.unwrap();
    state.shutdown().await;
}

#[tokio::test]
async fn http_sync_auth_get_post_limits_errors_and_conflict_status() {
    let (_dir, state, token) = http_state().await;
    let router = api::router(state.clone());
    for (method, url, body) in [
        ("GET", "/api/sync/status", "".into()),
        ("GET", "/api/sync/delta", "".into()),
        ("POST", "/api/sync/delta", "x".repeat(sync::MAX_BYTES + 1)),
    ] {
        assert_eq!(http(router.clone(), method, url, body, None).await.0, 401);
    }
    let status = http(
        router.clone(),
        "GET",
        "/api/sync/status",
        "".into(),
        Some(&token),
    )
    .await;
    assert_eq!(status.0, 200);
    assert_eq!(status.1["schemaVersion"], 4);
    assert_eq!(
        http(
            router.clone(),
            "POST",
            "/api/sync/delta",
            "x".repeat(sync::MAX_BYTES + 1),
            Some(&token)
        )
        .await
        .0,
        413
    );
    assert_eq!(
        http(
            router.clone(),
            "POST",
            "/api/sync/delta",
            "{".into(),
            Some(&token)
        )
        .await
        .0,
        400
    );
    let peer = Fixture::new().await;
    let c = peer.create().await;
    let snapshot = offer(&peer, &status.1["serverId"], &c).await;
    let request = batch(&peer.server, vec![push(&snapshot, Value::Null)]);
    let mirrored = http(
        router.clone(),
        "POST",
        "/api/sync/delta",
        request.to_string(),
        Some(&token),
    )
    .await;
    assert_eq!(mirrored.0, 200);
    assert_eq!(mirrored.1["ackBatch"], request["batchId"]);
    let get_url = format!(
        "/api/sync/delta?peerId={}&convIds=%5B%22{}%22%5D",
        peer.server.as_str().unwrap(),
        c
    );
    let get = http(router.clone(), "GET", &get_url, "".into(), Some(&token)).await;
    assert_eq!(get.0, 200);
    assert_eq!(get.1["snapshots"][0]["hash"], snapshot["hash"]);
    state
        .store
        .call(Request::PatchConversation {
            conv_id: c.clone(),
            patch: json!({"name":"PC concurrent"}),
            base_character_rev: None,
        })
        .await
        .unwrap();
    peer.store
        .call(Request::PatchConversation {
            conv_id: c.clone(),
            patch: json!({"name":"Phone concurrent"}),
            base_character_rev: None,
        })
        .await
        .unwrap();
    let incoming = offer(&peer, &status.1["serverId"], &c).await;
    let conflicted = http(
        router.clone(),
        "POST",
        "/api/sync/delta",
        batch(
            &peer.server,
            vec![push(&incoming, snapshot["hash"].clone())],
        )
        .to_string(),
        Some(&token),
    )
    .await;
    assert_eq!(conflicted.0, 409);
    assert_eq!(conflicted.1["code"], "CONFLICT");
    assert!(conflicted.1["ackBatch"].is_null());
    assert!(
        !api::sync_character(&state.store, &state.config, &c)
            .await
            .unwrap()
    );
    peer.close().await;
    state.shutdown().await;
}

#[tokio::test]
async fn status_cursor_hash_changes_only_on_replicable_data_and_known_skips_snapshots() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let a = pc.create().await;
    let b = pc.create().await;
    let status = pc.status().await;
    assert_eq!(pc.status().await, status);
    let page = pc
        .store
        .call(Request::SyncStatus(json!({"limit":1})))
        .await
        .unwrap();
    assert_eq!(page["hasMore"], true);
    let next = pc
        .store
        .call(Request::SyncStatus(
            json!({"after":page["nextCursor"],"limit":1}),
        ))
        .await
        .unwrap();
    assert_eq!(next["hasMore"], false);
    let mut p = envelope(&phone.server);
    p["known"] = json!(
        status["manifests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| json!({"convId":m["convId"],"hash":m["hash"]}))
            .collect::<Vec<_>>()
    );
    assert!(
        pc.delta(p.clone()).await.unwrap()["snapshots"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    pc.store
        .call(Request::AddDevice {
            name: "Excluded".into(),
            token_hash: id(),
        })
        .await
        .unwrap();
    assert_eq!(pc.status().await, status);
    memory(&pc, &a, "Changed").await;
    let changed = pc.delta(p).await.unwrap();
    assert_eq!(changed["snapshots"].as_array().unwrap().len(), 1);
    assert_eq!(changed["snapshots"][0]["convId"], a);
    assert_eq!(
        manifest(&pc.status().await, &b)["hash"],
        manifest(&status, &b)["hash"]
    );
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn all_narrative_tables_persona_operations_and_chunk_embeddings_keep_ids() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let persona = pc
        .store
        .call(Request::SavePersona(
            json!({"name":"Alex","description":"Persona PC"}),
        ))
        .await
        .unwrap();
    let c=pc.store.call(Request::CreateConversation(json!({"name":"Narrativa completa","routeId":"fixture","presetId":"generic",
        "prompt":"Mira guarda uma carta.","settings":{"personaId":persona["id"],"personaNotes":null,"thinkingEnabled":null}})))
        .await.unwrap()["id"].as_str().unwrap().to_owned();
    let g = turn(&pc, &c, "Mira recebe uma carta de Alex.", &id()).await;
    let edit = pc
        .store
        .call(Request::EditMessage {
            conv_id: c.clone(),
            part_id: g["partId"].as_str().unwrap().into(),
            content: "Mira: Recebi a carta de Alex.".into(),
            op_id: Some(id()),
        })
        .await
        .unwrap();
    let version = edit["message"]["versionId"].clone();
    pc.store.call(Request::ApplyExtraction(json!({"convId":c,"interactionId":g["interactionId"],"versionId":version,
        "versionIds":[version],"sourceText":"Mira recebe uma carta de Alex. Mira: Recebi a carta de Alex.",
        "extraction":{"summary":"Mira recebeu uma carta de Alex.","cast":[{"name":"Mira","description":"Guarda a carta"}],
            "scene":{"location":"sala","time":"manhã","mood":"tranquila","present":["Mira","Alex"],"objects":["carta"],
                "characters":[{"name":"Mira","location":"sala","position":"sentada","holding":["carta"]}]},
            "facts":[{"kind":"EVENT","subject":"Mira","content":"Mira recebeu uma carta de Alex.","importance":0.8,"confidence":0.9,
                "knowledge":[{"who":"Mira","state":"knows","via":"witnessed","evidence":"Recebi a carta"}]}],
            "relationships":[{"a":"Mira","b":"Alex","description":"São amigos."}],
            "openThreads":["Descobrir o remetente verdadeiro da carta."],"resolvedThreads":[]}}))).await.unwrap();
    pc.store
        .call(Request::CreateEpisode(
            json!({"convId":c,"fromSeq":1,"toSeq":1,"interactionIds":[g["interactionId"]],
        "summary":"Uma carta chegou à sala."}),
        ))
        .await
        .unwrap();
    pc.store.call(Request::AddFlags(json!({"convId":c,"versionId":version,"interactionId":g["interactionId"],
        "flags":[{"kind":"agency","detail":"Revisão fixture","evidence":"carta","source":"heuristic"}]}))).await.unwrap();
    let a = memory(&pc, &c, "A academia fica numa ilha.").await;
    let b = memory(&pc, &c, "A ilha abriga a academia.").await;
    pc.store
        .call(Request::ApplyCuration(
            json!({"convId":c,"decisions":[{"keepId":a["id"],"dropIds":[b["id"]],
        "merged":"A academia fica numa ilha."}]}),
        ))
        .await
        .unwrap();
    pc.store
        .call(Request::AddMemory {
            conv_id: c.clone(),
            value: json!({"kind":"SECRET","subject":"Mira","content":"O remetente é desconhecido.",
        "knowledge":[{"who":"Mira","state":"knows","via":"witnessed"}]}),
        })
        .await
        .unwrap();
    let pending = pc
        .store
        .call(Request::PendingEmbeddings {
            limit: 64,
            model: model(),
        })
        .await
        .unwrap();
    for row in pending.as_array().unwrap() {
        pc.store
            .call(Request::SaveEmbedding {
                conv_id: c.clone(),
                owner_type: row["t"].as_str().unwrap().into(),
                owner_id: row["id"].as_str().unwrap().into(),
                model: model(),
                source_text: row["text"].as_str().map(str::to_owned),
                vector: vec![0.5, -0.25],
            })
            .await
            .unwrap();
    }
    let first = mirror(&pc, &phone, &c).await;
    for table in [
        "personas",
        "character_revisions",
        "interactions",
        "parts",
        "versions",
        "characters",
        "memories",
        "memory_knowledge",
        "scene_states",
        "character_states",
        "relationships",
        "threads",
        "interaction_notes",
        "episodes",
        "review_flags",
        "knowledge_events",
        "memory_replacements",
        "jobs",
        "operations",
        "generations",
        "embeddings",
    ] {
        assert!(
            !first["snapshot"][table].as_array().unwrap().is_empty(),
            "Missing fixture table: {table}"
        );
    }
    assert_eq!(
        offer(&phone, &pc.server, &c).await["snapshot"],
        first["snapshot"]
    );
    let op = first["snapshot"]["operations"][0]["op_id"]
        .as_str()
        .unwrap();
    let replay = phone
        .store
        .call(Request::EditMessage {
            conv_id: c.clone(),
            part_id: g["partId"].as_str().unwrap().into(),
            content: "Nunca executar novamente.".into(),
            op_id: Some(op.into()),
        })
        .await
        .unwrap();
    assert_eq!(replay["message"]["versionId"], version);
    let chapter = phone
        .store
        .call(Request::Memories(c.clone()))
        .await
        .unwrap();
    assert!(
        chapter["memories"]
            .as_array()
            .is_some_and(|m| !m.is_empty())
    );
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn cross_conversation_uuid_collision_rolls_back_the_entire_batch() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let a = pc.create().await;
    let existing = memory(&pc, &a, "Existente PC").await;
    let first = mirror(&pc, &phone, &a).await;
    memory(&phone, &a, "Alteração válida").await;
    let valid = offer(&phone, &pc.server, &a).await;
    let b = phone.create().await;
    let other = memory(&phone, &b, "Conversa independente").await;
    let mut invalid = offer(&phone, &pc.server, &b).await;
    let row = invalid["snapshot"]["memories"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|r| r["id"] == other["id"])
        .unwrap();
    row["id"] = existing["id"].clone();
    for row in invalid["snapshot"]["memory_knowledge"]
        .as_array_mut()
        .unwrap()
    {
        if row["memory_id"] == other["id"] {
            row["memory_id"] = existing["id"].clone();
        }
    }
    rehash(&mut invalid);
    let before = offer(&pc, &phone.server, &a).await;
    let request = batch(
        &phone.server,
        vec![
            push(&valid, first["hash"].clone()),
            push(&invalid, Value::Null),
        ],
    );
    let out = pc.delta(request.clone()).await.unwrap();
    assert_eq!(out["code"], "CONFLICT");
    assert!(out["ackBatch"].is_null());
    assert_eq!(pc.delta(request).await.unwrap(), out);
    assert_eq!(offer(&pc, &phone.server, &a).await["hash"], before["hash"]);
    assert_eq!(
        pc.store
            .call(Request::GetConversation(b))
            .await
            .unwrap_err()
            .status,
        404
    );
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn compatible_vector_with_changed_owner_text_is_invalidated_for_recompute() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    let m = memory(&pc, &c, "Texto base").await;
    let first = mirror(&pc, &phone, &c).await;
    pc.store
        .call(Request::SaveEmbedding {
            conv_id: c.clone(),
            owner_type: "memory".into(),
            owner_id: m["id"].as_str().unwrap().into(),
            model: model(),
            source_text: None,
            vector: vec![1.0, 0.0],
        })
        .await
        .unwrap();
    phone
        .store
        .call(Request::PatchMemory {
            conv_id: c.clone(),
            memory_id: m["id"].as_str().unwrap().into(),
            value: json!({"content":"Novo texto offline"}),
        })
        .await
        .unwrap();
    let incoming = offer(&phone, &pc.server, &c).await;
    let out = pc
        .delta(batch(
            &phone.server,
            vec![push(&incoming, first["hash"].clone())],
        ))
        .await
        .unwrap();
    assert!(!out["ackBatch"].is_null(), "{out}");
    assert_eq!(out["ignoredEmbeddings"][0]["reason"], "source_changed");
    assert!(
        offer(&pc, &phone.server, &c).await["snapshot"]["embeddings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn sync_checkpoints_require_encryption_and_loopback_endpoints_always_require_auth() {
    let dir = temp();
    let store = Store::open(dir.path()).unwrap();
    let err = store
        .call(Request::SyncDelta {
            value: envelope(&json!(id())),
            embedding_model: model(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.extra["code"], "SYNC_ENCRYPTION_REQUIRED");
    store.call(Request::Close).await.unwrap();
    let state = AppState::new(Config {
        data_dir: dir.path().into(),
        dist: dir.path().join("absent"),
        address: "127.0.0.1:0".parse().unwrap(),
        require_pairing: false,
        providers: None,
        max_generations: 1,
        production: false,
        native_lib_dir: None,
        review: true,
    })
    .await
    .unwrap();
    assert!(!state.store.encrypted);
    assert_eq!(
        http(
            api::router(state.clone()),
            "GET",
            "/api/sync/status",
            "".into(),
            None
        )
        .await
        .0,
        401
    );
    state.shutdown().await;
}

#[tokio::test]
async fn purged_response_refs_keep_terminal_op_recovery_without_resurrecting_history() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let fresh = Fixture::new().await;
    let c = pc.create().await;
    let op = id();
    let g = turn(&pc, &c, "Turno com resposta apagável", &op).await;
    let editop = id();
    pc.store
        .call(Request::EditMessage {
            conv_id: c.clone(),
            part_id: g["partId"].as_str().unwrap().into(),
            content: "Mira: resposta que será purgada.".into(),
            op_id: Some(editop.clone()),
        })
        .await
        .unwrap();
    finish_jobs(&pc, &phone.server, &c).await;
    let first = mirror(&pc, &phone, &c).await;
    pc.store
        .call(Request::DeleteMessage {
            conv_id: c.clone(),
            message_id: g["partId"].as_str().unwrap().into(),
        })
        .await
        .unwrap();
    let trash = pc.store.call(Request::Trash).await.unwrap();
    let deleted_batch = trash
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["type"] == "responses" && r["convId"] == c)
        .unwrap()["batch"]
        .clone();
    pc.store
        .call(Request::PurgeTrash(
            json!({"type":"responses","convId":c,"batch":deleted_batch}),
        ))
        .await
        .unwrap();
    let removed = offer(&pc, &phone.server, &c).await;
    assert!(removed["snapshot"]["parts"].as_array().unwrap().is_empty());
    assert!(
        removed["snapshot"]["versions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !removed["snapshot"]["removed"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(removed["snapshot"]["generations"][0]["op_id"], op);
    let out = phone
        .delta(batch(
            &pc.server,
            vec![push(&removed, first["hash"].clone())],
        ))
        .await
        .unwrap();
    assert!(!out["ackBatch"].is_null(), "{out}");
    assert_eq!(offer(&phone, &pc.server, &c).await["hash"], removed["hash"]);
    fresh
        .delta(batch(&pc.server, vec![push(&removed, Value::Null)]))
        .await
        .unwrap();
    let request: TurnRequest =
        serde_json::from_value(json!({"convId":c,"mode":"send","content":"Não repetir","opId":op}))
            .unwrap();
    assert_eq!(
        fresh.store.call(Request::BeginTurn(request)).await.unwrap()["generationId"],
        g["generationId"]
    );
    let replay = fresh
        .store
        .call(Request::EditMessage {
            conv_id: c.clone(),
            part_id: g["partId"].as_str().unwrap().into(),
            content: "Não repetir a edição.".into(),
            op_id: Some(editop),
        })
        .await
        .unwrap();
    assert!(replay["message"]["content"].is_null());
    assert_eq!(
        fresh.store.call(Request::GetConversation(c)).await.unwrap()["history"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    pc.close().await;
    phone.close().await;
    fresh.close().await;
}

#[tokio::test]
async fn incompatible_peer_vector_does_not_block_core_merge_and_content_hash_converges() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    let m = memory(&pc, &c, "Proveniência idêntica").await;
    let first = mirror(&pc, &phone, &c).await;
    for (f, fp) in [
        (&pc, model()),
        (&phone, rules::hash("phone-incompatible-model")),
    ] {
        f.store
            .call(Request::SaveEmbedding {
                conv_id: c.clone(),
                owner_type: "memory".into(),
                owner_id: m["id"].as_str().unwrap().into(),
                model: fp,
                source_text: None,
                vector: vec![0.5, 1.0],
            })
            .await
            .unwrap();
    }
    memory(&phone, &c, "Nota offline independente do vetor").await;
    let incoming = offer(&phone, &pc.server, &c).await;
    let out = pc
        .delta(batch(
            &phone.server,
            vec![push(&incoming, first["hash"].clone())],
        ))
        .await
        .unwrap();
    assert!(!out["ackBatch"].is_null(), "{out}");
    assert_eq!(out["ignoredEmbeddings"][0]["reason"], "fingerprint");
    assert_eq!(out["accepted"][0]["contentHash"], incoming["contentHash"]);
    assert_ne!(out["accepted"][0]["hash"], incoming["hash"]);
    assert_eq!(
        offer(&pc, &phone.server, &c).await["snapshot"]["embeddings"][0]["model"],
        model()
    );
    pc.close().await;
    phone.close().await;
}

#[tokio::test]
async fn deleted_memory_tombstone_blocks_resurrection_after_a_new_checkpoint() {
    let pc = Fixture::new().await;
    let phone = Fixture::new().await;
    let c = pc.create().await;
    let m = memory(&pc, &c, "Remover explicitamente").await;
    let first = mirror(&pc, &phone, &c).await;
    pc.store
        .call(Request::DeleteMemory {
            conv_id: c.clone(),
            memory_id: m["id"].as_str().unwrap().into(),
        })
        .await
        .unwrap();
    let removed = offer(&pc, &phone.server, &c).await;
    assert_eq!(removed["snapshot"]["removed"][0]["table"], "memories");
    let out = phone
        .delta(batch(
            &pc.server,
            vec![push(&removed, first["hash"].clone())],
        ))
        .await
        .unwrap();
    assert!(!out["ackBatch"].is_null(), "{out}");
    assert_eq!(offer(&phone, &pc.server, &c).await["hash"], removed["hash"]);
    assert!(
        phone
            .store
            .call(Request::Memories(c.clone()))
            .await
            .unwrap()["memories"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let out = pc
        .delta(batch(
            &phone.server,
            vec![push(&first, removed["hash"].clone())],
        ))
        .await
        .unwrap();
    assert_eq!(out["code"], "CONFLICT");
    assert_eq!(offer(&pc, &phone.server, &c).await["hash"], removed["hash"]);
    pc.close().await;
    phone.close().await;
}
