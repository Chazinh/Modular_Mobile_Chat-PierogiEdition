use modularchat_server::{
    context, curation, memory_prompt, presets, provenance, rules,
    store::{Request, Store},
};
use serde_json::{Value, json};
use std::collections::HashSet;

fn line_json(prompt: &str, label: &str) -> Value {
    serde_json::from_str(
        prompt
            .lines()
            .find_map(|line| line.strip_prefix(label))
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn extraction_prompt_projects_memory_by_id_and_keeps_semantics_and_identified_text() {
    let content = "Mira guarda uma carta na torre.";
    let original = "Mira: A lua protege esta chave azul.";
    let quote = "A lua protege esta chave azul.";
    let a = json!({
        "id":"00000000-0000-4000-8000-000000000001","type":"memory","kind":"SECRET","subject":"Mira",
        "content":content,"text":content,"visibility":"private","sourceSeq":10,"seq":10,
        "sourceId":null,"sourceInteractionId":"00000000-0000-4000-8000-000000000099","interactionId":"00000000-0000-4000-8000-000000000099",
        "start":0,"end":0,"score":0.987,"parts":{"sem":0.95},"eligible":true,"importance":0.9,"confidence":0.8,
        "knowledge":[{"id":"internal-knowledge-id","name":"Mira","state":"knows","via":"witnessed","evidence":"Mira guardou a carta."},
            {"name":"Lia","state":"believes","belief":"A carta foi queimada.","via":"told","evidence":"Lia ouviu a mentira."}]
    });
    let mut b = a.clone();
    b["id"] = json!("00000000-0000-4000-8000-000000000002");
    b["visibility"] = json!("public");
    b["knowledge"] = json!([{"name":"Alex","state":"unaware"}]);
    let mut c = a.clone();
    c["id"] = json!("00000000-0000-4000-8000-000000000003");
    c["subject"] = json!("Nora");
    // Sanitized facts carried between chunks use `who`, retrieved knowledge uses `name`.
    c["knowledge"] =
        json!([{"who":"Nora","state":"knows","via":"received","evidence":"Nora recebeu a carta."}]);
    let source_id = "11111111-1111-4111-8111-111111111111";
    let source = json!({"id":"chunk-diagnostic-id","type":"raw","text":original,"sourceId":source_id,
        "sourceRole":"assistant","role":"assistant","canonicalSpeaker":"Mira","start":0,
        "end":rules::utf16_len(original),"offsets":[0,rules::utf16_len(original)],"evidence":original,"score":0.8});
    let scene = json!({"id":"scene-diagnostic-id","interactionId":"scene-interaction-id","seq":10,
        "location":"torre","time":"noite","mood":"tensão","present":["Mira"],"objects":["carta"],
        "characters":[{"key":"mira","id":"character-diagnostic-id","name":"Mira","location":"janela","position":"sentada","holding":["chave"]}],
        "positions":{"mira":"sentada"},"locations":{"mira":"janela"},"holding":{"mira":["chave"]}});
    let input = json!({"conv":{"persona":"Alex"},"userText":format!("Lia: “{quote}”"),"threads":["Devolver a chave"],
        "timeline":{"current":{"interactionId":"timeline-diagnostic-id","versionIds":["version-diagnostic-id"],"days":8,"months":2,"date":"2026-10-15",
            "boundaries":[{"sourceId":"cache-boundary-id","start":100}],"sources":[{"id":"cache-source-id"}],
            "anchors":[{"kind":"anchor","days":0,"months":2,"elapsedDays":8,"elapsedMonths":4,"evidence":"há dois meses","sourceId":"cache-anchor-id","start":123}],
            "advances":[{"kind":"advance","days":1,"months":0,"evidence":"no dia seguinte","start":50}],"scenes":[scene]}}});
    let related = json!([a, a, b]);
    let retrieval = json!({"candidates":[a,b,c,source]});
    let sources = vec![json!({"id":source_id,"role":"assistant","text":original})];
    let previous = provenance::source(source_id, "assistant", original, None);
    let turn = provenance::source(
        "current-text-source",
        "user",
        "Alex: Vou buscar a carta 🚀.",
        Some("Alex"),
    );
    let before = serde_json::to_vec(&json!([input, scene, related, retrieval, sources])).unwrap();
    let prompt = memory_prompt::desktop(
        &input, &scene, &related, &retrieval, &turn, &previous, &sources,
    );

    let prior = line_json(&prompt, "FATOS ANTERIORES: ");
    let recovered = line_json(&prompt, "RECUPERAÇÃO: ");
    assert_eq!(prior.as_array().unwrap().len(), 2);
    assert_eq!(recovered.as_array().unwrap().len(), 2);
    assert_eq!(
        prior[0]["content"], prior[1]["content"],
        "different IDs with equal content both survive"
    );
    assert_eq!(prior[0]["visibility"], "private");
    assert_eq!(prior[1]["visibility"], "public");
    assert_eq!(prior[0]["knowledge"][1]["state"], "believes");
    assert_eq!(prior[0]["knowledge"][1]["belief"], "A carta foi queimada.");
    assert_eq!(prior[0]["knowledge"][1]["evidence"], "Lia ouviu a mentira.");
    assert_eq!(prior[1]["knowledge"][0]["state"], "unaware");
    assert_eq!(recovered[0]["subject"], "Nora");
    assert_eq!(recovered[0]["knowledge"][0]["name"], "Nora");
    for key in [
        "sourceId",
        "sourceRole",
        "canonicalSpeaker",
        "start",
        "end",
        "offsets",
        "evidence",
    ] {
        assert_eq!(
            recovered[1][key], source[key],
            "functional provenance {key}"
        );
    }
    let projected_scene = line_json(&prompt, "CENA ANTERIOR: ");
    assert_eq!(
        projected_scene["characters"][0]["holding"],
        json!(["chave"])
    );
    for key in ["positions", "locations", "holding"] {
        assert_eq!(projected_scene[key], scene[key]);
    }
    let time = line_json(&prompt, "TEMPO: ");
    assert_eq!(time["days"], 8);
    assert_eq!(time["months"], 2);
    assert_eq!(time["anchors"][0]["elapsedMonths"], 4);
    assert_eq!(time["advances"][0]["evidence"], "no dia seguinte");
    assert!(prompt.contains(&previous) && prompt.ends_with(&turn));
    assert!(prompt.contains("foi dita originalmente por Mira"));
    for gone in [
        "00000000-0000-4000-8000-0000000000",
        "internal-knowledge-id",
        "scene-diagnostic-id",
        "scene-interaction-id",
        "character-diagnostic-id",
        "timeline-diagnostic-id",
        "version-diagnostic-id",
        "cache-boundary-id",
        "cache-source-id",
        "cache-anchor-id",
        "chunk-diagnostic-id",
        "\"score\"",
        "\"parts\"",
        "\"eligible\"",
        "\"importance\"",
        "\"confidence\"",
        "\"sourceSeq\"",
        "\"interactionId\"",
    ] {
        assert!(!prompt.contains(gone), "unnecessary metadata: {gone}");
    }
    assert_eq!(
        before,
        serde_json::to_vec(&json!([input, scene, related, retrieval, sources])).unwrap()
    );
}

#[tokio::test]
async fn assembling_and_projecting_disposable_store_memory_keeps_the_persisted_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let conv = store
        .call(Request::CreateConversation(
            json!({"persona":"Alex","prompt":"Mira estuda na torre.","routeId":"cc/claude-sonnet-5-5","presetId":"claude-sonnet-5-5","effort":"low"}),
        ))
        .await
        .unwrap();
    let cid = conv["id"].as_str().unwrap().to_owned();
    let memory = store.call(Request::AddMemory { conv_id:cid.clone(), value:json!({"kind":"SECRET","subject":"Mira","content":"Mira guarda uma carta na torre.","visibility":"private","knowledge":[{"name":"Mira","state":"knows"}]}) }).await.unwrap();
    let before = store.call(Request::Memories(cid.clone())).await.unwrap();
    assert_eq!(before["memories"][0]["id"], memory["id"]);
    let snapshot_before = store
        .call(Request::Export(Some(cid.clone())))
        .await
        .unwrap()["conversations"][0]
        .clone();
    let data = store
        .call(Request::Context(json!({"convId":cid,"maxPairs":30})))
        .await
        .unwrap();
    let retrieved = store.call(Request::Retrieve(json!({"convId":cid,"queryText":"Mira carta torre","windowStartSeq":1,"beforeSeq":1,"limit":8}))).await.unwrap();
    let plan = presets::request(&json!({"presetId":"claude-sonnet-5-5"}), vec![], &json!({}));
    context::assemble(&data, &retrieved, "send", &plan, None).unwrap();
    let input = json!({"conv":conv,"threads":[],"timeline":data["timeline"]});
    let prompt = memory_prompt::desktop(
        &input,
        &data["scene"],
        &before["memories"],
        &retrieved,
        "Alex: Oi.",
        "",
        &[],
    );
    assert!(prompt.contains("Mira guarda uma carta na torre."));
    let after = store.call(Request::Memories(cid.clone())).await.unwrap();
    let snapshot_after =
        store.call(Request::Export(Some(cid))).await.unwrap()["conversations"][0].clone();
    assert_eq!(
        serde_json::to_vec(&before).unwrap(),
        serde_json::to_vec(&after).unwrap()
    );
    assert_eq!(
        serde_json::to_vec(&snapshot_before).unwrap(),
        serde_json::to_vec(&snapshot_after).unwrap()
    );
}

// The previous implementation is the oracle: this optimization must preserve
// its ordered, all-pairs grouping decisions, including malformed item fields.
fn legacy_clusters(memories: &[Value]) -> Vec<Vec<Value>> {
    let text = |m: &Value, key: &str| m[key].as_str().unwrap_or_default().to_owned();
    let mut used: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for (i, a) in memories.iter().enumerate() {
        if out.len() >= 6 {
            break;
        }
        if used.contains(&text(a, "id")) {
            continue;
        }
        let mut group = vec![a.clone()];
        for b in memories.iter().skip(i + 1) {
            if group.len() >= 5 {
                break;
            }
            if used.contains(&text(b, "id"))
                || text(b, "kind") != text(a, "kind")
                || rules::key(&text(b, "subject")) != rules::key(&text(a, "subject"))
            {
                continue;
            }
            if group.iter().all(|g| {
                rules::jaccard(&text(g, "content"), &text(b, "content"))
                    >= curation::CLUSTER_MIN_SIMILARITY
            }) {
                group.push(b.clone());
            }
        }
        if group.len() > 1 {
            for g in &group {
                used.insert(text(g, "id"));
            }
            out.push(group);
        }
    }
    out
}

#[test]
fn prepared_clustering_matches_the_previous_algorithm_through_400_items() {
    let broad: Vec<Value> = (0..400)
        .map(|i| {
            json!({
                "id":if i % 29 == 0 { "repeated-id".to_owned() } else { format!("m{i}") },
                "kind":if i%7 == 0 {"SECRET"} else {"EVENT"},
                "subject":(["Míra","MIRA","Mira","Nora"][i%4]),
                "content":match i%8 {
                    0 => "a e de para".to_owned(),
                    1 => "".to_owned(),
                    2 => "Mira guarda carta secreta bolso casaco azul vermelho prata".to_owned(),
                    3 => "Mira guarda carta secreta bolso casaco azul vermelho".to_owned(),
                    4 => "Mira guarda carta secreta bolso casaco azul".to_owned(),
                    _ => format!("Mira guarda carta secreta bolso casaco grupo{}", i%13),
                }
            })
        })
        .collect();
    let late: Vec<Value> = (0..400)
        .map(|i| {
            json!({"id":format!("l{i}"),"kind":"EVENT",
        "subject":if i<370 {format!("Pessoa {i}")} else {"Mira".to_owned()},
        "content":"Mira guarda uma carta secreta no bolso do casaco azul."})
        })
        .collect();
    let sparse: Vec<Value> = (0..400)
        .map(|i| {
            json!({"id":format!("s{i}"),"kind":"EVENT","subject":"Mira",
        "content":format!("runa{i} estrela{i} chave{i}")})
        })
        .collect();
    for fixture in [&broad, &late, &sparse] {
        for count in [0, 1, 20, 100, 400] {
            assert_eq!(
                curation::build_clusters(&fixture[..count]),
                legacy_clusters(&fixture[..count]),
                "prefix {count}"
            );
        }
        let reordered: Vec<_> = (0..400).map(|i| fixture[(i * 73) % 400].clone()).collect();
        assert_eq!(
            curation::build_clusters(&reordered),
            legacy_clusters(&reordered)
        );
    }
    let groups = curation::build_clusters(&late);
    assert_eq!(groups.len(), 6);
    assert!(groups.iter().all(|g| g.len() == 5));
    let malformed = json!([{}, {"id":4,"kind":null,"content":1},
        {"content":"Mira guarda uma carta secreta no bolso do casaco azul."},
        {"content":"Mira guarda uma carta secreta no bolso do casaco azul."}]);
    assert_eq!(
        curation::build_clusters(malformed.as_array().unwrap()),
        legacy_clusters(malformed.as_array().unwrap())
    );
}

#[test]
fn malformed_minimum_i64_group_is_ignored_without_preventing_a_valid_merge() {
    let content = "Mira guarda uma carta secreta no bolso do casaco azul.";
    let clusters = vec![vec![
        json!({"id":"a","content":content}),
        json!({"id":"b","content":content}),
    ]];
    let raw = json!({"decisions":([i64::MIN,-1,0,i64::MAX,1].map(|group|json!({"group":group,"action":"merge","keep_id":"a","merged":content})))});
    assert_eq!(
        curation::sanitize(&raw, &clusters),
        json!([{"keepId":"a","dropIds":["b"],"merged":content}])
            .as_array()
            .unwrap()
            .clone()
    );
}
