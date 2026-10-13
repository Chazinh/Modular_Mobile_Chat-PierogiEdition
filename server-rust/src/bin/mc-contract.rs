//! Test-only protocol runner. Executes the Rust store, never the Node backend.
use modularchat_server::{
    Error, Result, s,
    store::{FinishRequest, Request, Store, TurnRequest},
};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Null)
}
fn string(args: &[Value], i: usize) -> String {
    arg(args, i).as_str().unwrap_or_default().into()
}
fn value<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|e| Error::new(400, e.to_string()))
}
async fn invoke(store: &Store, name: &str, args: &[Value]) -> Result<Value> {
    let p = arg(args, 0);
    let a = string(args, 0);
    let b = string(args, 1);
    let q = arg(args, 1);
    let request = match name {
        "createConversation" => Request::CreateConversation(p),
        "listConversations" => Request::ListConversations {
            archived: p["archived"].as_bool().unwrap_or(false),
            query: s(&p, "q"),
        },
        "getConversation" => Request::GetConversation(a),
        "getConversationMeta" => Request::GetMeta(a),
        "patchConversation" => Request::PatchConversation {
            conv_id: a,
            patch: q,
            base_character_rev: arg(args, 2)["baseCharacterRev"].as_i64(),
        },
        "setCharacterText" => Request::SetCharacter {
            conv_id: a,
            text: b,
            source: args.get(2).and_then(Value::as_str).unwrap_or("file").into(),
        },
        "duplicateConversation" => Request::DuplicateConversation(a),
        "deleteConversation" => Request::DeleteConversation(a),
        "restoreConversation" => Request::RestoreConversation(a),
        "listTrash" => Request::Trash,
        "restoreBatch" => Request::RestoreBatch {
            conv_id: a,
            batch: b,
        },
        "clearHistory" => Request::ClearHistory(a),
        "busyConversations" => Request::Busy,
        "purgeExpired" => Request::Purge(p),
        "pruneLogs" => Request::Prune(p),
        "compactDatabase" => Request::Compact,
        "beginTurn" => Request::BeginTurn(value::<TurnRequest>(p)?),
        "finishGeneration" => {
            let flags = p["flags"].as_array().cloned().unwrap_or_default();
            let begin = store
                .call(Request::GetGeneration(s(&p, "generationId")))
                .await?;
            let result = store
                .call(Request::FinishGeneration(value::<FinishRequest>(p)?))
                .await?;
            if !flags.is_empty() {
                store.call(Request::AddFlags(json!({"convId":begin["convId"],"versionId":begin["versionId"],"flags":flags,"source":"heuristic"}))).await?;
            }
            return Ok(result);
        }
        "flushVersion" => Request::FlushVersion {
            version_id: a,
            text: b,
        },
        "recoverInterrupted" => Request::RecoverInterrupted,
        "getGeneration" => Request::GetGeneration(a),
        "getGenerationByOp" => Request::GenerationByOp {
            conv_id: a,
            op_id: b,
        },
        "editMessage" => Request::EditMessage {
            conv_id: s(&p, "convId"),
            part_id: s(&p, "messageId"),
            content: s(&p, "content"),
            op_id: p["opId"].as_str().map(str::to_owned),
        },
        "selectVersion" => Request::SelectVersion {
            conv_id: s(&p, "convId"),
            part_id: s(&p, "partId"),
            version_id: s(&p, "versionId"),
        },
        "listVersions" => Request::Versions {
            conv_id: s(&p, "convId"),
            part_id: s(&p, "partId"),
        },
        "deleteMessage" => Request::DeleteMessage {
            conv_id: s(&p, "convId"),
            message_id: s(&p, "messageId"),
        },
        "listBranches" => Request::Branches(a),
        "activateBranch" => Request::ActivateBranch {
            conv_id: s(&p, "convId"),
            interaction_id: s(&p, "interactionId"),
        },
        "searchAll" => Request::Search {
            query: s(&p, "q"),
            conv_id: p["convId"].as_str().map(str::to_owned),
            limit: p["limit"].as_u64().unwrap_or(30) as usize,
        },
        "exportConversation" => {
            let export = store.call(Request::Export(Some(a))).await?;
            return Ok(export["conversations"][0].clone());
        }
        "exportAll" => Request::Export(None),
        "importConversation" => Request::ImportOne(p),
        "importAll" => Request::Import(p),
        "purgeTrash" => Request::PurgeTrash(p),
        "maybeQueueCuration" => Request::MaybeQueueCuration(a),
        "curationCandidates" => Request::CurationCandidates(a),
        "applyCuration" => Request::ApplyCuration(p),
        "getGlobalSettings" => Request::GlobalSettings,
        "setGlobalSettings" => Request::SetGlobalSettings(p),
        "importLegacy" => Request::ImportLegacy,
        "listPersonas" => Request::Personas,
        "seedPersonas" => Request::SeedPersonas,
        "savePersona" => Request::SavePersona(p),
        "listCast" => Request::Cast(a),
        "upsertCast" => {
            let mut ids = Vec::new();
            for entry in q.as_array().cloned().unwrap_or_else(|| vec![q]) {
                ids.push(
                    store
                        .call(Request::SaveCast {
                            conv_id: a.clone(),
                            value: entry,
                        })
                        .await?["id"]
                        .clone(),
                );
            }
            return Ok(json!({"ids":ids}));
        }
        "replaceCardCast" => Request::ReplaceCast {
            conv_id: a,
            list: q.as_array().cloned().unwrap_or_default(),
            persona: arg(args, 2).get("persona").cloned(),
        },
        "syncUserCast" => Request::SyncUserCast {
            conv_id: a,
            persona: q,
        },
        "addMemory" => Request::AddMemory {
            conv_id: a,
            value: q,
        },
        "updateMemory" => Request::PatchMemory {
            conv_id: a,
            memory_id: b,
            value: arg(args, 2),
        },
        "deleteMemory" => Request::DeleteMemory {
            conv_id: a,
            memory_id: b,
        },
        "memoryOverview" => Request::Memories(a),
        "syncLag" => Request::SyncLag(a),
        "jobSummary" => Request::Summary(a),
        "currentScene" => Request::CurrentScene {
            conv_id: a,
            before: q.as_i64().unwrap_or(i64::MAX),
        },
        "sceneHistory" => Request::SceneHistory {
            conv_id: a,
            entity: q.as_str().map(str::to_owned),
            limit: arg(args, 2).as_u64().unwrap_or(10) as usize,
        },
        "getContextData" => Request::Context(p),
        "retrieve" => Request::Retrieve(p),
        "applyExtraction" => Request::ApplyExtraction(p),
        "createEpisode" => Request::CreateEpisode(p),
        "saveEmbedding" => Request::SaveEmbedding {
            conv_id: s(&p, "convId"),
            owner_type: s(&p, "ownerType"),
            owner_id: s(&p, "ownerId"),
            model: s(&p, "model"),
            source_text: p["sourceText"].as_str().map(str::to_owned),
            vector: p["vec"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_f64().map(|v| v as f32))
                .collect(),
        },
        "pendingEmbeddings" => Request::PendingEmbeddings {
            limit: p.as_u64().unwrap_or(4) as usize,
            model: q.as_str().unwrap_or("bge-m3").into(),
        },
        "addFlags" => Request::AddFlags(p),
        "resolveFlag" => Request::ResolveFlag {
            conv_id: a,
            flag_id: b,
            action: string(args, 2),
        },
        "enqueueJob" => Request::EnqueueJob(p),
        "claimNextJob" => Request::ClaimJob,
        "finishJob" => Request::FinishJob {
            id: a,
            status: b,
            error: args.get(2).and_then(Value::as_str).map(str::to_owned),
        },
        "requeueRunningJobs" => Request::RequeueJobs,
        "retryFailedJobs" => Request::RetryJobs(a),
        "releaseLegacyReview" => Request::ReleaseLegacyReview,
        "getTurnInput" => Request::JobInput(a),
        "listDevices" => Request::Devices,
        "revokeDevice" => Request::RevokeDevice(a),
        "persistStats" => Request::PersistStats,
        _ => {
            return Err(Error::new(
                400,
                format!("Operação de contrato desconhecida: {name}"),
            ));
        }
    };
    store.call(request).await
}
#[tokio::main]
async fn main() {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("DATA_DIR isolado obrigatório");
        std::process::exit(1);
    };
    let store = match Store::open(std::path::Path::new(&dir)) {
        Ok(store) => store,
        Err(e) => {
            eprintln!("{}", e.message);
            std::process::exit(1)
        }
    };
    println!("{}", json!({"ready":true}));
    let _ = std::io::stdout().flush();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else {
            break;
        };
        if line.len() > 80 * 1024 * 1024 {
            eprintln!("Envelope excedeu o limite");
            break;
        }
        let Ok(input) = serde_json::from_str::<Value>(&line) else {
            eprintln!("Envelope inválido");
            break;
        };
        let args = input["args"].as_array().cloned().unwrap_or_default();
        let result = invoke(&store, &s(&input, "fn"), &args).await;
        let reply = match result {
            Ok(result) => json!({"id":input["id"],"ok":true,"result":result}),
            Err(e) => {
                json!({"id":input["id"],"ok":false,"error":{"message":e.message,"status":e.status,"extra":e.extra}})
            }
        };
        println!("{reply}");
        let _ = std::io::stdout().flush();
    }
    // End of input: flush pending derived state before the process ends.
    let _ = store.call(Request::Close).await;
}
