use modularchat_server::{context, presets};
use serde_json::{Value, json};

fn base_data() -> Value {
    json!({
        "conv": {
            "persona": "Alex",
            "prompt": "Lia e Carlos vivem na mansão.",
            "settings": {
                "context": {
                    "preferredPairs": 30,
                    "minPairs": 20,
                    "maxPairs": 50
                }
            }
        },
        "pairs": [
            {
                "id": "i1",
                "seq": 1,
                "user": "Olá Lia e Carlos, como estão?",
                "assistant": "*Lia e Carlos sorriem.*\n\nLia: Estamos bem, Alex.",
                "complete": true,
                "present_entities": ["Alex", "Lia", "Carlos"]
            }
        ],
        "totalPairs": 1,
        "beforeSeq": 2,
        "pathLength": 1,
        "cast": [],
        "scene": {
            "location": "Sala de estar",
            "time": "tarde",
            "mood": "calmo",
            "present": ["Alex", "Lia", "Carlos"],
            "objects": ["sofá", "mesa de centro", "lareira"],
            "characters": [
                {"name": "Lia", "position": "sofá"},
                {"name": "Carlos", "position": "poltrona"}
            ]
        },
        "pinned": [],
        "relationships": [],
        "threads": [],
        "notes": [],
        "episodes": [],
        "timeline": {"current": null},
        "sync": {"lag": 0, "synced": true},
        "unprocessed": [],
        "current": {
            "interactionId": null,
            "seq": 2,
            "userText": "O que faremos hoje?",
            "priorParts": []
        }
    })
}

fn plan() -> presets::Plan {
    presets::request(
        &json!({"presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5"}),
        vec![],
        &json!({}),
    )
}

fn assembled_text(ctx: &context::Context) -> String {
    ctx.messages
        .iter()
        .map(|m| format!("[{}]: {}", m["role"].as_str().unwrap_or("?"), m["content"].as_str().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\n---\n")
}

#[test]
fn probe_1_assistant_transition_turn_should_not_wipe_active_characters() {
    let mut d = base_data();
    // In pair 1, the assistant mentioned a conversational phrase that includes a movement prefix:
    // e.g. "A conversa segue para outro rumo" or "Lia volta para a cadeira"
    d["pairs"] = json!([
        {
            "id": "i1",
            "seq": 1,
            "user": "Podemos mudar de assunto?",
            "assistant": "*Lia ri baixinho.*\n\nLia: Claro, a conversa segue para onde você quiser.",
            "complete": true,
            "present_entities": ["Alex", "Lia", "Carlos"]
        }
    ]);
    // In current turn, the user merely replies without movement:
    d["current"]["userText"] = json!("Ótimo, vamos falar de música.");

    let ctx = context::assemble(&d, &json!({}), "send", &plan(), None).unwrap();
    let prompt = assembled_text(&ctx);

    println!("PROBE 1 PROMPT:\n{prompt}");

    // Check if Lia and Carlos were erroneously invalidated:
    let contains_invalidated_lia = prompt.contains("presença de (Lia") || prompt.contains("Lia, Carlos) na cena anterior foi invalidada") || prompt.contains("presença de (Carlos, Lia)");
    assert!(
        !contains_invalidated_lia,
        "BUG REPRODUCED: Assistant conversation turn erroneously triggered scene transition and invalidated present NPCs!"
    );
}

#[test]
fn probe_2_idiomatic_action_should_not_trigger_scene_transition_and_eviction() {
    let mut d = base_data();
    // User does an everyday idiomatic action in the room:
    d["current"]["userText"] = json!("*Deixo o copo de água na mesa de centro e olho para vocês.* O que faremos hoje?");

    let ctx = context::assemble(&d, &json!({}), "send", &plan(), None).unwrap();
    let prompt = assembled_text(&ctx);

    println!("PROBE 2 PROMPT:\n{prompt}");

    assert!(
        !prompt.contains("transição de cena detectada"),
        "BUG REPRODUCED: 'Deixo o copo' triggered false positive scene transition!"
    );
    assert!(
        !prompt.contains("foi invalidada"),
        "BUG REPRODUCED: Present characters were invalidated because user put down a glass!"
    );
}

#[test]
fn probe_3_emotional_state_phrase_should_not_evict_room_companions() {
    let mut d = base_data();
    // User expresses an emotional reaction:
    d["current"]["userText"] = json!("*Alex entra em choque ao ouvir a notícia.* Como isso é possível?!");

    let ctx = context::assemble(&d, &json!({}), "send", &plan(), None).unwrap();
    let prompt = assembled_text(&ctx);

    println!("PROBE 3 PROMPT:\n{prompt}");

    assert!(
        !prompt.contains("transição de cena detectada"),
        "BUG REPRODUCED: 'Alex entra em choque' triggered false positive scene transition!"
    );
    assert!(
        !prompt.contains("foi invalidada"),
        "BUG REPRODUCED: Companions in room were evicted because Alex entered shock!"
    );
}

#[tokio::test]
async fn probe_4_absent_persona_cannot_witness_private_facts_in_extraction() {
    let dir = tempfile::tempdir().unwrap();
    let store = modularchat_server::store::Store::open(dir.path()).unwrap();
    let conv = store
        .call(modularchat_server::store::Request::CreateConversation(json!({
            "name": "Mansão",
            "persona": "Alex",
            "routeId": "cc/claude-sonnet-5-5",
            "presetId": "claude-sonnet-5-5",
            "prompt": "Lia e Mark estão no porão."
        })))
        .await
        .unwrap();
    let cid = conv["id"].as_str().unwrap();

    let begin = store
        .call(modularchat_server::store::Request::BeginTurn(
            serde_json::from_value::<modularchat_server::store::TurnRequest>(json!({
                "convId": cid,
                "mode": "send",
                "content": "Turno narrativo fora da vista de Alex.",
                "targetId": null,
                "routeId": "cc/claude-sonnet-5-5",
                "presetId": "claude-sonnet-5-5",
                "effort": "low"
            }))
            .unwrap(),
        ))
        .await
        .unwrap();

    let iid = begin["interactionId"].as_str().unwrap();
    let vid = begin["versionId"].as_str().unwrap();

    // Scene in the cellar: only Lia and Mark are present. Alex (persona) is absent!
    let extraction = json!({
        "summary": "Lia e Mark conversam no porão.",
        "cast": [],
        "scene": {
            "location": "porão",
            "present": ["Lia", "Mark"]
        },
        "facts": [
            {
                "kind": "SECRET",
                "subject": "Lia",
                "content": "Lia escondeu o mapa secreto no forro.",
                "importance": 0.9,
                "confidence": 0.9,
                "visibility": "private",
                "knowledge": [
                    {
                        "who": "Alex",
                        "state": "knows",
                        "via": "witnessed",
                        "evidence": "Lia escondeu o mapa secreto no forro."
                    }
                ]
            }
        ],
        "relationships": [],
        "openThreads": [],
        "resolvedThreads": []
    });

    let res = store
        .call(modularchat_server::store::Request::ApplyExtraction(json!({
            "convId": cid,
            "interactionId": iid,
            "versionId": vid,
            "versionIds": [vid],
            "sourceText": "Lia escondeu o mapa secreto no forro.",
            "extraction": extraction
        })))
        .await
        .unwrap();
    assert_eq!(res["skipped"], false);

    // Now query context to see if Alex received "sabe" for this secret fact:
    let data = store
        .call(modularchat_server::store::Request::Context(json!({
            "convId": cid,
            "pendingUserText": "O que sei?",
            "maxPairs": 10
        })))
        .await
        .unwrap();

    let ctx = context::assemble(&data, &json!({}), "send", &plan(), None).unwrap();
    let prompt = assembled_text(&ctx);

    println!("PROBE 4 PROMPT:\n{prompt}");

    // Alex was NOT present in porão. Alex MUST NOT be projected as knowing this secret!
    let alex_knows_secret = prompt.contains("Lia escondeu o mapa secreto no forro")
        && (prompt.contains("Alex: sabe") || prompt.contains("sabe: Lia escondeu o mapa secreto"));

    assert!(
        !alex_knows_secret,
        "BUG REPRODUCED: Absent persona Alex was granted knowledge/witness of private fact in another room!"
    );
}

#[test]
fn probe_5_negative_stay_instruction_should_not_bring_character_along() {
    let mut d = base_data();
    // Alex leaves with Lia to the garden, explicitly telling Carlos to stay behind:
    d["current"]["userText"] = json!("*Seguro a mão de Lia: 'Carlos, fique aqui e vigie a porta.' Saímos para o jardim.*");

    let ctx = context::assemble(&d, &json!({}), "send", &plan(), None).unwrap();
    let prompt = assembled_text(&ctx);

    println!("PROBE 5 PROMPT:\n{prompt}");

    // Carlos was told to stay ("fique aqui"). Carlos MUST NOT be in effective_present:
    assert!(
        prompt.contains("presença de (Carlos) na cena anterior foi invalidada")
            || prompt.contains("Carlos) na cena anterior foi invalidada"),
        "BUG REPRODUCED: Carlos was brought along despite being ordered to stay behind ('fique aqui') because of plural movement pattern matching!"
    );
}

#[test]
fn probe_6_metaphorical_expressions_with_movement_words_should_not_invalidate_scene() {
    let cases = [
        "Alex entra em pânico com o barulho.",
        "Alex sai do sério com a provocação.",
        "Meu olhar volta para Lia.",
        "A conversa segue para um rumo inesperado.",
        "Deixo a porta entreaberta e sento na poltrona.",
        "Deixo o livro sobre o tapete e olho para vocês.",
        "Chego no meu limite de cansaço e deito no sofá."
    ];

    let mut failed_cases = Vec::new();

    for case in cases {
        let mut d = base_data();
        d["current"]["userText"] = json!(case);
        let ctx = context::assemble(&d, &json!({}), "send", &plan(), None).unwrap();
        let prompt = assembled_text(&ctx);

        if prompt.contains("transição de cena detectada") {
            failed_cases.push(case);
        }
    }

    println!("FAILED CASES TRIGGERING FALSE SCENE TRANSITIONS:\n{:#?}", failed_cases);
    assert!(
        failed_cases.is_empty(),
        "BUG REPRODUCED: The following non-transition expressions erroneously triggered scene transition and character eviction: {:?}",
        failed_cases
    );
}

#[tokio::test]
async fn probe_7_jobs_runner_must_not_allow_absent_persona_as_witness() {
    let dir = tempfile::tempdir().unwrap();
    let store = modularchat_server::store::Store::open(dir.path()).unwrap();
    let conv = store
        .call(modularchat_server::store::Request::CreateConversation(json!({
            "name": "Mansão",
            "persona": "Alex",
            "routeId": "cc/claude-sonnet-5-5",
            "presetId": "claude-sonnet-5-5",
            "prompt": "Lia e Mark estão no porão."
        })))
        .await
        .unwrap();
    let cid = conv["id"].as_str().unwrap();

    let begin = store
        .call(modularchat_server::store::Request::BeginTurn(
            serde_json::from_value::<modularchat_server::store::TurnRequest>(json!({
                "convId": cid,
                "mode": "send",
                "content": "Turno onde Alex não está.",
                "targetId": null,
                "routeId": "cc/claude-sonnet-5-5",
                "presetId": "claude-sonnet-5-5",
                "effort": "low"
            }))
            .unwrap(),
        ))
        .await
        .unwrap();

    let iid = begin["interactionId"].as_str().unwrap();
    let vid = begin["versionId"].as_str().unwrap();

    // In jobs.rs, lines 561-574:
    // If f has knowledge for Alex with via=witnessed, jobs.rs checks:
    // if present.iter().any(...) || w_key == persona_key { deterministic_witnesses.insert(who); }
    // simulating what jobs.rs produces when f.get("witnesses").is_none():
    // Because of `|| w_key == persona_key`, jobs.rs populates:
    let jobs_populated_witnesses = json!(["Alex", "Lia"]);

    let extraction = json!({
        "summary": "Lia e Mark conversam no porão.",
        "cast": [],
        "scene": {
            "location": "porão",
            "present": ["Lia", "Mark"]
        },
        "facts": [
            {
                "kind": "SECRET",
                "subject": "Lia",
                "content": "Lia escondeu a chave do cofre sob o piso.",
                "importance": 0.9,
                "confidence": 0.9,
                "visibility": "private",
                "witnesses": jobs_populated_witnesses,
                "knowledge": [
                    {
                        "who": "Alex",
                        "state": "knows",
                        "via": "witnessed",
                        "evidence": "Lia escondeu a chave do cofre sob o piso."
                    }
                ]
            }
        ],
        "relationships": [],
        "openThreads": [],
        "resolvedThreads": []
    });

    let res = store
        .call(modularchat_server::store::Request::ApplyExtraction(json!({
            "convId": cid,
            "interactionId": iid,
            "versionId": vid,
            "versionIds": [vid],
            "sourceText": "Lia escondeu a chave do cofre sob o piso.",
            "extraction": extraction
        })))
        .await
        .unwrap();
    assert_eq!(res["skipped"], false);

    // Check SQLite knowledge_events directly:
    let db_path = dir.path().join("modularchat.db");
    let conn = rusqlite::Connection::open(db_path).unwrap();
    let mut stmt = conn.prepare("SELECT entity_key, state, via FROM knowledge_events WHERE conv_id=?").unwrap();
    let rows: Vec<(String, String, String)> = stmt
        .query_map([cid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .filter_map(std::result::Result::ok)
        .collect();

    println!("KNOWLEDGE EVENTS IN DB:\n{:#?}", rows);

    let alex_has_witnessed = rows.iter().any(|(entity, state, via)| entity == "alex" && state == "knows" && via == "witnessed");
    assert!(
        !alex_has_witnessed,
        "BUG REPRODUCED: Absent persona Alex was recorded in SQLite knowledge_events as having witnessed a secret in a scene where Alex was absent!"
    );
}



