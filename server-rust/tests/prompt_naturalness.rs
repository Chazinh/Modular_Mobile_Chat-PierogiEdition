//! Regression: the generation prompt must not push characters into recapping what just happened,
//! forcing every character to react, or naming the owner of every object in narration.
use modularchat_server::{context, presets, style_policy};
use serde_json::json;

fn assembled() -> (String, String) {
    let d = json!({"conv":{"persona":"Alex","prompt":"Lia estuda magia.","settings":{}},"pairs":[],"totalPairs":0,"current":{"userText":"Oi","seq":1,"priorParts":[]}});
    let plan = presets::request(
        &json!({"presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5"}),
        vec![],
        &json!({}),
    );
    let c = context::assemble(&d, &json!({}), "send", &plan, None).unwrap();
    let system = c.messages[0]["content"].as_str().unwrap().to_owned();
    let tail = c.messages.last().unwrap()["content"].as_str().unwrap().to_owned();
    (system, tail)
}

#[test]
fn prompt_forbids_recaps_in_system_and_tail() {
    let (system, tail) = assembled();
    assert!(system.contains("SEM RECAPITULAÇÃO"));
    assert!(system.contains("sem descrevê-la de volta"));
    assert!(tail.contains("sem recapitular nem parafrasear o que acabou de acontecer"));
    // the possession rule is stated once (POV matrix), not repeated as a shouting block
    assert!(!system.contains("DISTINÇÃO DE PARENTESCO E POSSE (MINHA / SUA)"));
    assert!(!system.contains("{{"));
}

#[test]
fn narration_does_not_require_owner_names_on_every_mention() {
    let (system, _) = assembled();
    assert!(!system.contains("O narrador DEVE usar nomes próprios explícitos"));
    assert!(system.contains("Não repita o nome do dono a cada menção"));
}

#[test]
fn interactive_pacing_does_not_make_every_character_react() {
    let i = style_policy::turn_instruction(&json!({"turnPacing":"interactive"}));
    assert!(!i.contains("cada personagem relevante reage"));
    assert!(i.contains("só reage quem tem motivo"));
    assert!(i.contains("nem resumir a cena"));
}

#[test]
fn length_repair_does_not_pad_with_recap() {
    let before = json!({"minCharacters":800,"maxCharacters":1200,"characters":300,"dialoguePct":50.0});
    assert!(style_policy::instruction(&before, 65.0).contains("nunca preencha com recapitulação"));
}
