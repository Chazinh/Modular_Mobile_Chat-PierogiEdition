use modularchat_server::{context, presets};
use serde_json::{json, Value};
use std::time::Instant;

fn generate_large_context_data(persona: &str, turn_count: usize) -> Value {
    let mut pairs = Vec::with_capacity(turn_count);
    let mut notes = Vec::with_capacity(turn_count);
    let mut episodes = Vec::new();

    // Group older turns into periodic episodes (every 50 turns)
    for ep_start in (1..turn_count.saturating_sub(50)).step_by(50) {
        let ep_end = ep_start + 49;
        let interaction_ids: Vec<String> = (ep_start..=ep_end).map(|i| format!("i{i}")).collect();
        episodes.push(json!({
            "fromSeq": ep_start,
            "toSeq": ep_end,
            "summary": format!("Resumo consolidado do episódio cobrindo turnos {ep_start} a {ep_end} com eventos de magia e investigação."),
            "interactionIds": interaction_ids
        }));
    }

    for seq in 1..=turn_count {
        let id = format!("i{seq}");
        pairs.push(json!({
            "id": id,
            "seq": seq,
            "user": format!("Alex examina a runa misteriosa no turno {seq}."),
            "assistant": format!("Lia: No turno {seq}, deciframos parte da inscrição élfica.\n*Lia consulta o tomo antigo.*"),
            "complete": true,
            "present_entities": [persona, "Lia", "Carlos"]
        }));

        // Some turns have un-summarized notes
        if seq % 7 == 0 {
            notes.push(json!({
                "interactionId": id,
                "seq": seq,
                "summary": format!("Turno {seq}: Descoberta sobre os símbolos arcanos.")
            }));
        }
    }

    json!({
        "conv": {
            "persona": persona,
            "prompt": "História de alta complexidade com centenas de turnos, magia e linhagens antigas.",
            "settings": {
                "context": {
                    "preferredPairs": 30,
                    "minPairs": 20,
                    "maxPairs": 50
                }
            }
        },
        "pairs": pairs,
        "totalPairs": turn_count,
        "beforeSeq": turn_count + 1,
        "pathLength": turn_count,
        "cast": [
            {"name": "Lia", "role": "Maga pesquisadora"},
            {"name": "Carlos", "role": "Guerreiro guardião"},
            {"name": "Helena", "role": "Matriarca da linhagem"}
        ],
        "scene": {
            "location": "Santuário Subterrâneo",
            "time": "meia-noite",
            "mood": "ancestral",
            "present": [persona, "Lia", "Carlos", "Helena"],
            "objects": ["altar de obsidiana", "tochas azuis", "códice estelar"],
            "characters": [
                {
                    "name": "Lia",
                    "location": "Santuário Subterrâneo",
                    "position": "ao lado do altar",
                    "holding": ["cajado rúnico"],
                    "possessions": ["amuleto de prata", "anel arcano", "manto estelar"]
                },
                {
                    "name": "Carlos",
                    "location": "Santuário Subterrâneo",
                    "position": "guarnecendo a entrada",
                    "holding": ["escudo de ferro"],
                    "possessions": ["espada cerimonial", "armadura pesada"]
                },
                {
                    "name": "Helena",
                    "location": "Santuário Subterrâneo",
                    "position": "em frente ao códice",
                    "holding": [],
                    "possessions": ["diadema da família", "manto cerimonial"]
                }
            ]
        },
        "pinned": [
            {
                "id": "p1",
                "kind": "KINSHIP",
                "subject": "Helena",
                "content": format!("Helena é a mãe biológica de {persona}."),
                "importance": 1.0,
                "visibility": "private"
            },
            {
                "id": "p2",
                "kind": "POSSESSION",
                "subject": persona,
                "content": format!("{persona} é o herdeiro legítimo do selo arcano da família."),
                "importance": 1.0,
                "visibility": "private"
            }
        ],
        "relationships": [
            {
                "a": "Helena",
                "b": persona,
                "relation_type": "parent_of",
                "directed": true,
                "description": format!("mãe de {persona}")
            },
            {
                "a": persona,
                "b": "Helena",
                "relation_type": "child_of",
                "directed": true,
                "description": "filho de Helena"
            },
            {
                "a": "Lia",
                "b": "Carlos",
                "relation_type": "sibling_of",
                "directed": false,
                "description": "irmãos companheiros"
            }
        ],
        "threads": [
            {"id": "t1", "text": "O mistério do selo arcano de Helena"},
            {"id": "t2", "text": "A tradução do códice estelar"}
        ],
        "notes": notes,
        "episodes": episodes,
        "timeline": {"current": null},
        "sync": {"lag": 0, "synced": true},
        "unprocessed": [],
        "current": {
            "interactionId": null,
            "seq": turn_count + 1,
            "userText": "Helena, você reconhece o brasão da nossa linhagem neste altar?",
            "priorParts": []
        }
    })
}

fn test_plan_deepseek() -> presets::Plan {
    presets::request(
        &json!({
            "presetId": "deepseek-v41-flash",
            "routeId": "cbai/deepseek-v4.1-flash"
        }),
        vec![],
        &json!({}),
    )
}

fn test_plan_claude() -> presets::Plan {
    presets::request(
        &json!({
            "presetId": "claude-sonnet-5-5",
            "routeId": "cc/claude-sonnet-5-5"
        }),
        vec![],
        &json!({}),
    )
}

#[test]
fn test_challenger_assembly_performance_and_budget_stability_at_scale() {
    let plans = [("deepseek-v4.1", test_plan_deepseek()), ("claude-sonnet-5.5", test_plan_claude())];
    let turn_scales = [100, 720, 2000];

    for (model_name, plan) in plans {
        for &turns in &turn_scales {
            let data = generate_large_context_data("Alex", turns);

            let start = Instant::now();
            let result = context::assemble(&data, &json!({}), "send", &plan, None);
            let elapsed = start.elapsed();

            assert!(
                result.is_ok(),
                "[{model_name}] Context assembly must succeed without error on {turns} turns: {:?}",
                result.err()
            );

            let ctx = result.unwrap();

            // 1. Performance: In-memory assembly should be rapid (under 100ms even for 2000 turns)
            println!(
                "[{}] Scale {} turns: assembly took {:.3} ms, messages: {}, pairsIncluded: {}",
                model_name,
                turns,
                elapsed.as_secs_f64() * 1000.0,
                ctx.messages.len(),
                ctx.report["pairsIncluded"]
            );
            assert!(
                elapsed.as_millis() < 250,
                "[{model_name}] Assembly latency on {turns} turns must be under 250ms, took {:?}",
                elapsed
            );

            // 2. Budget verification: Exceeded budget must not be true (budget compression ladder works)
            assert_eq!(
                ctx.report["exceededEstimatedBudget"],
                false,
                "[{model_name}] Context must stay within estimated budget limits on {turns} turns!"
            );
            assert_eq!(
                ctx.report["overHardLimit"],
                false,
                "[{model_name}] Context must NOT exceed hardLimit on {turns} turns!"
            );

            // 3. POV Matrix verification: Initial instruction block must contain the complete 4-level POV matrix
            let initial_text = ctx.messages.first().unwrap()["content"].as_str().unwrap();

            assert!(
                initial_text.contains("[MATRIZ DE PONTO DE VISTA (POV) E DESAMBIGUAÇÃO DE POSSE — LEI DE PERSPECTIVA]"),
                "[{model_name}] POV matrix header must exist in initial instructions at scale {turns}"
            );
            assert!(
                initial_text.contains("1. NÍVEL 1 — NARRAÇÃO E AÇÕES (3ª Pessoa Nominativa Estrita):"),
                "[{model_name}] POV Level 1 must exist at scale {turns}"
            );
            assert!(
                initial_text.contains("2. NÍVEL 2 — PERSONAGEM FALANDO COM Alex (2ª Pessoa Direta):"),
                "[{model_name}] POV Level 2 must exist at scale {turns}"
            );
            assert!(
                initial_text.contains("3. NÍVEL 3 — DIÁLOGO ENTRE PERSONAGENS (NPC falando com outro NPC / Dinâmica Coral):"),
                "[{model_name}] POV Level 3 must exist at scale {turns}"
            );
            assert!(
                initial_text.contains("4. NÍVEL 4 — AUTORREFERÊNCIA DO FALANTE (1ª Pessoa do Personagem):"),
                "[{model_name}] POV Level 4 must exist at scale {turns}"
            );

            // 4. Tail verification: Positive tail exists, toxic negative rule never present
            let tail_msg = ctx.messages.last().expect("Last message must exist");
            let tail_text = tail_msg["content"].as_str().unwrap();

            assert!(
                tail_text.contains("desambiguação de posse e parentesco: siga a matriz de POV"),
                "[{model_name}] Tail must contain positive POV directive at scale {turns}"
            );
            assert!(
                !tail_text.contains("nunca como 'minha'"),
                "[{model_name}] Tail must NEVER contain the toxic negative rule at scale {turns}"
            );

            // 5. Scene inventory & directed relationships preserved
            assert!(
                tail_text.contains("Helena → Alex: mãe de Alex"),
                "[{model_name}] Directed kinship Helena → Alex must be formatted with arrow at scale {turns}"
            );
            assert!(
                tail_text.contains("Alex → Helena: filho de Helena"),
                "[{model_name}] Directed kinship Alex → Helena must be formatted with arrow at scale {turns}"
            );
            assert!(
                tail_text.contains("pertences: amuleto de prata, anel arcano, manto estelar"),
                "[{model_name}] Character possessions must be rendered in scene view at scale {turns}"
            );
        }
    }
}

#[test]
fn test_challenger_dense_relationships_and_inventory_budget_stress() {
    let plan = test_plan_deepseek();
    let mut data = generate_large_context_data("Alex", 100);

    // Inject 50 dense directed relationships
    let mut rels = Vec::new();
    for i in 1..=50 {
        rels.push(json!({
            "a": format!("Personagem_{i}"),
            "b": "Alex",
            "relation_type": "relative_of",
            "directed": true,
            "description": format!("parente distante de linhagem #{i}")
        }));
    }
    data["relationships"] = json!(rels);

    // Inject characters with massive inventories
    let mut dense_chars = Vec::new();
    for i in 1..=10 {
        let items: Vec<String> = (1..=20).map(|item_id| format!("item_raro_{item_id}")).collect();
        dense_chars.push(json!({
            "name": format!("Mercador_{i}"),
            "location": "Santuário Subterrâneo",
            "position": format!("bancada {i}"),
            "holding": [format!("chave_mestra_{i}")],
            "possessions": items
        }));
    }
    data["scene"]["characters"] = json!(dense_chars);

    let start = Instant::now();
    let result = context::assemble(&data, &json!({}), "send", &plan, None);
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "Assembly with dense relationships and inventories must succeed");
    let ctx = result.unwrap();

    println!(
        "Dense test: assembly took {:.3} ms, exceededEstimatedBudget: {}, total tokens: {}",
        elapsed.as_secs_f64() * 1000.0,
        ctx.report["exceededEstimatedBudget"],
        ctx.report["tokens"]["total"]
    );

    assert_eq!(ctx.report["overHardLimit"], false, "Must adaptively fit within hard budget limit!");
}
