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
    assert_eq!(
        context::assemble(&d, &json!({}), "send", &p, None)
            .err()
            .unwrap()
            .status,
        422
    );
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
fn native_context_order_reading_and_anchor_match_the_node_assembly() {
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
    // context/provenance BEFORE the current message; quote reading and reminder AFTER it
    let at = |k: &str| last.find(k).unwrap_or_else(|| panic!("faltou: {k}"));
    let pos = [
        at("[CONTEXTO ATUAL DA HISTÓRIA"),
        at("[PROVENIÊNCIA DE CITAÇÕES RECENTES"),
        at(&current[..30]),
        at("[LEITURA DA MENSAGEM ACIMA"),
        at("[Lembrete"),
    ];
    assert!(pos.windows(2).all(|w| w[0] < w[1]), "{pos:?}");
    assert!(last.contains(&format!(
        "“{}” foi dita originalmente por Mira",
        fx["quote"].as_str().unwrap()
    )));
    assert!(last.contains("comece DEPOIS dela, sem reescrevê-la"));
    assert!(!last.contains("Alex chamou"));
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
    assert!(last.contains("\"location\":\"sala\""));
    assert!(!last.contains("scene-uuid-1") && !last.contains("\"key\""));
}
