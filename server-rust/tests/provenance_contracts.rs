use axum::{Json, Router, extract::State, routing::post};
use modularchat_server::{
    api::AppState,
    config::Config,
    context, jobs, presets, provenance, review, s,
    store::{FinishRequest, Request, Store, TurnRequest},
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../tests/fixtures/quote-attribution.json")).unwrap()
}
fn original() -> Value {
    json!({"id":"v-original","role":"assistant","text":fixture()["messages"][0]["content"]})
}
fn input() -> Value {
    json!({"conv":{"persona":"Alex"},"userText":fixture()["messages"][1]["content"],"recentSources":[original()],"previous":fixture()["messages"][0]["content"]})
}
fn empty_review() -> Value {
    json!({"agency":[],"knowledge":[],"continuity":[],"attribution":[]})
}
fn flags(raw: &Value, input: &Value, text: &str) -> Vec<Value> {
    review::supported_with_sources(
        raw,
        text,
        input["previous"].as_str().unwrap_or_default(),
        &[],
        input,
    )
}

#[test]
fn mira_exact_utf8_quote_preserves_original_and_separate_comments() {
    let f = fixture();
    assert_eq!(f["messages"].as_array().unwrap().len(), 3);
    let q = provenance::quotes(f["messages"][1]["content"].as_str().unwrap(), &[original()]);
    assert_eq!(q.len(), 1);
    assert_eq!(q[0]["sourceId"], "v-original");
    assert_eq!(q[0]["originalSpeaker"], "Mira");
    assert!(q[0]["citingSpeaker"].is_null() && q[0]["referent"].is_null());
    assert_eq!(q[0]["use"], "citation_or_imitation");
    assert_eq!(q[0]["quote"], f["quote"]);
    assert_eq!(q[0]["originalEvidence"], f["messages"][0]["content"]);
    assert!(!q[0]["quote"].as_str().unwrap().contains("Engraçadinha"));
    assert!(q[0]["citingEvidence"].as_str().unwrap().contains("Engraçadinha"));
}

#[test]
fn quote_on_newline_after_lia_has_no_citing_speaker() {
    let f = fixture();
    for newline in ["\n", "\r\n"] {
        let text = format!(
            "Lia: Observação anterior.{newline}\"{}\" [imitando a voz dela]",
            f["quote"].as_str().unwrap()
        );
        let q = provenance::quotes(&text, &[original()]);
        assert_eq!(q.len(), 1);
        assert!(q[0]["citingSpeaker"].is_null());
    }
}

#[test]
fn user_role_with_multiple_speakers_does_not_label_entire_text_as_persona() {
    let f = fixture();
    let source = json!({"id":"u-multi","role":"user","text":format!("Mira: {}\nAlex: O caderno está aqui.",f["quote"].as_str().unwrap())});
    let q = provenance::quotes(f["messages"][1]["content"].as_str().unwrap(), &[source]);
    assert_eq!(q[0]["originalSpeaker"], "Mira");
    assert_eq!(q[0]["sourceRole"], "user");
}

#[test]
fn quoted_and_action_mentions_do_not_transfer_origin_and_canonical_repeats_remain() {
    let f = fixture();
    let quote = f["quote"].as_str().unwrap();
    let user = f["messages"][1]["content"].as_str().unwrap();
    for body in [
        format!("\"{quote}\" [imitando Mira]"),
        format!("*{quote}*"),
        format!("({quote})"),
        format!("[{quote}]"),
    ] {
        let mimic = json!({"id":"v-mimic","role":"user","text":format!("Alex: {body}")});
        assert!(provenance::quotes(user, std::slice::from_ref(&mimic)).is_empty());
        assert_eq!(
            provenance::quotes(user, &[original(), mimic])[0]["sourceId"],
            "v-original"
        );
    }
    let mut repeat = original();
    repeat["id"] = json!("v-repeat");
    assert_eq!(
        provenance::quotes(user, &[original(), repeat])[0]["sourceId"],
        "v-repeat"
    );
}

#[test]
fn speech_wrapped_in_outer_quotes_keeps_original_provenance_like_plain_speech() {
    let user = "Alex: Você disse “Eu nunca volto para cá, Alex.” e agora está aqui.";
    let source =
        |id: &str, body: &str| json!({"id":id,"role":"user","text":format!("Mira: {body}")});
    for body in [
        "Eu nunca volto para cá, Alex.",
        "\"Eu nunca volto para cá, Alex.\"",
        "“Eu nunca volto para cá, Alex.”",
    ] {
        let found = provenance::quotes(user, &[source("v-user", body)]);
        assert_eq!(found.len(), 1, "{body}");
        assert_eq!(found[0]["originalSpeaker"], "Mira");
        assert_eq!(found[0]["sourceId"], "v-user");
    }
    // inner quotes (a quotation inside the speech) still do not transfer the origin
    let nested = source(
        "v-nested",
        "\"Ela disse “Eu nunca volto para cá, Alex.” ontem\"",
    );
    assert!(provenance::quotes(user, &[nested]).is_empty());
    // the same sentence with different speakers stays ambiguous, even with outer quotes
    let other = json!({"id":"v-other","role":"assistant","text":"Nora: “Eu nunca volto para cá, Alex.”"});
    let both = [source("v-user", "“Eu nunca volto para cá, Alex.”"), other];
    assert!(provenance::quotes(user, &both).is_empty());
}

#[test]
fn ambiguous_original_speakers_abstain_in_provenance_and_review() {
    let f = fixture();
    let mut i = input();
    i["recentSources"].as_array_mut().unwrap().push(json!({"id":"v-other","role":"assistant","text":format!("Nora: {}",f["quote"].as_str().unwrap())}));
    assert!(
        provenance::quotes(
            i["userText"].as_str().unwrap(),
            i["recentSources"].as_array().unwrap()
        )
        .is_empty()
    );
    let mut raw = empty_review();
    raw["attribution"] = json!([f["attributionFlag"]]);
    assert!(flags(&raw, &i, f["messages"][2]["content"].as_str().unwrap()).is_empty());
}

#[test]
fn sources_are_per_version_and_bounded_without_assigning_last_id_to_first_quote() {
    let f = fixture();
    let second = json!({"id":"v-second","role":"assistant","text":format!("Nora: {}",f["secondQuote"].as_str().unwrap())});
    let pair = json!({"id":"i1","assistant":"consolidado sem IDs","versions":["v-original","v-second"],"sources":[original(),second]});
    let text = format!(
        "\"{}\" e \"{}\"",
        f["quote"].as_str().unwrap(),
        f["secondQuote"].as_str().unwrap()
    );
    let q = provenance::quotes(&text, &provenance::sources(std::slice::from_ref(&pair)));
    assert_eq!(
        q.iter()
            .map(|v| v["sourceId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["v-original", "v-second"]
    );
    let mut many = vec![original()];
    many.extend((0..9).map(|n|json!({"id":format!("v{n}"),"role":"assistant","text":format!("Mira: Continuação {n}.")})));
    let limited = provenance::sources(&[json!({"sources":many})]);
    assert_eq!(limited.len(), 8);
    assert!(provenance::quotes(f["messages"][1]["content"].as_str().unwrap(), &limited).is_empty());
    let mut empty = pair;
    empty["sources"] = json!([]);
    assert!(provenance::sources(&[empty]).is_empty());
    assert_eq!(
        provenance::sources(&[json!({"id":"legacy","assistant":"texto"})])[0]["id"],
        "legacy:assistant"
    );
    assert_eq!(
        provenance::sources(
            &(0..4)
                .map(|n| json!({"id":n,"user":"texto"}))
                .collect::<Vec<_>>()
        )
        .len(),
        3
    );
}

#[test]
fn mira_review_requires_exact_prior_quote_and_exact_target() {
    let f = fixture();
    let i = input();
    let text = f["messages"][2]["content"].as_str().unwrap();
    let mut raw = empty_review();
    raw["attribution"] = json!([f["attributionFlag"]]);
    let supported = flags(&raw, &i, text);
    assert_eq!(supported.len(), 1);
    assert_eq!(supported[0]["kind"], "continuity");
    assert_eq!(supported[0]["evidence"], f["attributionFlag"]["quote"]);
    for (key, value) in [
        ("previousQuote", ""),
        ("previousQuote", "fala inexistente"),
        ("quote", "alvo inexistente"),
        ("sourceQuote", "citação inventada"),
        ("claimedSpeaker", "Mira"),
    ] {
        let mut invalid = raw.clone();
        invalid["attribution"][0][key] = json!(value);
        assert!(flags(&invalid, &i, text).is_empty());
    }
}

#[test]
fn empty_review_accepts_normal_quotes_and_imitation_without_flags() {
    let f = fixture();
    let i = input();
    for text in [
        f["messages"][2]["content"].as_str().unwrap().to_owned(),
        format!(
            "Mira: \"{}\" (repete sua fala).",
            f["quote"].as_str().unwrap()
        ),
    ] {
        assert!(flags(&empty_review(), &i, &text).is_empty());
    }
}

#[test]
fn mira_extraction_guard_and_optional_runtime_schema_preserve_correct_fact_only() {
    let f = fixture();
    let raw = json!({"summary":f["summary"],"facts":[{"kind":"EVENT","content":f["wrongFact"],"attribution":{"quote":f["quote"],"originalSpeaker":"Alex"},"knowledge":[{"who":"Mira","state":"knows","via":"heard","evidence":f["quote"],"belief":""}]},{"kind":"EVENT","content":f["correctFact"],"attribution":{"quote":f["quote"],"originalSpeaker":"Mira"},"knowledge":[]}]});
    let out = jobs::sanitize_with_sources(&raw, &input()).unwrap();
    assert_eq!(out["facts"].as_array().unwrap().len(), 1);
    assert_eq!(out["facts"][0]["content"], f["correctFact"]);
    assert_eq!(out["summary"], f["summary"]);
    let schema: Value =
        serde_json::from_str(include_str!("../src/extraction-schema.json")).unwrap();
    let fact = &schema["properties"]["facts"]["items"];
    assert_eq!(
        fact["properties"]["attribution"]["required"],
        json!(["quote", "originalSpeaker"])
    );
    assert!(
        !fact["required"]
            .as_array()
            .unwrap()
            .contains(&json!("attribution"))
    );
    let mut legacy = raw;
    legacy["facts"] = json!([{"kind":"EVENT","content":f["correctFact"]}]);
    assert_eq!(
        jobs::sanitize_with_sources(&legacy, &input()).unwrap()["facts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        include_str!("../src/extraction-prompt.txt")
            .contains("ROLE=user é a fonte, não um rótulo de falante")
    );
}

#[test]
fn mira_context_native_preserves_raw_original_and_prompt_rules_for_both_profiles() {
    let f = fixture();
    let d = json!({"conv":{"persona":"Alex","prompt":"Nome: Mira","settings":{}},"pairs":[{"id":"i1","seq":1,"user":null,"assistant":f["messages"][0]["content"],"complete":true,"sources":[original()]}],"totalPairs":1,"unprocessed":[],"current":{"seq":2,"userText":f["messages"][1]["content"],"priorParts":[]},"cast":[],"pinned":[],"relationships":[],"threads":[],"notes":[],"episodes":[],"timeline":{"current":null},"sync":{"synced":true,"lag":0}});
    for (preset, route) in [
        ("deepseek-v41-flash", "cbai/deepseek-v4.1-flash"),
        ("claude-sonnet-5-5", "cc/claude-sonnet-5-5"),
    ] {
        let plan = presets::request(
            &json!({"presetId":preset,"routeId":route}),
            vec![],
            &json!({}),
        );
        let c = context::assemble(&d, &json!({}), "send", &plan, None).unwrap();
        let at = c
            .messages
            .iter()
            .position(|m| {
                m["role"] == "assistant"
                    && m["content"]
                        .as_str()
                        .unwrap()
                        .ends_with(f["messages"][0]["content"].as_str().unwrap())
            })
            .unwrap();
        let current = c
            .messages
            .iter()
            .position(|m| {
                m["content"]
                    .as_str()
                    .unwrap()
                    .contains(f["messages"][1]["content"].as_str().unwrap())
            })
            .unwrap();
        assert!(at < current);
        let text = c
            .messages
            .iter()
            .map(|m| m["content"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains("{{persona}}") && !text.contains("{{PERSONA}}"));
        assert!(
            text.contains("AGÊNCIA DE ALEX")
                && text.contains("quem pronunciou originalmente")
                && text.contains("ROLE=user identifica a fonte")
        );
        assert!(
            text.contains("preserve autor original de citações/imitações e separe comentários")
        );
    }
}

async fn turn(
    store: &Store,
    cid: &str,
    mode: &str,
    content: Value,
    reply: &str,
    op: &str,
) -> Value {
    let begin = store
        .call(Request::BeginTurn(
            serde_json::from_value::<TurnRequest>(
                json!({"convId":cid,"mode":mode,"content":content,"opId":op}),
            )
            .unwrap(),
        ))
        .await
        .unwrap();
    store
        .call(Request::FinishGeneration(
            serde_json::from_value::<FinishRequest>(
                json!({"generationId":begin["generationId"],"status":"complete","text":reply}),
            )
            .unwrap(),
        ))
        .await
        .unwrap();
    begin
}

#[tokio::test]
async fn native_store_sources_job_input_selected_versions_replay_and_cutoff() {
    let f = fixture();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let c=store.call(Request::CreateConversation(json!({"name":"Mira fixture","persona":"Alex","prompt":"Nome: Mira","presetId":"deepseek-v41-flash","routeId":"cbai/deepseek-v4.1-flash"}))).await.unwrap();
    let cid = c["id"].as_str().unwrap();
    let first = turn(
        &store,
        cid,
        "send",
        json!("Na Diretoria."),
        f["messages"][0]["content"].as_str().unwrap(),
        "first",
    )
    .await;
    let second = turn(
        &store,
        cid,
        "continue",
        Value::Null,
        &format!("Nora: {}", f["secondQuote"].as_str().unwrap()),
        "second",
    )
    .await;
    let replaced = turn(
        &store,
        cid,
        "regenerate",
        Value::Null,
        "Nora: O caderno sumiu.",
        "replace",
    )
    .await;
    let d = store
        .call(Request::Context(
            json!({"convId":cid,"pendingUserText":f["messages"][1]["content"]}),
        ))
        .await
        .unwrap();
    assert_eq!(d["pairs"][0]["sources"][1]["id"], first["versionId"]);
    assert_eq!(d["pairs"][0]["sources"][2]["id"], replaced["versionId"]);
    store
        .call(Request::SelectVersion {
            conv_id: cid.into(),
            part_id: second["partId"].as_str().unwrap().into(),
            version_id: second["versionId"].as_str().unwrap().into(),
        })
        .await
        .unwrap();
    let d = store
        .call(Request::Context(
            json!({"convId":cid,"pendingUserText":f["messages"][1]["content"]}),
        ))
        .await
        .unwrap();
    assert_eq!(d["pairs"][0]["sources"][2]["id"], second["versionId"]);
    // `parts` is no longer sent (it repeated `assistant`); the user source and both parts stay in `sources`
    assert!(d["pairs"][0]["parts"].is_null());
    assert_eq!(d["pairs"][0]["sources"].as_array().unwrap().len(), 3);
    let replay = store
        .call(Request::BeginTurn(
            serde_json::from_value::<TurnRequest>(
                json!({"convId":cid,"mode":"send","content":"Na Diretoria.","opId":"first"}),
            )
            .unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(replay["replay"], true);
    let current = turn(
        &store,
        cid,
        "send",
        f["messages"][1]["content"].clone(),
        f["messages"][2]["content"].as_str().unwrap(),
        "current",
    )
    .await;
    turn(
        &store,
        cid,
        "send",
        json!("FUTURO_NÃO_ENTRAR"),
        "Mira: FUTURO_NÃO_ENTRAR",
        "future",
    )
    .await;
    let job=store.call(Request::EnqueueJob(json!({"convId":cid,"interactionId":current["interactionId"],"versionId":current["versionId"],"kind":"turn"}))).await.unwrap();
    let input = store
        .call(Request::JobInput(job["id"].as_str().unwrap().into()))
        .await
        .unwrap();
    let previous = input["previous"].as_str().unwrap();
    assert!(
        previous.contains(f["messages"][0]["content"].as_str().unwrap())
            && previous.contains(first["versionId"].as_str().unwrap())
    );
    assert!(!previous.contains("FUTURO_NÃO_ENTRAR"));
    assert_eq!(input["assistantText"], f["messages"][2]["content"]);
    assert!(input["text"].as_str().unwrap().contains("role=user"));
    assert_eq!(
        provenance::quotes(
            input["userText"].as_str().unwrap(),
            input["recentSources"].as_array().unwrap()
        )[0]["sourceId"],
        first["versionId"]
    );
    drop(store);
}

// ---------------------------------------------------------------------------------------
// Authorship conflict: the sanitizer signals it, the runner refuses the whole batch.
// ---------------------------------------------------------------------------------------
fn extraction_fixture() -> Value {
    let f = fixture();
    let fact = |speaker: &str, content: &Value| json!({"kind":"EVENT","subject":speaker,"content":content,"importance":0.5,"confidence":1,"visibility":"private","replaces":"","existing_id":"","attribution":{"quote":f["quote"],"originalSpeaker":speaker},"knowledge":[{"who":"Mira","state":"knows","via":"heard","evidence":f["quote"],"belief":""}]});
    json!({"summary":f["summary"],"cast":[],"scene":{"location":"corredor","time":"","mood":"","present":["Alex","Mira"],"objects":[],"characters":[]},"facts":[fact("Alex",&f["wrongFact"]),fact("Mira",&f["correctFact"])],"relationships":[],"open_threads":[],"resolved_threads":[]})
}
// The batch's OWN summary carries the false accusation, next to the fact with inverted authorship.
fn contaminated_extraction() -> Value {
    let mut e = extraction_fixture();
    e["summary"] = fixture()["wrongFact"].clone();
    e
}
fn valid_extraction() -> Value {
    let mut e = extraction_fixture();
    e["facts"] = json!([e["facts"][1].clone()]);
    e
}
// Same valid fact WITHOUT attribution metadata (legacy/optional schema): never triggers the block.
fn legacy_extraction() -> Value {
    let mut e = valid_extraction();
    e["facts"][0].as_object_mut().unwrap().remove("attribution");
    e
}

#[test]
fn mira_sanitizer_signals_confirmed_attribution_conflict_and_keeps_valid_facts() {
    let f = fixture();
    let out = jobs::sanitize_with_sources(&extraction_fixture(), &input()).unwrap();
    assert_eq!(
        out["facts"].as_array().unwrap().len(),
        1,
        "pure contract: valid facts preserved"
    );
    assert_eq!(out["facts"][0]["content"], f["correctFact"]);
    let rejected = out["attributionRejected"].as_array().unwrap();
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0]["content"], f["wrongFact"]);
    assert_eq!(rejected[0]["quote"], f["quote"]);
    assert_eq!(rejected[0]["claimedSpeaker"], "Alex");
    assert_eq!(rejected[0]["documentedSpeaker"], "Mira");
    assert_eq!(rejected[0]["sourceId"], "v-original");
    let polluted = jobs::sanitize_with_sources(&contaminated_extraction(), &input()).unwrap();
    assert_eq!(
        polluted["summary"], f["wrongFact"],
        "the pure sanitizer does not rewrite the summary"
    );
    assert_eq!(polluted["attributionRejected"].as_array().unwrap().len(), 1);
}

#[test]
fn mira_sanitizer_has_no_signal_without_confirmed_conflict() {
    let f = fixture();
    let clean = |raw: &Value, input: &Value| {
        let out = jobs::sanitize_with_sources(raw, input).unwrap();
        assert!(out.get("attributionRejected").is_none(), "{out}");
        out
    };
    clean(&valid_extraction(), &input());
    // no attribution metadata: kept even when it resembles the false accusation
    let mut similar = legacy_extraction();
    similar["facts"][0]["content"] = f["wrongFact"].clone();
    let kept = clean(&similar, &input());
    assert_eq!(kept["facts"][0]["content"], f["wrongFact"]);
    // no source context, or a quote nobody documented
    assert_eq!(
        clean(&extraction_fixture(), &json!({}))["facts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let mut unknown = valid_extraction();
    unknown["facts"][0]["attribution"]["quote"] =
        json!("Frase que nenhuma fonte recente documenta");
    assert_eq!(
        clean(&unknown, &input())["facts"].as_array().unwrap().len(),
        1
    );
    // missing/empty/non-string declared speaker: incomplete metadata drops the fact but accuses nobody
    for speaker in [Value::Null, json!(""), json!("   "), json!(42)] {
        let mut incomplete = valid_extraction();
        incomplete["facts"][0]["attribution"]["originalSpeaker"] = speaker.clone();
        assert!(
            clean(&incomplete, &input())["facts"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{speaker}"
        );
    }
    let mut absent = valid_extraction();
    absent["facts"][0]["attribution"]
        .as_object_mut()
        .unwrap()
        .remove("originalSpeaker");
    assert!(
        clean(&absent, &input())["facts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // same speaker with other case, accents or full name is not a conflict and the fact stays
    for speaker in ["mira", " MIRA ", "Mira Rose"] {
        let mut same = valid_extraction();
        same["facts"][0]["attribution"]["originalSpeaker"] = json!(speaker);
        let out = clean(&same, &input());
        assert_eq!(out["facts"][0]["content"], f["correctFact"], "{speaker}");
    }
    // a fact dropped for being too short never counts as a conflict
    let mut tiny = extraction_fixture();
    tiny["facts"][0]["content"] = json!("x");
    clean(&tiny, &input());
}

#[test]
fn mira_review_attribution_fields_must_be_non_empty_strings() {
    let f = fixture();
    let i = input();
    let text = f["messages"][2]["content"].as_str().unwrap();
    let mut raw = empty_review();
    raw["attribution"] = json!([f["attributionFlag"]]);
    assert_eq!(flags(&raw, &i, text).len(), 1, "complete and faithful");
    for key in [
        "quote",
        "sourceQuote",
        "previousQuote",
        "originalSpeaker",
        "claimedSpeaker",
    ] {
        let good = f["attributionFlag"][key].clone();
        for bad in [
            Value::Null,
            json!(""),
            json!("   "),
            json!(0),
            json!(7),
            json!(true),
            json!({}),
            json!([]),
            json!([good]),
        ] {
            let mut invalid = raw.clone();
            invalid["attribution"][0][key] = bad.clone();
            assert!(flags(&invalid, &i, text).is_empty(), "{key}: {bad}");
        }
        let mut missing = raw.clone();
        missing["attribution"][0]
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(flags(&missing, &i, text).is_empty(), "{key} missing");
    }
    for odd in [
        json!(null),
        json!("texto"),
        json!(5),
        json!([]),
        json!([null]),
        json!([[]]),
        json!({}),
    ] {
        let mut invalid = raw.clone();
        invalid["attribution"] = odd.clone();
        assert!(flags(&invalid, &i, text).is_empty(), "{odd}");
    }
    for (key, forged) in [
        ("originalSpeaker", "Nora".to_owned()),
        ("sourceQuote", format!("{}!", f["quote"].as_str().unwrap())),
        (
            "previousQuote",
            format!("{} e mais", f["quote"].as_str().unwrap()),
        ),
        (
            "quote",
            format!(
                "{} (forjado)",
                f["attributionFlag"]["quote"].as_str().unwrap()
            ),
        ),
    ] {
        let mut invalid = raw.clone();
        invalid["attribution"][0][key] = json!(forged);
        assert!(flags(&invalid, &i, text).is_empty(), "{key} forged");
    }
}

fn normalized_review_schema(schema: &Value) -> Value {
    let sorted = |v: &Value| {
        let mut items = v
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        items.sort();
        json!(items)
    };
    let mut attribution = schema["properties"]["attribution"].clone();
    attribution["items"]["required"] = sorted(&attribution["items"]["required"]);
    json!({"topLevelRequired":sorted(&schema["required"]),"attribution":attribution})
}

#[test]
fn review_schema_attribution_matches_the_contract_shared_with_node() {
    // tests/fixtures/review-attribution-schema.json is also asserted against Node's REVIEW_SCHEMA and
    // against the real request of both runtimes (tests/provenance.test.js, provenance-runtime.test.js).
    let contract: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/review-attribution-schema.json"
    ))
    .unwrap();
    let schema: Value = serde_json::from_str(include_str!("../src/review-schema.json")).unwrap();
    assert_eq!(normalized_review_schema(&schema), contract);
    assert!(
        schema["properties"]["attribution"]["items"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("previousQuote"))
    );
}

// --- runner with a simulated local model (MC_TEST_LOCAL_URL fixture hook), disposable DB and loopback only ---
struct Model {
    requests: Mutex<Vec<Value>>,
    // extraction answers for the current turn: served in order, the last one repeats
    script: Mutex<(Vec<Value>, usize)>,
}
impl Model {
    fn set(&self, script: Vec<Value>) {
        *self.script.lock().unwrap() = (script, 0);
    }
    fn current_extractions(&self) -> usize {
        let current = fixture()["messages"][1]["content"]
            .as_str()
            .unwrap()
            .to_owned();
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|b| {
                b["response_format"]["json_schema"]["schema"]["properties"]["facts"].is_object()
                    && s(b["messages"].as_array().unwrap().last().unwrap(), "content")
                        .contains(&current)
            })
            .count()
    }
}
async fn model_chat(State(m): State<Arc<Model>>, Json(body): Json<Value>) -> Json<Value> {
    m.requests.lock().unwrap().push(body.clone());
    let props = &body["response_format"]["json_schema"]["schema"]["properties"];
    let text = body["messages"]
        .as_array()
        .and_then(|a| a.last())
        .map(|x| s(x, "content"))
        .unwrap_or_default();
    let result = if props["agency"].is_object() {
        empty_review()
    } else if props["facts"].is_object() {
        if text.contains(fixture()["messages"][1]["content"].as_str().unwrap()) {
            let mut served = m.script.lock().unwrap();
            let at = served.1.min(served.0.len() - 1);
            served.1 += 1;
            served.0[at].clone()
        } else {
            json!({"summary":"Mira fala com Alex na Diretoria.","cast":[],"scene":null,"facts":[],"relationships":[],"open_threads":[],"resolved_threads":[]})
        }
    } else {
        json!({"cast":[]})
    };
    Json(json!({"choices":[{"message":{"content":result.to_string()},"finish_reason":"stop"}]}))
}
async fn model_embedding() -> Json<Value> {
    Json(json!({"data":[{"embedding":[1.0,0.0,0.0]}]}))
}

// LocalModels reads MC_TEST_LOCAL_URL when it is built: the runner scenarios are serialized and each one
// points the variable at its own simulator (Windows: SetEnvironmentVariable is thread-safe).
static RUNNER_SCENARIO: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
struct Scenario {
    _guard: tokio::sync::MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
    state: AppState,
    model: Arc<Model>,
    cid: String,
}
async fn scenario(script: Vec<Value>) -> Scenario {
    let guard = RUNNER_SCENARIO.lock().await;
    let model = Arc::new(Model {
        requests: Mutex::default(),
        script: Mutex::default(),
    });
    model.set(script);
    let app = Router::new()
        .route("/v1/chat/completions", post(model_chat))
        .route("/v1/embeddings", post(model_embedding))
        .with_state(model.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    // SAFETY: serialized by RUNNER_SCENARIO; no other test in this binary builds LocalModels.
    unsafe { std::env::set_var("MC_TEST_LOCAL_URL", &url) };
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        data_dir: dir.path().into(),
        dist: dir.path().join("absent"),
        address: "127.0.0.1:0".parse().unwrap(),
        require_pairing: false,
        providers: None,
        max_generations: 1,
        production: false,
        review: true,
    };
    let state = AppState::new(config).await.unwrap();
    let f = fixture();
    let c=state.store.call(Request::CreateConversation(json!({"name":"Mira fixture","persona":"Alex","prompt":"Nome: Mira","presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5"}))).await.unwrap();
    let cid = c["id"].as_str().unwrap().to_owned();
    turn(
        &state.store,
        &cid,
        "send",
        json!("Na Diretoria."),
        f["messages"][0]["content"].as_str().unwrap(),
        "first",
    )
    .await;
    turn(
        &state.store,
        &cid,
        "send",
        f["messages"][1]["content"].clone(),
        f["messages"][2]["content"].as_str().unwrap(),
        "current",
    )
    .await;
    state.runner.kick();
    Scenario {
        _guard: guard,
        _dir: dir,
        state,
        model,
        cid,
    }
}
impl Scenario {
    async fn overview(&self) -> Value {
        self.state
            .store
            .call(Request::Memories(self.cid.clone()))
            .await
            .unwrap()
    }
    async fn data(&self) -> Value {
        self.state
            .store
            .call(Request::Export(Some(self.cid.clone())))
            .await
            .unwrap()["conversations"][0]
            .clone()
    }
    async fn until(&self, what: &str, done: impl Fn(&Value) -> bool) -> Value {
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            let overview = self.overview().await;
            if done(&overview) {
                return overview;
            }
            assert!(Instant::now() < end, "deadline: {what}: {overview}");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    async fn failed(&self) -> Value {
        self.until("job failed", |m| {
            m["jobs"]["failed"] == 1
                && m["jobs"]["pending"].is_null()
                && m["jobs"]["running"].is_null()
        })
        .await
    }
    async fn recovered(&self) -> Value {
        self.until("batch applied", |m| {
            m["sync"]["synced"] == true
                && !m["memories"].as_array().unwrap().is_empty()
                && m["jobs"]["failed"].is_null()
                && m["jobs"]["pending"].is_null()
                && m["jobs"]["running"].is_null()
        })
        .await
    }
    async fn finish(self) {
        self.state.shutdown().await;
        // SAFETY: still holding RUNNER_SCENARIO.
        unsafe { std::env::remove_var("MC_TEST_LOCAL_URL") };
    }
}
fn table_len(data: &Value, table: &str) -> usize {
    data[table].as_array().map_or(0, Vec::len)
}
// Nothing of the current turn (seq 2) was promoted: summary, note, facts, knowledge, scene, episodes...
async fn assert_nothing_promoted(sc: &Scenario) {
    let f = fixture();
    let overview = sc.overview().await;
    assert!(overview["memories"].as_array().unwrap().is_empty());
    assert_eq!(overview["flags"].as_array().unwrap().len(), 0);
    let data = sc.data().await;
    let notes = data["interaction_notes"].as_array().unwrap();
    assert!(
        notes.iter().all(|n| n["seq"] != 2),
        "the blocked turn creates no summary"
    );
    for table in [
        "memories",
        "knowledge_events",
        "memory_knowledge",
        "episodes",
        "relationships",
        "threads",
    ] {
        assert_eq!(table_len(&data, table), 0, "{table}");
    }
    assert!(
        data["scene_states"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["seq"] != 2)
    );
    let mut derived = data.clone();
    for source in ["versions", "interactions"] {
        derived[source] = Value::Null;
    }
    assert!(
        !derived
            .to_string()
            .contains(f["wrongFact"].as_str().unwrap()),
        "the false accusation is in no derived table"
    );
    // sources and history are intact (the wrong bot reply is never rewritten)
    let versions = data["versions"].to_string();
    assert!(versions.contains("Ele me chamou de carteira"));
}
async fn assert_recovered(sc: &Scenario) {
    let f = fixture();
    sc.recovered().await;
    let overview = sc.overview().await;
    let contents = overview["memories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| s(m, "content"))
        .collect::<Vec<_>>();
    assert_eq!(contents, vec![s(&f, "correctFact")]);
    let data = sc.data().await;
    let correct = data["memories"][0]["id"].clone();
    assert!(
        !data["knowledge_events"].as_array().unwrap().is_empty()
            && data["knowledge_events"]
                .as_array()
                .unwrap()
                .iter()
                .all(|k| k["memory_id"] == correct),
        "no knowledge row belongs to the discarded false fact"
    );
    let note = data["interaction_notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["seq"] == 2)
        .unwrap();
    assert_eq!(note["summary"], f["summary"]);
    assert!(
        !data["memories"]
            .to_string()
            .contains(f["wrongFact"].as_str().unwrap())
    );
}

async fn blocked_then_recovered(first: Value) {
    let sc = scenario(vec![first]).await;
    sc.failed().await;
    // all the job's attempts were refused; nothing was promoted
    assert!(sc.model.current_extractions() >= 3);
    assert_nothing_promoted(&sc).await;
    let last_error = sc.state.runner.state().await["lastError"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        last_error.contains("conflito de autoria")
            && last_error.contains("Alex")
            && last_error.contains("Mira"),
        "{last_error}"
    );
    assert!(!last_error.contains(fixture()["wrongFact"].as_str().unwrap()));
    // retry with valid JSON (no inverted authorship) applies normally
    sc.model.set(vec![valid_extraction()]);
    let requeued = sc
        .state
        .store
        .call(Request::RetryJobs(sc.cid.clone()))
        .await
        .unwrap();
    assert_eq!(requeued, 1);
    sc.state.runner.kick();
    assert_recovered(&sc).await;
    sc.finish().await;
}

#[tokio::test]
async fn runner_blocks_inverted_authorship_even_with_a_correct_summary_and_retry_applies() {
    blocked_then_recovered(extraction_fixture()).await;
}

#[tokio::test]
async fn runner_blocks_explicitly_false_summary_without_promoting_anything_and_retry_applies() {
    blocked_then_recovered(contaminated_extraction()).await;
}

#[tokio::test]
async fn runner_second_attempt_with_valid_json_applies_without_intervention() {
    let sc = scenario(vec![contaminated_extraction(), valid_extraction()]).await;
    assert_recovered(&sc).await;
    assert_eq!(sc.model.current_extractions(), 2);
    assert!(sc.overview().await["jobs"]["failed"].is_null());
    sc.finish().await;
}

#[tokio::test]
async fn runner_does_not_block_a_valid_fact_without_attribution_metadata() {
    let sc = scenario(vec![legacy_extraction()]).await;
    assert_recovered(&sc).await;
    assert_eq!(
        sc.model.current_extractions(),
        1,
        "applied on the first attempt"
    );
    assert!(sc.overview().await["jobs"]["failed"].is_null());
    let last_error = sc.state.runner.state().await["lastError"].clone();
    assert!(
        !last_error
            .as_str()
            .unwrap_or_default()
            .contains("conflito de autoria")
    );
    sc.finish().await;
}
