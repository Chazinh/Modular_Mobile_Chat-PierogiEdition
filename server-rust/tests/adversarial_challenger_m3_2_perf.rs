use modularchat_server::{context, presets};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

fn generate_bench_data(
    persona: &str,
    turn_count: usize,
    adaptive_history: bool,
    soft_limit: u64,
    preferred_pairs: usize,
    min_pairs: usize,
) -> Value {
    let mut pairs = Vec::with_capacity(turn_count);
    let mut episodes = Vec::new();

    // Group older turns into periodic episodes (every 50 turns)
    for ep_start in (1..turn_count.saturating_sub(preferred_pairs)).step_by(50) {
        let ep_end = (ep_start + 49).min(turn_count.saturating_sub(preferred_pairs));
        if ep_end >= ep_start {
            let interaction_ids: Vec<String> =
                (ep_start..=ep_end).map(|i| format!("i{i}")).collect();
            episodes.push(json!({
                "fromSeq": ep_start,
                "toSeq": ep_end,
                "summary": format!("Resumo consolidado do episódio cobrindo turnos {ep_start} a {ep_end}."),
                "interactionIds": interaction_ids
            }));
        }
    }

    for seq in 1..=turn_count {
        let id = format!("i{seq}");
        pairs.push(json!({
            "id": id,
            "seq": seq as i64,
            "user": format!("Turno {seq}: Alex pergunta sobre o artefato misterioso."),
            "assistant": format!("Lia: No turno {seq}, examinamos a relíquia no laboratório.\n*Lia anota os detalhes.*"),
            "complete": true,
            "present_entities": [persona, "Lia"]
        }));
    }

    json!({
        "conv": {
            "persona": persona,
            "prompt": "Investigação arcana e arqueológica de alta fidelidade.",
            "settings": {
                "context": {
                    "preferredPairs": preferred_pairs,
                    "minPairs": min_pairs,
                    "maxPairs": 50,
                    "adaptiveHistory": adaptive_history,
                    "targetTokens": 96000,
                    "softLimit": soft_limit,
                    "hardLimit": 128000
                }
            }
        },
        "pairs": pairs,
        "totalPairs": turn_count,
        "beforeSeq": (turn_count + 1) as i64,
        "pathLength": turn_count,
        "cast": [
            {"name": "Lia", "role": "Pesquisadora"},
            {"name": "Carlos", "role": "Guardião ausente"}
        ],
        "scene": {
            "location": "Laboratório de Alquimia",
            "time": "madrugada",
            "mood": "concentração",
            "present": [persona, "Lia"],
            "objects": ["grimório antigo", "frasco de éter"],
            "characters": [
                {
                    "name": "Lia",
                    "location": "Laboratório de Alquimia",
                    "position": "mesa de trabalho",
                    "holding": ["pena de ganso"],
                    "possessions": ["pingente de safira", "óculos de leitura"]
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
                "a": persona,
                "b": "Helena",
                "relation_type": "child_of",
                "directed": true,
                "description": "filho de Helena"
            }
        ],
        "threads": [],
        "notes": [],
        "episodes": episodes,
        "timeline": {"current": null},
        "sync": {"lag": 0, "synced": true},
        "unprocessed": [],
        "current": {
            "interactionId": null,
            "seq": (turn_count + 1) as i64,
            "userText": "Qual é a inscrição na base do pingente de safira?",
            "priorParts": []
        }
    })
}

fn sonnet_plan() -> presets::Plan {
    presets::request(
        &json!({"presetId": "claude-sonnet-5-5", "routeId": "cc/claude-sonnet-5-5"}),
        vec![],
        &json!({}),
    )
}

#[test]
fn test_retrieval_window_start_microbenchmark_sub_millisecond() {
    let scales = [50, 100, 720, 2000];
    let iterations = 1000;

    for &turns in &scales {
        let data = generate_bench_data("Alex", turns, true, 110000, 30, 20);

        // Warm up
        for _ in 0..50 {
            let _ = context::retrieval_window_start(&data);
        }

        let start = Instant::now();
        for _ in 0..iterations {
            let seq = context::retrieval_window_start(&data);
            assert!(seq > 0, "Window start seq must be positive");
        }
        let elapsed = start.elapsed();
        let avg_us = (elapsed.as_secs_f64() * 1_000_000.0) / (iterations as f64);
        let avg_ms = avg_us / 1000.0;

        println!(
            "[retrieval_window_start] Scale {:4} turns: total={:?} for {} calls, avg={:.2} µs ({:.4} ms/call)",
            turns, elapsed, iterations, avg_us, avg_ms
        );

        // Assert strictly < 1.0 ms in release (debug allows unoptimized overhead)
        let threshold_ms = if cfg!(debug_assertions) { 10.0 } else { 1.0 };
        assert!(
            avg_ms < threshold_ms,
            "retrieval_window_start average latency must be < {threshold_ms}ms, got {:.4}ms at scale {}",
            avg_ms,
            turns
        );
    }
}

#[test]
fn test_adaptive_history_ladder_overhead_delta() {
    let plan = sonnet_plan();
    let scales = [100, 500, 1000];
    let samples = 50;

    for &turns in &scales {
        let data_adaptive = generate_bench_data("Alex", turns, true, 110000, 30, 20);
        let data_fixed = generate_bench_data("Alex", turns, false, 110000, 30, 20);

        // Warm up
        let _ = context::assemble(&data_adaptive, &json!({}), "send", &plan, None);
        let _ = context::assemble(&data_fixed, &json!({}), "send", &plan, None);

        // Measure adaptive
        let start_adaptive = Instant::now();
        for _ in 0..samples {
            let res = context::assemble(&data_adaptive, &json!({}), "send", &plan, None);
            assert!(res.is_ok());
        }
        let elapsed_adaptive = start_adaptive.elapsed();
        let avg_adaptive_ms = (elapsed_adaptive.as_secs_f64() * 1000.0) / (samples as f64);

        // Measure fixed
        let start_fixed = Instant::now();
        for _ in 0..samples {
            let res = context::assemble(&data_fixed, &json!({}), "send", &plan, None);
            assert!(res.is_ok());
        }
        let elapsed_fixed = start_fixed.elapsed();
        let avg_fixed_ms = (elapsed_fixed.as_secs_f64() * 1000.0) / (samples as f64);

        let delta_ms = (avg_adaptive_ms - avg_fixed_ms).abs();

        println!(
            "[adaptiveHistory Delta] Scale {:4} turns: adaptive={:.3}ms, fixed={:.3}ms, delta={:.3}ms",
            turns, avg_adaptive_ms, avg_fixed_ms, delta_ms
        );

        // Verify that the ladder calculation overhead delta is strictly < 1.0 ms in release
        let threshold_delta = if cfg!(debug_assertions) { 35.0 } else { 1.0 };
        assert!(
            delta_ms < threshold_delta,
            "Overhead delta of adaptiveHistory must be < {threshold_delta}ms, got {:.3}ms at scale {}",
            delta_ms,
            turns
        );
    }
}

#[test]
fn test_ladder_full_step_down_stress_performance() {
    let plan = sonnet_plan();
    // Create scenario with 40 turns, all covered by an episode from 1..=32
    // and heavy text in each turn, with low softLimit so assemble() must step down through rungs
    let mut data = generate_bench_data("Alex", 40, true, 2000, 30, 20);

    // Provide episode covering turns 1..=32 so [16, 12, 10, 8] are all eligible
    data["episodes"] = json!([{
        "id": "e_full",
        "fromSeq": 1,
        "toSeq": 32,
        "summary": "Resumo completo dos eventos prévios cobrindo turnos 1 a 32.",
        "interactionIds": (1..=32).map(|i| format!("i{i}")).collect::<Vec<_>>()
    }]);

    // Inflate pair text to guarantee pressure
    let heavy_text = "Texto volumoso de narrativa detalhada. ".repeat(80);
    for p in data["pairs"].as_array_mut().unwrap() {
        p["assistant"] = json!(heavy_text.clone());
    }

    let iterations = 100;
    let start = Instant::now();
    for _ in 0..iterations {
        let res = context::assemble(&data, &json!({}), "send", &plan, None);
        assert!(res.is_ok());
        let ctx = res.unwrap();
        // Should have stepped down to smallest rung (8 pairs)
        assert_eq!(
            ctx.selected_history_len, 8,
            "Under heavy pressure, ladder must step down to 8 pairs"
        );
    }
    let elapsed = start.elapsed();
    let avg_step_down_ms = (elapsed.as_secs_f64() * 1000.0) / (iterations as f64);

    println!(
        "[ladder step-down stress] 100 full descent assemblies: total={:?}, avg={:.3}ms per full descent",
        elapsed, avg_step_down_ms
    );

    // Full 8-level step down should still be fast (< 350ms in debug, < 25ms in release)
    let threshold_descent = if cfg!(debug_assertions) { 350.0 } else { 25.0 };
    assert!(
        avg_step_down_ms < threshold_descent,
        "Full ladder step down average latency must be < {threshold_descent}ms, got {:.3}ms",
        avg_step_down_ms
    );
}

#[test]
fn test_concurrency_and_memory_stability_under_load() {
    let data = Arc::new(generate_bench_data("Alex", 200, true, 110000, 30, 20));

    let threads = 16;
    let iterations_per_thread = 25;
    let start = Instant::now();

    let mut handles = Vec::new();
    for _ in 0..threads {
        let d = Arc::clone(&data);
        handles.push(std::thread::spawn(move || {
            let p = sonnet_plan();
            for _ in 0..iterations_per_thread {
                let res = context::assemble(&d, &json!({}), "send", &p, None);
                assert!(res.is_ok());
                let ctx = res.unwrap();
                assert_eq!(ctx.selected_history_len, 30);
                assert_eq!(ctx.report["exceededEstimatedBudget"], false);
            }
        }));
    }

    for h in handles {
        h.join().expect("Worker thread must not panic");
    }

    let elapsed = start.elapsed();
    let total_ops = threads * iterations_per_thread;
    let throughput = (total_ops as f64) / elapsed.as_secs_f64();

    println!(
        "[concurrency load] {} parallel operations across {} threads completed in {:?}: {:.1} ops/sec",
        total_ops, threads, elapsed, throughput
    );

    assert!(
        throughput > 50.0,
        "Concurrent context assembly throughput must exceed 50 ops/sec, got {:.1}",
        throughput
    );
}

#[test]
fn test_m1_m2_invariant_preservation_under_m3_adaptive_ladder() {
    let plan = sonnet_plan();
    // Test that M1 presence rules and M2 POV/kinship rules remain 100% active at both
    // uncompressed (30 pairs) and compressed (8 pairs) ladder rungs.

    for &force_compression in &[false, true] {
        let soft_limit = if force_compression { 3000 } else { 110000 };
        let mut data = generate_bench_data("Alex", 40, true, soft_limit, 30, 20);

        if force_compression {
            data["episodes"] = json!([{
                "id": "e_comp",
                "fromSeq": 1,
                "toSeq": 32,
                "summary": "Resumo dos acontecimentos anteriores.",
                "interactionIds": (1..=32).map(|i| format!("i{i}")).collect::<Vec<_>>()
            }]);
            for p in data["pairs"].as_array_mut().unwrap() {
                p["assistant"] = json!("Diálogo extenso e detalhado sobre o laboratório. ".repeat(50));
            }
        }

        let res = context::assemble(&data, &json!({}), "send", &plan, None);
        assert!(res.is_ok());
        let ctx = res.unwrap();

        let expected_len = if force_compression { 8 } else { 30 };
        assert_eq!(
            ctx.selected_history_len, expected_len,
            "Selected history length must match expectation"
        );

        let initial_text = ctx.messages.first().unwrap()["content"].as_str().unwrap();
        let tail_text = ctx.messages.last().unwrap()["content"].as_str().unwrap();

        // 1. M1 Epistemic Isolation Invariants
        assert!(
            tail_text.contains("sem metagaming: personagens ausentes"),
            "Tail must contain anti-metagaming directive regardless of ladder level"
        );

        // 2. M2 POV Matrix Invariants
        assert!(
            initial_text.contains("[MATRIZ DE PONTO DE VISTA (POV) E DESAMBIGUAÇÃO DE POSSE — LEI DE PERSPECTIVA]"),
            "Initial instructions must contain POV matrix regardless of ladder level"
        );
        assert!(
            initial_text.contains("1. NÍVEL 1 — NARRAÇÃO E AÇÕES (3ª Pessoa Nominativa Estrita):"),
            "POV Level 1 must be present"
        );
        assert!(
            initial_text.contains("2. NÍVEL 2 — PERSONAGEM FALANDO COM Alex (2ª Pessoa Direta):"),
            "POV Level 2 must be present"
        );

        // 3. M2 Kinship & Possession Invariants
        assert!(
            tail_text.contains("Helena → Alex: mãe de Alex"),
            "Directed kinship must be preserved across all ladder rungs"
        );
        assert!(
            tail_text.contains("Alex → Helena: filho de Helena"),
            "Directed kinship must be preserved across all ladder rungs"
        );
        assert!(
            tail_text.contains("pertences: pingente de safira, óculos de leitura"),
            "Character possessions must be preserved across all ladder rungs"
        );
        assert!(
            !tail_text.contains("nunca como 'minha'"),
            "Toxic negative rule must never appear in tail"
        );
    }
}
