use axum::{body::Body, extract::ConnectInfo, http::Request as HttpRequest};
use modularchat_server::{
    api::{self, AppState},
    auth,
    config::Config,
    context, narrative, presets, provenance, review, rules,
    store::{FinishRequest, Request, Store, TurnRequest},
};
use modularchat_server::{cast, jobs};
use serde_json::{Value, json};
use tower::ServiceExt;

fn timeline(parts: Vec<(&str, &str)>) -> Value {
    narrative::derive(
        "Há 3 meses chegaram.",
        &[
            json!({"id":"i","seq":1,"parts":parts.iter().enumerate().map(|(idx,(role,text))|json!({"id":format!("s{idx}"),"role":role,"text":text})).collect::<Vec<_>>()}),
        ],
    )
}
#[test]
fn empty_timeline() {
    let v = narrative::derive("3 meses atrás chegaram.", &[]);
    assert!(v["current"].is_null());
    assert_eq!(v["anchors"][0]["months"], 3);
}
#[test]
fn timeline_eight_days() {
    assert_eq!(
        timeline(vec![
            ("user", "Dia seguinte."),
            ("assistant", "Sete dias depois.")
        ])["current"]["days"],
        8
    );
}
#[test]
fn timeline_month_anchors() {
    for card in ["Há 3 meses chegaram.", "Chegaram 3 meses atrás."] {
        let v = narrative::derive(
            card,
            &[json!({"id":"i","parts":[{"id":"u","role":"user","text":"7 meses depois."}]})],
        );
        assert_eq!(v["current"]["anchors"][0]["elapsedMonths"], 10);
        assert_eq!(v["current"]["days"], 0);
    }
}
#[test]
fn chat_month_anchors_count_from_the_moment_they_were_said() {
    let sources = |parts: Vec<(&str, &str)>| {
        vec![
            json!({"id":"i","seq":1,"parts":parts.iter().enumerate().map(|(idx,(role,text))|json!({"id":format!("s{idx}"),"role":role,"text":text})).collect::<Vec<_>>()}),
        ]
    };
    // said before the time skip: 3 months + 7 months later = 10
    let v = narrative::derive(
        "Lia mora na vila.",
        &sources(vec![
            ("user", "Há 3 meses ela chegou."),
            ("user", "Sete meses depois."),
        ]),
    );
    assert_eq!(v["current"]["anchors"][0]["elapsedMonths"], 10);
    // said after the time skip: counts only from there
    let v = narrative::derive(
        "Lia mora na vila.",
        &sources(vec![("user", "Sete meses depois. Há 3 meses ela chegou.")]),
    );
    assert_eq!(v["current"]["months"], 7);
    assert_eq!(v["current"]["anchors"][0]["elapsedMonths"], 3);
    // character and narrator (assistant) speech create no anchor; the sheet keeps the old formula
    let v = narrative::derive(
        "Há 2 meses chegaram.",
        &sources(vec![
            ("user", "Lia: Há 5 meses eu cheguei."),
            ("assistant", "Há 4 meses tudo começou."),
            ("user", "Um mês depois."),
        ]),
    );
    let anchors = v["current"]["anchors"].as_array().unwrap();
    assert_eq!(anchors.len(), 1);
    assert_eq!(anchors[0]["elapsedMonths"], 3);
    // the chat anchor survives incremental derivation (seed = previous snapshot)
    let first = narrative::derive_snapshots(
        "Lia mora na vila.",
        &sources(vec![("user", "Há 3 meses ela chegou.")]),
        None,
    );
    let next = narrative::derive_snapshots(
        "Lia mora na vila.",
        &[
            json!({"id":"j","seq":2,"parts":[{"id":"t0","role":"user","text":"Sete meses depois."}]}),
        ],
        first.last(),
    );
    assert_eq!(next[0]["anchors"][0]["elapsedMonths"], 10);
    assert_eq!(next[0]["anchors"].as_array().unwrap().len(), 1);
}
#[test]
fn card_absolute_date_is_the_initial_date_without_seed() {
    let sources =
        [json!({"id":"i","seq":1,"parts":[{"id":"u","role":"user","text":"Dois meses depois."}]})];
    let v = narrative::derive(
        "Nome: Lia
Data: 2024-01-31.",
        &sources,
    );
    assert_eq!(v["current"]["date"], "2024-03-31");
    assert_eq!(v["current"]["months"], 2);
    // without a "Data:" label the sheet's date (e.g. a birth date) does not start the timeline
    let v = narrative::derive("Em 12/05/1990 Lia nasceu.", &sources);
    assert!(v["current"]["date"].is_null());
    // with a seed, the seed's date wins
    let seeded = narrative::derive_snapshots(
        "Data: 2024-01-31.",
        &sources,
        Some(&json!({"days":0,"months":0,"date":"2020-02-10"})),
    );
    assert_eq!(seeded[0]["date"], "2020-04-10");
}
#[test]
fn distinct_equal_transitions() {
    let v = timeline(vec![("user", "Dia seguinte. Festa. Dia seguinte. Escola.")]);
    assert_eq!(v["current"]["days"], 2);
    assert_eq!(v["days"].as_array().unwrap().len(), 2);
}
#[test]
fn assistant_reiteration() {
    assert_eq!(
        timeline(vec![
            ("user", "Dia seguinte. Jardim."),
            ("assistant", "No dia seguinte, Lia rega flores.")
        ])["current"]["days"],
        1
    );
}
#[test]
fn continuation_next_day() {
    assert_eq!(
        timeline(vec![
            ("assistant", "Dia seguinte. Lia trabalha."),
            ("assistant", "Dia seguinte. Lia volta.")
        ])["current"]["days"],
        2
    );
}
#[test]
fn exact_continuation_repeat() {
    assert_eq!(
        timeline(vec![
            ("assistant", "Dia seguinte. Lia trabalha."),
            ("assistant", "Dia seguinte. Lia trabalha.")
        ])["current"]["days"],
        1
    );
}
#[test]
fn reported_past() {
    for text in [
        "Lia: Sete meses depois voltei.",
        "Ela lembra: há 3 meses.",
        "Flashback. Lia recorda o dia seguinte.",
        "“Dia seguinte.”",
    ] {
        assert!(narrative::evidence(text, false).is_empty());
    }
}
#[test]
fn leap_calendar() {
    assert_eq!(
        timeline(vec![("user", "Data: 28/02/2024. Dia seguinte.")])["current"]["date"],
        "2024-02-29"
    );
    assert!(narrative::evidence("Data: 31/02/2024.", false).is_empty());
}
#[test]
fn ambiguous_month_date() {
    let v = timeline(vec![("user", "Data: 31/01/2024. Um mês depois.")]);
    assert!(v["current"]["date"].is_null());
    assert_eq!(v["current"]["months"], 1);
}
#[test]
fn utf16_temporal_offsets() {
    let text = "✨🚀 Ela respira. Dia seguinte. Ela volta.";
    let v = timeline(vec![("user", text)]);
    let ev = &v["current"]["advances"][0];
    assert_eq!(
        rules::suffix(text, ev["start"].as_u64().unwrap() as usize)
            .unwrap()
            .get(
                ..rules::byte_at(
                    rules::suffix(text, ev["start"].as_u64().unwrap() as usize).unwrap(),
                    (ev["end"].as_u64().unwrap() - ev["start"].as_u64().unwrap()) as usize
                )
            )
            .unwrap(),
        ev["evidence"].as_str().unwrap()
    );
}
fn reviewed(old: &str, text: &str, between: &str) -> Vec<Value> {
    review::supported(
        &json!({"continuity":[{"quote":text,"previousQuote":old,"reason":"incompatível"}]}),
        text,
        &format!("{old}\n{between}"),
        &[],
    )
}
#[test]
fn legitimate_movement() {
    for (old, text, between) in [
        (
            "Lia estava no refeitório.",
            "Lia está no quarto.",
            "Lia entrou no quarto.",
        ),
        ("Lia estava deitada.", "Lia está sentada.", "Lia se sentou."),
        (
            "Lia estava sentada.",
            "Lia está sentada.",
            "Lia levantou. Lia sentou novamente.",
        ),
    ] {
        assert!(reviewed(old, text, between).is_empty());
    }
}
#[test]
fn other_subject_no_release() {
    assert_eq!(
        reviewed(
            "Lia estava amarrada sem poder mover-se.",
            "Lia está caminhando pelo jardim.",
            "Laura sentou na cadeira."
        )
        .len(),
        1
    );
}
#[test]
fn same_subject_release() {
    assert!(
        reviewed(
            "Lia estava amarrada.",
            "Lia está no jardim.",
            "Lia foi solta."
        )
        .is_empty()
    );
}
#[test]
fn simultaneous_contradiction() {
    assert_eq!(
        reviewed(
            "Lia está no quarto.",
            "Lia está no jardim e no quarto ao mesmo tempo.",
            ""
        )
        .len(),
        1
    );
}
#[test]
fn exact_multiline_evidence() {
    let q = "Alex decide.\n  Ele abre a porta.";
    let f = review::supported(
        &json!({"agency":[{"quote":q,"reason":"agência"}]}),
        q,
        "",
        &[],
    );
    assert_eq!(f[0]["evidence"], q);
}
#[test]
fn knowledge_evidence() {
    let secret = "Laura não sabe a cor da chave.";
    let q = "Laura: A chave é azul.";
    assert_eq!(
        review::supported(
            &json!({"knowledge":[{"quote":q,"previousQuote":secret,"reason":"segredo"}]}),
            q,
            "",
            &[secret.into()]
        )
        .len(),
        1
    );
}
#[test]
fn canonical_review_prompt() {
    let m = review::messages(
        &json!({"conv":{"persona":"Alex"},"userText":"Lia: Vamos ao quarto."}),
        "Laura: Sim.",
        "Lia entrou no quarto.",
        &[],
    );
    let t = m[1]["content"].as_str().unwrap();
    assert!(t.contains("Lia: Vamos ao quarto."));
    assert!(t.contains("Lia entrou no quarto."));
}
#[test]
fn quote_author_not_role() {
    let q = provenance::quotes(
        "Laura: \"A carta vermelha está no jardim.\"",
        &[json!({"id":"i","role":"user","text":"Lia: A carta vermelha está no jardim."})],
    );
    assert_eq!(q[0]["originalSpeaker"], "Lia");
    assert_eq!(q[0]["sourceRole"], "user");
}
#[test]
fn hidden_reasoning_split() {
    let mut f = rules::ThinkFilter::default();
    let result = ["<thi", "nk>segredo", "</th", "ink>✨🚀Visível"]
        .iter()
        .map(|s| f.push(s))
        .collect::<String>()
        + &f.flush();
    assert_eq!(result, "✨🚀Visível");
    assert!(f.hidden > 0);
}
#[test]
fn unicode_limit_and_offset() {
    let s = format!("{}🚀excesso", "x".repeat(7997));
    assert_eq!(rules::utf16_len(rules::clip(&s, 8000)), 8000);
    assert!(rules::suffix("a🚀b", 2).is_none());
    assert_eq!(rules::suffix("a🚀b", 3), Some("b"));
}
#[test]
fn emoji_chunk_progress() {
    let s = "🚀".repeat(100);
    let c = rules::chunks(&s, 2, 1);
    assert_eq!(c.len(), 100);
    assert!(c.iter().all(|(_, _, t)| t == "🚀"));
}
#[test]
fn vector_validation() {
    assert!(rules::vector_blob(&[f32::NAN]).is_err());
    assert!(rules::decode_vector(&[0, 0], 2).is_none());
    assert_eq!(rules::cosine(&[1., 0.], &[1., 0.]), 1.);
}
#[test]
fn presets_filter_historical() {
    assert!(
        presets::public(&["other/model".into()], &json!({}))
            .iter()
            .all(|p| p["id"] != "generic")
    );
    assert!(presets::resolve(&json!({"routeId":"other/model"}), None).is_err());
    assert!(
        presets::resolve(
            &json!({"routeId":"other/model"}),
            Some(&json!({"routeId":"other/model","presetId":"generic"}))
        )
        .is_ok()
    );
}
#[test]
fn new_presets_routes_thinking_and_unlisted_routes() {
    for (route, id) in [
        ("nvidia/moonshotai/kimi-k3", "kimi-k3"),
        ("oc/muse-spark-1.3-contributor-free", "muse-spark-13"),
        ("oc/step-5-preview-free", "step-5-preview"),
    ] {
        assert_eq!(presets::for_route(route)["id"], id);
    }
    // SpaceBunny was removed: neither the route nor the id forms a preset anymore
    assert_eq!(presets::for_route("oc/space-bunny-free")["id"], "generic");
    assert!(
        presets::public(&["oc/space-bunny-free".into()], &json!({}))
            .iter()
            .all(|p| p["id"] != "space-bunny" && p["family"] != "spacebunny")
    );
    assert!(
        presets::resolve(
            &json!({"presetId":"space-bunny","routeId":"oc/space-bunny-free"}),
            None
        )
        .is_err()
    );
    // shared 9Router aliases are not presets
    for alias in ["Kimi", "Muse", "SpaceBunny", "Step5-preview"] {
        assert_eq!(presets::for_route(alias)["id"], "generic");
    }
    // thinking turned on by the fixed effort
    for (preset, route) in [
        ("kimi-k3", "nvidia/moonshotai/kimi-k3"),
        ("muse-spark-13", "oc/muse-spark-1.3-contributor-free"),
        ("step-5-preview", "oc/step-5-preview-free"),
    ] {
        for asked in ["max", "none", "low"] {
            let p = presets::request(
                &json!({"presetId":preset,"routeId":route,"effort":asked,"temperature":0.7}),
                vec![],
                &json!({}),
            );
            assert_eq!(p.body["reasoning_effort"], "high");
            assert!(p.body.get("temperature").is_none());
            assert!(p.body["max_tokens"].as_u64().unwrap() >= 16384);
        }
    }
    // temperature only applies to "temperature" sampling presets (the generic one); null does not become 0
    let generic = presets::request(
        &json!({"presetId":"generic","routeId":"other/model","temperature":0.9}),
        vec![],
        &json!({}),
    );
    assert_eq!(generic.body["temperature"], 0.9);
    for k in ["reasoning_effort", "reasoning", "thinking", "output_config"] {
        assert!(generic.body.get(k).is_none(), "{k}");
    }
    let generic = presets::request(
        &json!({"presetId":"generic","routeId":"other/model","temperature":null}),
        vec![],
        &json!({}),
    );
    assert!(generic.body.get("temperature").is_none());
    // known direct routes appear even outside /models, only when the provider answered
    let listed = vec!["cbai/deepseek-v4.1-flash".to_string(), "Muse".to_string()];
    let pub_list = presets::public(&listed, &json!({}));
    let muse = pub_list
        .iter()
        .find(|p| p["id"] == "muse-spark-13")
        .unwrap();
    assert_eq!(muse["available"], true);
    assert_eq!(
        muse["routes"][0]["id"],
        "oc/muse-spark-1.3-contributor-free"
    );
    let kimi = pub_list.iter().find(|p| p["id"] == "kimi-k3").unwrap();
    assert_eq!(kimi["available"], false);
    assert!(
        presets::public(&[], &json!({}))
            .iter()
            .all(|p| p["available"].is_null())
    );
    assert!(
        presets::resolve(
            &json!({"routeId":"oc/step-5-preview-free","presetId":"step-5-preview"}),
            None
        )
        .is_ok()
    );
    assert!(
        presets::resolve(
            &json!({"routeId":"cbai/deepseek-v4.1-flash","presetId":"kimi-k3"}),
            None
        )
        .is_err()
    );
}
#[test]
fn gemini_has_routes_per_effort_level() {
    let preset = presets::by_id("gemini-38-flash");
    assert_eq!(preset["thinking"], "level");
    assert_eq!(
        preset["efforts"],
        json!(["low", "medium", "high"])
    );
    assert_eq!(preset["defaultEffort"], "low");
    assert_eq!(preset["effortByRoute"]["ag/gemini-3.8-flash-low"], "low");
    assert_eq!(preset["effortByRoute"]["ag/gemini-3.8-flash-medium"], "medium");
    assert_eq!(preset["effortByRoute"]["ag/gemini-3.8-flash-high"], "high");
    for (route, expected_effort) in [
        ("ag/gemini-3.8-flash-low", "low"),
        ("ag/gemini-3.8-flash-medium", "medium"),
        ("ag/gemini-3.8-flash-high", "high"),
    ] {
        assert_eq!(presets::for_route(route)["id"], "gemini-38-flash");
        let r = presets::resolve(
            &json!({"routeId":route,"presetId":"gemini-38-flash"}),
            None,
        )
        .unwrap();
        assert_eq!(r["routeId"], route);
        assert_eq!(r["effort"], expected_effort);

        let p = presets::request(
            &json!({"presetId":"gemini-38-flash","routeId":route}),
            vec![],
            &json!({}),
        );
        assert_eq!(p.body["model"], route);
        assert_eq!(p.controls["requestedEffort"], expected_effort);
    }
    // Route without a suffix falls back to the default route
    let unadorned = presets::resolve(
        &json!({"routeId":"ag/gemini-3.8-flash","presetId":"gemini-38-flash"}),
        None,
    )
    .unwrap();
    assert_eq!(unadorned["routeId"], "ag/gemini-3.8-flash-low");

    // Thinking margin in the budget
    let mut p_low = presets::request(
        &json!({"presetId":"gemini-38-flash","routeId":"ag/gemini-3.8-flash-low"}),
        vec![],
        &json!({}),
    );
    let length_low = presets::apply_length(&mut p_low, &json!({"botChars":600}));
    assert_eq!(length_low["reasoningReserve"], 2048);
    assert_eq!(p_low.body["max_tokens"], 240 + 2048);

    let mut p_med = presets::request(
        &json!({"presetId":"gemini-38-flash","routeId":"ag/gemini-3.8-flash-medium"}),
        vec![],
        &json!({}),
    );
    let length_med = presets::apply_length(&mut p_med, &json!({"botChars":600}));
    assert_eq!(length_med["reasoningReserve"], 4096);
    assert_eq!(p_med.body["max_tokens"], 240 + 4096);

    let mut p_high = presets::request(
        &json!({"presetId":"gemini-38-flash","routeId":"ag/gemini-3.8-flash-high"}),
        vec![],
        &json!({}),
    );
    let length_high = presets::apply_length(&mut p_high, &json!({"botChars":600}));
    assert_eq!(length_high["reasoningReserve"], 8192);
    assert_eq!(p_high.body["max_tokens"], 240 + 8192);

    // All three routes are offered when the provider lists them
    let listed = vec![
        "ag/gemini-3.8-flash-low".to_string(),
        "ag/gemini-3.8-flash-medium".to_string(),
        "ag/gemini-3.8-flash-high".to_string(),
    ];
    let public = presets::public(&listed, &json!({}));
    let gemini = public
        .iter()
        .find(|p| p["id"] == "gemini-38-flash")
        .unwrap();
    assert_eq!(gemini["routes"].as_array().unwrap().len(), 3);
}
#[tokio::test]
async fn retired_space_bunny_conversations_still_open_and_generate_with_the_default_preset() {
    let (route, preset, effort) =
        presets::migrate("space-bunny", "oc/space-bunny-free", &json!(null));
    assert_eq!(
        (route.as_str(), preset.as_str()),
        ("cbai/deepseek-v4.1-flash", "deepseek-v41-flash")
    );
    assert_eq!(effort, "high");
    // an old conversation stored in the database stays readable and generates with the current default preset; the record is not changed
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let c = store
        .call(Request::CreateConversation(json!({"name":"Antiga","persona":"Alex","routeId":"oc/space-bunny-free","presetId":"space-bunny","effort":null,"temperature":0.9,"prompt":"Lia é uma ladra."})))
        .await
        .unwrap();
    assert_eq!(c["presetId"], "deepseek-v41-flash");
    assert_eq!(c["routeId"], "cbai/deepseek-v4.1-flash");
    assert_eq!(c["effort"], "high");
    let cid = c["id"].as_str().unwrap();
    let read = store
        .call(Request::GetConversation(cid.into()))
        .await
        .unwrap();
    assert_eq!(read["presetId"], "deepseek-v41-flash");
    let ctx = store
        .call(Request::Context(
            json!({"convId":cid,"pendingUserText":"Oi"}),
        ))
        .await
        .unwrap();
    let plan = presets::request(&ctx["conv"], vec![], &json!({}));
    assert_eq!(plan.body["model"], "cbai/deepseek-v4.1-flash");
    assert_eq!(plan.body["reasoning_effort"], "high");
    // a SpaceBunny route under the generic preset also falls back to the default; current presets do not change
    assert_eq!(
        presets::migrate("generic", "oc/space-bunny-free", &json!(null)).1,
        "deepseek-v41-flash"
    );
    assert_eq!(
        presets::migrate("claude-sonnet-5-5", "cc/claude-sonnet-5-5", &json!("low")),
        (
            "cc/claude-sonnet-5-5".to_string(),
            "claude-sonnet-5-5".to_string(),
            json!("low")
        )
    );
    // Gemini keeps its route and effort
    let g = presets::migrate(
        "gemini-38-flash",
        "ag/gemini-3.8-flash-high",
        &json!("high"),
    );
    assert_eq!(g.0, "ag/gemini-3.8-flash-high");
    assert_eq!(g.2, "high");
    let unadorned = presets::migrate(
        "gemini-38-flash",
        "ag/gemini-3.8-flash",
        &json!(null),
    );
    assert_eq!(unadorned.0, "ag/gemini-3.8-flash-low");
}
#[test]
fn adapter_shapes() {
    for (shape, key) in [
        ("reasoning_effort", "reasoning_effort"),
        ("reasoning_object", "reasoning"),
        ("output_config", "output_config"),
        ("claude_adaptive", "thinking"),
        ("deepseek_thinking", "thinking"),
    ] {
        let p = presets::request(
            &json!({"presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5","effort":"max","temperature":1.9}),
            vec![],
            &json!({"routes":{"cc/claude-sonnet-5-5":{"shape":shape}}}),
        );
        assert!(!p.body[key].is_null());
        assert!(p.body.get("temperature").is_none());
        assert_eq!(p.controls["requestedEffort"], "medium");
    }
}
#[test]
fn token_estimate_same_contract() {
    assert_eq!(rules::estimate("Oi, Alex!"), 5);
    assert_eq!(rules::budget_raw(100, None), 120);
}
#[test]
fn private_addresses() {
    assert!(auth::private("192.168.0.1".parse().unwrap()));
    assert!(!auth::private("8.8.8.8".parse().unwrap()));
    assert!(auth::loopback("::ffff:127.0.0.1".parse().unwrap()));
}
#[tokio::test]
async fn lan_auth_before_large_body() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        data_dir: dir.path().into(),
        dist: dir.path().join("absent"),
        address: "127.0.0.1:0".parse().unwrap(),
        require_pairing: true,
        providers: None,
        max_generations: 1,
        production: false,
        native_lib_dir: None,
        review: true,
    };
    let state = AppState::new(config).await.unwrap();
    let req = HttpRequest::builder()
        .uri("/api/import")
        .method("POST")
        .extension(ConnectInfo(
            "192.168.0.9:1234".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .header("content-type", "application/json")
        .body(Body::from("x".repeat(2 * 1024 * 1024 + 1)))
        .unwrap();
    assert_eq!(
        api::router(state.clone())
            .oneshot(req)
            .await
            .unwrap()
            .status(),
        401
    );
    state.shutdown().await;
}
#[tokio::test]
async fn pairing_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let auth = auth::Auth::new(store.clone(), true);
    let code = auth.start().await["code"].as_str().unwrap().to_owned();
    let ip = "192.168.0.9".parse().unwrap();
    let paired = auth
        .pair(&json!({"code":code,"deviceName":"Fixture"}), ip)
        .await
        .unwrap();
    assert!(auth.pair(&json!({"code":code}), ip).await.is_err());
    let token = paired["token"].as_str().unwrap();
    assert_eq!(auth.verify(token).await.unwrap()["id"], paired["deviceId"]);
    store
        .call(Request::RevokeDevice(
            paired["deviceId"].as_str().unwrap().into(),
        ))
        .await
        .unwrap();
    assert!(auth.verify(token).await.unwrap().is_null());
}
#[tokio::test]
async fn pairing_bootstrap_ignores_revoked_token_but_protects_device_data() {
    use http_body_util::BodyExt;
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(Config {
        data_dir: dir.path().into(), dist: dir.path().join("absent"),
        address: "127.0.0.1:0".parse().unwrap(), require_pairing: true,
        providers: None, max_generations: 1, production: false,
        native_lib_dir: None, review: true,
    }).await.unwrap();
    let router = api::router(state.clone());
    let code = state.auth.start().await;
    let req = |method: &str, path: &str, token: &str, body: Value| {
        HttpRequest::builder().uri(path).method(method)
            .extension(ConnectInfo("192.168.0.9:1234".parse::<std::net::SocketAddr>().unwrap()))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(if body.is_null() { String::new() } else { body.to_string() })).unwrap()
    };
    let denied = router.clone().oneshot(req("GET", "/api/devices", "revoked", Value::Null)).await.unwrap();
    assert_eq!(denied.status(), 401);
    let paired = router.clone().oneshot(req("POST", "/api/pair", "revoked",
        json!({"code":code["code"],"deviceName":"Phone"}))).await.unwrap();
    assert_eq!(paired.status(), 201);
    let body: Value = serde_json::from_slice(&paired.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let token = body["token"].as_str().unwrap();
    assert_eq!(router.clone().oneshot(req("GET", "/api/devices", token, Value::Null)).await.unwrap().status(), 200);
    assert_eq!(router.oneshot(req("POST", "/api/pair", "revoked", json!({"code":code["code"]}))).await.unwrap().status(), 401);
    state.shutdown().await;
}
#[tokio::test]
async fn empty_first_turn_and_new_limit() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let c=store.call(Request::CreateConversation(json!({"name":"Fixture","persona":"Alex","routeId":"cbai/deepseek-v4.1-flash","presetId":"deepseek-v41-flash","effort":"high","prompt":"Há 3 meses chegaram."}))).await.unwrap();
    let cid = c["id"].as_str().unwrap();
    let data = store
        .call(Request::Context(
            json!({"convId":cid,"pendingUserText":"Oi"}),
        ))
        .await
        .unwrap();
    assert!(data["timeline"]["current"].is_null());
    let begin = store
        .call(Request::BeginTurn(
            serde_json::from_value::<TurnRequest>(
                json!({"convId":cid,"mode":"send","content":"Oi"}),
            )
            .unwrap(),
        ))
        .await
        .unwrap();
    let rejected=store.call(Request::FinishGeneration(serde_json::from_value::<FinishRequest>(json!({"generationId":begin["generationId"],"status":"complete","text":"x".repeat(10801)})).unwrap())).await.unwrap_err();
    assert_eq!(rejected.status, 422);
    let f=store.call(Request::FinishGeneration(serde_json::from_value::<FinishRequest>(json!({"generationId":begin["generationId"],"status":"length","text":"x".repeat(10800)})).unwrap())).await.unwrap();
    assert_eq!(f["status"], "length");
    assert_eq!(f["message"]["content"].as_str().unwrap().len(), 10800);
}
#[test]
fn mandatory_context_overflow() {
    let prompt = "word ".repeat(20000);
    for (hard, capacity) in [(1000, 128000), (128000, 1024)] {
        let data = json!({"conv":{"persona":"Alex","prompt":prompt,"settings":{"context":{"hardLimit":hard}}},"pairs":[],"current":{"userText":"Oi"}});
        let mut plan = presets::request(
            &json!({"presetId":"deepseek-v41-flash","routeId":"cbai/deepseek-v4.1-flash"}),
            vec![],
            &json!({}),
        );
        plan.capacity["contextWindow"] = json!(capacity);
        let built = context::assemble(&data, &json!({}), "send", &plan, None).unwrap();
        assert_eq!(built.raw_estimate, rules::estimate_messages(&built.messages));
        assert!(built.raw_estimate > 0);
        assert_eq!(built.report["exceededEstimatedBudget"], true);
        assert_eq!(built.report["overHardLimit"], hard == 1000);
        assert_eq!(built.report["overCapacity"], capacity == 1024);
        assert!(built.messages.iter().any(|m| m["content"].as_str().unwrap().contains(&prompt)));
        assert!(built.messages.last().unwrap()["content"].as_str().unwrap().contains("Oi"));
    }
}
#[tokio::test]
async fn bounded_store_queue() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_limits(dir.path(), 1, std::time::Duration::from_secs(30)).unwrap();
    let futures = (0..128).map(|_| store.call(Request::Status));
    let result = futures_util::future::join_all(futures).await;
    assert!(result.iter().any(|v| {
        v.as_ref()
            .is_err_and(|e| e.extra["code"] == "STORE_OVERLOADED")
    }));
    assert!(result.iter().any(std::result::Result::is_ok));
}

#[test]
fn nullable_continue_contract() {
    let req: TurnRequest =
        serde_json::from_value(json!({"convId":"fixture","mode":"continue","content":null}))
            .unwrap();
    assert_eq!(req.content, "");
    assert!(
        serde_json::from_value::<TurnRequest>(
            json!({"convId":"fixture","mode":"send","content":42})
        )
        .is_err()
    );
}
#[test]
fn last_subject_transition_wins() {
    let old = "Lia estava amarrada.";
    let current = "Lia está caminhando.";
    assert_eq!(
        reviewed(
            old,
            current,
            "Lia foi solta. Depois, Lia foi amarrada novamente. Laura foi solta."
        )
        .len(),
        1
    );
    assert!(
        reviewed(
            old,
            current,
            "Lia foi solta. Lia foi amarrada novamente. Lia foi solta."
        )
        .is_empty()
    );
}
#[test]
fn megabyte_chunks_cover_unicode_tail() {
    let text = format!(
        "{}\nCAUDA_🚀",
        "Texto ✨🚀 com emojis e quebras.\n".repeat(70000)
    );
    let chunks = rules::chunks(&text, 1200, 160);
    let mut covered = 0;
    let mut cursor = text.encode_utf16();
    let mut units = Vec::new();
    for (start, end, chunk) in &chunks {
        assert!(*start <= covered && *end > covered);
        assert!(*end - *start <= 1200);
        // Verify every new unit, including the final tail, without quadratic prefix reads.
        units.clear();
        units.extend(chunk.encode_utf16().skip(covered - *start));
        assert_eq!(
            units,
            cursor.by_ref().take(*end - covered).collect::<Vec<_>>()
        );
        covered = *end;
    }
    assert_eq!(covered, rules::utf16_len(&text));
    assert!(cursor.next().is_none());
    assert!(chunks.last().unwrap().2.ends_with("CAUDA_🚀"));
}

#[test]
fn extraction_sanitizes_names_scene_numbers_and_exact_evidence() {
    let q = "Lia ouviu.\n  Ela sabe.";
    let out=jobs::sanitize_extraction(&json!({"summary":" Resumo ","cast":[{"name":"ele"},{"name":"Lia"},{"name":"Mira e o diretor"},{"name":"Joana da Silva"}],"scene":{"location":"sala","time":"momento incerto","present":["Lia","ninguém","x"],"characters":[{"name":"Lia","position":"de pé","holding":["carta"]}]},"facts":[{"kind":"INVENTADO","content":"Lia guarda uma carta.","importance":5,"confidence":-1,"knowledge":[{"who":"Lia","state":"knows","via":"witnessed","evidence":q},{"who":"eu"}]}]})).unwrap();
    assert_eq!(
        out["cast"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].clone())
            .collect::<Vec<_>>(),
        vec![json!("Lia"), json!("Joana da Silva")]
    );
    assert_eq!(out["scene"]["present"], json!(["Lia"]));
    assert_eq!(out["scene"]["positions"]["Lia"], "de pé");
    assert!(out["scene"]["time"].is_null());
    assert_eq!(out["facts"][0]["kind"], "EVENT");
    assert_eq!(out["facts"][0]["importance"], 1.0);
    assert_eq!(out["facts"][0]["confidence"], 0.0);
    assert_eq!(out["facts"][0]["knowledge"].as_array().unwrap().len(), 1);
    assert_eq!(out["facts"][0]["knowledge"][0]["evidence"], q);
}
#[test]
fn structured_card_matches_world_contract() {
    let out = cast::from_card(
        include_str!("../../tests/fixtures/world-card.txt"),
        "Alex",
    );
    for name in [
        "Rose Ludwig",
        "Frederic Molotov",
        "Yulia Pavlova",
        "Barbara Król",
        "Joanna Pawlak",
        "Łukasz Grabowski",
        "Paweł Ostrowski",
        "Michał Lewandowski",
        "Humi Visconti",
        "Nyo García",
        "Vivian Voltaire",
        "Amanda Linspector",
        "Caio Ymir",
        "Iuno Volkov",
        "John Kennedy",
        "Elizabeth Wójcik",
        "Anna Nowak",
        "Natsuki Souza",
        "Vanessa Varga",
        "Miguel Nascimento",
        "Lili Varga",
    ] {
        assert!(out.iter().any(|c| c["name"] == name), "missing {name}");
    }
    for bad in [
        "Aptidão",
        "Celestial",
        "Comportamento",
        "Dados Gerais",
        "Perfil",
        "Personalidade",
        "Origem",
        "Rank S",
        "Raças",
        "Corpo Docente",
        "Personagens Externos",
        "Alex",
    ] {
        assert!(!out.iter().any(|c| c["name"] == bad), "false name {bad}");
    }
    let get = |name| out.iter().find(|c| c["name"] == name).unwrap();
    assert_eq!(get("Elizabeth Wójcik")["kind"], "main");
    assert_eq!(get("Rose Ludwig")["kind"], "npc");
    assert!(
        get("Frederic Molotov")["aliases"]
            .as_array()
            .unwrap()
            .contains(&json!("Prof. Molotov"))
    );
    // a surname shared by two people (Vanessa and Lili Varga) is never an alias
    assert!(
        !get("Vanessa Varga")["aliases"]
            .as_array()
            .unwrap()
            .contains(&json!("Varga"))
    );
    assert!(
        get("Lili Varga")["description"]
            .as_str()
            .unwrap()
            .contains("Prima materna de Vanessa Varga")
    );
}
#[test]
fn free_card_and_compound_names() {
    let out = cast::from_card(
        "Nome: Mira Rose\nIdade: 19\nNora: melhor amiga de Mira, divertida e leal.\nNora sempre defende Mira.\nVocê é Mira, estudante da Academia.",
        "Alex",
    );
    assert!(out.iter().any(|c| c["name"] == "Mira Rose"));
    assert!(out.iter().any(|c| c["name"] == "Nora"));
    assert!(cast::name("Dona Maria").is_some());
    assert!(cast::name("Joana da Silva").is_some());
    assert!(cast::name("Mira e o diretor").is_none());
}
#[test]
fn adapters_preserve_defaults_sampling_and_sonnet_low_option() {
    for (preset, route, effort, max) in [
        (
            "deepseek-v41-flash",
            "cbai/deepseek-v4.1-flash",
            "high",
            16384,
        ),
        ("claude-sonnet-5-5", "cc/claude-sonnet-5-5", "medium", 32768),
        ("claude-opus-5-5", "cc/claude-opus-5-5", "low", 32768),
    ] {
        for asked in ["max", "high", "low", "medium", "xhigh"] {
            let p = presets::request(
                &json!({"presetId":preset,"routeId":route,"effort":asked,"temperature":0.8}),
                vec![json!({"role":"user","content":"Oi"})],
                &json!({}),
            );
            let expected = if preset == "claude-sonnet-5-5" && asked == "low" {
                "low"
            } else {
                effort
            };
            assert_eq!(p.body["reasoning_effort"], expected);
            assert_eq!(p.body["max_tokens"], max);
            assert!(p.body.get("temperature").is_none());
            assert_eq!(p.controls["confirmation"], "not_validated");
        }
        let defaults = presets::request(
            &json!({"presetId":preset,"routeId":route}),
            vec![],
            &json!({}),
        );
        assert_eq!(defaults.body["reasoning_effort"], effort);
    }
    // Gemini: effort matching the route
    let g = presets::request(
        &json!({"presetId":"gemini-38-flash","routeId":"ag/gemini-3.8-flash-low","effort":"high","temperature":0.8}),
        vec![json!({"role":"user","content":"Oi"})],
        &json!({}),
    );
    assert_eq!(g.body["model"], "ag/gemini-3.8-flash-low");
    assert_eq!(g.body["reasoning_effort"], "low");
    assert!(g.body.get("temperature").is_none());
    assert_eq!(g.body["max_tokens"], 16384);
    assert_eq!(g.controls["requestedEffort"], "low");
    let p = presets::request(
        &json!({"presetId":"generic","routeId":"legacy/model","temperature":0.7}),
        vec![],
        &json!({}),
    );
    assert_eq!(p.body["temperature"], 0.7);
    assert!(p.body.get("reasoning_effort").is_none());
    assert_eq!(p.controls["confirmation"], "not_applicable");
}
#[test]
fn calibrated_tokens_and_length_table() {
    assert_eq!(rules::budget_raw(100, Some(1.3)), 156);
    assert_eq!(rules::length_hint("Oi.", 0, 5000)["visibleTokens"], 650);
    assert!(rules::length_hint(&format!("Eu grito com raiva e revelo o segredo! Depois corro e ataco com a espada? O que você faz? {}","texto ".repeat(300)),3,5000)["score"].as_u64().unwrap()>=5);
}

#[test]
fn extraction_drops_inverted_exact_attribution() {
    let input = json!({"recentSources":[{"id":"s1","role":"assistant","text":"Lia: Você sempre sabe o que dizer."}]});
    let raw = json!({"summary":"s","facts":[{"kind":"EVENT","content":"Laura afirmou que ele sabe o que dizer.","attribution":{"quote":"Você sempre sabe o que dizer.","originalSpeaker":"Laura"}},{"kind":"EVENT","content":"Lia afirmou que ele sabe o que dizer.","attribution":{"quote":"Você sempre sabe o que dizer.","originalSpeaker":"Lia"}}]});
    let out = jobs::sanitize_with_sources(&raw, &input).unwrap();
    assert_eq!(out["facts"].as_array().unwrap().len(), 1);
    assert_eq!(out["facts"][0]["attribution"]["originalSpeaker"], "Lia");
}
#[test]
fn router_reader_is_readonly_and_incompatible_stays_unverified() {
    use modularchat_server::router_log;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("router.db");
    assert_eq!(
        router_log::find(&file, "2026-01-01", "Oi", "claude")["verification"],
        "unverified"
    );
    assert!(!file.exists());
    let db = rusqlite::Connection::open(&file).unwrap();
    db.execute_batch(
        "CREATE TABLE requestDetails(timestamp TEXT,provider TEXT,model TEXT,data TEXT)",
    )
    .unwrap();
    db.execute("INSERT INTO requestDetails VALUES(?,?,?,?)",rusqlite::params!["2026-01-01T00:00:00.000Z","fixture","model",json!({"request":{"messages":[{"role":"user","content":"Oi"}]},"providerRequest":{"output_config":{"effort":"max"}},"tokens":{"prompt_tokens":952}}).to_string()]).unwrap();
    drop(db);
    let before = std::fs::read(&file).unwrap();
    let r = router_log::find(&file, "2026-01-01", "Oi", "claude");
    assert_eq!(r["effort"], "max");
    assert_eq!(r["usage"]["input"], 952);
    assert_eq!(std::fs::read(&file).unwrap(), before);
    let db = rusqlite::Connection::open(&file).unwrap();
    db.execute_batch("DROP TABLE requestDetails").unwrap();
    drop(db);
    assert_eq!(
        router_log::find(&file, "2026-01-01", "Oi", "claude")["verification"],
        "unverified"
    );
}
