use modularchat_server::{context, jobs, memory_backend, presets, store};
use serde_json::{Value, json};

fn base_context_data(persona: &str) -> Value {
    json!({
        "conv": {
            "persona": persona,
            "prompt": "História de teste com múltiplos NPCs e relações complexas.",
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
                "user": "Olá pessoal, alguém viu meu casaco?",
                "assistant": "*Lia e Carlos se entreolham.*\n\nLia: Não vi o casaco de Alex. Carlos, você viu?",
                "complete": true,
                "present_entities": [persona, "Lia", "Carlos"]
            }
        ],
        "totalPairs": 1,
        "beforeSeq": 2,
        "pathLength": 1,
        "cast": [],
        "scene": {
            "location": "Biblioteca Arcana",
            "time": "noite",
            "mood": "tenso",
            "present": [persona, "Lia", "Carlos", "Helena"],
            "objects": ["tomo antigo", "tocha de prata", "globo celestial"],
            "characters": [
                {
                    "name": "Lia",
                    "location": "Biblioteca Arcana",
                    "position": "estante leste",
                    "holding": ["adaga de cristal"],
                    "possessions": ["manto das sombras", "anel de safira"]
                },
                {
                    "name": "Carlos",
                    "location": "Biblioteca Arcana",
                    "position": "mesa central",
                    "holding": [],
                    "inventory": ["pergaminho de fogo", "bússola de latão"]
                },
                {
                    "name": "Helena",
                    "location": "Biblioteca Arcana",
                    "position": "alcova sombria",
                    "holding": ["orbe de luz"],
                    "possessions": []
                }
            ]
        },
        "pinned": [],
        "relationships": [
            {
                "a": "Helena",
                "b": persona,
                "relation_type": "parent_of",
                "directed": true,
                "description": format!("mãe de {persona}")
            },
            {
                "a": "Lia",
                "b": "Carlos",
                "relation_type": "sibling_of",
                "directed": false,
                "description": "irmãos gêmeos"
            }
        ],
        "threads": [],
        "notes": [],
        "episodes": [],
        "timeline": {"current": null},
        "sync": {"lag": 0, "synced": true},
        "unprocessed": [],
        "current": {
            "interactionId": null,
            "seq": 2,
            "userText": "Helena, você trouxe o medalhão da nossa família?",
            "priorParts": []
        }
    })
}

fn plan() -> presets::Plan {
    presets::request(
        &json!({"presetId": "claude-sonnet-5-5", "routeId": "cc/claude-sonnet-5-5"}),
        vec![],
        &json!({}),
    )
}

#[test]
fn test_challenger_negative_tail_rule_eradication_all_modes() {
    let modes = ["send", "continue", "regenerate", "edit"];
    let p = plan();

    for m in modes {
        let mut d = base_context_data("Alex");
        if m == "continue" || m == "regenerate" {
            d["current"]["priorParts"] = json!(["parte prévia..."]);
        }
        let ctx = context::assemble(&d, &json!({}), m, &p, None).unwrap();

        for msg in &ctx.messages {
            let content = msg["content"].as_str().unwrap();
            assert!(
                !content.contains("trate como 'sua mãe/família', nunca como 'minha'"),
                "Mode '{m}' must NOT contain the toxic negative tail rule anywhere in context!"
            );
            assert!(
                !content.contains("nunca como 'minha'"),
                "Mode '{m}' must NOT contain 'nunca como minha' anywhere in context!"
            );
        }

        let tail = ctx.messages.last().unwrap()["content"].as_str().unwrap();
        assert!(
            tail.contains("desambiguação de posse e parentesco: siga a matriz de POV"),
            "Mode '{m}' tail must contain positive POV matrix reminder"
        );
        assert!(
            tail.contains("pertences e familiares pertencem estritamente ao respectivo dono"),
            "Mode '{m}' tail must declare strict ownership"
        );
    }
}

#[test]
fn test_challenger_multi_npc_possession_and_choral_view() {
    let d = base_context_data("Alex");
    let p = plan();
    let ctx = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    let text = ctx.messages.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n");

    // Verify room objects vs character belongings
    assert!(text.contains("Objetos: tomo antigo, tocha de prata, globo celestial"));
    assert!(text.contains("Lia — local: Biblioteca Arcana; posição: estante leste; segura: adaga de cristal; pertences: manto das sombras, anel de safira"));
    assert!(text.contains("Carlos — local: Biblioteca Arcana; posição: mesa central; pertences: pergaminho de fogo, bússola de latão"));
    assert!(text.contains("Helena — local: Biblioteca Arcana; posição: alcova sombria; segura: orbe de luz"));

    // Verify system prompt contains multi-NPC POV Level 3
    let sys = ctx.messages.first().unwrap()["content"].as_str().unwrap();
    assert!(sys.contains("3. NÍVEL 3 — DIÁLOGO ENTRE PERSONAGENS (NPC falando com outro NPC / Dinâmica Coral)"));
    assert!(sys.contains("É TERMINANTEMENTE PROIBIDO usar \"sua mãe\" com outro NPC para se referir à família de Alex"));
}

#[test]
fn test_challenger_disambiguate_memory_content_extreme_accents_and_boundaries() {
    // 1. Accented vowels and diacritics
    let cases = [
        ("Alex", "minha mãe é médica em Belém", "A mãe de Alex é médica em Belém"),
        ("Alex", "Minha mãe é médica em Belém", "A mãe de Alex é médica em Belém"),
        ("João", "meu pai é pescador no Maranhão", "O pai de João é pescador no Maranhão"),
        ("Inês", "minha família mora em Goiás", "A família de Inês mora em Goiás"),
        ("Helena", "meu irmão caçula nasceu em São Paulo", "O irmão de Helena caçula nasceu em São Paulo"),
        ("Carlos", "minha irmã mais velha estuda em Brasília", "A irmã de Carlos mais velha estuda em Brasília"),
        ("Alex", "meu relógio de prata foi roubado", "O relógio de Alex de prata foi roubado"),
        ("Alex", "minha bolsa de couro rasgou", "A bolsa de Alex de couro rasgou"),
        // Single-noun possessive without trailing words
        ("Alex", "minha espada", "A espada de Alex"),
        ("Alex", "meu casaco", "O casaco de Alex"),
        // Non-matching stays intact
        ("Alex", "O livro antigo pertence a Lia", "O livro antigo pertence a Lia"),
        ("Alex", "Mundo em guerra", "Mundo em guerra"),
        ("mundo", "minha mãe querida", "minha mãe querida"),
        // Without space after possessive pronoun (single-word isolated tokens remain unchanged)
        ("Alex", "minha", "minha"),
        ("Alex", "meu", "meu"),
        ("Alex", "", ""),
    ];

    for (sub, input, expected) in cases {
        let result = context::disambiguate_memory_content(sub, input);
        assert_eq!(result, expected, "Failed for subject '{sub}' and input '{input}'");
    }
}

#[test]
fn test_challenger_normalize_fact_text_extreme_utf8_and_compound_phrases() {
    // Normalization across various speakers, personas and multi-byte UTF-8 text
    assert_eq!(
        jobs::normalize_fact_text("minha melhor amiga foi para Paris", "Cláudia", "Alex"),
        "a melhor amiga de Cláudia foi para Paris"
    );
    assert_eq!(
        jobs::normalize_fact_text("meus melhores amigos viajaram juntos", "João", "Alex"),
        "os melhores amigos de João viajaram juntos"
    );
    assert_eq!(
        jobs::normalize_fact_text("minhas melhores memórias são da infância", "Inês", "Alex"),
        "as melhores memórias de Inês são da infância"
    );
    assert_eq!(
        jobs::normalize_fact_text("sua mãe mandou lembranças para você", "Lia", "Alex"),
        "a mãe de Alex mandou lembranças para você"
    );
    assert_eq!(
        jobs::normalize_fact_text("seu carro foi lavado hoje de manhã", "Carlos", "Alex"),
        "o carro de Alex foi lavado hoje de manhã"
    );
    assert_eq!(
        jobs::normalize_fact_text("suas chaves estão sobre a mesa", "Helena", "Alex"),
        "as chaves de Alex estão sobre a mesa"
    );
    assert_eq!(
        jobs::normalize_fact_text("seus livros estão guardados", "Helena", "Alex"),
        "os livros de Alex estão guardados"
    );
    // Raw 'eu'
    assert_eq!(
        jobs::normalize_fact_text("eu viajei para as montanhas", "Valquíria", "Valquíria"),
        "Valquíria viajei para as montanhas"
    );
    // Empty inputs
    assert_eq!(jobs::normalize_fact_text("", "", ""), "");
    assert_eq!(jobs::normalize_fact_text("fato sem pronomes", "", ""), "fato sem pronomes");
}

#[tokio::test]
async fn test_challenger_directed_relationship_triad_and_mutation_in_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let store = store::Store::open(dir.path()).unwrap();

    let conv = store
        .call(store::Request::CreateConversation(json!({
            "name": "Triade Genealogica",
            "persona": "Alex",
            "routeId": "cc/claude-sonnet-5-5",
            "presetId": "claude-sonnet-5-5",
            "prompt": "Relações direcionadas entre múltiplos nós da família."
        })))
        .await
        .unwrap();
    let cid = conv["id"].as_str().unwrap();

    // Turn 1: Laura is mother of Alex (Laura -> Alex)
    let b1 = store
        .call(store::Request::BeginTurn(
            serde_json::from_value::<store::TurnRequest>(json!({
                "convId": cid,
                "mode": "send",
                "content": "Laura entra.",
                "targetId": null,
                "routeId": "cc/claude-sonnet-5-5",
                "presetId": "claude-sonnet-5-5",
                "effort": "low"
            }))
            .unwrap(),
        ))
        .await
        .unwrap();
    let (iid_1, vid_1, gid_1) = (b1["interactionId"].as_str().unwrap(), b1["versionId"].as_str().unwrap(), b1["generationId"].as_str().unwrap());
    store.call(store::Request::FinishGeneration(store::FinishRequest {
        generation_id: gid_1.to_string(),
        text: Some("Laura diz ser mãe de Alex.".to_string()),
        status: "complete".to_string(),
        usage: Value::Null,
        error: None,
    })).await.unwrap();

    store.call(store::Request::ApplyExtraction(json!({
        "convId": cid,
        "interactionId": iid_1,
        "versionId": vid_1,
        "extraction": {
            "summary": "Laura é mãe de Alex.",
            "cast": [],
            "scene": {"location": "casa", "present": ["Alex", "Laura"]},
            "facts": [],
            "relationships": [
                {
                    "a": "Laura",
                    "b": "Alex",
                    "relation_type": "parent_of",
                    "directed": true,
                    "description": "mãe de Alex"
                }
            ],
            "openThreads": [],
            "resolvedThreads": []
        }
    }))).await.unwrap();

    // Turn 2: Alex is son of Laura (Alex -> Laura) AND Laura is daughter of Vovó Carmen (Carmen -> Laura)
    let b2 = store
        .call(store::Request::BeginTurn(
            serde_json::from_value::<store::TurnRequest>(json!({
                "convId": cid,
                "mode": "send",
                "content": "Carmen entra.",
                "targetId": null,
                "routeId": "cc/claude-sonnet-5-5",
                "presetId": "claude-sonnet-5-5",
                "effort": "low"
            }))
            .unwrap(),
        ))
        .await
        .unwrap();
    let (iid_2, vid_2, gid_2) = (b2["interactionId"].as_str().unwrap(), b2["versionId"].as_str().unwrap(), b2["generationId"].as_str().unwrap());
    store.call(store::Request::FinishGeneration(store::FinishRequest {
        generation_id: gid_2.to_string(),
        text: Some("Carmen chega com Laura.".to_string()),
        status: "complete".to_string(),
        usage: Value::Null,
        error: None,
    })).await.unwrap();

    store.call(store::Request::ApplyExtraction(json!({
        "convId": cid,
        "interactionId": iid_2,
        "versionId": vid_2,
        "extraction": {
            "summary": "Alex confirma ser filho de Laura e Carmen é mãe de Laura.",
            "cast": [],
            "scene": {"location": "casa", "present": ["Alex", "Laura", "Carmen"]},
            "facts": [],
            "relationships": [
                {
                    "a": "Alex",
                    "b": "Laura",
                    "relation_type": "child_of",
                    "directed": true,
                    "description": "filho de Laura"
                },
                {
                    "a": "Carmen",
                    "b": "Laura",
                    "relation_type": "parent_of",
                    "directed": true,
                    "description": "mãe de Laura"
                }
            ],
            "openThreads": [],
            "resolvedThreads": []
        }
    }))).await.unwrap();

    // Verify context data from store
    let ctx_data = store
        .call(store::Request::Context(json!({
            "convId": cid,
            "pendingUserText": "Quem são vocês?",
            "mode": "send"
        })))
        .await
        .unwrap();

    let rels = ctx_data["relationships"].as_array().unwrap();

    // 1. Both directed edges Laura -> Alex AND Alex -> Laura must coexist simultaneously
    let has_laura_alex = rels.iter().any(|r| r["a_key"] == "laura" && r["b_key"] == "alex");
    let has_alex_laura = rels.iter().any(|r| r["a_key"] == "alex" && r["b_key"] == "laura");
    assert!(has_laura_alex, "Laura -> Alex relationship must exist");
    assert!(has_alex_laura, "Alex -> Laura relationship must exist (not overwritten by reverse edge)");

    // 2. Carmen -> Laura must also exist
    let has_carmen_laura = rels.iter().any(|r| r["a_key"] == "carmen" && r["b_key"] == "laura");
    assert!(has_carmen_laura, "Carmen -> Laura relationship must exist");

    // 3. Automatic reciprocal for Carmen -> Laura (Laura -> Carmen: filho(a) de Carmen)
    let has_laura_carmen = rels.iter().any(|r| r["a_key"] == "laura" && r["b_key"] == "carmen");
    assert!(has_laura_carmen, "Laura -> Carmen reciprocal must be derived");
}

#[test]
fn test_challenger_schema_edge_cases_and_unknown_fields() {
    let schema: Value = serde_json::from_str(include_str!("../src/extraction-schema.json")).unwrap();

    // Valid full kinship and possession extraction
    let valid = json!({
        "summary": "Resumo adversarial.",
        "cast": [],
        "scene": {
            "location": "Torre",
            "time": "madrugada",
            "mood": "sombrio",
            "present": ["Alex", "Valquíria"],
            "objects": ["candelabro"],
            "characters": [
                {"name": "Valquíria", "location": "Torre", "position": "janela", "holding": ["arco longo"]}
            ]
        },
        "facts": [
            {
                "kind": "KINSHIP",
                "subject": "Valquíria",
                "content": "Valquíria é prima de Alex.",
                "importance": 0.7,
                "confidence": 0.95,
                "visibility": "private",
                "replaces": "",
                "existing_id": "",
                "attribution": {"quote": "minha prima Valquíria", "originalSpeaker": "Alex"},
                "knowledge": [],
                "kinship": {
                    "subject": "Valquíria",
                    "relation": "relative_of",
                    "target": "Alex"
                }
            },
            {
                "kind": "POSSESSION",
                "subject": "Alex",
                "content": "Alex possui uma adaga élfica.",
                "importance": 0.8,
                "confidence": 0.9,
                "visibility": "private",
                "replaces": "",
                "existing_id": "",
                "knowledge": [],
                "possession": {
                    "owner": "Alex",
                    "item": "adaga élfica",
                    "status": "owned"
                }
            },
            {
                "kind": "INVENTORY",
                "subject": "Alex",
                "content": "Alex segura uma poção de cura.",
                "importance": 0.5,
                "confidence": 0.85,
                "visibility": "private",
                "replaces": "",
                "existing_id": "",
                "knowledge": [],
                "possession": {
                    "owner": "Alex",
                    "item": "poção de cura",
                    "status": "held"
                }
            }
        ],
        "relationships": [
            {
                "a": "Valquíria",
                "b": "Alex",
                "relation_type": "relative_of",
                "directed": true,
                "description": "prima paterna de Alex"
            }
        ],
        "open_threads": [],
        "resolved_threads": []
    });

    assert!(
        memory_backend::validate_schema(&valid, &schema).is_ok(),
        "Adversarial schema validation must succeed for complete schema"
    );

    // Reject unknown relation_type in relationship
    let mut bad_rel = valid.clone();
    bad_rel["relationships"][0]["relation_type"] = json!("arch_nemesis");
    assert!(
        memory_backend::validate_schema(&bad_rel, &schema).is_err(),
        "Unknown relation_type in relationships must fail"
    );

    // Reject unknown possession status in facts
    let mut bad_pos = valid.clone();
    bad_pos["facts"][1]["possession"]["status"] = json!("stolen_and_disintegrated");
    assert!(
        memory_backend::validate_schema(&bad_pos, &schema).is_err(),
        "Unknown possession status must fail"
    );
}
