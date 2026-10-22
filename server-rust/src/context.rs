//! Context assembly with coverage-checked history compression and explicit token estimates.
use crate::{Result, n, presets::Plan, rules, s, style_policy};
use serde_json::{Value, json};
use std::collections::HashSet;

pub const CONTINUE: &str = "Continue a cena exatamente de onde parou, no mesmo formato, sem repetir o que já foi escrito e sem agir por mim.";
pub struct Context {
    pub messages: Vec<Value>,
    pub report: Value,
    pub raw_estimate: usize,
    pub selected_history_len: usize,
}

/// Result of scene presence resolution during context assembly.
#[derive(Debug, Clone)]
pub struct ScenePresenceResolution {
    pub effective_scene: Value,
    pub effective_present: Vec<String>,
    pub invalidated_characters: Vec<String>,
    pub transition_detected: bool,
}

/// Detects scene/location transitions and invalidates stale N-1 presence cache to eliminate ghost characters.
pub fn resolve_scene_presence(
    scene: &Value,
    current_user: &str,
    _pairs: &[Value],
    persona: &str,
) -> ScenePresenceResolution {
    if scene.is_null() || !scene.is_object() {
        return ScenePresenceResolution {
            effective_scene: scene.clone(),
            effective_present: Vec::new(),
            invalidated_characters: Vec::new(),
            transition_detected: false,
        };
    }

    let raw_present: Vec<String> = array(&scene["present"])
        .iter()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect();

    let transition_detected = detect_transition_in_text(current_user);

    if !transition_detected {
        return ScenePresenceResolution {
            effective_scene: scene.clone(),
            effective_present: raw_present,
            invalidated_characters: Vec::new(),
            transition_detected: false,
        };
    }

    let is_solo = detect_solo_movement(current_user);
    let mut effective_present = Vec::<String>::new();
    let mut invalidated = Vec::<String>::new();
    let persona_key = rules::key(persona);

    effective_present.push(persona.to_string());

    if !is_solo {
        for char_name in &raw_present {
            let key = rules::key(char_name);
            if key == persona_key {
                continue;
            }
            if accompanies_transition(char_name, current_user) {
                if !effective_present.iter().any(|c| rules::key(c) == key) {
                    effective_present.push(char_name.clone());
                }
            } else if !invalidated.iter().any(|c| rules::key(c) == key) {
                invalidated.push(char_name.clone());
            }
        }
    } else {
        for char_name in &raw_present {
            let key = rules::key(char_name);
            if key != persona_key && !invalidated.iter().any(|c| rules::key(c) == key) {
                invalidated.push(char_name.clone());
            }
        }
    }

    let mut modified_scene = scene.clone();
    modified_scene["present"] = json!(effective_present);
    // Invalidate room-specific objects from previous room
    modified_scene["objects"] = json!([]);
    let present_keys: HashSet<String> = effective_present.iter().map(|n| rules::key(n)).collect();
    if let Some(chars) = modified_scene["characters"].as_array_mut() {
        chars.retain(|c| present_keys.contains(&rules::key(&s(c, "name"))));
    }
    modified_scene["transition_detected"] = json!(true);

    ScenePresenceResolution {
        effective_scene: modified_scene,
        effective_present,
        invalidated_characters: invalidated,
        transition_detected: true,
    }
}

fn detect_transition_in_text(text: &str) -> bool {
    let lower = text.to_lowercase();
    const NON_TRANSITION_EXCLUSIONS: &[&str] = &[
        "entra em pânico", "entra em panico", "entra em choque", "entra em transe", "entra em colapso", "entra em detalhes",
        "entro em pânico", "entro em panico", "entro em choque", "entro em transe", "entro em colapso",
        "sai do sério", "sai do serio", "saio do sério", "saio do serio", "saiu do sério", "saiu do serio",
        "sai do controle", "saio do controle", "saiu do controle",
        "olhar volta para", "olhos voltam para", "atenção volta para", "atencao volta para", "conversa segue para",
        "conversa volta para", "assunto volta para", "assunto segue para",
        "chego no meu limite", "chego ao meu limite", "chega no seu limite", "chega ao limite",
        "deixo o copo", "deixo a porta", "deixo o livro", "deixo o bilhete", "deixo a chave",
        "deixo a bolsa", "deixo o prato", "deixo o telefone", "deixo o celular",
        "deixa o copo", "deixa a porta", "deixa o livro", "deixa o bilhete", "deixa a chave",
        "deixando o copo", "deixando a porta", "deixando o livro",
    ];
    let mut sanitized = lower.clone();
    for exc in NON_TRANSITION_EXCLUSIONS {
        if sanitized.contains(exc) {
            sanitized = sanitized.replace(exc, " ");
        }
    }

    const MOVEMENT_PREFIXES: &[&str] = &[
        "vai para", "vai até", "vai ao", "vai à", "vou para", "vou até", "vou ao", "vou à",
        "vamos para", "vamos até", "vamos ao", "vamos à", "foi para", "foi até", "foi ao", "foi à",
        "fui para", "fui até", "fui ao", "fui à", "fomos para", "fomos até", "fomos ao", "fomos à",
        "caminha para", "caminha até", "caminho para", "caminho até", "caminhamos para", "caminhamos até",
        "sai da", "sai do", "sai de", "saio da", "saio do", "saio de",
        "saímos da", "saímos do", "saímos de", "saímos para", "saímos até", "saímos ao", "saímos à",
        "saimos da", "saimos do", "saimos de", "saimos para", "saimos até", "saimos ao", "saimos à",
        "saiu da", "saiu do", "saiu de", "deixa a", "deixa o", "deixo a", "deixo o", "deixando a", "deixando o",
        "entra na", "entra no", "entra em", "entro na", "entro no", "entro em", "entramos na", "entramos no",
        "chega na", "chega no", "chego na", "chego no", "chegamos na", "chegamos no", "chegou na", "chegou no",
        "segue para", "sigo para", "seguimos para", "dirige-se para", "dirijo-me para",
        "volta para", "volto para", "voltamos para", "retorna para", "retorno para",
        "indo para", "saindo da", "saindo do", "entrando na", "entrando no",
    ];
    if MOVEMENT_PREFIXES.iter().any(|prefix| contains_word_sequence(&sanitized, prefix)) {
        return true;
    }
    const TIME_JUMPS: &[&str] = &[
        "mais tarde", "no dia seguinte", "horas depois", "minutos depois",
        "na manhã seguinte", "à noite", "no outro dia", "dias depois",
    ];
    if TIME_JUMPS.iter().any(|jump| lower.contains(jump)) {
        return true;
    }
    if lower.contains("[cena:") || lower.contains("[local:") {
        return true;
    }
    false
}

fn contains_word_sequence(text: &str, phrase: &str) -> bool {
    let mut search_from = 0;
    while let Some(pos) = text[search_from..].find(phrase) {
        let abs_pos = search_from + pos;
        let before_ok = abs_pos == 0 || !text[..abs_pos].chars().last().map_or(false, |c| c.is_alphanumeric());
        let after_pos = abs_pos + phrase.len();
        let after_ok = after_pos == text.len() || !text[after_pos..].chars().next().map_or(false, |c| c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        if let Some(ch) = text[abs_pos..].chars().next() {
            search_from = abs_pos + ch.len_utf8();
        } else {
            break;
        }
    }
    false
}

fn detect_solo_movement(text: &str) -> bool {
    let lower = text.to_lowercase();
    const SOLO_INDICATORS: &[&str] = &[
        "sozinho", "sozinha", "sozinhos", "sozinhas",
        "a sós", "sem ninguém", "desacompanhado", "desacompanhada",
        "por conta própria", "sozinho.", "sozinha.",
    ];
    SOLO_INDICATORS.iter().any(|ind| lower.contains(ind))
}

fn accompanies_transition(char_name: &str, text: &str) -> bool {
    let lower_text = text.to_lowercase();
    let lower_name = char_name.to_lowercase();
    if !lower_text.contains(&lower_name) {
        return false;
    }
    let left_behind = [
        format!("deixo {lower_name}"),
        format!("deixa {lower_name}"),
        format!("deixando {lower_name}"),
        format!("sem {lower_name}"),
    ];
    if left_behind.iter().any(|p| lower_text.contains(p)) {
        return false;
    }

    let stay_indicators = [
        format!("{lower_name}, fique"),
        format!("{lower_name} fique"),
        format!("{lower_name}, fica"),
        format!("{lower_name} fica"),
        format!("{lower_name}, espere"),
        format!("{lower_name} espere"),
        format!("{lower_name}, espera"),
        format!("{lower_name} espera"),
        format!("{lower_name}, permaneça"),
        format!("{lower_name} permaneça"),
        format!("{lower_name}, permanece"),
        format!("{lower_name} permanece"),
        format!("fique aqui, {lower_name}"),
        format!("fica aqui, {lower_name}"),
        format!("espere aqui, {lower_name}"),
        format!("espera aqui, {lower_name}"),
        format!("{lower_name}, cuide"),
        format!("{lower_name}, vigie"),
    ];
    if stay_indicators.iter().any(|p| lower_text.contains(p)) {
        return false;
    }

    let words: Vec<&str> = lower_text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    let word_stream = words.join(" ");
    let normalized_stay = [
        format!("{lower_name} fique"),
        format!("{lower_name} fica"),
        format!("{lower_name} espere"),
        format!("{lower_name} espera"),
        format!("{lower_name} permaneça"),
        format!("{lower_name} permaneca"),
        format!("{lower_name} permanece"),
        format!("{lower_name} aguarde"),
        format!("{lower_name} vigie"),
        format!("{lower_name} cuide"),
        format!("fique aqui {lower_name}"),
        format!("fica aqui {lower_name}"),
        format!("espere aqui {lower_name}"),
        format!("espera aqui {lower_name}"),
    ];
    if normalized_stay.iter().any(|p| word_stream.contains(p)) {
        return false;
    }

    let companion_patterns = [
        format!("com {lower_name}"),
        format!("e {lower_name}"),
        format!("{lower_name} e eu"),
        format!("{lower_name} me acompanha"),
        format!("{lower_name} nos acompanha"),
        format!("{lower_name} me segue"),
        format!("{lower_name} vem"),
        format!("junto com {lower_name}"),
        format!("juntos com {lower_name}"),
        format!("levando {lower_name}"),
        format!("puxando {lower_name}"),
        format!("chamando {lower_name}"),
        format!("mão de {lower_name}"),
        format!("olha para {lower_name}"),
        format!("diz a {lower_name}"),
        format!("diz para {lower_name}"),
        format!("chama {lower_name}"),
        format!("pega {lower_name}"),
        format!("abraça {lower_name}"),
    ];
    if companion_patterns.iter().any(|p| lower_text.contains(p)) {
        return true;
    }
    let plural_movement = ["saímos", "vamos", "caminhamos", "fomos", "juntos", "juntas"];
    if plural_movement.iter().any(|m| lower_text.contains(m)) {
        return true;
    }
    false
}

/// (minimum, maximum, preferred) recent history pairs, clamped (8–50).
pub fn pair_limits(conv: &Value) -> (usize, usize, usize) {
    let settings = &conv["settings"]["context"];
    let maximum = settings["maxPairs"].as_u64().unwrap_or(50).clamp(8, 50) as usize;
    let minimum = settings["minPairs"]
        .as_u64()
        .unwrap_or(20)
        .clamp(8, maximum as u64) as usize;
    let preferred = settings["preferredPairs"]
        .as_u64()
        .unwrap_or(30)
        .clamp(minimum as u64, maximum as u64) as usize;
    (minimum, maximum, preferred)
}
/// How many recent pairs the store must load: the `assemble` ladder never goes past the preferred value, and
/// retrieval expands to the smallest coverage-checked window. Loading 50 by default only multiplied the exchanged JSON.
pub fn pairs_to_load(conv: &Value) -> usize {
    let (_, maximum, preferred) = pair_limits(conv);
    preferred.max(30).min(maximum)
}

/// Retrieval window start aligned strictly with the active history window cutoff.
/// Does not lock to 8; starts with preferred pairs (clamped by maxPairs) and only descends
/// to summarized rungs when history token volume exceeds softLimit.
pub fn retrieval_window_start(data: &Value) -> i64 {
    let pairs = array(&data["pairs"]);
    let (minimum, maximum, preferred) = pair_limits(&data["conv"]);
    let settings = &data["conv"]["settings"]["context"];
    let preset = crate::presets::by_id(&crate::s(&data["conv"], "presetId"));
    let preset_capacity = preset["contextWindow"].as_u64().unwrap_or(128000);
    let hard = settings["hardLimit"].as_u64().unwrap_or(preset_capacity) as usize;
    let soft = settings["softLimit"]
        .as_u64()
        .unwrap_or_else(|| ((hard as f64) * 0.85) as u64)
        .clamp(1, hard as u64) as usize;

    let available = &pairs[pairs.len().saturating_sub(maximum)..];
    let mut ladder = vec![
        preferred,
        preferred.saturating_sub(2).max(minimum),
        preferred.saturating_sub(5).max(minimum),
        minimum,
    ];
    if settings["adaptiveHistory"] != false {
        let summarized = [16, 12, 10, 8]
            .into_iter()
            .filter(|take| *take < minimum && summarized_before(data, available, *take));
        ladder.extend(summarized);
    }
    ladder.dedup();

    let mut selected_take = ladder.last().copied().unwrap_or(minimum).min(available.len());
    for &step in &ladder {
        let take = step.min(available.len());
        let slice = &available[available.len().saturating_sub(take)..];
        let estimated_history_tokens: usize = slice
            .iter()
            .map(|p| {
                let u = p["user"].as_str().unwrap_or_default().len();
                let a = p["assistant"].as_str().unwrap_or_default().len();
                (u + a) / 3 + 8
            })
            .sum();
        if estimated_history_tokens <= soft {
            selected_take = take;
            break;
        }
    }

    pairs
        .get(pairs.len().saturating_sub(selected_take))
        .map(|p| n(p, "seq"))
        .unwrap_or(n(&data["current"], "seq"))
}
pub fn assemble(
    data: &Value,
    retrieval: &Value,
    mode: &str,
    plan: &Plan,
    calibration: Option<f64>,
) -> Result<Context> {
    let conv = &data["conv"];
    let persona = conv["persona"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("Alex");
    let settings = &conv["settings"]["context"];
    let compact = plan.controls["mobileCompact"] == true;
    let (minimum, maximum, preferred) = if compact {
        (0, 6, 6)
    } else {
        pair_limits(conv)
    };
    let input_capacity = plan.capacity["contextWindow"]
        .as_u64()
        .unwrap_or(128000)
        .saturating_sub(plan.body["max_tokens"].as_u64().unwrap_or(0) + 64)
        .max(1);
    let hard_cap = if compact {
        input_capacity
    } else {
        input_capacity.max(128000)
    };
    let hard = settings["hardLimit"]
        .as_u64()
        .unwrap_or(hard_cap)
        .clamp(1, hard_cap) as usize;
    let soft = settings["softLimit"]
        .as_u64()
        .unwrap_or_else(|| ((hard as f64) * 0.85) as u64)
        .clamp(1, hard as u64) as usize;
    let target = settings["targetTokens"]
        .as_u64()
        .unwrap_or_else(|| ((soft as f64) * 0.87) as u64)
        .clamp(1, soft as u64) as usize;
    // The ladder below repeats up to ~10 times: everything it reads from `data`/`retrieval` is borrowed, not cloned.
    let pairs = array(&data["pairs"]);
    let available = &pairs[pairs.len().saturating_sub(maximum)..];
    let mut ladder = vec![
        preferred,
        preferred.saturating_sub(2).max(minimum),
        preferred.saturating_sub(5).max(minimum),
        minimum,
    ];
    if !compact && settings["adaptiveHistory"] != false {
        let summarized = [16, 12, 10, 8]
            .into_iter()
            .filter(|take| *take < minimum && summarized_before(data, available, *take));
        ladder.extend(summarized);
    }
    ladder.dedup();
    // global settings (character limit, dialogue × narration ratio, content text): same text as the former Node server
    let gs = &data["globalSettings"];
    let dialogue = gs["dialoguePct"].as_i64().unwrap_or(65).clamp(0, 100);
    static STYLE_EXAMPLES: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let style_examples = STYLE_EXAMPLES.get_or_init(|| {
        serde_json::from_str(include_str!("style-examples.json"))
            .expect("bundled style examples must be valid JSON")
    });
    let style = style_examples
        .as_array()
        .unwrap()
        .iter()
        .find(|s| dialogue <= s["max"].as_i64().unwrap())
        .unwrap();
    let style_detail = style["detail"].as_str().unwrap();
    // 0% and 100% do not allow the mixed format: rules and reminder follow the same restriction as the example and the check.
    let (format_rules, format_short) = match dialogue {
        0 => (
            "- Somente narração e ações em terceira pessoa, entre *asteriscos*, em parágrafos curtos. Não escreva falas diretas nem blocos \"Nome: fala\".
- Sem listas, sem títulos, sem marcações extras, sem emojis de enfeite.",
            "somente narração entre *asteriscos*",
        ),
        100 => (
            "- Somente falas em texto claro, sempre com o nome do personagem: \"Nome: fala\". Não escreva narração, ações entre asteriscos nem indicações de tom.
- Sem listas, sem títulos, sem marcações extras, sem emojis de enfeite.",
            "somente falas Nome: fala",
        ),
        _ => (
            "- Narração e ações em terceira pessoa, entre *asteriscos*, em parágrafos curtos.
- Falas em texto claro, sempre com o nome do personagem: \"Nome: fala\". Indicação de tom é opcional, entre parênteses, no fim da fala.
- Ações e falas podem se alternar livremente na mesma resposta. Sem listas, sem títulos, sem marcações extras, sem emojis de enfeite.",
            "formato *ação* / Nome: fala",
        ),
    };
    let bot_chars = gs["botChars"]
        .as_i64()
        .unwrap_or(rules::BOT_LIMIT as i64)
        .clamp(500, rules::BOT_LIMIT as i64);
    let content = gs["content"].as_str().unwrap_or_default().trim();
    let size_target = if rules::target_mode(gs) {
        let (minimum, stretch, hard) = rules::reply_bounds(gs);
        format!(
            "Tamanho alvo: em torno de {} unidades UTF-16 (faixa aceitável de {} a {}). É um alvo aproximado, não um teto: desenvolva a cena até concluí-la, sem enchimento, terminando numa frase completa. Nunca passe de {} unidades.",
            thousands(bot_chars),
            thousands(minimum as i64),
            thousands(stretch as i64),
            thousands(hard as i64)
        )
    } else {
        format!(
            "Tamanho máximo: até {} unidades UTF-16. Responda menos quando a reação já estiver completa; não preencha o limite nem prolongue a cena para atingir um mínimo.",
            thousands(bot_chars)
        )
    };
    let turn_control = style_policy::turn_instruction(gs);
    let token_hint = format!(
        "Planeje o texto antes de escrevê-lo: orçamento visível estimado de {} tokens (conversão aproximada de caracteres). Conclua as frases e a reação dentro do limite de caracteres; o orçamento da API pode incluir raciocínio e não é uma meta de tamanho.",
        (bot_chars + 2) / 3
    );
    let size_target = format!("{size_target} {token_hint}");
    let content_block = if content.is_empty() {
        String::new()
    } else {
        format!(
            "\n\n[DIRETRIZES GLOBAIS DE PERSONALIZAÇÃO DO CHAT — prioridade em todas as cenas]\nCONTEÚDO E EXPLICITUDE (definido pelo usuário; vale para toda a história):\n{content}"
        )
    };
    let temp = gs["temperature"].as_f64().unwrap_or(0.8);
    let creativity_rule = if temp < 0.6 {
        "SEGUIMENTO NARRATIVO E CRIATIVIDADE: Mantenha fidelidade estrita ao rumo estabelecido da história, aos fatos consolidados e às reações esperadas dos personagens, priorizando coerência, consistência e continuidade sobre reviravoltas repentinas."
    } else if temp > 0.95 {
        "SEGUIMENTO NARRATIVO E CRIATIVIDADE: Explore maior liberdade narrativa e criatividade no desenvolvimento da cena, trazendo reviravoltas orgânicas, detalhes inesperados e iniciativas ricas dos personagens, mantendo a coerência do mundo."
    } else {
        "SEGUIMENTO NARRATIVO E CRIATIVIDADE: Equilibre fidelidade à continuidade da narrativa com criatividade espontânea nas reações e desdobramentos da cena."
    };
    let core = include_str!("core-prompt.txt")
        .replace("{{persona}}", persona)
        .replace("{{PERSONA}}", &persona.to_uppercase())
        .replace(
            "{{style_lead}}",
            if dialogue == 0 {
                "Use somente narração"
            } else if dialogue == 100 {
                "Use somente falas"
            } else if dialogue >= 50 {
                "Priorize as falas dos personagens, respeitando o controle de turnos"
            } else {
                "Priorize a narração, com falas pontuais"
            },
        )
        .replace("{{dialogue}}", &dialogue.to_string())
        .replace("{{narration}}", &(100 - dialogue).to_string())
        .replace("{{format_rules}}", format_rules)
        .replace("{{format_example}}", style["example"].as_str().unwrap())
        .replace("{{style_detail}}", style_detail)
        .replace("{{creativity_rule}}", creativity_rule)
        .replace("{{size_target}}", &size_target)
        .replace("{{turn_control}}", &turn_control)
        .replace("{{limit}}", &thousands(rules::reply_bounds(gs).2 as i64))
        .replace("{{content_block}}", &content_block);
    let char_replacement = resolve_char_replacement(conv, data);
    let card = s(conv, "prompt")
        .replace("{{user}}", persona)
        .replace("{{char}}", &char_replacement);
    let notes = conv["personaProfile"]["description"]
        .as_str()
        .or(conv["settings"]["personaNotes"].as_str())
        .unwrap_or_default();
    let pov_matrix = build_pov_matrix(persona);
    let system = format!(
        "{core}\n\n[PERSONALIZAÇÃO DO PERSONAGEM E DO MUNDO — lei da história]\n{card}\n\n[PERSONA DO USUÁRIO]\nO usuário interpreta {persona}, controlado exclusivamente por ele.\n{notes}\n\n{pov_matrix}"
    );
    let current = &data["current"];
    let current_user = s(current, "userText");
    let continuation = mode == "continue"
        || mode == "regenerate"
            && current["priorParts"]
                .as_array()
                .is_some_and(|a| !a.is_empty());
    let scene_resolution = resolve_scene_presence(&data["scene"], &current_user, pairs, persona);
    let present = &scene_resolution.effective_present;
    // Attribution appears once, before the current message. IDs and full evidence
    // remain available to the local attribution/review pipeline.
    let recent_sources = crate::provenance::sources(pairs);
    let provenance = crate::provenance::block(&current_user, &recent_sources);
    let where_to = if continuation {
        "continue exatamente de onde a resposta parou"
    } else {
        "tudo na mensagem acima já aconteceu como escrito: comece DEPOIS dela, sem reescrevê-la"
    };
    // Size, ratio, examples and turn pacing are already in the stable system prefix.
    let global_anchor = if !content.is_empty() {
        "; obedeça estritamente às diretrizes globais de personalização e à ficha"
    } else {
        "; obedeça à ficha de personalização"
    };
    let tail = format!(
        "[Lembrete: pt-BR; {format_short}; {where_to}; não escreva nada por {persona}{global_anchor}; sem metagaming: personagens ausentes nos turnos anteriores NÃO sabem e NUNCA agem sobre o que foi dito ou ocorrido fora de sua presença (só passam a saber se forem expressamente informados em cena); desambiguação de posse e parentesco: siga a matriz de POV — pertences e familiares pertencem estritamente ao respectivo dono; use 'sua/seu' apenas ao falar com o dono, 'meu/minha' para si mesmo e nomes explícitos entre terceiros e na narração.]"
    );
    let mut pinned_ids = HashSet::new();
    let pinned = array(&data["pinned"])
        .iter()
        .filter(|m| unique_id(m, &mut pinned_ids))
        .take(12)
        .collect::<Vec<_>>();
    let (mut quota, mut secondary, mut timeline_mode, mut level) = (8usize, true, 0usize, 0usize);
    let mut compression = Vec::<String>::new();
    loop {
        let take = ladder[level].min(available.len());
        let selected = &available[available.len() - take..];
        let summarized_history = !compact && take < minimum;
        let window = selected
            .first()
            .map(|p| n(p, "seq"))
            .unwrap_or(n(current, "seq"));
        let uncovered = array(&data["unprocessed"])
            .iter()
            .filter(|p| n(p, "seq") < window)
            .collect::<Vec<&Value>>();
        let mut knowledge_ids = pinned_ids.clone();
        let memories = array(&retrieval["candidates"])
            .iter()
            .filter(|m| n(m, "sourceSeq") < window)
            .filter(|m| m["type"] == "memory")
            .filter(|m| unique_id(m, &mut knowledge_ids))
            .take(quota)
            .collect::<Vec<&Value>>();
        let mut knowledge = pinned.clone();
        knowledge.extend(memories.iter().copied());
        if secondary && !present.is_empty() {
            for m in array(&retrieval["privateMemories"])
                .iter()
                .filter(|m| n(m, "sourceSeq") < window)
                .filter(|m| unique_id(m, &mut knowledge_ids))
                .take(4)
            {
                knowledge.push(m);
            }
        }
        let mut live = Vec::new();
        let episodes = array(&data["episodes"])
            .iter()
            .filter(|e| n(e, "toSeq") < window)
            .collect::<Vec<&Value>>();
        let covered = episodes
            .iter()
            .flat_map(|e| array(&e["interactionIds"]).iter())
            .filter_map(Value::as_str)
            .collect::<HashSet<&str>>();
        // (from, to, text) of each timeline entry, still borrowed from `data`
        let mut timeline = episodes
            .iter()
            .map(|e| (&e["fromSeq"], &e["toSeq"], &e["summary"]))
            .collect::<Vec<_>>();
        timeline.extend(
            array(&data["notes"])
                .iter()
                .filter(|note| {
                    n(note, "seq") < window
                        && !covered.contains(note["interactionId"].as_str().unwrap_or_default())
                })
                .map(|note| (&note["seq"], &note["seq"], &note["summary"])),
        );
        timeline.sort_by_key(|t| t.0.as_i64().unwrap_or_default());
        let timeline_total = timeline.len();
        if !summarized_history && timeline_mode == 2 && timeline.len() > 8 {
            timeline = std::iter::once(timeline[0])
                .chain(timeline[timeline.len() - 6..].iter().copied())
                .collect();
        }
        if !timeline.is_empty() {
            live.push(format!(
                "[LINHA DO TEMPO ANTERIOR]\n{}",
                timeline
                    .iter()
                    .map(|(_, _, text)| {
                        let text = text.as_str().unwrap_or_default();
                        format!(
                            "- {}",
                            if timeline_mode == 0 || summarized_history {
                                text
                            } else {
                                rules::clip(text, 240)
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if !data["scene"].is_null() {
            if scene_resolution.transition_detected {
                let invalidation_clause = if !scene_resolution.invalidated_characters.is_empty() {
                    format!("; presença de ({}) na cena anterior foi invalidada", scene_resolution.invalidated_characters.join(", "))
                } else {
                    String::new()
                };
                live.push(format!(
                    "[CENA ATUAL — transição de cena detectada{invalidation_clause}]\n{}",
                    scene_view(&scene_resolution.effective_scene)
                ));
            } else {
                live.push(format!("[CENA ANTERIOR — referência possivelmente atrasada; transições recentes prevalecem]\n{}",scene_view(&data["scene"])));
            }
        }
        if !data["timeline"]["current"].is_null() {
            let t = &data["timeline"]["current"];
            // only what is needed (days, months, date and anchors); no boundaries/UUIDs
            live.push(format!(
                "[TEMPO NARRATIVO ANTERIOR — unidades independentes, sem converter meses em dias]\n{}",
                narrative_time(t)
            ));
        }
        if !provenance.is_empty() {
            live.push(provenance.clone());
        }
        let mut who = present
            .iter()
            .map(String::as_str)
            .collect::<Vec<&str>>();
        if !who.iter().any(|n| rules::key(n) == rules::key(persona)) {
            who.push(persona);
        }
        if !knowledge.is_empty() {
            let mut lines = vec!["[CONHECIMENTO POR PERSONAGEM — sem metagaming]".into()];
            for name in who {
                let name_key = rules::key(name);
                // Grouped by state (knows / believes / does NOT know): the label appears once per person,
                // not once per memory, without losing who knows what.
                let (mut knows, mut believes, mut unaware) = (Vec::new(), Vec::new(), Vec::new());
                for (i, m) in knowledge.iter().enumerate() {
                    let rows = m["knowledge"].as_array();
                    let k = rows
                        .into_iter()
                        .flatten()
                        .find(|k| {
                            let n = k.get("name").or_else(|| k.get("who")).and_then(Value::as_str).unwrap_or_default();
                            rules::key(n) == name_key
                        });
                    let kind = m["kind"].as_str().unwrap_or_default();
                    let subject = m["subject"].as_str().unwrap_or_default();
                    let has_source_turn = m.get("sourceSeq").and_then(Value::as_i64).is_some_and(|s| s > 0)
                        || m.get("sourceInteractionId").is_some();
                    let is_universal_world_fact = m["visibility"] == "public"
                        && !has_source_turn
                        && (kind == "WORLD_FACT" || rules::key(subject) == "mundo" || (kind == "fact" && rules::key(subject) == "mundo"));
                    let state = k.and_then(|k| k["state"].as_str()).unwrap_or(
                        if is_universal_world_fact {
                            "knows"
                        } else {
                            "unaware"
                        },
                    );
                    let item = format!(
                        "[M{}]{}",
                        i + 1,
                        k.and_then(|k| k["belief"].as_str())
                            .map(|b| format!(" ({b})"))
                            .unwrap_or_default()
                    );
                    match state {
                        "knows" => knows.push(item),
                        "believes" => believes.push(item),
                        _ => unaware.push(item),
                    }
                }
                let mut groups = Vec::new();
                for (label, items) in [
                    ("sabe", knows),
                    ("acredita (pode estar errado)", believes),
                    ("NÃO sabe e não pode agir sobre", unaware),
                ] {
                    if !items.is_empty() {
                        groups.push(format!("{label} {}", items.join(", ")));
                    }
                }
                lines.push(format!("{name}: {}", groups.join("; ")));
            }
            live.push(lines.join("\n"));
            live.push(format!(
                "[MEMÓRIAS RELEVANTES]\n{}",
                knowledge
                    .iter()
                    .enumerate()
                    .map(|(i, m)| {
                        let subject = s(m, "subject");
                        let content = disambiguate_memory_content(&subject, &s(m, "content"));
                        format!(
                            "[M{}] ({}{}) {}: {}",
                            i + 1,
                            if pinned_ids.contains(id_of(m)) {
                                "fixada, "
                            } else {
                                ""
                            },
                            s(m, "kind"),
                            subject,
                            content
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        let rels = array(&data["relationships"]);
        if !rels.is_empty() {
            live.push(format!(
                "[RELAÇÕES]\n{}",
                rels.iter()
                    .filter(|r| secondary
                        || present.iter().any(|p| p == s(r, "a").as_str() || p == s(r, "b").as_str())
                        || r["a"] == persona
                        || r["b"] == persona)
                    .take(if secondary { 12 } else { 6 })
                    .map(|r| {
                        let is_directed = r["directed"].as_bool().unwrap_or(false)
                            || r["direction"].as_str().is_some_and(|d| d == "a_to_b" || d == "forward" || d == "directed")
                            || r["kind"].as_str().is_some_and(|k| k == "kinship" || k == "directed");
                        let arrow = if is_directed { "→" } else { "↔" };
                        format!("{} {} {}: {}", s(r, "a"), arrow, s(r, "b"), s(r, "description"))
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        let threads = array(&data["threads"]);
        if !threads.is_empty() {
            live.push(format!(
                "[THREADS ABERTAS]\n{}",
                threads
                    .iter()
                    .take(8)
                    .map(|t| s(t, "text"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        let raw = if secondary {
            raw_excerpts(array(&retrieval["raw"]), selected, &uncovered, window)
        } else {
            Vec::new()
        };
        if !raw.is_empty() {
            live.push(format!(
                "[TRECHOS ORIGINAIS ANTIGOS CITADOS]\n{}",
                raw.iter()
                    .map(|r| r.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n")
            ));
        }
        let live = if live.is_empty() {
            String::new()
        } else {
            format!(
                "[CONTEXTO ATUAL DA HISTÓRIA — informação do narrador, não é fala de {persona}; use-o, não responda a ele]\n\n{}\n[FIM DO CONTEXTO]",
                live.join("\n\n")
            )
        };
        let mut messages = if plan.system_mode == "user_prefix" {
            vec![
                json!({"role":"user","content":format!("[INSTRUÇÕES DO SISTEMA — siga estritamente]\n{system}")}),
                json!({"role":"assistant","content":"Entendido. Seguirei estas instruções e o formato definido."}),
            ]
        } else {
            vec![json!({"role":"system","content":system})]
        };
        for pair in uncovered.iter().copied().chain(selected) {
            let present_val = if pair["present_entities"].is_array() {
                &pair["present_entities"]
            } else {
                &pair["present"]
            };
            let present_entities = array(present_val);
            let has_present = !present_entities.is_empty();
            let presence_tag = if has_present {
                let who = text_list(present_val);
                format!("[Presentes na cena: {who}]")
            } else {
                String::new()
            };

            if !pair["user"].is_null() {
                let user_text = s(pair, "user");
                let content = if has_present && !user_text.starts_with("[Presentes") {
                    if user_text.is_empty() {
                        presence_tag
                    } else {
                        format!("{presence_tag}\n{user_text}")
                    }
                } else {
                    user_text.to_string()
                };
                messages.push(json!({"role":"user","content":content}));
            } else if has_present {
                messages.push(json!({"role":"user","content":presence_tag}));
            }
            if pair["assistant"].as_str().is_some_and(|s| !s.is_empty()) {
                messages.push(json!({"role":"assistant","content":pair["assistant"]}));
            }
        }
        if !current_user.is_empty() {
            messages.push(json!({"role":"user","content":current_user}));
        }
        if continuation {
            let parts = array(&current["priorParts"]);
            if !parts.is_empty() {
                messages.push(json!({"role":"assistant","content":parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n\n")}));
            }
            messages.push(json!({"role":"user","content":CONTINUE}));
        }
        let head = if live.is_empty() {
            String::new()
        } else {
            format!("{live}\n\n")
        };
        if let Some(last) = messages.last_mut().filter(|m| m["role"] == "user") {
            last["content"] = json!(format!("{head}{}\n\n{tail}", s(last, "content")));
        } else {
            messages.push(json!({"role":"user","content":format!("{head}{tail}")}));
        }
        let mut merged = Vec::<Value>::new();
        for message in messages {
            if let Some(last) = merged
                .last_mut()
                .filter(|m| m["role"] == message["role"] && m["role"] != "system")
            {
                last["content"] = json!(format!(
                    "{}\n\n{}",
                    s(last, "content"),
                    s(&message, "content")
                ));
            } else {
                merged.push(message);
            }
        }
        let raw_estimate = rules::estimate_messages(&merged);
        let total = rules::budget_raw(raw_estimate, calibration);
        if total > target {
            if quota > 6 {
                quota = 6;
                compression.push("recuperação reduzida para 6 memórias".into());
                continue;
            }
            if quota > 4 {
                quota = 4;
                compression.push("recuperação reduzida para 4 memórias".into());
                continue;
            }
            if timeline_mode == 0 {
                timeline_mode = 1;
                compression.push("linha do tempo condensada".into());
                continue;
            }
            if secondary {
                secondary = false;
                compression.push("informações secundárias removidas".into());
                continue;
            }
            if quota > 3 {
                quota = 3;
                compression.push("recuperação reduzida para 3 memórias".into());
                continue;
            }
            if timeline_mode == 1 {
                timeline_mode = 2;
                compression.push("linha do tempo reduzida".into());
                continue;
            }
        }
        if total > soft && level + 1 < ladder.len() && ladder[level + 1].min(available.len()) < take
        {
            level += 1;
            let next_take = ladder[level];
            if next_take < minimum {
                compression.push(format!(
                    "histórico resumido por episódios verificados; {} pares recentes",
                    next_take
                ));
            } else {
                compression.push(format!(
                    "histórico recente reduzido para {} pares",
                    next_take
                ));
            }
            continue;
        }
        let report = json!({"tokens":{"total":total,"system":rules::budget(&system,calibration),"live":rules::budget(&live,calibration),"history":total.saturating_sub(rules::budget(&system,calibration)+rules::budget(&live,calibration)),"method":if calibration.is_some(){"calibrated_estimate"}else{"estimated"},"calibrated":calibration.is_some()},"limits":{"target":target,"soft":soft,"hard":hard},"pairsIncluded":take,"pairsAvailable":data["totalPairs"],"memoriesIncluded":memories.iter().map(|m|m["id"].clone()).collect::<Vec<_>>(),"episodesIncluded":episodes.iter().map(|e|e["id"].clone()).collect::<Vec<_>>(),"timelineItems":timeline_total,"pinnedIncluded":pinned.len(),"rawIncluded":raw.len(),"compression":compression,"pendingTurnsPreserved":uncovered.len(),"retrievalMethod":retrieval["method"].as_str().unwrap_or("none"),"embeddingModel":retrieval["embeddingModel"],"sync":data["sync"]});
        let reserve = plan.body["max_tokens"].as_u64().unwrap_or(0) as usize;
        let capacity = plan.capacity["contextWindow"].as_u64().unwrap_or(128000) as usize;
        // Heuristic budgets describe the prompt; the provider enforces its actual window.
        let mut report = report;
        report["overHardLimit"] = json!(total > hard);
        report["overCapacity"] = json!(total.saturating_add(reserve) > capacity);
        report["exceededEstimatedBudget"] = json!(report["overHardLimit"] == true || report["overCapacity"] == true);
        report["limits"]["capacity"] = json!(capacity);
        report["limits"]["responseReserve"] = json!(reserve);
        return Ok(Context {
            messages: merged,
            report,
            raw_estimate,
            selected_history_len: take,
        });
    }
}

/// Items of a JSON array (empty when absent or not an array), borrowed.
fn array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
/// `id` as text (empty when absent), the way `s(m, "id")` reads it, without allocating.
fn id_of(m: &Value) -> &str {
    m["id"].as_str().unwrap_or_default()
}

fn unique_id<'a>(m: &'a Value, seen: &mut HashSet<&'a str>) -> bool {
    let id = id_of(m);
    id.is_empty() || seen.insert(id)
}

struct RawExcerpt<'a> {
    source: &'a Value,
    range: Option<(usize, usize)>,
    text: String,
}

fn raw_excerpts<'a>(
    raw: &'a [Value],
    selected: &[Value],
    pending: &[&Value],
    window: i64,
) -> Vec<RawExcerpt<'a>> {
    let mut interactions = HashSet::new();
    let mut sources = HashSet::new();
    for pair in pending.iter().copied().chain(selected) {
        if !id_of(pair).is_empty() {
            interactions.insert(id_of(pair));
            sources.insert(id_of(pair)); // The user source is the interaction itself.
        }
        sources.extend(
            array(&pair["sources"])
                .iter()
                .map(id_of)
                .filter(|id| !id.is_empty()),
        );
        sources.extend(array(&pair["versions"]).iter().filter_map(Value::as_str));
    }
    let mut out: Vec<RawExcerpt<'a>> = Vec::new();
    for r in raw {
        if r["sourceSeq"]
            .as_i64()
            .or(r["seq"].as_i64())
            .is_some_and(|seq| seq >= window)
            || ["sourceInteractionId", "interactionId"]
                .iter()
                .any(|key| r[key].as_str().is_some_and(|id| interactions.contains(id)))
            || r["sourceId"]
                .as_str()
                .is_some_and(|id| sources.contains(id))
        {
            continue;
        }
        let text = s(r, "text");
        if text.is_empty() {
            continue;
        }
        let range = r["start"]
            .as_u64()
            .zip(r["end"].as_u64())
            .and_then(|(a, b)| {
                let (a, b) = (usize::try_from(a).ok()?, usize::try_from(b).ok()?);
                (b.checked_sub(a)? == rules::utf16_len(&text)).then_some((a, b))
            });
        out.push(RawExcerpt {
            source: r,
            range,
            text,
        });
    }
    // Retrieval order is priority order. A merged interval keeps its first slot;
    // repeated passes also join a third chunk bridging two earlier intervals.
    let mut i = 0;
    while i < out.len() {
        let mut j = i + 1;
        while j < out.len() {
            if let Some((range, text)) = merge_raw(&out[i], &out[j]) {
                out[i].range = Some(range);
                out[i].text = text;
                out.remove(j);
                j = i + 1;
            } else {
                j += 1;
            }
        }
        i += 1;
    }
    out
}

fn merge_raw(a: &RawExcerpt<'_>, b: &RawExcerpt<'_>) -> Option<((usize, usize), String)> {
    let source = a.source["sourceId"].as_str().filter(|s| !s.is_empty())?;
    let role = a.source["role"].as_str().filter(|s| !s.is_empty())?;
    if b.source["sourceId"] != source
        || b.source["role"] != role
        || a.source["canonicalSpeaker"] != b.source["canonicalSpeaker"]
    {
        return None;
    }
    let (mut left, mut right) = (a, b);
    let ((mut start, mut end), (mut other_start, mut other_end)) = (a.range?, b.range?);
    if other_start < start {
        std::mem::swap(&mut left, &mut right);
        std::mem::swap(&mut start, &mut other_start);
        std::mem::swap(&mut end, &mut other_end);
    }
    let overlap_end = end.min(other_end);
    if other_start >= overlap_end {
        return None;
    }
    // suffix rejects offsets in the middle of a UTF-16 surrogate pair.
    let overlap = rules::suffix(&left.text, other_start - start)?;
    let tail = rules::suffix(&left.text, overlap_end - start)?;
    let other_tail = rules::suffix(&right.text, overlap_end - other_start)?;
    if overlap[..overlap.len() - tail.len()] != right.text[..right.text.len() - other_tail.len()] {
        return None;
    }
    let mut text = left.text.clone();
    if other_end > end {
        text.push_str(rules::suffix(&right.text, end - other_start)?);
    }
    Some(((start, end.max(other_end)), text))
}

/// A smaller window is eligible only if every omitted pair has a nonempty,
/// already consolidated episode on this branch, with no still-pending turn.
fn summarized_before(data: &Value, pairs: &[Value], take: usize) -> bool {
    if pairs.len() <= take {
        return false;
    }
    let split = pairs.len() - take;
    let window = n(&pairs[split], "seq");
    let covered = array(&data["episodes"])
        .iter()
        .filter(|e| {
            n(e, "toSeq") < window && e["summary"].as_str().is_some_and(|s| !s.trim().is_empty())
        })
        .flat_map(|e| array(&e["interactionIds"]).iter().filter_map(Value::as_str))
        .collect::<HashSet<_>>();
    let pending = array(&data["unprocessed"])
        .iter()
        .map(id_of)
        .collect::<HashSet<_>>();
    pairs[..split]
        .iter()
        .all(|p| !id_of(p).is_empty() && covered.contains(id_of(p)) && !pending.contains(id_of(p)))
}

fn text_list(v: &Value) -> String {
    array(v)
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Prompt-only view; serialized state and all internal references stay unchanged.
fn scene_view(scene: &Value) -> String {
    let mut lines = Vec::new();
    for (key, label) in [
        ("location", "Local"),
        ("time", "Momento"),
        ("mood", "Clima"),
    ] {
        if let Some(text) = scene[key].as_str().filter(|s| !s.is_empty()) {
            lines.push(format!("{label}: {text}"));
        }
    }
    for (key, label) in [("present", "Presentes"), ("objects", "Objetos")] {
        let text = text_list(&scene[key]);
        if !text.is_empty() {
            lines.push(format!("{label}: {text}"));
        }
    }
    for c in array(&scene["characters"]) {
        let mut facts = Vec::new();
        for (key, label) in [("location", "local"), ("position", "posição")] {
            if let Some(text) = c[key].as_str().filter(|s| !s.is_empty()) {
                facts.push(format!("{label}: {text}"));
            }
        }
        let holding = text_list(&c["holding"]);
        if !holding.is_empty() {
            facts.push(format!("segura: {holding}"));
        }
        let possessions = text_list(
            if c.get("possessions").is_some() {
                &c["possessions"]
            } else {
                &c["inventory"]
            },
        );
        if !possessions.is_empty() {
            facts.push(format!("pertences: {possessions}"));
        }
        if !facts.is_empty() {
            lines.push(format!("{} — {}", s(c, "name"), facts.join("; ")));
        }
    }
    lines.join("\n")
}

fn narrative_time(time: &Value) -> String {
    let mut lines = vec![format!(
        "Decorridos: {} dias e {} meses (contagens independentes).",
        n(time, "days"),
        n(time, "months")
    )];
    if let Some(date) = time["date"].as_str() {
        lines.push(format!("Data: {date}"));
    }
    for anchor in array(&time["anchors"]) {
        let evidence = s(anchor, "evidence");
        let days = anchor["elapsedDays"]
            .as_i64()
            .or(anchor["days"].as_i64())
            .unwrap_or(0);
        let months = anchor["elapsedMonths"]
            .as_i64()
            .or(anchor["months"].as_i64())
            .unwrap_or(0);
        lines.push(format!(
            "Âncora: {evidence}; intervalo atual: {days} dias e {months} meses (independentes)."
        ));
    }
    lines.join("\n")
}

/// 8000 -> "8.000" (pt-BR thousands separator).
fn thousands(n: i64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// Builds the 4-level Point-of-View (POV) matrix for narrative perspective and entity possession disambiguation.
pub fn build_pov_matrix(persona: &str) -> String {
    format!(
r#"[MATRIZ DE PONTO DE VISTA (POV) E DESAMBIGUAÇÃO DE POSSE — LEI DE PERSPECTIVA]
Esta matriz governa rigorosamente o uso de pronomes possessivos e a atribuição de parentesco e pertences em todas as falas e narrações:

1. NÍVEL 1 — NARRAÇÃO E AÇÕES (3ª Pessoa Nominativa Estrita):
   - O narrador DEVE usar nomes próprios explícitos para atribuir posse e parentesco: "a mãe de {persona}", "o carro de Lia", "a espada de Kael".
   - NUNCA use possessivos ambíguos ("seu/sua/dele/dela") na narração entre *asteriscos* quando houver dúvida sobre quem é o dono. Escreva sempre a quem pertence de forma nominativa: "o casaco de {persona}", "a própria espada".

2. NÍVEL 2 — PERSONAGEM FALANDO COM {persona} (2ª Pessoa Direta):
   - Ao se referir aos pertences ou familiares de {persona}: use 2ª pessoa direta ("você", "seu/sua", "sua mãe", "seu carro").
   - Ao se referir aos PRÓPRIOS pertences ou familiares do personagem que está falando: use 1ª pessoa ("eu", "meu/minha", "minha mãe", "meu casaco"). Exemplo: se Lia fala de sua própria mãe para {persona}, Lia diz "minha mãe", JAMAIS "sua mãe".
   - Ao se referir a terceiros (outros personagens): use 3ª pessoa nominativa ("a irmã de Carlos", "a mochila de Kael").

3. NÍVEL 3 — DIÁLOGO ENTRE PERSONAGENS (NPC falando com outro NPC / Dinâmica Coral):
   - Quando um NPC conversa com outro NPC (ex: Lia falando com Kael):
     * Ao mencionar {persona} ou a família/bens de {persona}: use OBRIGATORIAMENTE 3ª pessoa nominativa ("a mãe de {persona}", "o carro de {persona}"). É TERMINANTEMENTE PROIBIDO usar "sua mãe" com outro NPC para se referir à família de {persona}, pois isso atribuiria falsamente a mãe ao NPC ouvinte.
     * Ao mencionar a família ou bens do NPC ouvinte: use 2ª pessoa ("seu pai", "sua espada").
     * Ao mencionar a própria família ou bens do falante: use 1ª pessoa ("minha irmã", "meu arco").

4. NÍVEL 4 — AUTORREFERÊNCIA DO FALANTE (1ª Pessoa do Personagem):
   - Cada personagem é dono exclusivo de sua própria 1ª pessoa ("eu", "meu", "minha", "meus", "minhas").
   - O usuário ({persona}) é o único dono da 1ª pessoa do jogador.
   - Nenhum personagem assume os familiares ou pertences de outrem como se fossem seus."#
    )
}

/// Disambiguates and normalizes 1st person possessive pronouns in memory content
/// into 3rd person nominative forms anchored to the subject entity.
pub fn disambiguate_memory_content(subject: &str, content: &str) -> String {
    let trimmed = content.trim();
    if subject.is_empty() || subject.eq_ignore_ascii_case("mundo") {
        return trimmed.to_string();
    }

    const PREFIXES: &[(&str, &str, &str)] = &[
        ("minha mãe ", "minha mae ", "A mãe de"),
        ("meu pai ", "meu pai ", "O pai de"),
        ("meu irmão ", "meu irmao ", "O irmão de"),
        ("minha irmã ", "minha irma ", "A irmã de"),
        ("minha família ", "minha familia ", "A família de"),
    ];

    for (with_acc, without_acc, prefix_out) in PREFIXES {
        if let Some(rest) = strip_prefix_ci(trimmed, with_acc)
            .or_else(|| strip_prefix_ci(trimmed, without_acc))
        {
            return format!("{prefix_out} {subject} {rest}");
        }
    }

    if let Some(rest) = strip_prefix_ci(trimmed, "meu ") {
        if let Some((first_word, after)) = rest.split_once(' ') {
            return format!("O {first_word} de {subject} {after}");
        } else {
            return format!("O {rest} de {subject}");
        }
    }

    if let Some(rest) = strip_prefix_ci(trimmed, "minha ") {
        if let Some((first_word, after)) = rest.split_once(' ') {
            return format!("A {first_word} de {subject} {after}");
        } else {
            return format!("A {rest} de {subject}");
        }
    }

    trimmed.to_string()
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let lower_text = text.to_lowercase();
    let lower_prefix = prefix.to_lowercase();
    if lower_text.starts_with(&lower_prefix) {
        let char_count = prefix.chars().count();
        let mut indices = text.char_indices();
        let split_byte = indices.nth(char_count).map(|(i, _)| i).unwrap_or(text.len());
        Some(&text[split_byte..])
    } else {
        None
    }
}

/// Resolves the non-destructive replacement for `{{char}}` in character cards.
/// If a character name is found in the conversation metadata or cast, it is used;
/// otherwise, a singular contextual descriptor ("o personagem") is used instead of
/// the destructive "o narrador e os personagens" which corrupts single-character card definitions.
pub fn resolve_char_replacement(conv: &Value, data: &Value) -> String {
    if let Some(name) = conv["characterName"].as_str().filter(|s| !s.trim().is_empty()) {
        return name.trim().to_owned();
    }
    if let Some(name) = conv["settings"]["characterName"].as_str().filter(|s| !s.trim().is_empty()) {
        return name.trim().to_owned();
    }
    if let Some(name) = conv["character"].as_str().filter(|s| !s.trim().is_empty()) {
        return name.trim().to_owned();
    }
    if let Some(cast) = data["cast"].as_array() {
        for entry in cast {
            let kind = entry["kind"].as_str().unwrap_or_default();
            if kind == "main" || kind == "character" || kind == "npc" {
                if let Some(name) = entry["name"].as_str().filter(|s| !s.trim().is_empty()) {
                    return name.trim().to_owned();
                }
            }
        }
    }
    if let Some(name) = conv["name"].as_str().filter(|s| !s.trim().is_empty()) {
        let trimmed = name.trim();
        let lower = trimmed.to_lowercase();
        if !lower.starts_with("chat") && !lower.starts_with("conversa") && !lower.starts_with("nova ") && lower != "novo chat" {
            return trimmed.to_owned();
        }
    }
    "o personagem".to_owned()
}
