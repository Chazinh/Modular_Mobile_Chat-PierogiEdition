//! Context assembly with a mandatory raw-history floor and explicit token estimates.
use crate::{Error, Result, n, presets::Plan, rules, s, style_policy};
use serde_json::{Value, json};
use std::collections::HashSet;

pub const CONTINUE: &str = "Continue a cena exatamente de onde parou, no mesmo formato, sem repetir o que já foi escrito e sem agir por mim.";
pub struct Context {
    pub messages: Vec<Value>,
    pub report: Value,
    pub raw_estimate: usize,
}
/// (minimum, maximum, preferred) recent history pairs, already clamped (20–50).
pub fn pair_limits(conv: &Value) -> (usize, usize, usize) {
    let settings = &conv["settings"]["context"];
    let maximum = settings["maxPairs"].as_u64().unwrap_or(50).clamp(20, 50) as usize;
    let minimum = settings["minPairs"]
        .as_u64()
        .unwrap_or(20)
        .clamp(20, maximum as u64) as usize;
    let preferred = settings["preferredPairs"]
        .as_u64()
        .unwrap_or(30)
        .clamp(minimum as u64, maximum as u64) as usize;
    (minimum, maximum, preferred)
}
/// How many recent pairs the store must load: the `assemble` ladder never goes past the preferred value, and
/// retrieval still measures the window over 30 pairs. Loading the maximum of 50 only multiplied the exchanged JSON.
pub fn pairs_to_load(conv: &Value) -> usize {
    let (_, maximum, preferred) = pair_limits(conv);
    preferred.max(30).min(maximum)
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
    let (minimum, maximum, preferred) = pair_limits(conv);
    let hard = settings["hardLimit"]
        .as_u64()
        .unwrap_or(128000)
        .clamp(1, 128000) as usize;
    let soft = settings["softLimit"]
        .as_u64()
        .unwrap_or(110000)
        .clamp(1, hard as u64) as usize;
    let target = settings["targetTokens"]
        .as_u64()
        .unwrap_or(96000)
        .clamp(1, soft as u64) as usize;
    let pairs = data["pairs"].as_array().cloned().unwrap_or_default();
    let available = &pairs[pairs.len().saturating_sub(maximum)..];
    let mut ladder = vec![
        preferred,
        preferred.saturating_sub(2).max(minimum),
        preferred.saturating_sub(5).max(minimum),
        minimum,
    ];
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
            "\n\nCONTEÚDO E EXPLICITUDE (definido pelo usuário; vale para toda a história):\n{content}"
        )
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
        .replace("{{size_target}}", &size_target)
        .replace("{{turn_control}}", &turn_control)
        .replace("{{limit}}", &thousands(rules::reply_bounds(gs).2 as i64))
        .replace("{{content_block}}", &content_block);
    let card = s(conv, "prompt")
        .replace("{{user}}", persona)
        .replace("{{char}}", "o narrador e os personagens");
    let notes = conv["personaProfile"]["description"]
        .as_str()
        .or(conv["settings"]["personaNotes"].as_str())
        .unwrap_or_default();
    let system = format!(
        "{core}\n\n[PERSONALIZAÇÃO DO PERSONAGEM E DO MUNDO — lei da história]\n{card}\n\n[PERSONA DO USUÁRIO]\nO usuário interpreta {persona}, controlado exclusivamente por ele.\n{notes}"
    );
    let current = &data["current"];
    let current_user = s(current, "userText");
    let continuation = mode == "continue"
        || mode == "regenerate"
            && current["priorParts"]
                .as_array()
                .is_some_and(|a| !a.is_empty());
    let present = data["scene"]["present"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    // Same assembly as the former Node server: live state BEFORE the current message; AFTER it, the deterministic reading of
    // quotes (exact matches only) and the reminder. The provenance JSON goes in the live state.
    let recent_sources = crate::provenance::sources(&pairs);
    let provenance = crate::provenance::block(&current_user, &recent_sources);
    let reading = if continuation || current_user.is_empty() {
        String::new()
    } else {
        crate::provenance::reading(&current_user, &recent_sources)
    };
    let where_to = if continuation {
        "continue exatamente de onde a resposta parou"
    } else {
        "tudo na mensagem acima já aconteceu como escrito: comece DEPOIS dela, sem reescrevê-la"
    };
    // The size/turn budget goes BEFORE the current message: the model reads the limits first and composes the reply
    // inside them, instead of discovering the ceiling only after reading (and mentally drafting) the turn.
    let plan_block = format!(
        "[PLANO DA RESPOSTA — leia antes da mensagem abaixo e monte a resposta dentro destes limites desde a primeira linha]\n{size_target}\nComposição: {dialogue}% falas / {}% narração. {style_detail}\n{turn_control}\nDecida primeiro quantas reações e falas cabem neste orçamento; depois escreva, terminando numa frase completa antes do teto.\n[FIM DO PLANO]",
        100 - dialogue
    );
    let anchor = format!(
        "[Lembrete: responda integralmente em português do Brasil (pt-BR), sem alternar idiomas; {format_short}; {where_to}; preserve autor original de citações/imitações e separe comentários; não escreva nada por {persona}; siga o PLANO DA RESPOSTA acima (tamanho, composição {dialogue}/{} e turnos).]",
        100 - dialogue
    );
    let tail = if reading.is_empty() {
        anchor
    } else {
        format!("{reading}\n{anchor}")
    };
    let pinned = data["pinned"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .take(12)
        .collect::<Vec<_>>();
    let pinned_ids: HashSet<_> = pinned.iter().map(|m| s(m, "id")).collect();
    let (mut quota, mut secondary, mut timeline_mode, mut level) = (8usize, true, 0usize, 0usize);
    let mut compression = Vec::<String>::new();
    loop {
        let take = ladder[level].min(available.len());
        let selected = &available[available.len() - take..];
        let window = selected
            .first()
            .map(|p| n(p, "seq"))
            .unwrap_or(n(current, "seq"));
        let candidates = retrieval["candidates"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| n(m, "sourceSeq") < window && !pinned_ids.contains(&s(m, "id")))
            .collect::<Vec<_>>();
        let memories = candidates
            .iter()
            .filter(|m| m["type"] == "memory")
            .take(quota)
            .cloned()
            .collect::<Vec<_>>();
        let mut knowledge = pinned.clone();
        knowledge.extend(memories.iter().cloned());
        if secondary && !present.is_empty() {
            for m in retrieval["privateMemories"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|m| n(m, "sourceSeq") < window)
                .take(4)
            {
                if !knowledge.iter().any(|k| k["id"] == m["id"]) {
                    knowledge.push(m.clone());
                }
            }
        }
        let mut live = Vec::new();
        let episodes = data["episodes"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|e| n(e, "toSeq") < window)
            .collect::<Vec<_>>();
        let covered = episodes
            .iter()
            .flat_map(|e| {
                e["interactionIds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
            })
            .collect::<HashSet<_>>();
        let mut timeline = episodes
            .iter()
            .map(|e| json!({"fromSeq":e["fromSeq"],"toSeq":e["toSeq"],"text":e["summary"]}))
            .collect::<Vec<_>>();
        timeline.extend(data["notes"].as_array().into_iter().flatten().filter(|note|n(note,"seq")<window&&!covered.contains(&s(note,"interactionId"))).map(|note|json!({"fromSeq":note["seq"],"toSeq":note["seq"],"text":note["summary"]})));
        timeline.sort_by_key(|t| n(t, "fromSeq"));
        let timeline_total = timeline.len();
        if timeline_mode == 2 && timeline.len() > 8 {
            timeline = std::iter::once(timeline[0].clone())
                .chain(timeline[timeline.len() - 6..].iter().cloned())
                .collect();
        }
        if !timeline.is_empty() {
            live.push(format!(
                "[LINHA DO TEMPO ANTERIOR]\n{}",
                timeline
                    .iter()
                    .map(|t| format!(
                        "- turnos {}–{}: {}",
                        t["fromSeq"],
                        t["toSeq"],
                        if timeline_mode == 0 {
                            s(t, "text")
                        } else {
                            rules::clip(&s(t, "text"), 240).to_owned()
                        }
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if !data["scene"].is_null() {
            live.push(format!("[CENA ANTERIOR — referência possivelmente atrasada; transições recentes prevalecem]\n{}",scene_view(&data["scene"])));
        }
        if !data["timeline"]["current"].is_null() {
            let t = &data["timeline"]["current"];
            // only what is needed (days, months, date and anchors); no boundaries/UUIDs
            live.push(format!(
                "[TEMPO NARRATIVO ANTERIOR — unidades independentes, sem converter meses em dias]\n{}",
                json!({"days":t["days"],"months":t["months"],"date":t["date"],"anchors":t["anchors"]})
            ));
        }
        if !provenance.is_empty() {
            live.push(provenance.clone());
        }
        let mut who = present
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if !who.iter().any(|n| rules::key(n) == rules::key(persona)) {
            who.push(persona.into());
        }
        if !knowledge.is_empty() {
            let mut lines = vec!["[CONHECIMENTO POR PERSONAGEM — sem metagaming]".into()];
            for name in who {
                // Grouped by state (knows / believes / does NOT know): the label appears once per person,
                // not once per memory, without losing who knows what.
                let (mut knows, mut believes, mut unaware) = (Vec::new(), Vec::new(), Vec::new());
                for (i, m) in knowledge.iter().enumerate() {
                    let rows = m["knowledge"].as_array();
                    let k = rows.into_iter().flatten().find(|k| {
                        rules::key(k["name"].as_str().unwrap_or_default()) == rules::key(&name)
                    });
                    let state = k.and_then(|k| k["state"].as_str()).unwrap_or(
                        if m["visibility"] == "public" {
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
                    .map(|(i, m)| format!(
                        "[M{}] ({}{}) {}: {}",
                        i + 1,
                        if pinned_ids.contains(&s(m, "id")) {
                            "fixada, "
                        } else {
                            ""
                        },
                        s(m, "kind"),
                        s(m, "subject"),
                        s(m, "content")
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        let rels = data["relationships"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !rels.is_empty() {
            live.push(format!(
                "[RELAÇÕES]\n{}",
                rels.iter()
                    .filter(|r| secondary
                        || present.iter().any(|p| p == &r["a"] || p == &r["b"])
                        || r["a"] == persona
                        || r["b"] == persona)
                    .take(if secondary { 12 } else { 6 })
                    .map(|r| format!("{} ↔ {}: {}", s(r, "a"), s(r, "b"), s(r, "description")))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        let threads = data["threads"].as_array().cloned().unwrap_or_default();
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
        if secondary {
            let raw = retrieval["raw"].as_array().cloned().unwrap_or_default();
            if !raw.is_empty() {
                live.push(format!(
                    "[TRECHOS ORIGINAIS ANTIGOS CITADOS]\n{}",
                    raw.iter()
                        .map(|r| s(r, "text"))
                        .collect::<Vec<_>>()
                        .join("\n\n")
                ));
            }
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
        let uncovered = data["unprocessed"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| n(p, "seq") < window)
            .collect::<Vec<_>>();
        for pair in uncovered.iter().chain(selected) {
            if !pair["user"].is_null() {
                messages.push(json!({"role":"user","content":pair["user"]}));
            }
            if pair["assistant"].as_str().is_some_and(|s| !s.is_empty()) {
                messages.push(json!({"role":"assistant","content":pair["assistant"]}));
            }
        }
        if !current_user.is_empty() {
            messages.push(json!({"role":"user","content":current_user}));
        }
        if continuation {
            let parts = current["priorParts"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if !parts.is_empty() {
                messages.push(json!({"role":"assistant","content":parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n\n")}));
            }
            messages.push(json!({"role":"user","content":CONTINUE}));
        }
        let head = if live.is_empty() {
            format!("{plan_block}\n\n")
        } else {
            format!("{live}\n\n{plan_block}\n\n")
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
            compression.push(format!(
                "histórico recente reduzido para {} pares",
                ladder[level]
            ));
            continue;
        }
        let report = json!({"tokens":{"total":total,"system":rules::budget(&system,calibration),"live":rules::budget(&live,calibration),"history":total.saturating_sub(rules::budget(&system,calibration)+rules::budget(&live,calibration)),"method":if calibration.is_some(){"calibrated_estimate"}else{"estimated"},"calibrated":calibration.is_some()},"limits":{"target":target,"soft":soft,"hard":hard},"pairsIncluded":take,"pairsAvailable":data["totalPairs"],"memoriesIncluded":memories.iter().map(|m|m["id"].clone()).collect::<Vec<_>>(),"episodesIncluded":episodes.iter().map(|e|e["id"].clone()).collect::<Vec<_>>(),"timelineItems":timeline_total,"pinnedIncluded":pinned.len(),"rawIncluded":if secondary{retrieval["raw"].as_array().map(Vec::len).unwrap_or(0)}else{0},"compression":compression,"pendingTurnsPreserved":uncovered.len(),"retrievalMethod":retrieval["method"].as_str().unwrap_or("none"),"embeddingModel":retrieval["embeddingModel"],"sync":data["sync"]});
        let reserve = plan.body["max_tokens"].as_u64().unwrap_or(0) as usize;
        let capacity = plan.capacity["contextWindow"].as_u64().unwrap_or(128000) as usize;
        if total > hard || total + reserve > capacity {
            return Err(Error::new(422,format!("O contexto obrigatório estimado em {total} tokens excede o orçamento. Nada foi enviado ou gravado. Aguarde a memória pendente ou reduza a personalização.")).extra("code","CONTEXT_TOO_LARGE").extra("report",report));
        }
        return Ok(Context {
            messages: merged,
            report,
            raw_estimate,
        });
    }
}

/// Scene without what only the server needs: the interaction ID and each character's normalized key
/// (which repeats the name) spent tokens on every turn without telling the narrator anything.
fn scene_view(scene: &Value) -> Value {
    let mut view = scene.clone();
    if let Some(map) = view.as_object_mut() {
        map.remove("interactionId");
        for c in map
            .get_mut("characters")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if let Some(c) = c.as_object_mut() {
                c.remove("key");
            }
        }
    }
    view
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
