//! Vault persistence: what the client sees as confirmed is written before the ack; derived state goes in
//! batches (write-behind) and is flushed on deadline, on shutdown, on backup and on export.
//! The vault uses DPAPI: Windows only. Everything runs in temporary directories.
#![cfg(windows)]
use modularchat_server::{
    privacy::read_snapshot,
    store::{FinishRequest, Request, Store, TurnRequest},
};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

fn vault(dir: &Path) -> std::path::PathBuf {
    dir.join("modularchat.db.vault")
}
async fn conv(store: &Store) -> String {
    let c = store
        .call(Request::CreateConversation(json!({"name":"Teste","persona":"Alex","routeId":"cc/claude-sonnet-5-5","presetId":"claude-sonnet-5-5","effort":"low","prompt":"Mira é uma maga arrogante."})))
        .await
        .unwrap();
    c["id"].as_str().unwrap().to_owned()
}
async fn begin(store: &Store, cid: &str, text: &str) -> Value {
    store
        .call(Request::BeginTurn(
            serde_json::from_value::<TurnRequest>(json!({
                "convId":cid,"mode":"send","content":text,
                "routeId":"cc/claude-sonnet-5-5","presetId":"claude-sonnet-5-5","effort":"low"
            }))
            .unwrap(),
        ))
        .await
        .unwrap()
}
async fn finish(store: &Store, begin: &Value, text: &str) {
    store
        .call(Request::FinishGeneration(
            serde_json::from_value::<FinishRequest>(json!({
                "generationId":begin["generationId"],"status":"complete","text":text,"usage":{}
            }))
            .unwrap(),
        ))
        .await
        .unwrap();
}
async fn stats(store: &Store) -> Value {
    store.call(Request::PersistStats).await.unwrap()
}
/// Rows visible in the file on disk (what would be left if the process died now).
fn on_disk(dir: &Path, sql: &str) -> i64 {
    read_snapshot(&vault(dir))
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}
async fn usage(store: &Store, route: &str) {
    store
        .call(Request::RecordUsage {
            route_id: route.into(),
            raw: 1000,
            measured: 1200,
        })
        .await
        .unwrap();
}
fn calibrations(dir: &Path) -> i64 {
    on_disk(dir, "SELECT count(*) FROM token_calibration")
}

#[tokio::test]
async fn turno_sintetico_grava_muito_menos_que_uma_vez_por_mudanca() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_encrypted(dir.path()).unwrap();
    let cid = conv(&store).await;
    let base = stats(&store).await;
    let turns = 10u64;
    for i in 0..turns {
        let b = begin(&store, &cid, &format!("Pergunta {i}")).await;
        store
            .call(Request::SetContext {
                generation_id: b["generationId"].as_str().unwrap().into(),
                value: json!({"tokens":{"total":10}}),
            })
            .await
            .unwrap();
        let version = b["versionId"].as_str().unwrap().to_owned();
        for text in ["Lia: parcial", "Lia: parcial", "Lia: parcial maior"] {
            store
                .call(Request::FlushVersion {
                    version_id: version.clone(),
                    text: text.into(),
                })
                .await
                .unwrap();
        }
        usage(&store, "cc/claude-sonnet-5-5").await;
        finish(&store, &b, "Lia: resposta final.").await;
        let job = store.call(Request::ClaimJob).await.unwrap();
        if !job.is_null() {
            store
                .call(Request::FinishJob {
                    id: job["id"].as_str().unwrap().into(),
                    status: "done".into(),
                    error: None,
                })
                .await
                .unwrap();
        }
    }
    let now = stats(&store).await;
    let delta = |k: &str| now[k].as_u64().unwrap() - base[k].as_u64().unwrap();
    let (changing, saves) = (delta("changingRequests"), delta("saves"));
    println!(
        "por turno: {:.1} mudanças (modelo antigo: uma gravação cada) -> {:.1} gravações",
        changing as f64 / turns as f64,
        saves as f64 / turns as f64
    );
    // Only BeginTurn and FinishGeneration are durable immediately; the batch window may add a few flushes.
    assert!(
        changing >= turns * 6,
        "cenário deve alterar ~8 vezes por turno: {changing}"
    );
    assert_eq!(delta("immediateSaves"), turns * 2);
    assert!(saves <= turns * 2 + 2, "gravações demais: {saves}");
    assert!(
        saves * 3 <= changing,
        "esperava ao menos 3x menos gravações: {saves} de {changing}"
    );
    store.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn ack_duravel_sem_descarga_ao_aceitar_a_mensagem_e_a_resposta() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_encrypted(dir.path()).unwrap();
    let cid = conv(&store).await;
    assert_eq!(on_disk(dir.path(), "SELECT count(*) FROM conversations"), 1);
    let b = begin(&store, &cid, "MENSAGEM_ACEITA").await;
    // No Close and no wait: the file is already what a process killed now would leave.
    assert_eq!(
        on_disk(
            dir.path(),
            "SELECT count(*) FROM interactions WHERE user_text='MENSAGEM_ACEITA'"
        ),
        1
    );
    finish(&store, &b, "RESPOSTA_FINAL").await;
    assert_eq!(
        on_disk(
            dir.path(),
            "SELECT count(*) FROM versions WHERE content='RESPOSTA_FINAL' AND status='complete'"
        ),
        1
    );
    store
        .call(Request::PatchConversation {
            conv_id: cid.clone(),
            patch: json!({"name":"Renomeada"}),
            base_character_rev: None,
        })
        .await
        .unwrap();
    assert_eq!(
        on_disk(
            dir.path(),
            "SELECT count(*) FROM conversations WHERE name='Renomeada'"
        ),
        1
    );
    assert_eq!(stats(&store).await["pending"], false);
    store.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn estado_derivado_sujo_e_descarregado_no_prazo_e_no_encerramento() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_encrypted(dir.path()).unwrap();
    usage(&store, "rota-a").await;
    assert_eq!(
        calibrations(dir.path()),
        0,
        "derivado ainda não vai ao disco"
    );
    assert_eq!(stats(&store).await["pending"], true);
    // The ~1 s deadline writes without any other request.
    tokio::time::sleep(Duration::from_millis(1800)).await;
    assert_eq!(calibrations(dir.path()), 1, "descarga por prazo");
    assert_eq!(stats(&store).await["pending"], false);
    usage(&store, "rota-b").await;
    assert_eq!(calibrations(dir.path()), 1);
    store.call(Request::Close).await.unwrap();
    assert_eq!(
        calibrations(dir.path()),
        2,
        "Close descarrega o que estava sujo"
    );
}

#[tokio::test]
async fn fila_fechada_sem_close_ainda_descarrega_o_derivado() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open_encrypted(dir.path()).unwrap();
        usage(&store, "rota-a").await;
        assert_eq!(calibrations(dir.path()), 0);
    }
    // Without Close, dropping the Store closes the queue and the thread writes before ending.
    for _ in 0..50 {
        if calibrations(dir.path()) == 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("estado derivado perdido após o descarte do Store");
}

#[tokio::test]
async fn gravacao_duravel_leva_o_derivado_pendente_junto() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_encrypted(dir.path()).unwrap();
    usage(&store, "rota-a").await;
    assert_eq!(calibrations(dir.path()), 0);
    let cid = conv(&store).await;
    assert_eq!(
        calibrations(dir.path()),
        1,
        "o cofre inteiro é gravado de uma vez"
    );
    assert!(!cid.is_empty());
    assert_eq!(stats(&store).await["pending"], false);
    store.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn backup_e_exportacao_enxergam_o_estado_mais_recente() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_encrypted(dir.path()).unwrap();
    let cid = conv(&store).await;
    usage(&store, "rota-backup").await;
    assert_eq!(calibrations(dir.path()), 0);
    let backup = store.call(Request::Backup).await.unwrap();
    let file = std::path::PathBuf::from(backup["file"].as_str().unwrap());
    let copied: i64 = read_snapshot(&file)
        .unwrap()
        .query_row("SELECT count(*) FROM token_calibration", [], |r| r.get(0))
        .unwrap();
    assert_eq!(copied, 1, "o backup contém o estado derivado mais recente");
    assert_eq!(
        calibrations(dir.path()),
        1,
        "o cofre em disco foi atualizado antes do backup"
    );
    usage(&store, "rota-export").await;
    assert_eq!(stats(&store).await["pending"], true);
    let export = store.call(Request::Export(Some(cid))).await.unwrap();
    assert!(export["conversations"].is_array());
    assert_eq!(
        stats(&store).await["pending"],
        false,
        "a exportação descarrega antes de ler"
    );
    assert_eq!(calibrations(dir.path()), 2);
    store.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn ultimo_acesso_do_aparelho_nao_regrava_a_cada_requisicao() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_encrypted(dir.path()).unwrap();
    store
        .call(Request::AddDevice {
            name: "PC".into(),
            token_hash: "hash-aparelho".into(),
        })
        .await
        .unwrap();
    let find = || store.call(Request::FindDevice("hash-aparelho".into()));
    assert!(!find().await.unwrap().is_null());
    let after_first = stats(&store).await["changingRequests"].as_u64().unwrap();
    let seen = store.call(Request::Devices).await.unwrap()[0]["last_seen_at"].clone();
    assert!(seen.is_string());
    for _ in 0..5 {
        assert!(!find().await.unwrap().is_null());
    }
    assert_eq!(
        stats(&store).await["changingRequests"].as_u64().unwrap(),
        after_first
    );
    assert_eq!(
        store.call(Request::Devices).await.unwrap()[0]["last_seen_at"],
        seen
    );
    store.call(Request::Close).await.unwrap();
}

#[tokio::test]
async fn texto_parcial_identico_nao_conta_como_mudanca() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_encrypted(dir.path()).unwrap();
    let cid = conv(&store).await;
    let b = begin(&store, &cid, "oi").await;
    let version = b["versionId"].as_str().unwrap().to_owned();
    let flush = |text: &'static str| {
        store.call(Request::FlushVersion {
            version_id: version.clone(),
            text: text.into(),
        })
    };
    flush("parcial").await.unwrap();
    let before = stats(&store).await["changingRequests"].as_u64().unwrap();
    for _ in 0..4 {
        flush("parcial").await.unwrap();
    }
    assert_eq!(
        stats(&store).await["changingRequests"].as_u64().unwrap(),
        before
    );
    // A version that already finished keeps refusing the partial text.
    finish(&store, &b, "final").await;
    assert!(flush("parcial").await.is_err());
    store.call(Request::Close).await.unwrap();
}
