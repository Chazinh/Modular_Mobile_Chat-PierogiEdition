use modularchat_server::{context, presets};
use serde_json::{Value, json};

fn data() -> Value {
    json!({"conv":{"persona":"Alex","prompt":"Lia estuda magia.","settings":{"context":{"preferredPairs":30,"minPairs":20,"maxPairs":50}}},"pairs":(51..=100).map(|seq|json!({"id":format!("i{seq}"),"seq":seq,"user":format!("Pergunta {seq}"),"assistant":format!("Lia: Resposta integral {seq}."),"complete":true})).collect::<Vec<_>>(),"totalPairs":100,"beforeSeq":101,"pathLength":100,"cast":[],"scene":null,"pinned":[],"relationships":[],"threads":[],"notes":[],"episodes":[],"timeline":{"current":null},"sync":{"lag":1,"synced":false},"unprocessed":[{"id":"old","seq":2,"user":"A pergunta antiga.","assistant":format!("{} PENDING_TAIL_🚀","descrição ".repeat(800)),"complete":true}],"current":{"interactionId":null,"seq":101,"userText":"O que aconteceu?","priorParts":[]}})
}
#[test]
fn native_context_keeps_old_pending_in_full_and_recent_floor() {
    let d = data();
    let p = presets::request(
        &json!({"presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5"}),
        vec![],
        &json!({}),
    );
    let c = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    assert!(
        c.messages
            .iter()
            .any(|m| m["content"] == d["unprocessed"][0]["assistant"])
    );
    assert_eq!(c.report["pendingTurnsPreserved"], 1);
    assert_eq!(c.report["pairsIncluded"], 30);
    assert_eq!(c.messages.first().unwrap()["role"], "system");
    assert!(
        c.messages.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("O que aconteceu?")
    );
}
#[test]
fn native_context_overflow_keeps_pending_mandatory() {
    let mut d = data();
    d["conv"]["settings"]["context"]["hardLimit"] = json!(3000);
    d["unprocessed"][0]["assistant"] = json!(format!("{} PENDING_TAIL", "word ".repeat(10000)));
    let p = presets::request(
        &json!({"presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5"}),
        vec![],
        &json!({}),
    );
    let built = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    assert!(built.raw_estimate > 0);
    assert_eq!(built.report["overHardLimit"], true);
    assert_eq!(built.report["overCapacity"], false);
    assert_eq!(built.report["exceededEstimatedBudget"], true);
    assert!(built.messages.iter().any(|m| m["content"] == d["unprocessed"][0]["assistant"]));
    assert!(built.report["pairsIncluded"].as_u64().unwrap() >= 20);
}
#[test]
fn native_context_continuation_includes_canonical_user_and_all_parts() {
    let mut d = data();
    d["current"]["priorParts"] = json!(["Lia: PRIMEIRA_🚀", "Lia: SEGUNDA_✨"]);
    let p = presets::request(
        &json!({"presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5"}),
        vec![],
        &json!({}),
    );
    let c = context::assemble(&d, &json!({}), "continue", &p, None).unwrap();
    assert!(
        c.messages
            .iter()
            .any(|m| m["content"] == "Lia: PRIMEIRA_🚀\n\nLia: SEGUNDA_✨")
    );
    assert!(
        c.messages.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains(context::CONTINUE)
    );
}
#[test]
fn native_context_first_turn_rules_and_no_future_note() {
    let mut d = data();
    d["notes"] =
        json!([{"interactionId":"future","seq":101,"summary":"FUTURE_SHOULD_NEVER_APPEAR"}]);
    let p = presets::request(
        &json!({"presetId":"deepseek-v41-flash","routeId":"cbai/deepseek-v4.1-flash"}),
        vec![],
        &json!({}),
    );
    let c = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    assert_eq!(c.messages.first().unwrap()["role"], "user");
    assert!(c.messages.iter().all(|m| m["role"] != "system"));
    assert!(
        c.messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("INSTRUÇÕES DO SISTEMA")
    );
    assert!(c.messages.iter().all(|m| {
        !m["content"]
            .as_str()
            .unwrap_or_default()
            .contains("FUTURE_SHOULD_NEVER_APPEAR")
    }));
}

#[test]
fn native_context_attribution_appears_once_before_the_current_message() {
    let fx: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/quote-attribution.json")).unwrap();
    let (previous, current) = (
        fx["messages"][0]["content"].as_str().unwrap(),
        fx["messages"][1]["content"].as_str().unwrap(),
    );
    let mut d = data();
    d["pairs"] = json!([{"id":"p1","seq":1,"user":"Entro na sala.","assistant":previous,"complete":true,"versions":["v-original"],"sources":[{"id":"v-original","role":"assistant","text":previous}]}]);
    d["totalPairs"] = json!(1);
    d["unprocessed"] = json!([]);
    d["current"]["userText"] = json!(current);
    let p = presets::request(
        &json!({"presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5"}),
        vec![],
        &json!({}),
    );
    let c = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    let last = c.messages.last().unwrap()["content"].as_str().unwrap();
    // Plain-language provenance precedes the current message; only a short anchor follows it.
    let at = |k: &str| last.find(k).unwrap_or_else(|| panic!("faltou: {k}"));
    let pos = [
        at("[CONTEXTO ATUAL DA HISTÓRIA"),
        at("[PROVENIÊNCIA DE CITAÇÕES RECENTES"),
        at(&current[..30]),
        at("[Lembrete"),
    ];
    assert!(pos.windows(2).all(|w| w[0] < w[1]), "{pos:?}");
    assert!(last.contains(&format!(
        "“{}” foi dita originalmente por Mira",
        fx["quote"].as_str().unwrap()
    )));
    assert!(last.contains("comece DEPOIS dela, sem reescrevê-la"));
    assert!(!last.contains("Alex chamou"));
    assert!(!last.contains("[PLANO DA RESPOSTA") && !last.contains("[LEITURA DA MENSAGEM ACIMA"));
    assert!(
        !last.contains("v-original")
            && !last.contains("sourceRole")
            && !last.contains("originalEvidence")
    );
    // no matching quotes: no reading is asserted
    d["current"]["userText"] = json!("Sento na cadeira e espero.");
    let c = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    assert!(
        !c.messages.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("[LEITURA DA MENSAGEM ACIMA")
    );
    // continuation: no reading and its own reminder
    d["current"]["userText"] = json!(current);
    d["current"]["priorParts"] = json!(["*Ela sorri.*"]);
    let c = context::assemble(&d, &json!({}), "continue", &p, None).unwrap();
    let last = c.messages.last().unwrap()["content"].as_str().unwrap();
    assert!(!last.contains("[LEITURA DA MENSAGEM ACIMA"));
    assert!(last.contains("continue exatamente de onde a resposta parou"));
}
#[test]
fn core_prompt_has_the_canonical_input_and_style_rules() {
    let d = data();
    let p = presets::request(
        &json!({"presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5"}),
        vec![],
        &json!({}),
    );
    let c = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    let system = c.messages[0]["content"].as_str().unwrap();
    for phrase in [
        "ENTRADA CANÔNICA DE ALEX:",
        "JÁ ACONTECEU exatamente como foi escrito",
        "Cada frase de narração deve mudar algo na cena",
        "Reaja primeiro ao que a mensagem atual acabou de trazer",
        "aproximadamente 65% de diálogo e 35% de narração",
        "quem pronunciou originalmente",
    ] {
        assert!(system.contains(phrase), "faltou: {phrase}");
    }
    assert!(!system.contains("{{"));
}

#[test]
fn knowledge_lines_group_memories_by_state_once_per_character() {
    let mut d = data();
    d["scene"] = json!({"interactionId":"scene-uuid-1","seq":100,"location":"sala","time":null,"mood":null,"present":["Lia","Laura"],"objects":[],"characters":[{"key":"lia","name":"Lia","location":"sala","position":null,"holding":[]}]});
    d["pinned"] = json!([
        {"id":"m1","kind":"fact","subject":"Lia","content":"Lia escondeu a chave.","visibility":"private","knowledge":[{"name":"Lia","state":"knows"},{"name":"Laura","state":"believes","belief":"acha que foi o vento"}]},
        {"id":"m2","kind":"fact","subject":"Mundo","content":"A cidade tem um farol.","visibility":"public","knowledge":[]},
        {"id":"m3","kind":"fact","subject":"Lia","content":"Lia teme o mar.","visibility":"private","knowledge":[]}
    ]);
    let p = presets::request(
        &json!({"presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5"}),
        vec![],
        &json!({}),
    );
    let c = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    let last = c.messages.last().unwrap()["content"].as_str().unwrap();
    // the state label appears once per person; the order is knows, believes, does NOT know
    assert!(
        last.contains("\nLia: sabe [M1], [M2]; NÃO sabe e não pode agir sobre [M3]\n"),
        "{last}"
    );
    assert!(last.contains(
        "\nLaura: sabe [M2]; acredita (pode estar errado) [M1] (acha que foi o vento); NÃO sabe e não pode agir sobre [M3]\n"
    ));
    assert!(last.contains("\nAlex: sabe [M2]; NÃO sabe e não pode agir sobre [M1], [M3]\n"));
    // the scene stays in the prompt, without the interaction ID or the normalized key (which repeats the name)
    assert!(last.contains("[CENA ANTERIOR"));
    assert!(last.contains("Local: sala"));
    assert!(!last.contains("scene-uuid-1") && !last.contains("\"key\""));
}

#[test]
fn episode_coverage_reduces_history_without_dropping_pending_or_summaries() {
    let mut d = data();
    for pair in d["pairs"].as_array_mut().unwrap() {
        pair["assistant"] = json!("Lia: Uma lembrança detalhada. ".repeat(180));
    }
    d["episodes"] = json!([{"id":"episode-local-id","fromSeq":51,"toSeq":90,"summary":"Resumo completo que deve permanecer. ".repeat(80),"interactionIds":(51..=90).map(|seq|format!("i{seq}")).collect::<Vec<_>>()}]);
    let p = presets::request(
        &json!({"presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5"}),
        vec![],
        &json!({}),
    );
    d["conv"]["settings"]["context"]["adaptiveHistory"] = json!(false);
    let full = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    d["conv"]["settings"]["context"]["adaptiveHistory"] = json!(true);
    let compact = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    assert_eq!(compact.report["pairsIncluded"], 10);
    assert_eq!(context::retrieval_window_start(&d), 91);
    assert_eq!(
        compact.messages[0], full.messages[0],
        "stable rules do not change"
    );
    assert!(
        compact
            .messages
            .iter()
            .any(|m| m["content"] == d["unprocessed"][0]["assistant"])
    );
    assert!(
        compact.messages.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains(d["episodes"][0]["summary"].as_str().unwrap())
    );
    assert!(compact.raw_estimate < full.raw_estimate / 2);
    println!(
        "history fixture: full={} tokens, summarized={} tokens (estimates)",
        full.raw_estimate, compact.raw_estimate
    );
    // A single coverage hole, stale endpoint or empty summary disables the lower floor.
    for invalid in ["hole", "future", "empty", "pending"] {
        let mut bad = d.clone();
        match invalid {
            "hole" => {
                bad["episodes"][0]["interactionIds"]
                    .as_array_mut()
                    .unwrap()
                    .remove(20);
            }
            "future" => bad["episodes"][0]["toSeq"] = json!(101),
            "empty" => bad["episodes"][0]["summary"] = json!(" "),
            _ => bad["unprocessed"] = json!([bad["pairs"][0]]),
        }
        let built = context::assemble(&bad, &json!({}), "send", &p, None).unwrap();
        assert!(
            built.report["pairsIncluded"].as_u64().unwrap() >= 20,
            "{invalid}"
        );
        assert_eq!(context::retrieval_window_start(&bad), 71);
    }
    // Exceeding the estimate must preserve the sole historical record and pending text.
    d["conv"]["settings"]["context"]["hardLimit"] = json!(3000);
    let built = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    assert_eq!(built.report["exceededEstimatedBudget"], true);
    assert!(built.messages.iter().any(|m| m["content"] == d["unprocessed"][0]["assistant"]));
    assert!(built.messages.last().unwrap()["content"].as_str().unwrap().contains(d["episodes"][0]["summary"].as_str().unwrap()));
}

#[test]
fn prompt_views_keep_narrative_facts_but_exclude_diagnostics_and_ids() {
    let mut d = data();
    d["scene"] = json!({"interactionId":"secret-scene-id","seq":100,"location":"torre","time":"noite","mood":"tensão","present":["Lia"],"objects":["carta"],"characters":[{"key":"lia","name":"Lia","location":"janela","position":"sentada","holding":["chave"]}]});
    d["timeline"]["current"] = json!({"days":8,"months":2,"date":"2026-10-15","anchors":[{"sourceId":"secret-source-id","start":1234,"evidence":"há dois meses","elapsedDays":8,"elapsedMonths":4}]});
    d["usage"] = json!({"error":"SECRET_DIAGNOSTIC","provider":"SECRET_PROVIDER"});
    let p = presets::request(
        &json!({"presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5"}),
        vec![],
        &json!({}),
    );
    let built = context::assemble(&d, &json!({}), "send", &p, None).unwrap();
    let prompt = serde_json::to_string(&built.messages).unwrap();
    for kept in [
        "torre",
        "noite",
        "tensão",
        "carta",
        "janela",
        "sentada",
        "chave",
        "há dois meses",
        "2026-10-15",
        "8 dias e 4 meses",
    ] {
        assert!(prompt.contains(kept), "{kept}");
    }
    for removed in [
        "secret-scene-id",
        "secret-source-id",
        "1234",
        "SECRET_DIAGNOSTIC",
        "SECRET_PROVIDER",
        "PLANO DA RESPOSTA",
        "sourceId",
    ] {
        assert!(!prompt.contains(removed), "{removed}");
    }
}

fn assembled_text(built: &context::Context) -> String {
    built
        .messages
        .iter()
        .filter_map(|m| m["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn raw_excludes_the_actual_recent_window_and_pending_sources_without_text_dedup() {
    let plan = presets::request(
        &json!({"presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5"}),
        vec![],
        &json!({}),
    );
    for adaptive in [false, true] {
        let mut d = data();
        d["pairs"][49]["sources"] =
            json!([{"id":"v100","role":"assistant","text":d["pairs"][49]["assistant"]}]);
        if adaptive {
            d["episodes"] = json!([(51,90),(91,92),(93,94)].map(|(from,to)| json!({
                "id":format!("e{from}"), "fromSeq":from, "toSeq":to, "summary":format!("Resumo {from} a {to}."),
                "interactionIds":(from..=to).map(|seq|format!("i{seq}")).collect::<Vec<_>>()
            })));
            assert_eq!(context::retrieval_window_start(&d), 95);
        }
        let repeated = d["pairs"][42]["assistant"].as_str().unwrap().to_owned();
        let retrieval = json!({"raw":[
            {"sourceSeq":93,"text":repeated},
            {"sourceInteractionId":"old","text":d["unprocessed"][0]["user"]},
            {"sourceId":"v100","text":d["pairs"][49]["assistant"]},
            {"sourceId":"different-old-version","sourceInteractionId":"i10","sourceSeq":10,"text":repeated}
        ]});
        let before = serde_json::to_vec(&json!([d, retrieval])).unwrap();
        let built = context::assemble(&d, &retrieval, "send", &plan, None).unwrap();
        let prompt = assembled_text(&built);
        assert_eq!(
            built.report["pairsIncluded"],
            if adaptive { 10 } else { 30 }
        );
        assert_eq!(built.report["rawIncluded"], 1);
        assert_eq!(built.report["pendingTurnsPreserved"], 1);
        assert_eq!(
            prompt.matches(&repeated).count(),
            2,
            "same words from different interactions survive"
        );
        assert_eq!(prompt.matches("A pergunta antiga.").count(), 1);
        assert_eq!(prompt.matches("Lia: Resposta integral 100.").count(), 1);
        assert!(
            built
                .messages
                .iter()
                .any(|m| m["content"] == d["unprocessed"][0]["assistant"])
        );
        assert_eq!(before, serde_json::to_vec(&json!([d, retrieval])).unwrap());
    }
}

fn old_chunk(source: &str, role: &str, full: &str, from: usize, to: usize) -> Value {
    use modularchat_server::rules;
    let a = full
        .char_indices()
        .map(|(i, _)| i)
        .nth(from)
        .unwrap_or(full.len());
    let b = full
        .char_indices()
        .map(|(i, _)| i)
        .nth(to)
        .unwrap_or(full.len());
    json!({"sourceId":source,"role":role,"sourceSeq":10,"start":rules::utf16_len(&full[..a]),"end":rules::utf16_len(&full[..b]),"text":&full[a..b]})
}

#[test]
fn raw_merges_matching_utf16_intervals_in_priority_order_and_preserves_quotes() {
    let d = data();
    let plan = presets::request(&json!({"presetId":"claude-sonnet-5-5"}), vec![], &json!({}));
    let full = "Mira: “Olá 🚀, ação e\u{301} água.”\nLia: Eu guardo a chave 🗝️.";
    let end = full.chars().count();
    let independent = "Nora: Uma citação diferente.";
    let raw = json!({"raw":[
        old_chunk("v-old","assistant",full,18,end),
        old_chunk("v-other","assistant",independent,0,independent.chars().count()),
        old_chunk("v-old","assistant",full,0,33),
        old_chunk("v-old","assistant",full,20,29)
    ]});
    let before = raw.clone();
    let built = context::assemble(&d, &raw, "send", &plan, None).unwrap();
    let prompt = assembled_text(&built);
    assert_eq!(built.report["rawIncluded"], 2);
    assert_eq!(prompt.matches(full).count(), 1);
    assert!(prompt.find(full).unwrap() < prompt.find(independent).unwrap());
    assert_eq!(raw, before);

    // A bridging chunk must also unite earlier disjoint intervals.
    let bridge = json!({"raw":[
        old_chunk("v-old","assistant",full,0,16),
        old_chunk("v-old","assistant",full,30,end),
        old_chunk("v-old","assistant",full,14,34)
    ]});
    let built = context::assemble(&d, &bridge, "send", &plan, None).unwrap();
    assert_eq!(built.report["rawIncluded"], 1);
    assert!(assembled_text(&built).contains(full));
}

#[test]
fn raw_keeps_different_sources_roles_divergent_text_and_invalid_utf16_boundaries() {
    let plan = presets::request(&json!({"presetId":"claude-sonnet-5-5"}), vec![], &json!({}));
    let text = "Mira: A chave está na torre 🚀.";
    let mut divergent = old_chunk("v-a", "assistant", text, 0, text.chars().count());
    divergent["text"] = json!(text.replace("torre", "ponte"));
    let raw = json!({"raw":[
        old_chunk("v-a","assistant",text,0,text.chars().count()),
        old_chunk("v-b","assistant",text,0,text.chars().count()),
        old_chunk("v-a","user",text,0,text.chars().count()),
        divergent
    ]});
    let built = context::assemble(&data(), &raw, "send", &plan, None).unwrap();
    assert_eq!(built.report["rawIncluded"], 4);
    let prompt = assembled_text(&built);
    assert_eq!(prompt.matches(text).count(), 3);
    assert!(prompt.contains("chave está na ponte 🚀"));

    let raw = json!({"raw":[
        {"sourceId":"emoji","role":"assistant","sourceSeq":10,"start":0,"end":3,"text":"😀A"},
        {"sourceId":"emoji","role":"assistant","sourceSeq":10,"start":1,"end":3,"text":"BC"},
        {"sourceId":"emoji","role":"assistant","sourceSeq":10,"start":0,"end":99,"text":"OUTRA_🚀"}
    ]});
    let built = context::assemble(&data(), &raw, "send", &plan, None).unwrap();
    assert_eq!(built.report["rawIncluded"], 3);
    for kept in ["😀A", "BC", "OUTRA_🚀"] {
        assert!(assembled_text(&built).contains(kept));
    }
}

#[test]
fn memory_identity_is_unique_across_pinned_candidates_and_private_without_content_dedup() {
    let mut d = data();
    d["scene"] = json!({"present":["Lia"]});
    let pinned = json!({"id":"p","kind":"SECRET","content":"SEGREDO_FIXADO","visibility":"private","knowledge":[{"name":"Lia","state":"knows"}]});
    d["pinned"] = json!([pinned, pinned]);
    let a = json!({"id":"a","type":"memory","sourceSeq":10,"kind":"EVENT","content":"CONTEUDO_IGUAL","visibility":"private","knowledge":[{"name":"Lia","state":"knows"}]});
    let mut b = a.clone();
    b["id"] = json!("b");
    b["knowledge"][0]["state"] = json!("unaware");
    let private = json!({"id":"private","sourceSeq":10,"kind":"SECRET","content":"SEGREDO_PRIVADO","visibility":"private","knowledge":[{"name":"Lia","state":"believes","belief":"A chave está perdida."}]});
    let mut duplicate = a.clone();
    duplicate["content"] = json!("DUPLICATA_NAO_PRIORITARIA");
    let retrieval =
        json!({"candidates":[pinned,a,duplicate,b],"privateMemories":[a,pinned,private,private]});
    let before = serde_json::to_vec(&json!([d, retrieval])).unwrap();
    let plan = presets::request(&json!({"presetId":"claude-sonnet-5-5"}), vec![], &json!({}));
    let built = context::assemble(&d, &retrieval, "send", &plan, None).unwrap();
    let prompt = assembled_text(&built);
    assert_eq!(built.report["pinnedIncluded"], 1);
    assert_eq!(built.report["memoriesIncluded"], json!(["a", "b"]));
    assert_eq!(prompt.matches("SEGREDO_FIXADO").count(), 1);
    assert_eq!(prompt.matches("SEGREDO_PRIVADO").count(), 1);
    assert_eq!(prompt.matches("CONTEUDO_IGUAL").count(), 2);
    assert!(!prompt.contains("DUPLICATA_NAO_PRIORITARIA"));
    assert!(prompt.contains("sabe [M1], [M2]"));
    assert!(prompt.contains("NÃO sabe e não pode agir sobre [M3]"));
    assert!(prompt.contains("[M4] (A chave está perdida.)"));
    assert_eq!(before, serde_json::to_vec(&json!([d, retrieval])).unwrap());
}
