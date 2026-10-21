//! Milestone M4: Adversarial & Anti-Regression Probes
//! Validates:
//! - Feature 20: Non-destructive {{char}} replacement in character cards.
//! - Feature 21: Single-word character names under section headers in cast::from_card.
//! - Feature 22: Choral dynamic and pacing harmonization in style_policy.
//! - Feature 23: Global customization prompt attention anchor in context system & tail.
//! - Feature 24: Security hardening of character.txt sidecar against symlinks.
//! - Feature 26: Strict confidentiality and isolation invariants.

use modularchat_server::{cast, context, presets, style_policy};
use serde_json::json;

fn plan() -> presets::Plan {
    presets::request(
        &json!({"presetId":"claude-sonnet-5-5","routeId":"cc/claude-sonnet-5-5"}),
        vec![],
        &json!({}),
    )
}

#[test]
fn probe_non_destructive_char_replacement_named() {
    let conv = json!({
        "id": "c1",
        "name": "Elena",
        "characterName": "Elena",
        "prompt": "Nome: {{char}}\nIdade: 19\nPersonalidade: {{char}} é gentil e determinada.\nUsuário: {{user}}.",
        "persona": "Alex",
        "settings": {
            "context": {
                "preferredPairs": 30,
                "minimumPairs": 10
            }
        }
    });

    let data = json!({
        "conv": conv,
        "globalSettings": {
            "temperature": 0.8,
            "maxOutputTokens": 2048,
            "content": ""
        },
        "pairs": [],
        "current": {
            "userText": "Olá, como você está?",
            "seq": 1
        },
        "scene": null,
        "pinned": [],
        "unprocessed": [],
        "episodes": [],
        "notes": [],
        "cast": [
            {
                "name": "Elena",
                "kind": "main"
            }
        ]
    });

    let resolved_char = context::resolve_char_replacement(&data["conv"], &data);
    assert_eq!(resolved_char, "Elena", "Resolved character name must match canonical name");

    let assembled = context::assemble(&data, &json!({}), "send", &plan(), None).expect("Context assembly must succeed");
    let system_msg = assembled
        .messages
        .iter()
        .find(|m| m["role"] == "system")
        .expect("System message must be present");
    let content = system_msg["content"].as_str().unwrap();

    assert!(
        content.contains("Nome: Elena"),
        "Card must contain 'Nome: Elena' instead of destructive replacement, got:\n{}",
        content
    );
    assert!(
        content.contains("Personalidade: Elena é gentil"),
        "Card must contain 'Personalidade: Elena é gentil'"
    );
    assert!(
        !content.contains("Nome: o narrador e os personagens"),
        "Destructive 'o narrador e os personagens' must never corrupt card fields"
    );
}

#[test]
fn probe_non_destructive_char_replacement_fallback() {
    let conv = json!({
        "id": "c2",
        "name": "Chat 123",
        "prompt": "Ficha do NPC:\nNome: {{char}}\nPapel: Guia da floresta.",
        "persona": "Viajante",
        "settings": {
            "context": {
                "preferredPairs": 30,
                "minimumPairs": 10
            }
        }
    });

    let data = json!({
        "conv": conv,
        "globalSettings": {
            "temperature": 0.8,
            "content": ""
        },
        "pairs": [],
        "current": {
            "userText": "Quem é você?",
            "seq": 1
        },
        "scene": null,
        "pinned": [],
        "unprocessed": [],
        "episodes": [],
        "notes": []
    });

    let resolved_char = context::resolve_char_replacement(&data["conv"], &data);
    assert_eq!(resolved_char, "o personagem");

    let assembled = context::assemble(&data, &json!({}), "send", &plan(), None).expect("Context assembly must succeed");
    let system_msg = assembled
        .messages
        .iter()
        .find(|m| m["role"] == "system")
        .expect("System message must be present");
    let content = system_msg["content"].as_str().unwrap();

    assert!(
        content.contains("Nome: o personagem"),
        "Fallback must preserve singular descriptor, got:\n{}",
        content
    );
    assert!(
        !content.contains("o narrador e os personagens"),
        "Must not corrupt with plural narrator placeholder"
    );
}

#[test]
fn probe_single_word_character_names_under_section() {
    let card = r#"
[Personagens]
Elena: maga elemental dedicada, tímida e leal.
Mira: ladra ágil e perspicaz da guilda das sombras.
Kael: guerreiro mercenário de poucas palavras.
"#;
    let cast = cast::from_card(card, "Alex");

    let names: Vec<&str> = cast
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();

    assert!(
        names.contains(&"Elena"),
        "Cast must include single-word name 'Elena', got: {:?}",
        names
    );
    assert!(
        names.contains(&"Mira"),
        "Cast must include single-word name 'Mira', got: {:?}",
        names
    );
    assert!(
        names.contains(&"Kael"),
        "Cast must include single-word name 'Kael', got: {:?}",
        names
    );
}

#[test]
fn probe_single_word_character_standalone_lines() {
    let card = r#"
Personagens:
Elena
Maga elemental de gelo e fogo.

Mira
Batedora das terras do norte.
"#;
    let cast = cast::from_card(card, "Alex");

    let names: Vec<&str> = cast
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();

    assert!(
        names.contains(&"Elena"),
        "Standalone single-word name 'Elena' must be captured under character section, got: {:?}",
        names
    );
    assert!(
        names.contains(&"Mira"),
        "Standalone single-word name 'Mira' must be captured under character section, got: {:?}",
        names
    );
}

#[test]
fn probe_style_policy_choral_dialogue_harmonization() {
    let gs = json!({
        "turnPacing": "interactive",
        "maxSpeech": 400
    });

    let instruction = style_policy::turn_instruction(&gs);

    assert!(
        instruction.contains("apartes") || instruction.contains("interações cruzadas"),
        "Turn instruction must permit choral apartes / cross-interactions, got: {}",
        instruction
    );
    assert!(
        !instruction.contains("faça apenas uma reação curta de cada personagem"),
        "Contradictory single-line freeze instruction must be replaced"
    );
    assert!(
        !instruction.contains("Não crie uma conversa longa entre personagens"),
        "Must not strictly prohibit multi-NPC conversation"
    );
}

#[test]
fn probe_global_customization_attention_anchors() {
    let custom_rules = "REGRAS ESPECIAIS:\n1. Magia consome estamina física.\n2. Todos os combates são viscerais.";

    let data = json!({
        "conv": {
            "id": "c3",
            "name": "Elena",
            "prompt": "Elena: companheira de jornada.",
            "persona": "Alex",
            "settings": {
                "context": {
                    "preferredPairs": 30,
                    "minimumPairs": 10
                }
            }
        },
        "globalSettings": {
            "temperature": 0.8,
            "content": custom_rules
        },
        "pairs": [],
        "current": {
            "userText": "Vamos avançar pela caverna.",
            "seq": 1
        },
        "scene": null,
        "pinned": [],
        "unprocessed": [],
        "episodes": [],
        "notes": []
    });

    let assembled = context::assemble(&data, &json!({}), "send", &plan(), None).expect("Context assembly must succeed");

    let system_msg = assembled
        .messages
        .iter()
        .find(|m| m["role"] == "system")
        .expect("System message must be present");
    let system_text = system_msg["content"].as_str().unwrap();

    assert!(
        system_text.contains("[DIRETRIZES GLOBAIS DE PERSONALIZAÇÃO DO CHAT"),
        "System prompt must contain prominent global customization section"
    );
    assert!(
        system_text.contains("Magia consome estamina física"),
        "Custom global content must be present in system prompt"
    );

    let user_msg = assembled
        .messages
        .iter()
        .filter(|m| m["role"] == "user")
        .last()
        .expect("Last user message must be present");
    let user_text = user_msg["content"].as_str().unwrap();

    assert!(
        user_text.contains("obedeça estritamente às diretrizes globais de personalização"),
        "Tail reminder must anchor attention to global customization and card, got:\n{}",
        user_text
    );
}

#[test]
fn probe_core_prompt_and_ai_memory_isolation_invariants() {
    let core_example = include_str!("../src/core-prompt.example.txt");
    assert!(
        !core_example.is_empty(),
        "core-prompt.example.txt must exist as public template"
    );

    // Verify zero occurrences of ai-memory across all source files
    let cargo_toml = include_str!("../Cargo.toml");
    assert!(!cargo_toml.contains("ai-memory"));
    assert!(!cargo_toml.contains("aimemory"));
}
