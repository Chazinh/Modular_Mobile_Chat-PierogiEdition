//! Behavior cases that used to be checked by tests of the old Node server and are now
//! checked directly in Rust (context assembly with real Store data, curation,
//! style policy and heuristic agency review).
use modularchat_server::{
    context::{self, Context},
    curation, presets, review,
    store::{FinishRequest, Request, Store, TurnRequest},
    style_policy,
};
use serde_json::{Value, json};

struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
}
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    Fixture { _dir: dir, store }
}
async fn conv(store: &Store, over: Value) -> String {
    let mut body = json!({"name":"Teste","persona":"Alex","routeId":"cc/claude-sonnet-5-5","presetId":"claude-sonnet-5-5","effort":"low","prompt":"Mira é uma maga arrogante. Nora é sua melhor amiga."});
    for (k, v) in over.as_object().unwrap() {
        body[k] = v.clone();
    }
    let c = store.call(Request::CreateConversation(body)).await.unwrap();
    c["id"].as_str().unwrap().to_owned()
}
/// Narrow context (20 pairs) so that a few turns already leave the recent window.
fn narrow() -> Value {
    json!({"settings":{"context":{"preferredPairs":20,"minPairs":20}}})
}
async fn turn(
    store: &Store,
    cid: &str,
    user: Option<&str>,
    reply: &str,
    mode: &str,
    target: Option<&str>,
) -> Value {
    let begin = store
        .call(Request::BeginTurn(
            serde_json::from_value::<TurnRequest>(json!({
                "convId":cid,"mode":mode,"content":user,"targetId":target,
                "routeId":"cc/claude-sonnet-5-5","presetId":"claude-sonnet-5-5","effort":"low"
            }))
            .unwrap(),
        ))
        .await
        .unwrap();
    store
        .call(Request::FinishGeneration(
            serde_json::from_value::<FinishRequest>(json!({
                "generationId":begin["generationId"],"status":"complete","text":reply,"usage":{}
            }))
            .unwrap(),
        ))
        .await
        .unwrap();
    begin
}
async fn say(store: &Store, cid: &str, user: &str, reply: &str) -> Value {
    turn(store, cid, Some(user), reply, "send", None).await
}
fn ext(over: Value) -> Value {
    let mut base = json!({"summary":"s","cast":[],"scene":null,"facts":[],"relationships":[],"openThreads":[],"resolvedThreads":[]});
    for (k, v) in over.as_object().unwrap() {
        base[k] = v.clone();
    }
    base
}
fn scene(location: &str, present: Value) -> Value {
    json!({"location":location,"time":null,"mood":null,"present":present,"objects":[],"positions":{},"holding":{},"locations":{}})
}
async fn apply(store: &Store, cid: &str, t: &Value, extraction: Value, source: &str) -> Value {
    store
        .call(Request::ApplyExtraction(json!({
            "convId":cid,"interactionId":t["interactionId"],"versionId":t["versionId"],
            "versionIds":[t["versionId"]],"sourceText":source,"extraction":extraction
        })))
        .await
        .unwrap()
}
fn plan(route: &str, preset: &str) -> presets::Plan {
    presets::request(
        &json!({"presetId":preset,"routeId":route}),
        vec![],
        &json!({}),
    )
}
/// Same flow as production generation: Store data, lexical retrieval and assembly.
async fn build(
    store: &Store,
    cid: &str,
    user: &str,
    mode: &str,
) -> (Value, Value, modularchat_server::Result<Context>) {
    let data = store
        .call(Request::Context(
            json!({"convId":cid,"pendingUserText":user}),
        ))
        .await
        .unwrap();
    let pairs = data["pairs"].as_array().unwrap();
    // start of the recent window = preferred pair count configured in the conversation (default 30)
    let preferred = data["conv"]["settings"]["context"]["preferredPairs"]
        .as_u64()
        .unwrap_or(30) as usize;
    let window = pairs
        .get(pairs.len().saturating_sub(preferred))
        .map(|p| p["seq"].clone())
        .unwrap_or(data["current"]["seq"].clone());
    let retrieval = store
        .call(Request::Retrieve(json!({
            "convId":cid,"queryText":user,"queryVec":null,"beforeSeq":data["beforeSeq"],
            "windowStartSeq":window,"limit":24
        })))
        .await
        .unwrap();
    let ctx = context::assemble(
        &data,
        &retrieval,
        mode,
        &plan("cc/claude-sonnet-5-5", "claude-sonnet-5-5"),
        None,
    );
    (data, retrieval, ctx)
}
fn last(ctx: &Context) -> &str {
    ctx.messages.last().unwrap()["content"].as_str().unwrap()
}
fn system(ctx: &Context) -> &str {
    ctx.messages[0]["content"].as_str().unwrap()
}

#[tokio::test]
async fn secret_from_another_room_is_not_known_by_the_absent_character() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, narrow()).await;
    let a = say(
        s,
        &cid,
        "Sigo Lia até a garagem.",
        "*Lia mexe no freio do carro sem ninguém ver.*\n\nLia: Agora ele não vai chegar longe.",
    )
    .await;
    apply(s, &cid, &a, ext(json!({
        "scene": {"location":"garagem","time":"noite","mood":"tenso","present":["Lia","Mark"],"objects":["carro"],"positions":{"Lia":"ao lado do carro"},"holding":{},"locations":{}},
        "facts": [{"kind":"SECRET","subject":"Lia","content":"Lia sabotou o freio do carro.","importance":0.95,"confidence":0.9,"visibility":"private",
            "knowledge":[{"who":"Lia","state":"knows","via":"witnessed","evidence":"","belief":null},{"who":"Mark","state":"knows","via":"witnessed","evidence":"","belief":null}]}]
    })), "Lia mexe no freio do carro sem ninguém ver. Lia: Agora ele não vai chegar longe. Mark observa da porta.").await;
    let b = say(
        s,
        &cid,
        "Volto para a sala com Laura.",
        "*Laura serve café.*\n\nLaura: Onde vocês estavam?",
    )
    .await;
    apply(
        s,
        &cid,
        &b,
        ext(json!({"scene":scene("sala", json!(["Laura"]))})),
        "Laura serve café. Laura: Onde vocês estavam?",
    )
    .await;
    for i in 0..24 {
        say(
            s,
            &cid,
            &format!("Conversa qualquer {i}"),
            &format!("Laura: Resposta qualquer {i}."),
        )
        .await;
    }
    let (_, _, ctx) = build(
        s,
        &cid,
        "Pergunto a Laura se ela notou algo estranho no carro.",
        "send",
    )
    .await;
    let ctx = ctx.unwrap();
    let text = last(&ctx);
    assert!(
        text.contains("Lia sabotou o freio do carro"),
        "o narrador pode conhecer"
    );
    let line = text
        .lines()
        .find(|l| l.starts_with("Laura:"))
        .expect("há linha de conhecimento de Laura");
    assert!(line.contains("NÃO sabe"), "{line}");
    assert!(
        !line.split("NÃO sabe").next().unwrap().contains("sabe [M"),
        "{line}"
    );
}

#[tokio::test]
async fn long_history_recovers_old_fact_with_source_inside_the_budget() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, json!({})).await;
    let mut first = Value::Null;
    for i in 1..=45 {
        let r = say(
            s,
            &cid,
            &format!("Mensagem {i} sobre assuntos comuns do dia"),
            &format!("Mira: Resposta {i} sobre o dia."),
        )
        .await;
        if i == 3 {
            first = r["interactionId"].clone();
            apply(s, &cid, &r, ext(json!({
                "summary":"Mira guarda a chave dourada",
                "facts":[{"kind":"OBJECT","subject":"Mira","content":"Mira guarda a chave dourada da biblioteca proibida no baú.","importance":0.9,"confidence":0.9,"visibility":"public","knowledge":[]}]
            })), "x").await;
        }
    }
    let (data, retrieval, ctx) = build(
        s,
        &cid,
        "Pergunto a Mira sobre a chave dourada da biblioteca.",
        "send",
    )
    .await;
    let ctx = ctx.unwrap();
    assert_eq!(data["totalPairs"], 45);
    assert_eq!(ctx.report["pairsIncluded"], 30, "30 pares preferidos");
    let hit = retrieval["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["content"].as_str().unwrap().contains("chave dourada"))
        .expect("fato antigo recuperado");
    assert_eq!(hit["sourceInteractionId"], first, "com a fonte");
    assert!(
        ctx.report["memoriesIncluded"]
            .as_array()
            .unwrap()
            .contains(&hit["id"]),
        "e incluído no contexto"
    );
    assert!(ctx.report["tokens"]["total"].as_u64().unwrap() < 96_000);
    assert!(
        last(&ctx).contains("chave dourada"),
        "memória vai junto da mensagem atual"
    );
    let (_, _, recent) = build(s, &cid, "Mensagem 44", "send").await;
    assert_eq!(recent.unwrap().report["pairsIncluded"], 30);
}

#[tokio::test]
async fn budget_reduces_optional_parts_before_pairs_and_never_cuts_messages() {
    let f = fixture();
    let s = &f.store;
    let big = "palavra ".repeat(500);
    let cid = conv(
        s,
        json!({"settings":{"context":{"targetTokens":40000,"softLimit":50000,"hardLimit":128000}}}),
    )
    .await;
    for i in 1..=34 {
        say(
            s,
            &cid,
            &format!("{big} {i}"),
            &format!("Mira: {big} {i}"),
        )
        .await;
    }
    let (data, _, ctx) = build(s, &cid, "continuo", "send").await;
    let ctx = ctx.unwrap();
    let included = ctx.report["pairsIncluded"].as_u64().unwrap();
    assert!(included < 30, "reduziu os pares recentes: {included}");
    assert!(included >= 20, "respeitou o mínimo: {included}");
    assert!(
        ctx.report["compression"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c.as_str().unwrap().contains("histórico recente reduzido"))
    );
    assert!(
        ctx.messages
            .iter()
            .filter(|m| m["role"] == "assistant")
            .all(|m| m["content"].as_str().unwrap().len() >= big.len()),
        "nenhuma mensagem foi cortada"
    );
    assert_eq!(data["totalPairs"], 34);
    // Above the estimated budget even at the minimum: keep complete messages
    // and report the overflow; the provider enforces its actual context window.
    let tight = conv(
        s,
        json!({"settings":{"context":{"targetTokens":5000,"softLimit":6000,"hardLimit":20000}}}),
    )
    .await;
    for i in 1..=34 {
        say(
            s,
            &tight,
            &format!("{big} {i}"),
            &format!("Mira: {big} {i}"),
        )
        .await;
    }
    let tight_ctx = build(s, &tight, "continuo", "send").await.2.unwrap();
    assert_eq!(tight_ctx.report["pairsIncluded"], 20);
    assert_eq!(tight_ctx.report["limits"]["hard"], 20000);
    assert!(
        tight_ctx.report["tokens"]["total"].as_u64().unwrap()
            > tight_ctx.report["limits"]["hard"].as_u64().unwrap()
    );
    assert_eq!(tight_ctx.report["overHardLimit"], true);
    assert_eq!(tight_ctx.report["exceededEstimatedBudget"], true);
    assert!(
        tight_ctx
            .messages
            .iter()
            .filter(|m| m["role"] == "assistant")
            .all(|m| m["content"].as_str().unwrap().len() >= big.len()),
        "over budget messages must also remain complete"
    );
}

/// A turn where the model omits place/time/mood/position inherits the last non-empty value; an explicit change wins.
#[tokio::test]
async fn omitted_scene_fields_carry_forward_but_explicit_changes_win() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, json!({})).await;
    let first = say(s, &cid, "a", "x").await;
    apply(s, &cid, &first, ext(json!({"scene":{"location":"Quarto","time":"noite","mood":"tenso","present":["Lia"],"objects":[],
        "positions":{"Lia":"sentada na cama"},"holding":{},"locations":{}}})), "a x").await;
    // the model omits place, time, mood and Lia's position
    let second = say(s, &cid, "b", "y").await;
    apply(
        s,
        &cid,
        &second,
        ext(
            json!({"scene":{"location":null,"time":null,"mood":"","present":["Lia"],"objects":[],
        "positions":{},"holding":{"Lia":[]},"locations":{}}}),
        ),
        "b y",
    )
    .await;
    let (data, _, _) = build(s, &cid, "c", "send").await;
    assert_eq!(data["scene"]["interactionId"], second["interactionId"]);
    assert_eq!(data["scene"]["location"], "Quarto");
    assert_eq!(data["scene"]["time"], "noite");
    assert_eq!(data["scene"]["mood"], "tenso");
    assert_eq!(data["scene"]["characters"][0]["location"], "Quarto");
    assert_eq!(
        data["scene"]["characters"][0]["position"],
        "sentada na cama"
    );
    // explicit change of place: nothing of the old place (not even the position) is kept
    let third = say(s, &cid, "c", "z").await;
    apply(s, &cid, &third, ext(json!({"scene":{"location":"Cozinha","time":null,"mood":null,"present":["Lia"],"objects":[],
        "positions":{},"holding":{"Lia":[]},"locations":{}}})), "c z").await;
    let (data, _, _) = build(s, &cid, "d", "send").await;
    assert_eq!(data["scene"]["location"], "Cozinha");
    assert_eq!(
        data["scene"]["time"], "noite",
        "tempo omitido continua herdado"
    );
    assert_eq!(data["scene"]["characters"][0]["location"], "Cozinha");
    assert!(data["scene"]["characters"][0]["position"].is_null());
}

#[tokio::test]
async fn lagging_scene_state_is_flagged_and_recent_history_prevails() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, json!({})).await;
    let a = say(s, &cid, "a", "x").await;
    apply(
        s,
        &cid,
        &a,
        ext(json!({"scene":scene("cozinha", json!(["Lara"]))})),
        "a x",
    )
    .await;
    say(s, &cid, "Fomos para o jardim", "Lara: Que lindo aqui.").await; // extraction has not run yet
    let (data, _, ctx) = build(s, &cid, "Olho para Lara", "send").await;
    assert_eq!(data["sync"]["synced"], false);
    assert_eq!(data["sync"]["lag"], 1);
    let ctx = ctx.unwrap();
    assert!(last(&ctx).contains("possivelmente atrasada"));
    assert!(last(&ctx).contains("transições recentes prevalecem"));
}

#[tokio::test]
async fn context_follows_the_plan_order_and_uses_user_prefix_when_system_is_dropped() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, json!({})).await;
    let a = say(s, &cid, "Oi", "Mira: Oi.").await;
    apply(
        s,
        &cid,
        &a,
        ext(json!({
            "scene": scene("sala", json!(["Mira"])),
            "openThreads": ["Mira ainda esconde a carta"]
        })),
        "x",
    )
    .await;
    let data = s
        .call(Request::Context(
            json!({"convId":cid,"pendingUserText":"Pergunto da carta"}),
        ))
        .await
        .unwrap();
    let empty = json!({"candidates":[],"privateMemories":[],"raw":[]});
    let built = context::assemble(
        &data,
        &empty,
        "send",
        &plan("cc/claude-sonnet-5-5", "claude-sonnet-5-5"),
        None,
    )
    .unwrap();
    let sys = system(&built);
    let order: Vec<usize> = [
        "[REGRAS DA CENA]",
        "[PERSONALIZAÇÃO",
        "[PERSONA DO USUÁRIO]",
    ]
    .iter()
    .map(|k| {
        sys.find(k)
            .unwrap_or_else(|| panic!("faltou bloco estável: {k}"))
    })
    .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    assert!(
        !sys.contains("[CENA ANTERIOR") && !sys.contains("[THREADS ABERTAS]"),
        "estado vivo fora do system"
    );
    let live = last(&built);
    let pos: Vec<usize> = [
        "[CONTEXTO ATUAL DA HISTÓRIA",
        "[CENA ANTERIOR",
        "[THREADS ABERTAS]",
        "Pergunto da carta",
        "[Lembrete",
    ]
    .iter()
    .map(|k| live.find(k).unwrap_or_else(|| panic!("faltou: {k}")))
    .collect();
    assert!(
        pos.windows(2).all(|w| w[0] < w[1]),
        "ordem: contexto, cena, threads, mensagem, lembrete: {pos:?}"
    );
    assert!(
        sys.contains(
            "Nunca escreva falas, pensamentos, sentimentos internos, decisões ou ações de Alex"
        ),
        "regra de autoria do usuário preservada"
    );
    // route that drops system: rules become the first user turn and the roles alternate
    let pre = context::assemble(
        &data,
        &empty,
        "send",
        &plan("cbai/deepseek-v4.1-flash", "deepseek-v41-flash"),
        None,
    )
    .unwrap();
    assert_eq!(pre.messages[0]["role"], "user");
    assert!(
        pre.messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("INSTRUÇÕES DO SISTEMA")
    );
    assert_eq!(pre.messages[1]["role"], "assistant");
    assert!(
        pre.messages
            .windows(2)
            .all(|w| w[0]["role"] != w[1]["role"]),
        "papéis alternam"
    );
    assert_eq!(pre.messages.last().unwrap()["role"], "user");
    assert!(last(&pre).contains("não escreva nada por Alex"));
}

#[tokio::test]
async fn memory_pinned_by_the_user_always_enters() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, json!({})).await;
    let m = s
        .call(Request::AddMemory {
            conv_id: cid.clone(),
            value: json!({"kind":"WORLD_FACT","content":"Na Academia Aurora é proibido usar magia de fogo.","importance":0.5}),
        })
        .await
        .unwrap();
    s.call(Request::PatchMemory {
        conv_id: cid.clone(),
        memory_id: m["id"].as_str().unwrap().to_owned(),
        value: json!({"pinned":true}),
    })
    .await
    .unwrap();
    for i in 0..3 {
        say(s, &cid, &format!("Assunto {i}"), &format!("Resposta {i}")).await;
    }
    let (_, _, ctx) = build(s, &cid, "Vamos tomar sorvete?", "send").await;
    let ctx = ctx.unwrap();
    assert!(last(&ctx).contains("proibido usar magia de fogo"));
    assert!(last(&ctx).contains("fixada"));
    assert_eq!(ctx.report["pinnedIncluded"], 1);
}

#[tokio::test]
async fn timeline_puts_episodes_and_notes_outside_the_window_in_order_without_gaps() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, narrow()).await;
    let mut ids = Vec::new();
    for i in 1..=24 {
        let r = say(s, &cid, &format!("Turno {i}"), &format!("Resposta {i}")).await;
        ids.push(r["interactionId"].clone());
        apply(
            s,
            &cid,
            &r,
            ext(json!({"summary":format!("Nota do turno {i}.")})),
            "x",
        )
        .await;
    }
    s.call(Request::CreateEpisode(json!({
        "convId":cid,"interactionIds":ids[..2],"fromSeq":1,"toSeq":2,"summary":"Episódio inicial: chegada à academia."
    })))
    .await
    .unwrap();
    let (_, _, ctx) = build(s, &cid, "E agora?", "send").await;
    let ctx = ctx.unwrap();
    let live = last(&ctx);
    assert!(live.contains("LINHA DO TEMPO ANTERIOR"));
    assert!(live.contains("- Episódio inicial"));
    for n in [3, 4] {
        assert!(
            live.contains(&format!("- Nota do turno {n}.")),
            "turno {n}"
        );
    }
    assert!(
        !live.contains("Nota do turno 2."),
        "turnos cobertos pelo episódio não se repetem"
    );
    assert!(
        !live.contains("Nota do turno 5."),
        "turnos da janela recente não se repetem"
    );
    assert!(live.find("Episódio inicial").unwrap() < live.find("Nota do turno 3").unwrap());
}

#[tokio::test]
async fn request_prefix_does_not_change_between_turns() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, json!({})).await;
    for i in 0..3 {
        say(
            s,
            &cid,
            &format!("Turno {i}"),
            &format!("Mira: Resposta {i}."),
        )
        .await;
    }
    let (_, _, a) = build(s, &cid, "Primeira pergunta", "send").await;
    let a = a.unwrap();
    let r = say(s, &cid, "Primeira pergunta", "Mira: Ok.").await;
    apply(
        s,
        &cid,
        &r,
        ext(json!({"scene":scene("jardim", json!(["Mira"]))})),
        "x",
    )
    .await;
    let (_, _, b) = build(s, &cid, "Segunda pergunta", "send").await;
    let b = b.unwrap();
    assert_eq!(
        a.messages[0]["content"], b.messages[0]["content"],
        "system idêntico"
    );
    let previous: Vec<&Value> = a.messages[1..a.messages.len() - 1].iter().collect();
    let next: Vec<&Value> = b.messages[1..=previous.len()].iter().collect();
    assert_eq!(
        next, previous,
        "o histórico anterior é prefixo do novo pedido"
    );
}

#[tokio::test]
async fn persona_profiles_reach_the_system_prompt() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(
        s,
        json!({"prompt":"Você é Mira. {{char}} nunca age por {{user}}."}),
    )
    .await;
    s.call(Request::SaveCast {
        conv_id: cid.clone(),
        value: json!({"name":"Alex","kind":"user","source":"manual","confirmed":true,"description":"30 anos, óculos redondos"}),
    })
    .await
    .unwrap();
    assert_eq!(s.call(Request::SeedPersonas).await.unwrap(), json!(1));
    let lucas = s
        .call(Request::SavePersona(
            json!({"name":"Lucas","description":"Mercenário de 30 anos, cicatriz no rosto."}),
        ))
        .await
        .unwrap();
    s.call(Request::PatchConversation {
        conv_id: cid.clone(),
        patch: json!({"persona":"Lucas","settings":{"personaId":lucas["id"]}}),
        base_character_rev: None,
    })
    .await
    .unwrap();
    s.call(Request::SyncUserCast {
        conv_id: cid.clone(),
        persona: lucas.clone(),
    })
    .await
    .unwrap();
    let (_, _, ctx) = build(s, &cid, "Oi", "send").await;
    let ctx = ctx.unwrap();
    let sys = system(&ctx);
    assert!(sys.contains("O usuário interpreta Lucas"));
    assert!(sys.contains("Mercenário de 30 anos"));
    assert!(
        sys.contains("o narrador e os personagens nunca age por Lucas"),
        "{{char}}/{{user}} substituídos"
    );
    assert!(!sys.contains("{{"));
}

#[tokio::test]
async fn global_settings_reach_the_system_prompt_and_the_anchor() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, json!({})).await;
    say(s, &cid, "Oi", "Mira: Oi.").await;
    // default: 8,000 units, 65/35, no content block
    let base = build(s, &cid, "E agora?", "send").await.2.unwrap();
    let sys = system(&base);
    assert!(sys.contains("Limite obrigatório: 8.000 unidades UTF-16"));
    assert!(sys.contains(
        "Priorize as falas dos personagens, respeitando o controle de turnos: aproximadamente 65% de diálogo e 35% de narração"
    ));
    assert!(!sys.contains("CONTEÚDO E EXPLICITUDE"));
    assert!(!sys.contains("{{"));
    assert!(
        !sys.contains("Priorize a conversa"),
        "padrão interativo não pede conversa longa"
    );
    assert!(sys.contains("Ações e falas podem se alternar"));
    // ajustes escolhidos
    s.call(Request::SetGlobalSettings(
        json!({"botChars":3000,"dialoguePct":30,"content":"Conteúdo +18: proibido."}),
    ))
    .await
    .unwrap();
    let custom = build(s, &cid, "E agora?", "send").await.2.unwrap();
    let sys = system(&custom);
    assert!(sys.contains("Limite obrigatório: 3.000 unidades UTF-16"));
    assert!(sys.contains(
        "Priorize a narração, com falas pontuais: aproximadamente 30% de diálogo e 70% de narração"
    ));
    assert!(sys.contains("CONTEÚDO E EXPLICITUDE (definido pelo usuário; vale para toda a história):\nConteúdo +18: proibido."));
    assert!(system(&custom).contains("30% de diálogo e 70% de narração"));
    // extremes: narration only / speech only, with no contradictory example
    s.call(Request::SetGlobalSettings(
        json!({"botChars":500,"dialoguePct":0,"content":""}),
    ))
    .await
    .unwrap();
    let narration = build(s, &cid, "E agora?", "send").await.2.unwrap();
    assert!(system(&narration).contains("Somente narração e ações"));
    assert!(system(&narration).contains("Use somente narração: aproximadamente 0%"));
    assert!(!system(&narration).contains("Ações e falas podem se alternar"));
    assert!(!last(&narration).contains("formato *ação* / Nome: fala"));
    assert!(last(&narration).contains("somente narração entre *asteriscos*"));
    assert!(!system(&narration).contains("Mira: O endereço está na carta."));
    assert!(system(&narration).contains("Limite obrigatório: 500 unidades UTF-16"));
    s.call(Request::SetGlobalSettings(json!({"dialoguePct":100})))
        .await
        .unwrap();
    let dialogue = build(s, &cid, "E agora?", "send").await.2.unwrap();
    assert!(system(&dialogue).contains("Somente falas diretas"));
    assert!(!system(&dialogue).contains("Narração e ações em terceira pessoa"));
    assert!(!system(&dialogue).contains("Ações e falas podem se alternar"));
    assert!(!last(&dialogue).contains("formato *ação* / Nome: fala"));
    assert!(last(&dialogue).contains("somente falas Nome: fala"));
    assert!(!system(&dialogue).contains("*Mira abre o envelope"));
}

#[test]
fn core_prompt_keeps_the_style_and_canon_rules() {
    let d = json!({"conv":{"persona":"Alex","prompt":"Lia estuda magia.","settings":{}},"pairs":[],"totalPairs":0,"current":{"userText":"Oi","seq":1,"priorParts":[]}});
    let c = context::assemble(
        &d,
        &json!({}),
        "send",
        &plan("cc/claude-sonnet-5-5", "claude-sonnet-5-5"),
        None,
    )
    .unwrap();
    let sys = system(&c);
    for phrase in [
        "ENTRADA CANÔNICA DE ALEX:",
        "JÁ ACONTECEU exatamente como foi escrito",
        "a terceira pessoa vale só para o que VOCÊ acrescenta",
        "a primeira linha da resposta deve ser algo NOVO",
        "Cada frase de narração deve mudar algo na cena",
        "Reaja primeiro ao que a mensagem atual acabou de trazer",
        "aproximadamente 65% de diálogo e 35% de narração",
        "Limite obrigatório: 8.000 unidades UTF-16",
        "quem pronunciou originalmente",
        "ROLE=user identifica a fonte",
        "sentar, levantar e sentar novamente são transições legítimas",
    ] {
        assert!(sys.contains(phrase), "faltou: {phrase}");
    }
    // the format examples (one per ratio band) do not teach filler gestures
    let examples: Value = serde_json::from_str(include_str!("../src/style-examples.json")).unwrap();
    for example in examples.as_array().unwrap() {
        let text = example["example"].as_str().unwrap().to_lowercase();
        for filler in [
            "arruma o cabelo",
            "cruza os braços",
            "suspira",
            "olha de relance",
        ] {
            assert!(
                !text.contains(filler),
                "gesto de enchimento no exemplo: {filler}"
            );
        }
    }
    let modelled = examples
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["max"].as_i64().unwrap() >= 65)
        .unwrap();
    assert!(
        sys.contains(modelled["example"].as_str().unwrap()),
        "o exemplo da faixa 65% está no prompt"
    );
}

#[tokio::test]
async fn long_simulation_keeps_sources_old_topic_return_and_full_episode() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, json!({})).await;
    let mut first = Value::Null;
    for i in 1..=720 {
        let r = say(
            s,
            &cid,
            &format!("Assunto {i}"),
            &format!("Lia: Estamos no local {}.", i / 40),
        )
        .await;
        let facts = if i == 1 {
            first = r["interactionId"].clone();
            json!([{"kind":"PROMISE","subject":"Lia","content":"A promessa do farol é voltar na lua azul.","importance":0.9,"confidence":1,"visibility":"private","knowledge":[]}])
        } else {
            json!([])
        };
        apply(
            s,
            &cid,
            &r,
            ext(json!({"summary":"Turno processado.","facts":facts})),
            "",
        )
        .await;
    }
    let detail = format!(
        "{}A promessa do farol exige a chave de ametista.",
        "Introdução. ".repeat(60)
    );
    let episode = s
        .call(Request::CreateEpisode(json!({
            "convId":cid,"interactionIds":[first],"fromSeq":1,"toSeq":1,"summary":detail
        })))
        .await
        .unwrap();
    let (data, _, ctx) = build(s, &cid, "Vamos retomar a promessa do farol.", "send").await;
    let ctx = ctx.unwrap();
    let live = last(&ctx);
    assert!(live.contains("lua azul"));
    assert!(live.contains("chave de ametista"));
    assert!(
        ctx.report["episodesIncluded"]
            .as_array()
            .unwrap()
            .contains(&episode["id"])
    );
    assert_eq!(ctx.report["pairsIncluded"], 30);
    assert_eq!(data["sync"]["lag"], 0);
    assert_eq!(data["totalPairs"], 720);
}

#[tokio::test]
async fn old_unprocessed_turn_is_preserved_and_old_edit_gets_no_future_scene() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, json!({})).await;
    let a = say(s, &cid, "Oi", "Lia: SENHA_PRIMEIRO_TURNO").await;
    for i in 0..55 {
        say(s, &cid, "Dia seguinte", &format!("Lia: Dia {i}.")).await;
    }
    let (_, _, ctx) = build(s, &cid, "Oi", "send").await;
    let ctx = ctx.unwrap();
    assert!(ctx.messages.iter().any(|m| {
        m["content"]
            .as_str()
            .unwrap()
            .contains("SENHA_PRIMEIRO_TURNO")
    }));
    assert_eq!(ctx.report["pendingTurnsPreserved"], 26);
    let b = say(s, &cid, "Mudamos de país", "Lia: Paris.").await;
    apply(
        s,
        &cid,
        &b,
        ext(json!({"scene":scene("Paris", json!(["Lia"]))})),
        "",
    )
    .await;
    let old = s
        .call(Request::Context(
            json!({"convId":cid,"interactionId":a["interactionId"],"overrideUserText":"Outro começo"}),
        ))
        .await
        .unwrap();
    assert!(old["scene"].is_null());
}

#[test]
fn heuristic_review_flags_user_speech_and_action_without_accusing_third_party_reactions() {
    let bad = review::heuristic(
        "*Mira sorri.*\n\nAlex: Tudo bem.\n\n*Alex decide ir embora e fecha a porta.*",
        "Alex",
    );
    assert_eq!(bad.iter().filter(|f| f["kind"] == "agency").count(), 2);
    assert!(
        review::heuristic(
            "*Mira olha para Alex com curiosidade.*\n\nMira: Alex, você está bem?",
            "Alex"
        )
        .is_empty()
    );
    // a vocative inside another character's speech is NOT an action by Alex
    assert!(
        review::heuristic(
            "*Nora se assusta.*\n\nNora: Ai! Alex, você aparece do nada, isso não é jeito de chegar.",
            "Alex"
        )
        .is_empty()
    );
    assert!(
        review::heuristic(
            "Nora: Eu *abraço Alex* e rio. Alex ri também.",
            "Alex"
        )
        .is_empty()
    );
    // an action of his narrated inside someone else's speech is detected
    assert_eq!(
        review::heuristic(
            "Nora: Ufa! *Alex se levanta e vai embora sem dizer nada.*",
            "Alex"
        )
        .len(),
        1
    );
}

fn mem(id: &str, content: &str, kind: &str, subject: &str, importance: f64) -> Value {
    json!({"id":id,"kind":kind,"subject":subject,"content":content,"importance":importance})
}

#[test]
fn curation_groups_only_near_duplicates_of_the_same_kind_and_subject() {
    let a = mem(
        "a",
        "Mira guarda uma carta secreta no bolso do casaco azul.",
        "OBJECT",
        "Mira",
        0.6,
    );
    let b = mem(
        "b",
        "Mira guarda uma carta secreta no bolso do casaco.",
        "OBJECT",
        "Mira",
        0.9,
    );
    let c = mem(
        "c",
        "Alex prometeu devolver o caderno ao vento na sexta-feira.",
        "EVENT",
        "Alex",
        0.6,
    );
    let groups = curation::build_clusters(&[a.clone(), c.clone(), b.clone()]);
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0]
            .iter()
            .map(|m| m["id"].clone())
            .collect::<Vec<_>>(),
        vec![json!("a"), json!("b")]
    );
    let same_text = "Mira guarda uma carta secreta no bolso do casaco.";
    assert!(
        curation::build_clusters(&[a.clone(), mem("x", same_text, "EVENT", "Mira", 0.6)])
            .is_empty(),
        "tipo diferente"
    );
    assert!(
        curation::build_clusters(&[a.clone(), mem("y", same_text, "OBJECT", "Nora", 0.6)])
            .is_empty(),
        "assunto diferente"
    );
    assert!(curation::build_clusters(&[a, c]).is_empty());
    let many: Vec<Value> = (0..40)
        .map(|i| {
            mem(
                &format!("m{i}"),
                &format!("Mira guarda uma carta secreta número {i} no bolso"),
                "OBJECT",
                "Mira",
                0.6,
            )
        })
        .collect();
    assert!(
        curation::build_clusters(&many).len() <= 6,
        "no máximo 6 grupos por chamada"
    );
}

#[test]
fn curation_sanitize_accepts_only_safe_merges() {
    let a = mem(
        "a",
        "Mira guarda uma carta secreta no bolso do casaco azul.",
        "OBJECT",
        "Mira",
        0.6,
    );
    let b = mem(
        "b",
        "Mira guarda uma carta secreta no bolso do casaco.",
        "OBJECT",
        "Mira",
        0.9,
    );
    let clusters = vec![vec![a.clone(), b.clone()]];
    let merged = "Mira guarda uma carta secreta no bolso do casaco azul.";
    let ok = curation::sanitize(
        &json!({"decisions":[{"group":1,"action":"merge","keep_id":"b","merged":merged}]}),
        &clusters,
    );
    assert_eq!(
        ok,
        vec![json!({"keepId":"b","dropIds":["a"],"merged":merged})]
    );
    let bad = |d: Value| curation::sanitize(&json!({"decisions":[d]}), &clusters);
    assert!(
        bad(json!({"group":1,"action":"keep_all"})).is_empty(),
        "keep_all não muda nada"
    );
    assert!(bad(json!({"group":2,"action":"merge","keep_id":"a","merged":"Mira guarda uma carta secreta."})).is_empty(), "grupo inexistente");
    assert!(bad(json!({"group":1,"action":"merge","keep_id":"zzz","merged":"Mira guarda uma carta secreta no bolso."})).is_empty(), "id fora do grupo");
    assert!(bad(json!({"group":1,"action":"merge","keep_id":"a","merged":"Mira contou a carta secreta a Laura, que a levou até a diretora Helena, e brigaram feio."})).is_empty(), "texto inventado");
    assert!(
        bad(json!({"group":1,"action":"merge","keep_id":"a","merged":"carta"})).is_empty(),
        "curto demais"
    );
    assert!(
        bad(json!({"group":1,"action":"merge","keep_id":"a","merged":"x".repeat(401)})).is_empty(),
        "longo demais"
    );
    assert!(curation::sanitize(&json!("não é json"), &clusters).is_empty());
    assert!(curation::sanitize(&json!({"decisions":"x"}), &clusters).is_empty());
    let twice = curation::sanitize(
        &json!({"decisions":[
            {"group":1,"action":"merge","keep_id":"a","merged":a["content"]},
            {"group":1,"action":"merge","keep_id":"b","merged":b["content"]}]}),
        &clusters,
    );
    assert_eq!(twice.len(), 1, "um grupo só é decidido uma vez");
    let msgs = curation::messages(&clusters);
    assert!(msgs[1]["content"].as_str().unwrap().contains("GRUPO 1:"));
    assert!(
        msgs[0]["content"]
            .as_str()
            .unwrap()
            .contains("Na dúvida, keep_all")
    );
}

#[test]
fn style_policy_measures_size_ratio_and_language() {
    let good = format!(
        "Mira: {}",
        "Vamos examinar o endereço da carta antes de sair. ".repeat(9)
    );
    assert_eq!(style_policy::assess(&good, 500, 100.0)["ok"], true);
    assert_eq!(
        style_policy::assess("*Ela abre a carta.*", 500, 0.0)["ratioOk"],
        true
    );
    assert_eq!(
        style_policy::assess("Mira: **Olá** *Ela abre a carta.*", 500, 100.0)["ratioOk"],
        false
    );
    assert_eq!(
        style_policy::assess(
            "Mira: You said that she was there with the letter.",
            500,
            100.0
        )["languageOk"],
        false
    );
    assert_eq!(
        style_policy::assess(&format!("Mira: {}", "中文".repeat(15)), 500, 100.0)["languageOk"],
        false
    );
    assert_eq!(
        style_policy::assess("Mira: Oi.", 500, 100.0)["sizeOk"],
        false
    );
    assert_eq!(
        style_policy::assess(&format!("{good}{}", "x".repeat(100)), 500, 100.0)["sizeOk"],
        false
    );
    // the review instruction mentions the band and the language; the score favors the improvement
    let before = style_policy::assess("*Ela abre uma carta.*", 500, 100.0);
    let text = style_policy::instruction(&before, 100.0);
    assert!(text.contains("português do Brasil") && text.contains("425") && text.contains("500"));
    let after = style_policy::assess(&good, 500, 100.0);
    assert!(style_policy::score(&after, 500, 100.0) < style_policy::score(&before, 500, 100.0));
}

// --- Token and record savings ---------------------------------------------------------

#[tokio::test]
async fn context_data_sends_each_line_once_and_sources_only_for_the_last_three() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, json!({})).await;
    for i in 1..=8 {
        say(
            s,
            &cid,
            &format!("Pergunta única {i}"),
            &format!("Laura: Resposta única {i}."),
        )
        .await;
    }
    let data = s
        .call(Request::Context(
            json!({"convId":cid,"pendingUserText":"E agora?","maxPairs":50}),
        ))
        .await
        .unwrap();
    let pairs = data["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 8);
    for (at, pair) in pairs.iter().enumerate() {
        assert!(pair["parts"].is_null(), "parts repetia assistant");
        assert_eq!(pair["user"], format!("Pergunta única {}", at + 1));
        assert_eq!(
            pair["assistant"],
            format!("Laura: Resposta única {}.", at + 1)
        );
        assert_eq!(
            !pair["sources"].is_null(),
            at >= 5,
            "sources só nos 3 últimos: {at}"
        );
        let text = pair.to_string();
        let copies = text.matches(&format!("Resposta única {}.", at + 1)).count();
        assert_eq!(copies, if at >= 5 { 2 } else { 1 }, "cópias no par {at}");
    }
    // quote reading and the extraction job still receive the sources of the last 3 pairs
    let sources = modularchat_server::provenance::sources(pairs);
    assert_eq!(sources.len(), 6);
}

#[tokio::test]
async fn prompt_is_identical_when_only_the_needed_pairs_are_loaded() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, json!({})).await;
    for i in 1..=40 {
        let t = say(
            s,
            &cid,
            &format!("Pergunta {i}"),
            &format!("Laura: Resposta {i}."),
        )
        .await;
        apply(
            s,
            &cid,
            &t,
            ext(json!({"summary":format!("nota {i}")})),
            "fonte",
        )
        .await;
    }
    let conv_meta = s.call(Request::GetMeta(cid.clone())).await.unwrap();
    let needed = context::pairs_to_load(&conv_meta);
    assert_eq!(needed, 30);
    let mut texts = Vec::new();
    let mut sent = Vec::new();
    for max_pairs in [50, needed] {
        let data = s
            .call(Request::Context(
                json!({"convId":cid,"pendingUserText":"E agora?","maxPairs":max_pairs}),
            ))
            .await
            .unwrap();
        let ctx = context::assemble(
            &data,
            &json!({}),
            "send",
            &plan("cc/claude-sonnet-5-5", "claude-sonnet-5-5"),
            None,
        )
        .unwrap();
        sent.push((
            data["pairs"].as_array().unwrap().len(),
            serde_json::to_string(&data).unwrap().len(),
        ));
        texts.push(serde_json::to_string(&ctx.messages).unwrap());
    }
    assert_eq!(texts[0], texts[1], "o prompt não pode mudar");
    assert!(sent[0].0 == 40 && sent[1].0 == 30, "{sent:?}");
    assert!(sent[1].1 < sent[0].1, "{sent:?}");
    // the timeline brings the notes before the window (turns 1–10) and none after it
    assert!(texts[0].contains("- nota 10"));
    assert!(!texts[0].contains("- nota 11"));
}

#[tokio::test]
async fn notes_outside_every_window_or_covered_by_an_episode_are_not_sent() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, json!({})).await;
    let mut ids = Vec::new();
    for i in 1..=40 {
        let t = say(
            s,
            &cid,
            &format!("Pergunta {i}"),
            &format!("Laura: Resposta {i}."),
        )
        .await;
        apply(
            s,
            &cid,
            &t,
            ext(json!({"summary":format!("nota {i}")})),
            "fonte",
        )
        .await;
        ids.push(t["interactionId"].clone());
    }
    let ask = json!({"convId":cid,"pendingUserText":"E agora?","maxPairs":30});
    let seqs_of = |data: &Value| {
        data["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["seq"].as_i64().unwrap())
            .collect::<Vec<_>>()
    };
    let data = s.call(Request::Context(ask.clone())).await.unwrap();
    // ladder 30/28/25/20: the narrowest window (20 pairs) starts at turn 21
    assert_eq!(seqs_of(&data), (1..=20).collect::<Vec<_>>());
    assert_eq!(data["sync"]["lastSyncedSeq"], 40);
    assert_eq!(data["sync"]["synced"], true);
    // an episode covering turns 1–5 (before the window of the preferred step, which starts at turn 11)
    s.call(Request::CreateEpisode(json!({
        "convId":cid,"interactionIds":ids[..5],"fromSeq":1,"toSeq":5,"summary":"episódio inicial",
        "expectedNotes":[]
    })))
    .await
    .unwrap();
    let data = s.call(Request::Context(ask)).await.unwrap();
    assert_eq!(seqs_of(&data), (6..=20).collect::<Vec<_>>());
    assert_eq!(data["episodes"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn search_finds_long_messages_once_through_their_chunks() {
    let f = fixture();
    let s = &f.store;
    let cid = conv(s, json!({})).await;
    let long = format!(
        "Laura: {} palavra-rara-alfa {} palavra-rara-alfa {}",
        "enchimento ".repeat(150),
        "enchimento ".repeat(150),
        "enchimento ".repeat(150)
    );
    say(s, &cid, "Pergunta curta sobre o jardim", &long).await;
    let found = s
        .call(Request::Search {
            query: "palavra-rara-alfa".into(),
            conv_id: None,
            limit: 20,
        })
        .await
        .unwrap();
    let found = found.as_array().unwrap();
    assert_eq!(
        found.len(),
        1,
        "a mensagem longa aparece uma vez: {found:?}"
    );
    assert_eq!(found[0]["role"], "assistant");
    assert!(found[0]["snippet"].as_str().unwrap().contains('«'));
    let user = s
        .call(Request::Search {
            query: "jardim".into(),
            conv_id: Some(cid.clone()),
            limit: 20,
        })
        .await
        .unwrap();
    assert_eq!(user.as_array().unwrap()[0]["role"], "user");
}

/// Opens and closes the store on `dir` and waits for the SQLite thread to finish writing.
fn reopen(dir: &std::path::Path) {
    drop(Store::open(dir).unwrap());
    std::thread::sleep(std::time::Duration::from_millis(400));
}
fn sql_count(dir: &std::path::Path, sql: &str) -> i64 {
    rusqlite::Connection::open(dir.join("modularchat.db"))
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

#[tokio::test]
async fn old_full_text_index_rows_are_migrated_away_without_losing_search() {
    let dir = tempfile::tempdir().unwrap();
    let cid;
    {
        let store = Store::open(dir.path()).unwrap();
        cid = conv(&store, json!({})).await;
        say(
            &store,
            &cid,
            "Vamos à catedral antiga",
            "Laura: A catedral está em ruínas.",
        )
        .await;
    }
    std::thread::sleep(std::time::Duration::from_millis(400));
    let full = "SELECT count(*) FROM search_fts WHERE kind IN ('user','asst')";
    assert_eq!(sql_count(dir.path(), full), 0, "índice novo: só trechos");
    // reproduces the old database: one full-text row per message and the migration not done yet
    rusqlite::Connection::open(dir.path().join("modularchat.db"))
        .unwrap()
        .execute_batch(
            "INSERT INTO search_fts(text,conv_id,kind,owner_id) SELECT text,conv_id,kind,owner_id FROM text_chunks;
             DELETE FROM meta WHERE key='fts_chunks_only_v1';",
        )
        .unwrap();
    assert_eq!(sql_count(dir.path(), full), 2);
    reopen(dir.path());
    assert_eq!(sql_count(dir.path(), full), 0);
    assert_eq!(
        sql_count(
            dir.path(),
            "SELECT count(*) FROM search_fts WHERE kind='chunk'"
        ),
        2
    );
    let store = Store::open(dir.path()).unwrap();
    let found = store
        .call(Request::Search {
            query: "catedral ruinas".into(),
            conv_id: Some(cid),
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(found.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn automatic_prune_trims_only_operational_records_on_open() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        let cid = conv(&store, json!({})).await;
        say(&store, &cid, "Oi", "Laura: Oi.").await;
    }
    std::thread::sleep(std::time::Duration::from_millis(400));
    let generations = sql_count(dir.path(), "SELECT count(*) FROM generations");
    let versions = sql_count(dir.path(), "SELECT count(*) FROM versions");
    assert!(
        sql_count(
            dir.path(),
            "SELECT count(*) FROM generations WHERE context_json IS NOT NULL OR request_json IS NOT NULL"
        ) >= 1
    );
    // recent records survive opening
    reopen(dir.path());
    assert!(
        sql_count(
            dir.path(),
            "SELECT count(*) FROM generations WHERE request_json IS NOT NULL"
        ) >= 1
    );
    // aged beyond the 14-day retention: only operational records go
    rusqlite::Connection::open(dir.path().join("modularchat.db"))
        .unwrap()
        .execute_batch(
            "UPDATE generations SET started_at='2020-01-01T00:00:00.000Z';
             INSERT OR IGNORE INTO operations(conv_id,op_id,kind,result_json,created_at) VALUES('c','o','edit','{}','2020-01-01T00:00:00.000Z');
             UPDATE jobs SET status='done',updated_at='2020-01-01T00:00:00.000Z';",
        )
        .unwrap();
    reopen(dir.path());
    assert_eq!(
        sql_count(
            dir.path(),
            "SELECT count(*) FROM generations WHERE context_json IS NOT NULL OR request_json IS NOT NULL"
        ),
        0
    );
    assert_eq!(
        sql_count(dir.path(), "SELECT count(*) FROM generations"),
        generations
    );
    assert_eq!(
        sql_count(
            dir.path(),
            "SELECT count(*) FROM operations WHERE created_at<'2021'"
        ),
        0
    );
    assert_eq!(
        sql_count(dir.path(), "SELECT count(*) FROM jobs WHERE status='done'"),
        0
    );
    assert_eq!(
        sql_count(dir.path(), "SELECT count(*) FROM versions"),
        versions
    );
    // idempotente
    reopen(dir.path());
    assert_eq!(
        sql_count(dir.path(), "SELECT count(*) FROM versions"),
        versions
    );
}
