use modularchat_server::{context, presets};
use serde_json::{Value, json};

fn base_cohesion_data(num_pairs: usize, preferred: usize, min_pairs: usize) -> Value {
    let pairs: Vec<Value> = (1..=num_pairs)
        .map(|seq| {
            json!({
                "id": format!("i{seq}"),
                "seq": seq as i64,
                "user": format!("Pergunta no turno {seq}"),
                "assistant": format!("*Lia responde no turno {seq}.*\n\nLia: Diálogo detalhado do turno {seq} sobre o mistério da torre."),
                "complete": true,
                "present_entities": ["Alex", "Lia"]
            })
        })
        .collect();

    json!({
        "conv": {
            "persona": "Alex",
            "prompt": "Lia e Alex investigam os mistérios arcanos da torre.",
            "settings": {
                "context": {
                    "preferredPairs": preferred,
                    "minPairs": min_pairs,
                    "maxPairs": 50,
                    "adaptiveHistory": true,
                    "targetTokens": 96000,
                    "softLimit": 110000,
                    "hardLimit": 128000
                }
            }
        },
        "pairs": pairs,
        "totalPairs": num_pairs,
        "beforeSeq": (num_pairs + 1) as i64,
        "pathLength": num_pairs,
        "cast": [],
        "scene": {
            "location": "Torre Arcana",
            "time": "noite",
            "mood": "mistério",
            "present": ["Alex", "Lia"],
            "objects": ["tomo antigo"],
            "characters": [
                {"name": "Lia", "location": "Torre Arcana", "position": "janela", "holding": ["lanterna"]}
            ]
        },
        "pinned": [],
        "relationships": [
            {
                "a": "Alex",
                "b": "Lia",
                "relation_type": "partner",
                "directed": false,
                "description": "Parceiros de investigação."
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
            "seq": (num_pairs + 1) as i64,
            "userText": "O que encontramos no topo da escadaria?",
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

fn assembled_text(ctx: &context::Context) -> String {
    ctx.messages
        .iter()
        .map(|m| format!("[{}]: {}", m["role"].as_str().unwrap_or("?"), m["content"].as_str().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\n---\n")
}

#[test]
fn probe_1_preferred_history_ladder_preserved_when_budget_permits() {
    let mut data = base_cohesion_data(35, 30, 20);
    // Active history: 30 pairs out of 35 (turns 6..=35, window = 6)
    // Episode 1 covers prior turns 1..5
    data["episodes"] = json!([{
        "id": "e1",
        "fromSeq": 1,
        "toSeq": 5,
        "summary": "Resumo do episódio inicial cobrindo turnos 1 a 5.",
        "interactionIds": (1..=5).map(|s| format!("i{s}")).collect::<Vec<_>>()
    }]);

    let ctx = context::assemble(&data, &json!({}), "send", &plan(), None).unwrap();

    // Invariant: Preferred 30 pairs is preserved when budget permits (no premature drop to 16)
    assert_eq!(ctx.report["pairsIncluded"].as_u64().unwrap(), 30);
    assert_eq!(ctx.selected_history_len, 30);
    assert!(
        !ctx.report["compression"]
            .to_string()
            .contains("histórico resumido por episódios verificados; 16 pares recentes")
    );

    // Turn 6 (the 30th from the end: 35 - 30 + 1 = 6) must be present in raw messages
    let text = assembled_text(&ctx);
    assert!(text.contains("Pergunta no turno 6"));
    // Turn 5 (omitted from raw history) is represented in the timeline
    assert!(!text.contains("Pergunta no turno 5"));
    assert!(text.contains("Resumo do episódio inicial cobrindo turnos 1 a 5."));
}

#[test]
fn probe_2_adaptive_ladder_descends_to_summarized_steps_only_under_soft_limit_pressure() {
    let mut data = base_cohesion_data(35, 30, 20);
    data["episodes"] = json!([
        {
            "id": "e1",
            "fromSeq": 1,
            "toSeq": 15,
            "summary": "Resumo do episódio inicial cobrindo turnos 1 a 15.",
            "interactionIds": (1..=15).map(|s| format!("i{s}")).collect::<Vec<_>>()
        },
        {
            "id": "e2",
            "fromSeq": 16,
            "toSeq": 19,
            "summary": "Resumo do segundo episódio cobrindo turnos 16 a 19.",
            "interactionIds": (16..=19).map(|s| format!("i{s}")).collect::<Vec<_>>()
        }
    ]);

    // Inflate pair text and set a tight softLimit to force token budget overflow
    for pair in data["pairs"].as_array_mut().unwrap() {
        pair["assistant"] = json!("Diálogo extenso com descrição atmosférica densa e detalhada. ".repeat(80));
    }
    data["conv"]["settings"]["context"]["softLimit"] = json!(6000);

    let ctx = context::assemble(&data, &json!({}), "send", &plan(), None).unwrap();

    // Invariant: Under pressure, ladder steps down below minimum (20) into eligible summarized rungs (16 or lower)
    assert!(ctx.report["pairsIncluded"].as_u64().unwrap() <= 16);
    assert!(ctx.selected_history_len <= 16);
    let comp_str = ctx.report["compression"].to_string();
    assert!(
        comp_str.contains("histórico recente reduzido")
            || comp_str.contains("histórico resumido")
    );
}

#[test]
fn probe_3_symmetrical_rag_retrieval_window_aligned_with_active_history_slice() {
    let data = base_cohesion_data(40, 30, 20);
    // With 40 pairs and preferred = 30, active history window covers turns 11..40 (cutoff = 11)
    let win_start = context::retrieval_window_start(&data);
    assert_eq!(win_start, 11, "Retrieval window must align symmetrically with active history start");

    let retrieval = json!({
        "candidates": [
            {
                "id": "m_old",
                "type": "memory",
                "sourceSeq": 5,
                "content": "Fato antigo do turno 5 sobre a chave oculta."
            },
            {
                "id": "m_recent",
                "type": "memory",
                "sourceSeq": 25,
                "content": "Fato recente do turno 25 presente na história ativa."
            }
        ]
    });

    let ctx = context::assemble(&data, &retrieval, "send", &plan(), None).unwrap();

    // Invariant: Memory before window (seq 5 < 11) is retained; memory within window (seq 25 >= 11) is excluded
    let memories_included = ctx.report["memoriesIncluded"].as_array().unwrap();
    assert!(memories_included.contains(&json!("m_old")));
    assert!(!memories_included.contains(&json!("m_recent")));

    let text = assembled_text(&ctx);
    assert!(text.contains("Fato antigo do turno 5 sobre a chave oculta."));
    assert!(!text.contains("Fato recente do turno 25 presente na história ativa."));
}

#[test]
fn probe_4_multi_episode_timeline_continuity_no_temporal_dead_zone() {
    let mut data = base_cohesion_data(45, 20, 15);
    // Active history: 20 pairs (turns 26..=45), window start = 26
    data["episodes"] = json!([
        {
            "id": "e1",
            "fromSeq": 1,
            "toSeq": 10,
            "summary": "Episódio 1: A descoberta da torre arcana.",
            "interactionIds": (1..=10).map(|s| format!("i{s}")).collect::<Vec<_>>()
        },
        {
            "id": "e2",
            "fromSeq": 11,
            "toSeq": 20,
            "summary": "Episódio 2: O pacto secreto no observatório.",
            "interactionIds": (11..=20).map(|s| format!("i{s}")).collect::<Vec<_>>()
        }
    ]);
    data["notes"] = json!([
        {"interactionId": "i21", "seq": 21, "summary": "Nota 21: Decifrando o glifo primordial."},
        {"interactionId": "i25", "seq": 25, "summary": "Nota 25: A chave estelar foi revelada."}
    ]);

    let ctx = context::assemble(&data, &json!({}), "send", &plan(), None).unwrap();
    let text = assembled_text(&ctx);

    // Invariant: All episodes and uncovered notes appear in chronological order without dead zones
    assert!(text.contains("Episódio 1: A descoberta da torre arcana."));
    assert!(text.contains("Episódio 2: O pacto secreto no observatório."));
    assert!(text.contains("Nota 21: Decifrando o glifo primordial."));
    assert!(text.contains("Nota 25: A chave estelar foi revelada."));

    let pos_e1 = text.find("Episódio 1: A descoberta").unwrap();
    let pos_e2 = text.find("Episódio 2: O pacto").unwrap();
    let pos_n21 = text.find("Nota 21: Decifrando").unwrap();
    let pos_n25 = text.find("Nota 25: A chave").unwrap();

    assert!(pos_e1 < pos_e2, "Episode 1 must precede Episode 2");
    assert!(pos_e2 < pos_n21, "Episode 2 must precede Note 21");
    assert!(pos_n21 < pos_n25, "Note 21 must precede Note 25");
}

#[test]
fn probe_5_unprocessed_pending_turns_must_never_be_dropped_during_compression() {
    let mut data = base_cohesion_data(35, 30, 20);
    // Active history: 30 pairs (turns 6..=35, window = 6)
    data["episodes"] = json!([{
        "id": "e1",
        "fromSeq": 1,
        "toSeq": 5,
        "summary": "Resumo dos primeiros 5 turnos.",
        "interactionIds": (1..=5).map(|s| format!("i{s}")).collect::<Vec<_>>()
    }]);

    // Turn 3 is pending in unprocessed (seq 3 < window 6)
    data["unprocessed"] = json!([{
        "id": "i3",
        "seq": 3,
        "user": "Pergunta pendente do turno 3",
        "assistant": "Lia afirma: PENDING_CRITICAL_DISCLOSURE_UNTOUCHED",
        "complete": true
    }]);

    let ctx = context::assemble(&data, &json!({}), "send", &plan(), None).unwrap();

    // Invariant: Pending turn is never dropped during compression and is chained into messages
    assert!(ctx.report["pendingTurnsPreserved"].as_u64().unwrap() >= 1);
    let text = assembled_text(&ctx);
    assert!(text.contains("PENDING_CRITICAL_DISCLOSURE_UNTOUCHED"));
    assert!(text.contains("Pergunta pendente do turno 3"));
}

#[test]
fn probe_6_factual_and_emotional_cohesion_between_episodes_and_recent_turns() {
    let mut data = base_cohesion_data(25, 15, 10);
    // Active history: turns 11..25 (window = 11). Episode covers 1..10
    data["episodes"] = json!([{
        "id": "e1",
        "fromSeq": 1,
        "toSeq": 10,
        "summary": "Sob intensa névoa e desconfiança mútua, Lia confessou a Alex que teme a maldição da torre e prometeu nunca abandoná-lo diante do perigo.",
        "interactionIds": (1..=10).map(|s| format!("i{s}")).collect::<Vec<_>>()
    }]);
    data["relationships"] = json!([
        {
            "a": "Alex",
            "b": "Lia",
            "relation_type": "confidant",
            "directed": false,
            "description": "Lia prometeu nunca abandonar Alex diante do perigo."
        }
    ]);

    let ctx = context::assemble(&data, &json!({}), "send", &plan(), None).unwrap();
    let text = assembled_text(&ctx);

    // Invariant: Emotional anchor and relationship state are both preserved in context
    assert!(text.contains("teme a maldição da torre e prometeu nunca abandoná-lo diante do perigo"));
    assert!(text.contains("Lia prometeu nunca abandonar Alex diante do perigo."));
    assert!(text.contains("[LINHA DO TEMPO ANTERIOR]"));
    assert!(text.contains("[RELAÇÕES]"));
}

#[test]
fn probe_7_verbatim_raw_excerpts_deduplication_and_window_exclusion() {
    let data = base_cohesion_data(35, 15, 10);
    // Active history: 15 pairs (turns 21..35, window = 21)
    let retrieval = json!({
        "raw": [
            {
                "sourceId": "s10",
                "role": "assistant",
                "sourceInteractionId": "i10",
                "sourceSeq": 10,
                "start": 0,
                "end": 43,
                "text": "O tomo sagrado contém sete selos ancestrais"
            },
            {
                "sourceId": "s10",
                "role": "assistant",
                "sourceInteractionId": "i10",
                "sourceSeq": 10,
                "start": 22,
                "end": 65,
                "text": "sete selos ancestrais que protegem a torre."
            },
            {
                "sourceId": "s25",
                "role": "assistant",
                "sourceInteractionId": "i25",
                "sourceSeq": 25,
                "start": 0,
                "end": 49,
                "text": "RAW_EXCERPT_INSIDE_ACTIVE_WINDOW_MUST_BE_EXCLUDED"
            }
        ]
    });

    let ctx = context::assemble(&data, &retrieval, "send", &plan(), None).unwrap();
    let text = assembled_text(&ctx);

    // Invariant: Excerpt with seq 25 (inside window >= 21) is excluded from raw section
    assert!(!text.contains("RAW_EXCERPT_INSIDE_ACTIVE_WINDOW_MUST_BE_EXCLUDED"));
    assert_eq!(ctx.report["rawIncluded"].as_u64().unwrap(), 1);

    // Invariant: Overlapping chunks A and B are merged cleanly without duplicate substrings
    assert!(text.contains("O tomo sagrado contém sete selos ancestrais que protegem a torre."));
    assert!(!text.contains("sete selos ancestrais sete selos ancestrais"));
}

#[test]
fn probe_8_continuous_timeline_coverage_preserved_when_context_budget_allows() {
    let mut data = base_cohesion_data(100, 20, 15);
    // Active history: 20 pairs (turns 81..=100, window = 81)
    // 10 distinct episodes covering turns 1..80
    let mut episodes = Vec::new();
    for i in 0..10 {
        let from = i * 8 + 1;
        let to = from + 7;
        episodes.push(json!({
            "id": format!("e{i}"),
            "fromSeq": from,
            "toSeq": to,
            "summary": format!("Episódio {}: Crônica dos acontecimentos de {} a {}.", i + 1, from, to),
            "interactionIds": (from..=to).map(|s| format!("i{s}")).collect::<Vec<_>>()
        }));
    }
    data["episodes"] = json!(episodes);

    let ctx = context::assemble(&data, &json!({}), "send", &plan(), None).unwrap();
    let text = assembled_text(&ctx);

    // Invariant: All 10 episodes are retained in timeline when within budget (no intermediate drops)
    assert_eq!(ctx.report["timelineItems"].as_u64().unwrap(), 10);
    for i in 1..=10 {
        assert!(
            text.contains(&format!("Episódio {}: Crônica", i)),
            "Episode {} must not be dropped when context budget allows",
            i
        );
    }
}

#[test]
fn probe_9_boundary_anchor_transition_between_condensed_episode_and_active_dialogue() {
    let mut data = base_cohesion_data(25, 8, 8);
    // Active history: 8 pairs (turns 18..=25, window = 18)
    data["episodes"] = json!([{
        "id": "e1",
        "fromSeq": 1,
        "toSeq": 17,
        "summary": "Lia e Alex exploraram os corredores sombrios e alcançaram o Observatório celeste, onde observaram as constelações seladas.",
        "interactionIds": (1..=17).map(|s| format!("i{s}")).collect::<Vec<_>>()
    }]);
    data["scene"] = json!({
        "location": "Observatório Celeste",
        "time": "meia-noite",
        "mood": "maravilhamento e tensão",
        "present": ["Alex", "Lia"],
        "objects": ["telescópio estelar", "mapa astral"],
        "characters": [
            {"name": "Lia", "location": "Observatório Celeste", "position": "telescópio", "holding": []}
        ]
    });
    data["current"]["userText"] = json!("Olho através da lente principal do telescópio estelar.");

    let ctx = context::assemble(&data, &json!({}), "send", &plan(), None).unwrap();
    let text = assembled_text(&ctx);

    // Invariant: Scene location, time and mood carry over smoothly at the episode-dialogue boundary
    assert!(text.contains("Observatório Celeste"));
    assert!(text.contains("meia-noite"));
    assert!(text.contains("maravilhamento e tensão"));
    assert!(text.contains("[LINHA DO TEMPO ANTERIOR]"));
    assert!(text.contains("[CENA ANTERIOR") || text.contains("[CENA ATUAL"));
}

#[test]
fn probe_10_queue_episodes_chunking_boundary_and_notes_coverage() {
    let mut data = base_cohesion_data(30, 10, 8);
    // Active history: 10 pairs (turns 21..30, window = 21)
    // Episode covers turns 1..15.
    data["episodes"] = json!([{
        "id": "e1",
        "fromSeq": 1,
        "toSeq": 15,
        "summary": "Resumo dos primeiros 15 turnos da investigação.",
        "interactionIds": (1..=15).map(|s| format!("i{s}")).collect::<Vec<_>>()
    }]);

    // Turns 16..20 are waiting to form the next episode chunk (< 15 turns threshold)
    // Their notes must appear in timeline to prevent epistemic dead zones
    let notes: Vec<Value> = (16..=20)
        .map(|s| {
            json!({
                "interactionId": format!("i{s}"),
                "seq": s,
                "summary": format!("Nota intermediária do turno {s}: segredo revelado.")
            })
        })
        .collect();
    data["notes"] = json!(notes);

    let ctx = context::assemble(&data, &json!({}), "send", &plan(), None).unwrap();
    let text = assembled_text(&ctx);

    // Invariant: Both episode 1 and all trailing notes 16..20 are present in [LINHA DO TEMPO ANTERIOR]
    assert!(text.contains("Resumo dos primeiros 15 turnos da investigação."));
    for s in 16..=20 {
        assert!(
            text.contains(&format!("Nota intermediária do turno {s}: segredo revelado.")),
            "Turn {} note must be included in timeline",
            s
        );
    }
}

#[test]
fn probe_11_ladder_boundary_conditions_zero_and_minimal_history() {
    // Zero history pairs (initial state of a conversation)
    let data_zero = base_cohesion_data(0, 30, 20);
    let win_zero = context::retrieval_window_start(&data_zero);
    assert_eq!(win_zero, 1, "With zero history, window start defaults to current seq");

    let ctx_zero = context::assemble(&data_zero, &json!({}), "send", &plan(), None).unwrap();
    assert_eq!(ctx_zero.selected_history_len, 0);
    assert_eq!(ctx_zero.report["pairsIncluded"].as_u64().unwrap(), 0);

    // Minimal history (3 pairs, less than minPairs = 20)
    let data_min = base_cohesion_data(3, 30, 20);
    let ctx_min = context::assemble(&data_min, &json!({}), "send", &plan(), None).unwrap();
    assert_eq!(ctx_min.selected_history_len, 3);
    assert_eq!(ctx_min.report["pairsIncluded"].as_u64().unwrap(), 3);
}

#[test]
fn probe_12_episodic_gap_blocks_premature_ladder_stepdown() {
    let mut data = base_cohesion_data(35, 30, 20);
    // Episode covers only turns 1..10; turns 11..19 have NO episode coverage
    data["episodes"] = json!([{
        "id": "e1",
        "fromSeq": 1,
        "toSeq": 10,
        "summary": "Resumo parcial dos turnos 1 a 10.",
        "interactionIds": (1..=10).map(|s| format!("i{s}")).collect::<Vec<_>>()
    }]);

    // Force extreme budget pressure
    for pair in data["pairs"].as_array_mut().unwrap() {
        pair["assistant"] = json!("Texto extenso para forçar pressão orçamentária máxima. ".repeat(60));
    }
    data["conv"]["settings"]["context"]["softLimit"] = json!(6000);

    let ctx = context::assemble(&data, &json!({}), "send", &plan(), None).unwrap();

    // Invariant: Because turns 11..19 lack episodic consolidation, summarized_before returns false.
    // The ladder must NOT descend into summarized rungs (< 20) and must preserve minimum = 20.
    assert_eq!(
        ctx.selected_history_len, 20,
        "Ladder must not descend below minimum (20) when intermediate turns lack episodic coverage"
    );
    assert_eq!(ctx.report["pairsIncluded"].as_u64().unwrap(), 20);
}

#[test]
fn probe_13_retrieval_window_start_under_tight_soft_limit() {
    let mut data = base_cohesion_data(35, 30, 20);
    data["episodes"] = json!([{
        "id": "e1",
        "fromSeq": 1,
        "toSeq": 27,
        "summary": "Resumo completo até o turno 27.",
        "interactionIds": (1..=27).map(|s| format!("i{s}")).collect::<Vec<_>>()
    }]);

    for pair in data["pairs"].as_array_mut().unwrap() {
        pair["assistant"] = json!("Diálogo volumoso consumindo tokens consideráveis no histórico. ".repeat(60));
    }
    data["conv"]["settings"]["context"]["softLimit"] = json!(6000);

    // retrieval_window_start must calculate ladder step-down under softLimit pressure without panic
    let win_start = context::retrieval_window_start(&data);
    assert!(win_start > 1, "Window start must be greater than 1 under step-down");
    assert!(win_start <= 35, "Window start must not exceed total pairs");
}

#[test]
fn probe_14_raw_excerpts_transitive_multi_chunk_merges() {
    let data = base_cohesion_data(35, 15, 10);
    // Active history: turns 21..=35 (window = 21)
    // 3 overlapping chunks from the same source at seq 10 (< 21)
    let t1 = "O tomo antigo continha ";
    let t2 = "continha segredos ";
    let t3 = "segredos arcanos da torre.";

    let retrieval = json!({
        "raw": [
            {
                "sourceId": "src_book",
                "role": "assistant",
                "canonicalSpeaker": "Lia",
                "sourceInteractionId": "i10",
                "sourceSeq": 10,
                "start": 0,
                "end": 23,
                "text": t1
            },
            {
                "sourceId": "src_book",
                "role": "assistant",
                "canonicalSpeaker": "Lia",
                "sourceInteractionId": "i10",
                "sourceSeq": 10,
                "start": 14,
                "end": 32,
                "text": t2
            },
            {
                "sourceId": "src_book",
                "role": "assistant",
                "canonicalSpeaker": "Lia",
                "sourceInteractionId": "i10",
                "sourceSeq": 10,
                "start": 23,
                "end": 49,
                "text": t3
            }
        ]
    });

    let ctx = context::assemble(&data, &retrieval, "send", &plan(), None).unwrap();
    let text = assembled_text(&ctx);

    // Invariant: All 3 overlapping chunks merge transitively into 1 single excerpt
    assert_eq!(ctx.report["rawIncluded"].as_u64().unwrap(), 1);
    assert!(text.contains("O tomo antigo continha segredos arcanos da torre."));
    assert!(!text.contains("continha continha"));
    assert!(!text.contains("segredos segredos"));
}

#[test]
fn probe_15_extreme_budget_starvation_terminates_cleanly() {
    let mut data = base_cohesion_data(25, 20, 10);
    data["conv"]["settings"]["context"]["softLimit"] = json!(1);
    data["conv"]["settings"]["context"]["hardLimit"] = json!(1);
    data["conv"]["settings"]["context"]["targetTokens"] = json!(1);

    // assemble must not panic, overflow, or loop infinitely under starvation
    let ctx = context::assemble(&data, &json!({}), "send", &plan(), None).unwrap();
    assert_eq!(ctx.report["overHardLimit"], json!(true));
    assert_eq!(ctx.report["exceededEstimatedBudget"], json!(true));
    assert!(ctx.selected_history_len > 0);
}

