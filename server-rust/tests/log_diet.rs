//! Diagnostics diet: what a message keeps of its usage, the one-time migration, pruning, the provider check
//! that no longer touches the revision, the BLOB-direct vector search and the router-log prefilter.
use modularchat_server::{
    router_log,
    store::{FinishRequest, Request, Store, TurnRequest},
};
use serde_json::{Value, json};
use std::path::Path;

async fn conv(store: &Store) -> String {
    let c = store
        .call(Request::CreateConversation(json!({"name":"Teste","persona":"Alex","routeId":"cc/claude-sonnet-5-5","presetId":"claude-sonnet-5-5","effort":"low","prompt":"Mira é uma maga."})))
        .await
        .unwrap();
    c["id"].as_str().unwrap().to_owned()
}
/// A finished turn whose usage is the full diagnostic one (what `generation::run` produces).
async fn turn_with_full_usage(store: &Store, cid: &str) -> Value {
    let begin = store
        .call(Request::BeginTurn(
            serde_json::from_value::<TurnRequest>(json!({
                "convId":cid,"mode":"send","content":"Oi, Mira.",
                "routeId":"cc/claude-sonnet-5-5","presetId":"claude-sonnet-5-5","effort":"low"
            }))
            .unwrap(),
        ))
        .await
        .unwrap();
    let usage = json!({
        "input":1200,"output":300,"reasoning":null,
        "method":{"input":"measured","output":"measured","reasoning":"unknown"},
        "visibleEstimate":280,"finish":"stop","reasoningHidden":false,
        "length":{"maxCharacters":2000,"mode":"maximum","apiMaxTokens":2288,"visibleTokens":667,"thinkingEnabled":false},
        "style":{"attempted":true,"accepted":false,
            "before":{"characters":2400,"ok":false,"sizeOk":false,"monologueOk":true},
            "after":{"characters":2400,"ok":false,"sizeOk":false,"ratioOk":true,"languageOk":true,"monologueOk":true,"maxMonologueCharacters":300,"longestMonologueCharacters":120,"dialoguePct":60.5},
            "candidate":{"characters":2100,"ok":false},
            "usage":{"prompt_tokens":1500,"completion_tokens":400},
            "error":"Upstream: connection reset"}
    });
    store
        .call(Request::FinishGeneration(
            serde_json::from_value::<FinishRequest>(json!({
                "generationId":begin["generationId"],"status":"complete","text":"Mira: Oi.","usage":usage
            }))
            .unwrap(),
        ))
        .await
        .unwrap();
    begin
}
fn db(dir: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(dir.join("modularchat.db")).unwrap()
}
fn reopen(dir: &Path) {
    drop(Store::open(dir).unwrap());
    std::thread::sleep(std::time::Duration::from_millis(400));
}

#[tokio::test]
async fn message_keeps_only_the_summary_and_the_generation_keeps_the_full_record() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let cid = conv(&store).await;
    let begin = turn_with_full_usage(&store, &cid).await;
    let message = |chat: &Value| chat["history"].as_array().unwrap().last().unwrap().clone();
    let chat = store
        .call(Request::GetConversation(cid.clone()))
        .await
        .unwrap();
    let usage = message(&chat)["usage"].clone();
    // what the UI reads stays
    assert_eq!(usage["input"], 1200);
    assert_eq!(usage["method"]["input"], "measured");
    assert_eq!(usage["finish"], "stop");
    assert_eq!(usage["length"]["apiMaxTokens"], 2288);
    assert_eq!(usage["length"]["thinkingEnabled"], false);
    assert_eq!(usage["style"]["attempted"], true);
    assert_eq!(usage["style"]["accepted"], false);
    assert_eq!(usage["style"]["after"]["characters"], 2400);
    assert_eq!(usage["style"]["after"]["longestMonologueCharacters"], 120);
    // the diagnostic does not
    for gone in ["before", "candidate", "usage", "error"] {
        assert!(
            usage["style"].get(gone).is_none(),
            "style.{gone} must not be kept on the message"
        );
    }
    assert!(usage["style"]["after"].get("dialoguePct").is_none());
    // the generation keeps everything
    let full = store
        .call(Request::GetGeneration(s(&begin, "generationId")))
        .await
        .unwrap();
    assert_eq!(full["usage"]["style"]["before"]["characters"], 2400);
    assert_eq!(full["usage"]["style"]["candidate"]["characters"], 2100);
    assert_eq!(
        full["usage"]["style"]["error"],
        "Upstream: connection reset"
    );

    // The provider check lands on both records without moving the conversation revision.
    let revision = chat["revision"].clone();
    store
        .call(Request::ProviderCheck {
            generation_id: s(&begin, "generationId"),
            value: json!({"checkedAt":"2026-01-01T00:00:00.000Z","provider":"p","model":"m","effortExpected":"low","effortSent":"low","effortOk":true,
                "thinkingEnabled":false,"thinkingOk":true,"thinkingMode":{"type":"disabled"},"maxTokens":2288,"usage":{"input":1190,"cacheRead":900}}),
        })
        .await
        .unwrap();
    let chat = store
        .call(Request::GetConversation(cid.clone()))
        .await
        .unwrap();
    assert_eq!(
        chat["revision"], revision,
        "diagnostic evidence is not a content change"
    );
    let provider = message(&chat)["usage"]["provider"].clone();
    assert_eq!(provider["effortOk"], true);
    assert_eq!(provider["thinkingOk"], true);
    assert_eq!(provider["usage"]["cacheRead"], 900);
    assert!(provider.get("thinkingMode").is_none() && provider.get("maxTokens").is_none());
    let full = store
        .call(Request::GetGeneration(s(&begin, "generationId")))
        .await
        .unwrap();
    assert_eq!(
        full["usage"]["provider"]["thinkingMode"]["type"],
        "disabled"
    );
    assert_eq!(full["usage"]["style"]["before"]["characters"], 2400);
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn old_message_usage_is_summarized_once_and_old_generation_usage_is_pruned() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        let cid = conv(&store).await;
        turn_with_full_usage(&store, &cid).await;
    }
    std::thread::sleep(std::time::Duration::from_millis(400));
    // Reproduce the old database: the message holds the full usage and the migration has not run.
    {
        let c = db(dir.path());
        c.execute_batch(
            "UPDATE versions SET usage_json=(SELECT usage_json FROM generations LIMIT 1);
             DELETE FROM meta WHERE key='usage_summary_v1';",
        )
        .unwrap();
        let has_before = |t: &str| -> i64 {
            c.query_row(
                &format!("SELECT count(*) FROM {t} WHERE instr(usage_json,'\"before\"')>0"),
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(has_before("versions"), 1);
        assert_eq!(has_before("generations"), 1);
    }
    reopen(dir.path());
    let c = db(dir.path());
    let has_before = |t: &str| -> i64 {
        c.query_row(
            &format!("SELECT count(*) FROM {t} WHERE instr(usage_json,'\"before\"')>0"),
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        has_before("versions"),
        0,
        "the migration summarizes the messages"
    );
    assert_eq!(
        has_before("generations"),
        1,
        "recent generations keep the full record"
    );
    let marker: i64 = c
        .query_row(
            "SELECT count(*) FROM meta WHERE key='usage_summary_v1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(marker, 1);
    let summary: String = c
        .query_row("SELECT usage_json FROM versions", [], |r| r.get(0))
        .unwrap();
    // idempotent: another open changes nothing
    drop(c);
    reopen(dir.path());
    let again: String = db(dir.path())
        .query_row("SELECT usage_json FROM versions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(summary, again);

    // aged beyond the retention, the generation's own usage shrinks to the same summary
    db(dir.path())
        .execute_batch("UPDATE generations SET started_at='2020-01-01T00:00:00.000Z'")
        .unwrap();
    reopen(dir.path());
    let c = db(dir.path());
    let old: String = c
        .query_row("SELECT usage_json FROM generations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(old, summary);
    let count: i64 = c
        .query_row("SELECT count(*) FROM generations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "pruning never removes the generation row");
}

#[tokio::test]
async fn vector_search_decodes_the_blob_directly_and_ranks_by_cosine() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let cid = conv(&store).await;
    let mut ids = Vec::new();
    for (text, vec) in [
        (
            "A chave dourada está guardada no porão.",
            vec![1.0f32, 0.0, 0.0],
        ),
        (
            "O farol foi construído há muito tempo.",
            vec![0.0, 1.0, 0.0],
        ),
        ("Uma terceira lembrança distinta.", vec![1.0, 0.0, 0.0]),
    ] {
        let m = store
            .call(Request::AddMemory {
                conv_id: cid.clone(),
                value: json!({"content":text,"kind":"WORLD_FACT"}),
            })
            .await
            .unwrap();
        // the third one is embedded with another model: it must not take part
        let model = if text.starts_with("Uma terceira") {
            "old"
        } else {
            "m1"
        };
        store
            .call(Request::SaveEmbedding {
                conv_id: cid.clone(),
                owner_type: "memory".into(),
                owner_id: s(&m, "id"),
                model: model.into(),
                source_text: None,
                vector: vec,
            })
            .await
            .unwrap();
        ids.push(s(&m, "id"));
    }
    let out = store
        .call(Request::Retrieve(json!({
            "convId":cid,"queryText":"paráfrase sem palavras em comum","queryVec":[1.0,0.0,0.0],
            "embeddingModel":"m1","windowStartSeq":10000
        })))
        .await
        .unwrap();
    assert_eq!(out["method"], "hybrid");
    let found = out["candidates"].as_array().unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["id"], ids[0]);
    assert!((found[0]["parts"]["dense"].as_f64().unwrap() - 1.0).abs() < 1e-6);
}

#[test]
fn router_log_prefilter_keeps_the_exact_match_and_skips_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("router.db");
    let db = rusqlite::Connection::open(&file).unwrap();
    db.execute_batch(
        "CREATE TABLE requestDetails(timestamp TEXT,provider TEXT,model TEXT,data TEXT)",
    )
    .unwrap();
    // the final user message carries quotes, accents, a newline and a long ASCII run
    let wanted = "[CONTEXTO]\nA \"chave dourada\" está no porão da torre norte desde o último inverno rigoroso.\n[Lembrete: responda em pt-BR]";
    let other = "A chave dourada está no porão da torre norte desde o último inverno rigoroso, mas outra conversa.";
    let insert = |ts: &str, last: &str, effort: &str, padding: usize| {
        db.execute(
            "INSERT INTO requestDetails VALUES(?,?,?,?)",
            rusqlite::params![
                ts,
                "prov",
                "model",
                json!({"request":{"messages":[{"role":"system","content":"x".repeat(padding)},{"role":"user","content":last},{"role":"assistant","content":"ok"}]},
                    "providerRequest":{"output_config":{"effort":effort},"max_tokens":2288,"messages":[{"role":"user","content":last}]},
                    "providerResponse":{"usage":{"cache_read_input_tokens":7},"choices":[{"message":{"content":"y".repeat(padding)}}]},
                    "tokens":{"prompt_tokens":952,"completion_tokens":40}})
                .to_string()
            ],
        )
        .unwrap();
    };
    // the newest row has a different last message; the wanted one is older; the assistant turn after it is ignored
    insert("2026-01-01T00:00:03.000Z", other, "low", 10);
    insert("2026-01-01T00:00:02.000Z", wanted, "max", 2000);
    insert(
        "2026-01-01T00:00:01.000Z",
        "texto sem relação alguma com este pedido",
        "low",
        10,
    );
    drop(db);
    let hit = router_log::find(&file, "2026-01-01", wanted, "claude");
    assert_eq!(hit["found"], true, "{hit}");
    assert_eq!(hit["effort"], "max");
    assert_eq!(hit["maxTokens"], 2288);
    assert_eq!(hit["usage"]["input"], 952);
    assert_eq!(hit["usage"]["output"], 40);
    assert_eq!(hit["usage"]["cacheRead"], 7);
    assert_eq!(hit["provider"], "prov");
    let miss = router_log::find(
        &file,
        "2026-01-01",
        "nada parecido com o que foi gravado no registro",
        "claude",
    );
    assert_eq!(miss["found"], false);
    // the older row is reached through its own fragment too
    let short = router_log::find(
        &file,
        "2026-01-01",
        "texto sem relação alguma com este pedido",
        "claude",
    );
    assert_eq!(short["found"], true);
    assert_eq!(short["effort"], "low");
}
