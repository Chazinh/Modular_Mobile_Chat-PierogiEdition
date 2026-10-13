//! Measures the REAL context cost in the native store, without IPC or JSON text between processes.
//! Manual: MC_BENCH_FIXTURE=<context-N.json> cargo test --release --test bench_context -- --ignored --nocapture
//! (the fixtures are the ones from scripts/benchmark-context.mjs; without the variable the test is ignored).
use modularchat_server::store::{Request, Store};
use serde_json::json;
use std::time::Instant;

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() as f64 * p).ceil() as usize).saturating_sub(1)]
}

#[tokio::test]
#[ignore = "benchmark manual; defina MC_BENCH_FIXTURE"]
async fn context_data_wall_clock_without_ipc() {
    let Some(path) = std::env::var_os("MC_BENCH_FIXTURE") else {
        eprintln!("MC_BENCH_FIXTURE ausente: nada a medir");
        return;
    };
    let fixture: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let imported = store.call(Request::ImportOne(fixture)).await.unwrap();
    let input = json!({"convId":imported["id"],"pendingUserText":"Onde está a chave dourada?","maxPairs":50});
    let first = Instant::now();
    let data = store.call(Request::Context(input.clone())).await.unwrap();
    let cold = first.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(data["pairs"].as_array().unwrap().len(), 50);
    for _ in 0..5 {
        store.call(Request::Context(input.clone())).await.unwrap();
    }
    let mut samples = Vec::new();
    for _ in 0..60 {
        let t = Instant::now();
        store.call(Request::Context(input.clone())).await.unwrap();
        samples.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    println!(
        "native-context turns={} cold={cold:.2}ms p50={:.2}ms p95={:.2}ms",
        data["pathLength"],
        percentile(&samples, 0.5),
        percentile(&samples, 0.95)
    );
}

/// Prompt composition per section (tokens by the `rules` estimator), on the same path as generation:
/// getContextData -> lexical retrieval -> `context::assemble`. The prompt `sha` proves that a
/// store optimization changed nothing of what goes to the model.
/// Manual: MC_BENCH_FIXTURE=<file.json> [MC_BENCH_MAXPAIRS=30] cargo test --test bench_context prompt_composition -- --ignored --nocapture
#[tokio::test]
#[ignore = "benchmark manual; defina MC_BENCH_FIXTURE"]
async fn prompt_composition_report() {
    use modularchat_server::{context, presets, rules};
    let Some(path) = std::env::var_os("MC_BENCH_FIXTURE") else {
        eprintln!("MC_BENCH_FIXTURE ausente: nada a medir");
        return;
    };
    let max_pairs: u64 = std::env::var("MC_BENCH_MAXPAIRS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    let fixture: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    // generous deadline: the 1st read of a long conversation derives the narrative snapshots (slow in debug)
    let store =
        Store::open_with_limits(dir.path(), 64, std::time::Duration::from_secs(600)).unwrap();
    let imported = store.call(Request::ImportOne(fixture)).await.unwrap();
    let conv_id = imported["id"].clone();
    let query = "Onde está a chave dourada?";
    let data = store
        .call(Request::Context(
            json!({"convId":conv_id,"pendingUserText":query,"maxPairs":max_pairs}),
        ))
        .await
        .unwrap();
    let conv = store
        .call(Request::GetMeta(conv_id.as_str().unwrap().to_owned()))
        .await
        .unwrap();
    let start = data["pairs"]
        .as_array()
        .and_then(|a| a.get(a.len().saturating_sub(30)))
        .map(|p| p["seq"].clone())
        .unwrap_or(data["current"]["seq"].clone());
    let retrieval = store
        .call(Request::Retrieve(json!({"convId":conv_id,"queryText":query,"beforeSeq":data["beforeSeq"],"windowStartSeq":start,"limit":24})))
        .await
        .unwrap();
    let plan = presets::request(&conv, vec![], &json!({}));
    let ctx = context::assemble(&data, &retrieval, "send", &plan, None).unwrap();
    let bytes = |k: &str| serde_json::to_string(&data[k]).unwrap().len();
    let last = ctx.messages.last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_owned();
    let live_end = last.find("[FIM DO CONTEXTO]").unwrap_or(0);
    let mut blocks = Vec::<(String, usize)>::new();
    for chunk in last[..live_end].split("\n\n[") {
        let header: String = chunk
            .trim_start_matches('[')
            .chars()
            .take_while(|c| *c != ']' && *c != '—')
            .collect();
        blocks.push((header.trim().to_owned(), rules::estimate(chunk)));
    }
    println!(
        "composition {}",
        json!({
            "turns": data["pathLength"],
            "maxPairsAsked": max_pairs,
            "pairsLoaded": data["pairs"].as_array().map(Vec::len),
            "tokens": ctx.report["tokens"],
            "pairsIncluded": ctx.report["pairsIncluded"],
            "timelineItems": ctx.report["timelineItems"],
            "liveBlocks": blocks,
            "dataBytes": {"total": serde_json::to_string(&data).unwrap().len(), "pairs": bytes("pairs"), "unprocessed": bytes("unprocessed"), "notes": bytes("notes"), "episodes": bytes("episodes")},
            "promptSha": rules::hash(&serde_json::to_string(&ctx.messages).unwrap()),
        })
    );
}
