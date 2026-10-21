use modularchat_server::{context, jobs, memory_backend, presets, store};
use serde_json::{Value, json};

fn base_context_data() -> Value {
    json!({
        "conv": {
            "persona": "Alex",
            "prompt": "Lia estuda magia com Alex.",
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
                "user": "Olá Lia, você viu meu carro?",
                "assistant": "*Lia sorri e guarda o livro.*\n\nLia: Vi seu carro na entrada, Alex. O meu carro está na oficina.",
                "complete": true,
                "present_entities": ["Alex", "Lia"]
            }
        ],
        "totalPairs": 1,
        "beforeSeq": 2,
        "pathLength": 1,
        "cast": [],
        "scene": {
            "location": "Laboratório",
            "time": "tarde",
            "mood": "calmo",
            "present": ["Alex", "Lia"],
            "objects": ["grimório"],
            "characters": [
                {"name": "Lia", "location": "Laboratório", "position": "mesa", "holding": ["pena"]}
            ]
        },
        "pinned": [],
        "relationships": [
            {
                "a": "Lia",
                "b": "Alex",
                "relation_type": "friend_of",
                "directed": false,
                "description": "Amigos e parceiros de pesquisa."
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
            "userText": "Minha mãe Laura mandou lembranças.",
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
fn probe_1_tail_prompt_eliminates_negative_rule_and_enforces_positive_4level_pov_matrix() {
    let d = base_context_data();
    let p = plan();
    let ctx = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    let last = ctx.messages.last().unwrap()["content"].as_str().unwrap();

    // Invariant 1: Negative tail rule MUST NOT exist anywhere in context
    assert!(
        !last.contains("trate como 'sua mãe/família', nunca como 'minha'"),
        "Context tail must not contain toxic negative possession rule:\n{last}"
    );

    // Invariant 2: Positive 4-level POV matrix directive must be present in tail
    assert!(
        last.contains("desambiguação de posse e parentesco: siga a matriz de POV"),
        "Context tail must contain positive POV matrix guidance:\n{last}"
    );
    assert!(
        last.contains("pertences e familiares pertencem estritamente ao respectivo dono"),
        "Context tail must assert strict owner attribution:\n{last}"
    );
    assert!(
        last.contains("use 'sua/seu' apenas ao falar com o dono, 'meu/minha' para si mesmo e nomes explícitos entre terceiros e na narração"),
        "Context tail must guide pronoun usage by conversational perspective:\n{last}"
    );

    // Invariant 3: System prompt contains the formal 4-level POV matrix
    let sys = ctx.messages.first().unwrap()["content"].as_str().unwrap();
    assert!(
        sys.contains("[MATRIZ DE PONTO DE VISTA (POV) E DESAMBIGUAÇÃO DE POSSE — LEI DE PERSPECTIVA]"),
        "System prompt must contain the formal POV matrix block:\n{sys}"
    );
    assert!(sys.contains("1. NÍVEL 1 — NARRAÇÃO E AÇÕES (3ª Pessoa Nominativa Estrita)"));
    assert!(sys.contains("2. NÍVEL 2 — PERSONAGEM FALANDO COM Alex (2ª Pessoa Direta)"));
    assert!(sys.contains("3. NÍVEL 3 — DIÁLOGO ENTRE PERSONAGENS (NPC falando com outro NPC / Dinâmica Coral)"));
    assert!(sys.contains("4. NÍVEL 4 — AUTORREFERÊNCIA DO FALANTE (1ª Pessoa do Personagem)"));
}

#[test]
fn probe_2_extraction_schema_validates_kinship_and_possession_structures() {
    let schema: Value = serde_json::from_str(include_str!("../src/extraction-schema.json")).unwrap();

    let valid_extraction = json!({
        "summary": "Lia fala de sua família e itens.",
        "cast": [],
        "scene": {
            "location": "sala",
            "time": "tarde",
            "mood": "tranquilo",
            "present": ["Alex", "Lia"],
            "objects": ["mesa"],
            "characters": [
                {"name": "Lia", "location": "sala", "position": "cadeira", "holding": []}
            ]
        },
        "facts": [
            {
                "kind": "KINSHIP",
                "subject": "Laura",
                "content": "Laura é mãe de Alex.",
                "importance": 0.8,
                "confidence": 0.95,
                "visibility": "private",
                "replaces": "",
                "existing_id": "",
                "attribution": {"quote": "Minha mãe Laura", "originalSpeaker": "Alex"},
                "knowledge": [{"who": "Alex", "state": "knows", "via": "witnessed", "evidence": "Minha mãe Laura", "belief": ""}],
                "kinship": {
                    "subject": "Laura",
                    "relation": "parent_of",
                    "target": "Alex"
                }
            },
            {
                "kind": "POSSESSION",
                "subject": "Alex",
                "content": "Alex possui um carro azul.",
                "importance": 0.6,
                "confidence": 0.9,
                "visibility": "private",
                "replaces": "",
                "existing_id": "",
                "knowledge": [],
                "possession": {
                    "owner": "Alex",
                    "item": "carro azul",
                    "status": "owned"
                }
            },
            {
                "kind": "INVENTORY",
                "subject": "Lia",
                "content": "Lia carrega um grimório raro.",
                "importance": 0.5,
                "confidence": 0.9,
                "visibility": "private",
                "replaces": "",
                "existing_id": "",
                "knowledge": [],
                "possession": {
                    "owner": "Lia",
                    "item": "grimório raro",
                    "status": "held"
                }
            }
        ],
        "relationships": [
            {
                "a": "Laura",
                "b": "Alex",
                "relation_type": "parent_of",
                "directed": true,
                "description": "Laura é mãe biológica de Alex."
            }
        ],
        "open_threads": [],
        "resolved_threads": []
    });

    assert!(
        memory_backend::validate_schema(&valid_extraction, &schema).is_ok(),
        "Extraction with KINSHIP, POSSESSION, INVENTORY, and directed relationship must validate cleanly"
    );

    // Invalid relation enum fails:
    let mut invalid_kinship = valid_extraction.clone();
    invalid_kinship["facts"][0]["kinship"]["relation"] = json!("nemesis_of");
    assert!(
        memory_backend::validate_schema(&invalid_kinship, &schema).is_err(),
        "Invalid kinship relation enum must fail schema validation"
    );

    // Invalid possession status fails:
    let mut invalid_possession = valid_extraction.clone();
    invalid_possession["facts"][1]["possession"]["status"] = json!("vaporized");
    assert!(
        memory_backend::validate_schema(&invalid_possession, &schema).is_err(),
        "Invalid possession status enum must fail schema validation"
    );

    // Invalid kind enum fails:
    let mut invalid_kind = valid_extraction.clone();
    invalid_kind["facts"][0]["kind"] = json!("MAGIC_SPELL");
    assert!(
        memory_backend::validate_schema(&invalid_kind, &schema).is_err(),
        "Unregistered kind enum must fail schema validation"
    );
}

#[test]
fn probe_3_sanitizer_preserves_kinship_and_possession_kinds() {
    let raw = json!({
        "summary": "Resumo de teste de posse e parentesco.",
        "cast": [],
        "scene": null,
        "facts": [
            {
                "kind": "KINSHIP",
                "subject": "Helena",
                "content": "Helena é irmã de Lia.",
                "importance": 0.8,
                "confidence": 0.9,
                "visibility": "private",
                "replaces": "",
                "existing_id": "",
                "knowledge": [],
                "kinship": {
                    "subject": "Helena",
                    "relation": "sibling_of",
                    "target": "Lia"
                }
            },
            {
                "kind": "POSSESSION",
                "subject": "Lia",
                "content": "Lia é proprietária do diário mágico.",
                "importance": 0.7,
                "confidence": 0.85,
                "visibility": "private",
                "replaces": "",
                "existing_id": "",
                "knowledge": [],
                "possession": {
                    "owner": "Lia",
                    "item": "diário mágico",
                    "status": "owned"
                }
            },
            {
                "kind": "INVENTORY",
                "subject": "Alex",
                "content": "Alex carrega uma lanterna de bolso.",
                "importance": 0.4,
                "confidence": 0.8,
                "visibility": "private",
                "replaces": "",
                "existing_id": "",
                "knowledge": []
            }
        ],
        "relationships": [],
        "open_threads": [],
        "resolved_threads": []
    });

    let sanitized = jobs::sanitize_extraction(&raw).unwrap();
    let facts = sanitized["facts"].as_array().unwrap();
    assert_eq!(facts.len(), 3);
    assert_eq!(facts[0]["kind"], "KINSHIP", "KINSHIP kind must be preserved, not defaulted to EVENT");
    assert_eq!(facts[1]["kind"], "POSSESSION", "POSSESSION kind must be preserved, not defaulted to EVENT");
    assert_eq!(facts[2]["kind"], "INVENTORY", "INVENTORY kind must be preserved, not defaulted to EVENT");
    assert!(facts[0].get("kinship").is_some(), "Structured kinship sub-object must be preserved");
    assert!(facts[1].get("possession").is_some(), "Structured possession sub-object must be preserved");
}

#[test]
fn probe_4_programmatic_fact_and_relationship_normalization() {
    // 1. Direct unit verification of normalize_fact_text
    assert_eq!(
        jobs::normalize_fact_text("minha mãe viajou ontem", "Alex", "Alex"),
        "a mãe de Alex viajou ontem"
    );
    assert_eq!(
        jobs::normalize_fact_text("meu carro está na garagem", "Alex", "Alex"),
        "o carro de Alex está na garagem"
    );
    assert_eq!(
        jobs::normalize_fact_text("minhas chaves sumiram", "Lia", "Alex"),
        "as chaves de Lia sumiram"
    );
    assert_eq!(
        jobs::normalize_fact_text("meus livros antigos", "Lia", "Alex"),
        "os livros de Lia antigos"
    );
    assert_eq!(
        jobs::normalize_fact_text("sua mãe mandou lembranças", "Lia", "Alex"),
        "a mãe de Alex mandou lembranças"
    );
    assert_eq!(
        jobs::normalize_fact_text("seu carro foi consertado", "Lia", "Alex"),
        "o carro de Alex foi consertado"
    );
    assert_eq!(
        jobs::normalize_fact_text("eu estive no laboratório", "Alex", "Alex"),
        "Alex estive no laboratório"
    );

    // 2. Integration verification through sanitize_extraction
    let raw = json!({
        "summary": "Alex comenta sobre seus bens e família.",
        "cast": [],
        "scene": null,
        "persona": "Alex",
        "facts": [
            {
                "kind": "CHARACTER_FACT",
                "subject": "minha mãe",
                "content": "meu carro foi rebocado ontem à noite.",
                "importance": 0.8,
                "confidence": 0.9,
                "visibility": "private",
                "replaces": "",
                "existing_id": "",
                "attribution": {"quote": "meu carro foi rebocado", "originalSpeaker": "Alex"},
                "knowledge": []
            }
        ],
        "relationships": [
            {
                "a": "eu",
                "b": "Lia",
                "relation_type": "friend_of",
                "directed": false,
                "description": "minha melhor amiga"
            }
        ],
        "open_threads": [],
        "resolved_threads": []
    });

    let sanitized = jobs::sanitize_extraction(&raw).unwrap();
    let facts = sanitized["facts"].as_array().unwrap();
    assert_eq!(facts[0]["subject"], "a mãe de Alex");
    assert_eq!(facts[0]["content"], "o carro de Alex foi rebocado ontem à noite.");

    let rels = sanitized["relationships"].as_array().unwrap();
    assert_eq!(rels[0]["a"], "Alex");
    assert_eq!(rels[0]["b"], "Lia");
    assert_eq!(rels[0]["description"], "a melhor amiga de Alex");
    assert_eq!(rels[0]["relation_type"], "friend_of");
    assert_eq!(rels[0]["directed"], false);
}

#[test]
fn probe_5_scene_view_disambiguates_holding_possessions_and_room_objects() {
    let mut d = base_context_data();
    d["scene"]["objects"] = json!(["mesa de mogno", "lustre de cristal"]);
    d["scene"]["characters"] = json!([
        {
            "name": "Lia",
            "location": "Laboratório",
            "position": "bancada",
            "holding": ["varinha mágica"],
            "possessions": ["capa élfica", "grimório ancestral"]
        },
        {
            "name": "Alex",
            "location": "Laboratório",
            "position": "cadeira",
            "holding": [],
            "inventory": ["smartphone", "mochila de couro"]
        }
    ]);

    let p = plan();
    let ctx = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    let text = ctx.messages.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n");

    // Room objects:
    assert!(text.contains("Objetos: mesa de mogno, lustre de cristal"));
    // Lia holding vs possessions:
    assert!(text.contains("Lia — local: Laboratório; posição: bancada; segura: varinha mágica; pertences: capa élfica, grimório ancestral"));
    // Alex inventory as pertences:
    assert!(text.contains("Alex — local: Laboratório; posição: cadeira; pertences: smartphone, mochila de couro"));
}

#[test]
fn probe_6_relationships_renders_directed_arrows_for_kinship() {
    let mut d = base_context_data();
    d["relationships"] = json!([
        {
            "a": "Laura",
            "b": "Alex",
            "directed": true,
            "relation_type": "parent_of",
            "description": "mãe biológica de Alex"
        },
        {
            "a": "Alex",
            "b": "Carlos",
            "directed": false,
            "relation_type": "friend_of",
            "description": "amigos de infância"
        }
    ]);

    let p = plan();
    let ctx = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    let text = ctx.messages.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n");

    assert!(text.contains("[RELAÇÕES]"));
    assert!(
        text.contains("Laura → Alex: mãe biológica de Alex"),
        "Directed relationship must render with directional arrow →:\n{text}"
    );
    assert!(
        text.contains("Alex ↔ Carlos: amigos de infância"),
        "Bidirectional relationship must render with bidirectional arrow ↔:\n{text}"
    );
}

#[test]
fn probe_7_disambiguate_memory_content_unit_safety() {
    assert_eq!(
        context::disambiguate_memory_content("Alex", "minha mãe é médica renomada"),
        "A mãe de Alex é médica renomada"
    );
    assert_eq!(
        context::disambiguate_memory_content("Alex", "Minha mãe é médica renomada"),
        "A mãe de Alex é médica renomada"
    );
    assert_eq!(
        context::disambiguate_memory_content("Alex", "meu pai trabalha no porto"),
        "O pai de Alex trabalha no porto"
    );
    assert_eq!(
        context::disambiguate_memory_content("Alex", "meu irmão mais velho"),
        "O irmão de Alex mais velho"
    );
    assert_eq!(
        context::disambiguate_memory_content("Alex", "minha irmã caçula"),
        "A irmã de Alex caçula"
    );
    assert_eq!(
        context::disambiguate_memory_content("Alex", "minha família é nobre"),
        "A família de Alex é nobre"
    );
    assert_eq!(
        context::disambiguate_memory_content("Alex", "meu relógio de ouro"),
        "O relógio de Alex de ouro"
    );
    assert_eq!(
        context::disambiguate_memory_content("Alex", "minha espada encantada"),
        "A espada de Alex encantada"
    );
    // Non-possessive text remains untouched
    assert_eq!(
        context::disambiguate_memory_content("Alex", "A espada é afiada"),
        "A espada é afiada"
    );
    // Subject "mundo" remains untouched
    assert_eq!(
        context::disambiguate_memory_content("mundo", "minha terra natal"),
        "minha terra natal"
    );
    // UTF-8 multibyte boundary safety (accents: ã, é, í, ó, ú, ç)
    let complex_accented = "Minha mãe é médica em São Paulo com bênção";
    let normalized = context::disambiguate_memory_content("Alex", complex_accented);
    assert_eq!(normalized, "A mãe de Alex é médica em São Paulo com bênção");
}

#[tokio::test]
async fn probe_8_store_directed_relationships_and_non_destructive_reciprocal_edges() {
    let dir = tempfile::tempdir().unwrap();
    let store = store::Store::open(dir.path()).unwrap();

    let conv = store
        .call(store::Request::CreateConversation(json!({
            "name": "Genealogia",
            "persona": "Alex",
            "routeId": "cc/claude-sonnet-5-5",
            "presetId": "claude-sonnet-5-5",
            "prompt": "História de família de Alex e Helena."
        })))
        .await
        .unwrap();
    let cid = conv["id"].as_str().unwrap();

    // Turn 1: Insert Helena -> Alex ("mãe de Alex")
    let begin_1 = store
        .call(store::Request::BeginTurn(
            serde_json::from_value::<store::TurnRequest>(json!({
                "convId": cid,
                "mode": "send",
                "content": "Helena se apresenta para Alex.",
                "targetId": null,
                "routeId": "cc/claude-sonnet-5-5",
                "presetId": "claude-sonnet-5-5",
                "effort": "low"
            }))
            .unwrap(),
        ))
        .await
        .unwrap();

    let iid_1 = begin_1["interactionId"].as_str().unwrap();
    let vid_1 = begin_1["versionId"].as_str().unwrap();
    let gid_1 = begin_1["generationId"].as_str().unwrap();

    store
        .call(store::Request::FinishGeneration(store::FinishRequest {
            generation_id: gid_1.to_string(),
            text: Some("Helena afirma ser mãe de Alex.".to_string()),
            status: "complete".to_string(),
            usage: Value::Null,
            error: None,
        }))
        .await
        .unwrap();

    store
        .call(store::Request::ApplyExtraction(json!({
            "convId": cid,
            "interactionId": iid_1,
            "versionId": vid_1,
            "extraction": {
                "summary": "Helena afirma ser mãe de Alex.",
                "cast": [],
                "scene": {"location": "sala", "present": ["Alex", "Helena"]},
                "facts": [],
                "relationships": [
                    {
                        "a": "Helena",
                        "b": "Alex",
                        "relation_type": "parent_of",
                        "directed": true,
                        "description": "mãe de Alex"
                    }
                ],
                "openThreads": [],
                "resolvedThreads": []
            }
        })))
        .await
        .unwrap();

    // Turn 2: Insert Alex -> Helena ("filho de Helena")
    let begin_2 = store
        .call(store::Request::BeginTurn(
            serde_json::from_value::<store::TurnRequest>(json!({
                "convId": cid,
                "mode": "send",
                "content": "Alex reconhece Helena.",
                "targetId": null,
                "routeId": "cc/claude-sonnet-5-5",
                "presetId": "claude-sonnet-5-5",
                "effort": "low"
            }))
            .unwrap(),
        ))
        .await
        .unwrap();

    let iid_2 = begin_2["interactionId"].as_str().unwrap();
    let vid_2 = begin_2["versionId"].as_str().unwrap();
    let gid_2 = begin_2["generationId"].as_str().unwrap();

    store
        .call(store::Request::FinishGeneration(store::FinishRequest {
            generation_id: gid_2.to_string(),
            text: Some("Alex confirma ser filho de Helena.".to_string()),
            status: "complete".to_string(),
            usage: Value::Null,
            error: None,
        }))
        .await
        .unwrap();

    store
        .call(store::Request::ApplyExtraction(json!({
            "convId": cid,
            "interactionId": iid_2,
            "versionId": vid_2,
            "extraction": {
                "summary": "Alex confirma ser filho de Helena.",
                "cast": [],
                "scene": {"location": "sala", "present": ["Alex", "Helena"]},
                "facts": [],
                "relationships": [
                    {
                        "a": "Alex",
                        "b": "Helena",
                        "relation_type": "child_of",
                        "directed": true,
                        "description": "filho de Helena"
                    }
                ],
                "openThreads": [],
                "resolvedThreads": []
            }
        })))
        .await
        .unwrap();

    // Retrieve context data from store
    let ctx_data = store
        .call(store::Request::Context(json!({
            "convId": cid,
            "pendingUserText": "O que sabemos sobre nossa família?",
            "mode": "send"
        })))
        .await
        .unwrap();

    let rels = ctx_data["relationships"].as_array().unwrap();

    // CRITICAL INVARIANT: Both directed edges must coexist! Inserting Alex -> Helena must NOT supersede Helena -> Alex!
    assert!(
        rels.iter().any(|r| r["a_key"] == "helena" && r["b_key"] == "alex"),
        "Helena -> Alex directed relation must exist in store"
    );
    assert!(
        rels.iter().any(|r| r["a_key"] == "alex" && r["b_key"] == "helena"),
        "Alex -> Helena directed relation must exist in store"
    );
}

#[test]
fn probe_9_multi_npc_conversation_preserves_distinct_kinship_and_items() {
    let mut d = base_context_data();
    d["scene"]["present"] = json!(["Alex", "Lia", "Carlos"]);
    d["scene"]["characters"] = json!([
        {"name": "Lia", "location": "Laboratório", "position": "mesa", "holding": []},
        {"name": "Carlos", "location": "Laboratório", "position": "porta", "holding": ["chave de Alex"]}
    ]);
    d["pinned"] = json!([
        {
            "id": "m1",
            "kind": "KINSHIP",
            "subject": "Lia",
            "content": "Rodrigo é pai de Lia.",
            "visibility": "private",
            "knowledge": [
                {"name": "Lia", "who": "Lia", "state": "knows"},
                {"name": "Carlos", "who": "Carlos", "state": "knows"},
                {"name": "Alex", "who": "Alex", "state": "unaware"}
            ]
        },
        {
            "id": "m2",
            "kind": "POSSESSION",
            "subject": "Alex",
            "content": "Alex é o proprietário da chave do cofre.",
            "visibility": "private",
            "knowledge": [
                {"name": "Carlos", "who": "Carlos", "state": "knows"},
                {"name": "Alex", "who": "Alex", "state": "knows"}
            ]
        }
    ]);
    d["current"]["userText"] = json!("Carlos, onde está a chave?");

    let p = plan();
    let ctx = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    let text = ctx.messages.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n");

    // Lia and Carlos know Rodrigo is Lia's father; Alex is unaware
    assert!(
        text.contains("Alex: sabe [M2]; NÃO sabe e não pode agir sobre [M1]"),
        "Alex must be tagged unaware of Lia's private kinship knowledge:\n{text}"
    );
    assert!(text.contains("Rodrigo é pai de Lia"));
    assert!(text.contains("Alex é o proprietário da chave do cofre"));
    assert!(text.contains("Carlos — local: Laboratório; posição: porta; segura: chave de Alex"));
}

#[tokio::test]
async fn probe_10_automatic_reciprocal_kinship_derivation() {
    let dir = tempfile::tempdir().unwrap();
    let store = store::Store::open(dir.path()).unwrap();

    let conv = store
        .call(store::Request::CreateConversation(json!({
            "name": "Família",
            "persona": "Alex",
            "routeId": "cc/claude-sonnet-5-5",
            "presetId": "claude-sonnet-5-5",
            "prompt": "História familiar."
        })))
        .await
        .unwrap();
    let cid = conv["id"].as_str().unwrap();

    let begin = store
        .call(store::Request::BeginTurn(
            serde_json::from_value::<store::TurnRequest>(json!({
                "convId": cid,
                "mode": "send",
                "content": "Laura se revela mãe de Alex.",
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
    let gid = begin["generationId"].as_str().unwrap();

    store
        .call(store::Request::FinishGeneration(store::FinishRequest {
            generation_id: gid.to_string(),
            text: Some("Laura se revela mãe de Alex.".to_string()),
            status: "complete".to_string(),
            usage: Value::Null,
            error: None,
        }))
        .await
        .unwrap();

    // Extraction provides only one direction: Laura is mother of Alex
    store
        .call(store::Request::ApplyExtraction(json!({
            "convId": cid,
            "interactionId": iid,
            "versionId": vid,
            "extraction": {
                "summary": "Laura se revela mãe de Alex.",
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
        })))
        .await
        .unwrap();

    let ctx_data = store
        .call(store::Request::Context(json!({
            "convId": cid,
            "pendingUserText": "Você é minha mãe mesmo?",
            "mode": "send"
        })))
        .await
        .unwrap();

    let rels = ctx_data["relationships"].as_array().unwrap();

    // Verify Laura -> Alex is stored
    assert!(rels.iter().any(|r| r["a_key"] == "laura" && r["b_key"] == "alex"));
    // Verify reciprocal inverse kinship Alex -> Laura was automatically derived!
    assert!(
        rels.iter().any(|r| r["a_key"] == "alex" && r["b_key"] == "laura" && r["description"].as_str().unwrap().contains("filho(a) de Laura")),
        "Reciprocal inverse kinship Alex -> Laura must be automatically derived:\n{rels:?}"
    );
}
