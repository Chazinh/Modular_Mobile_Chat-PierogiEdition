use crate::rules;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::LazyLock;

static SPEECH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\p{Lu}[\p{L}\p{M}'’.\-]*(?:\s+(?:(?:de|da|do|das|dos|di|du|van|von|la|le|el|al)\s+)?\p{Lu}[\p{L}\p{M}'’.\-]*){0,2})\s*:\s*(.*)$").unwrap()
});
// "*Lia:* Olá" and "*Lia*: Olá" are labeled speech too; the asterisks around the label do not become an action.
static LABEL_STARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\*(\p{Lu}[^*:]{0,40}?)(?::\*|\*\s*:)\s*").unwrap());
static FOREIGN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\p{Han}\p{Hiragana}\p{Katakana}\p{Hangul}\p{Cyrillic}]").unwrap()
});
static ENGLISH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(the|you|your|with|said|she|he|that|this|what|would|could|are|was)\b")
        .unwrap()
});
static PORTUGUESE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(que|você|ele|ela|não|para|com|uma|como|disse|essa|isso)\b").unwrap()
});
static TONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\([^()]{1,60}\)\s*$").unwrap());

/// "Name: speech" label (with or without asterisks around the label): returns (name, rest of the line).
fn label(line: &str) -> Option<(String, String)> {
    let direct = |l: &str| {
        SPEECH
            .captures(l)
            .map(|m| (m[1].to_owned(), m[2].to_owned()))
    };
    direct(line).or_else(|| {
        let fixed = LABEL_STARS.replace(line, "$1: ");
        if matches!(fixed, std::borrow::Cow::Owned(_)) {
            direct(&fixed)
        } else {
            None
        }
    })
}

/// Splits a line body into (span, is_action), carrying the action state across the lines
/// of the same paragraph: an action opened with `*` on one line and closed on the next stays an action.
fn pieces<'a>(body: &'a str, in_action: &mut bool) -> Vec<(&'a str, bool)> {
    body.split('*')
        .enumerate()
        .map(|(i, segment)| {
            if i > 0 {
                *in_action = !*in_action;
            }
            (segment, *in_action)
        })
        .collect()
}

pub fn assess(text: &str, limit: usize, pct: f64) -> Value {
    let mut speech = 0;
    let mut narration = 0;
    let mut speaker = String::new();
    let mut in_action = false;
    let clean = text.replace("**", "");
    for line in clean.lines() {
        let line = line.trim();
        if line.is_empty() {
            in_action = false;
            continue;
        }
        let matched = label(line);
        if let Some((name, _)) = &matched {
            speaker = name.clone();
            in_action = false;
        }
        let is_speech = !speaker.is_empty();
        let mut body = matched.as_ref().map_or(line, |(_, rest)| rest.as_str());
        if is_speech
            && let Some(tone) = TONE.find(body)
            && !body[..tone.start()].trim().is_empty()
        {
            body = &body[..tone.start()];
        }
        // Same criterion as the longest monologue: only text outside `*action*` after a speech label is speech.
        for (segment, action) in pieces(body, &mut in_action) {
            let n = rules::utf16_len(segment.trim());
            if is_speech && !action {
                speech += n;
            } else {
                narration += n;
            }
        }
    }
    let actual = if speech + narration > 0 {
        100.0 * speech as f64 / (speech + narration) as f64
    } else {
        0.0
    };
    let chars = rules::utf16_len(text);
    let minimum = (limit * 85).div_ceil(100);
    let en = ENGLISH.find_iter(text).count();
    let pt = PORTUGUESE.find_iter(text).count();
    let language_ok = FOREIGN.find_iter(text).count() <= 12 && !(en >= 5 && en > pt * 2);
    let size_ok = chars >= minimum && chars <= limit;
    let tolerance = if pct == 0.0 || pct == 100.0 {
        0.0
    } else {
        10.0
    };
    let ratio_ok = (actual - pct).abs() <= tolerance;
    json!({"characters":chars,"minCharacters":minimum,"maxCharacters":limit,"dialoguePct":(actual*10.0).round()/10.0,"targetDialoguePct":pct,"sizeOk":size_ok,"ratioOk":ratio_ok,"languageOk":language_ok,"ok":size_ok&&ratio_ok&&language_ok})
}

/// Counts consecutive speeches by the same character, even across line breaks,
/// stage directions or actions between them. Only another speaker starts a new run.
fn longest_monologue(text: &str) -> usize {
    let clean = text.replace("**", "");
    let (mut speaker, mut current, mut longest, mut in_action) = (String::new(), 0, 0, false);
    for line in clean.lines().map(str::trim) {
        if line.is_empty() {
            in_action = false;
            continue;
        }
        let matched = label(line);
        if let Some((name, _)) = &matched {
            in_action = false;
            if speaker != *name {
                current = 0;
                speaker = name.clone();
            }
        }
        if speaker.is_empty() {
            continue;
        }
        let mut body = matched.as_ref().map_or(line, |(_, rest)| rest.as_str());
        if let Some(tone) = TONE.find(body) {
            body = &body[..tone.start()];
        }
        // Action spans (between asterisks, even across several lines) do not count.
        current += pieces(body, &mut in_action)
            .into_iter()
            .filter(|(_, action)| !action)
            .map(|(s, _)| rules::utf16_len(s.trim()))
            .sum::<usize>();
        longest = longest.max(current);
    }
    longest
}

fn distinct_speakers(text: &str) -> usize {
    let clean = text.replace("**", "");
    let mut speakers = HashSet::new();
    for line in clean.lines().map(str::trim) {
        if let Some((name, _)) = label(line) {
            let key = rules::key(&name);
            if !key.is_empty() {
                speakers.insert(key);
            }
        }
    }
    speakers.len()
}

pub fn effective_monologue_limit(settings: &Value, speaker_count: Option<usize>) -> usize {
    let configured = settings["monologueChars"].as_u64().unwrap_or(300) as usize;
    if configured == 0 {
        return 0;
    }
    let dialogue_pct = settings["dialoguePct"].as_u64().unwrap_or(65).clamp(0, 100) as usize;
    if dialogue_pct == 0 {
        return configured;
    }
    let size = rules::reply_size(settings);
    let target_speech = (size * dialogue_pct).div_ceil(100);
    let speakers = speaker_count.unwrap_or(1).max(1);
    let needed_per_speaker = target_speech.div_ceil(speakers);
    if rules::target_mode(settings) || (speakers == 1 && dialogue_pct >= 50) {
        configured.max(needed_per_speaker).min(size)
    } else {
        configured
    }
}

fn effective_monologue_for_text(
    configured: usize,
    limit: usize,
    pct: f64,
    settings: &Value,
    text_chars: usize,
    speakers: usize,
) -> usize {
    if configured == 0 || pct <= 0.0 {
        return configured;
    }
    let upper_pct = if pct >= 100.0 {
        100.0
    } else {
        (pct + 10.0).min(100.0)
    };
    if rules::target_mode(settings) {
        let stretch = (limit * rules::TARGET_STRETCH_PCT).div_ceil(100);
        let basis = text_chars.max(limit).min(stretch);
        let needed = ((basis as f64) * (upper_pct / 100.0)).ceil() as usize;
        return configured.max(needed.div_ceil(speakers.max(1))).min(stretch);
    }
    if speakers <= 1 && pct >= 50.0 {
        let needed = ((text_chars as f64) * (upper_pct / 100.0)).ceil() as usize;
        return configured.max(needed).min(limit);
    }
    configured
}

pub fn assess_settings(text: &str, limit: usize, pct: f64, settings: &Value) -> Value {
    let mut a = assess(text, limit, pct);
    let text_chars = rules::utf16_len(text);
    if settings["lengthMode"] != "target" {
        a["minCharacters"] = json!(0);
        a["sizeOk"] = json!(!text.trim().is_empty() && text_chars <= limit);
    } else {
        // `limit` is the centre of the approximate target; the acceptable range reaches 125% of it.
        let stretch = (limit * rules::TARGET_STRETCH_PCT).div_ceil(100);
        a["maxCharacters"] = json!(stretch);
        a["sizeOk"] = json!(
            text_chars >= a["minCharacters"].as_u64().unwrap_or(0) as usize
                && text_chars <= stretch
        );
    }
    let speakers = distinct_speakers(text).max(1);
    let configured = settings["monologueChars"].as_u64().unwrap_or(300) as usize;
    let monologue_limit =
        effective_monologue_for_text(configured, limit, pct, settings, text_chars, speakers);
    let longest = longest_monologue(text);
    a["uniqueSpeakers"] = json!(speakers);
    a["maxMonologueCharacters"] = json!(monologue_limit);
    a["longestMonologueCharacters"] = json!(longest);
    a["monologueOk"] = json!(monologue_limit == 0 || longest <= monologue_limit);
    a["ok"] = json!(
        a["sizeOk"] == true
            && a["ratioOk"] == true
            && a["languageOk"] == true
            && a["monologueOk"] == true
    );
    a
}

pub fn turn_instruction(settings: &Value) -> String {
    turn_instruction_for(settings, None)
}

pub fn turn_instruction_for(settings: &Value, speaker_count: Option<usize>) -> String {
    let effective_limit = effective_monologue_limit(settings, speaker_count);
    turn_instruction_with_limit(settings, effective_limit)
}

fn turn_instruction_with_limit(settings: &Value, effective_limit: usize) -> String {
    let configured = settings["monologueChars"].as_u64().unwrap_or(300) as usize;
    let speech = if effective_limit == 0 {
        "Falas sem limite individual, respeitando o teto total.".into()
    } else if effective_limit > configured {
        format!(
            "No máximo {effective_limit} unidades UTF-16 de falas seguidas do mesmo personagem (limite ampliado de {configured} para compatibilizar a proporção de falas escolhida nesta cena com poucos personagens, sem precisar encher de narração)."
        )
    } else {
        format!(
            "No máximo {effective_limit} unidades UTF-16 de falas seguidas do mesmo personagem, contando todos os trechos mesmo separados por ações ou quebras de parágrafo. Não divida um monólogo em várias linhas para contornar o limite; reduza a fala e devolva espaço ao usuário."
        )
    };
    let pacing = match settings["turnPacing"].as_str() {
        Some("balanced") => {
            "Ritmo equilibrado: desenvolva a reação atual e no máximo um acontecimento novo; devolva a vez antes de encadear outra etapa da cena."
        }
        Some("free") => {
            "Ritmo livre: desenvolva a cena conforme necessário, sempre parando antes de uma escolha ou ação do usuário."
        }
        _ => {
            "Ritmo interativo: só reage quem tem motivo para reagir à entrada atual; apartes e interações cruzadas breves entre personagens são bem-vindos quando surgirem naturalmente, sem que todos comentem a mesma coisa nem encadeiem acontecimentos excessivos antes da resposta do usuário. Termine numa pausa natural, sem forçar uma pergunta nem resumir a cena, e devolva a vez."
        }
    };
    let dialogue_pct = settings["dialoguePct"].as_u64().unwrap_or(65);
    let balance_rule = if rules::target_mode(settings) || dialogue_pct >= 50 {
        "Cumpra a proporção configurada entre falas e narração sem preencher com narração redundante; preserve a agência do usuário."
    } else {
        "Reduza o texto antes de aumentar falas só para cumprir a proporção; preserve a agência do usuário."
    };
    format!("{speech} {pacing} {balance_rule}")
}

pub fn instruction(before: &Value, pct: f64) -> String {
    let format = if pct == 0.0 {
        "Use somente narração entre asteriscos."
    } else if pct == 100.0 {
        "Use somente falas Nome: fala, sem ações nem rubricas."
    } else {
        "Narração entre *asteriscos* e falas Nome: fala."
    };
    let size = if before["minCharacters"] == 0 {
        format!(
            "Use até {} unidades UTF-16, sem tamanho mínimo. Não alongue uma reação já completa.",
            before["maxCharacters"]
        )
    } else {
        format!(
            "O tamanho deve ficar entre {} e {} unidades UTF-16, sem ultrapassar o teto.",
            before["minCharacters"], before["maxCharacters"]
        )
    };
    format!(
        "[AJUSTE DA RESPOSTA ANTERIOR — devolva somente a narrativa final]\nReescreva a resposta anterior preservando os acontecimentos, autoria, personalidade, conhecimento e agência do usuário. Não repita a entrada do usuário nem invente escolhas por ele. Escreva integralmente em português do Brasil (pt-BR), sem alternar idiomas. {size} O volume de texto deve conter aproximadamente {pct}% falas e {}% narração. {format} A versão anterior tem {} caracteres e {}% falas. Ajuste o volume dos blocos, não só a quantidade de parágrafos. Para alongar, desenvolva o mesmo momento com reações ou detalhes novos; nunca preencha com recapitulação do que já aconteceu. Não mostre explicações nem comentários sobre esta revisão.",
        100.0 - pct,
        before["characters"],
        before["dialoguePct"]
    )
}

pub fn instruction_settings(before: &Value, pct: f64, settings: &Value) -> String {
    let configured = settings["monologueChars"].as_u64().unwrap_or(300) as usize;
    let speakers = before["uniqueSpeakers"].as_u64().map(|n| n as usize);
    let effective_limit = if before["monologueOk"] == false {
        before["maxMonologueCharacters"]
            .as_u64()
            .unwrap_or(configured as u64) as usize
    } else {
        effective_monologue_limit(settings, speakers)
    };
    let mut out = format!(
        "{} {}",
        instruction(before, pct),
        turn_instruction_with_limit(settings, effective_limit)
    );
    if before["monologueOk"] == false {
        out.push_str(&format!(
            " A maior sequência de fala anterior tinha {} unidades UTF-16; condense as falas que excederem o limite.",
            before["longestMonologueCharacters"]
        ));
    }
    if before["ratioOk"] == false {
        let actual = before["dialoguePct"].as_f64().unwrap_or(0.0);
        if actual < pct {
            out.push_str(&format!(
                " A resposta anterior teve poucas falas ({actual:.1}% vs meta de {pct:.0}%): substitua o excesso de narração entre *asteriscos* por falas diretas \"Nome: fala\" até atingir ~{pct:.0}% falas e ~{:.0}% narração.",
                100.0 - pct
            ));
        } else if actual > pct {
            out.push_str(&format!(
                " A resposta anterior teve falas em excesso ({actual:.1}% vs meta de {pct:.0}%): reduza o volume de falas diretas e amplie a narração entre *asteriscos* até atingir ~{pct:.0}% falas e ~{:.0}% narração.",
                100.0 - pct
            ));
        }
    }
    out
}

/// Editing uses the stable rules, current scene and immediate exchange. The
/// supplied draft is the source of truth; this call must not advance the story.
pub fn repair_body(body: &Value, visible: &str, instruction: &str) -> Value {
    let mut repair = body.clone();
    repair["stream"] = json!(false);
    repair.as_object_mut().unwrap().remove("stream_options");
    let original = body["messages"].as_array().unwrap();
    let prefix = if original.first().is_some_and(|m| m["role"] == "system") {
        1
    } else {
        2.min(original.len())
    };
    let mut messages = original[..prefix].to_vec();
    messages.extend_from_slice(&original[prefix.max(original.len().saturating_sub(3))..]);
    messages.push(json!({"role":"assistant","content":visible}));
    messages.push(json!({"role":"user","content":format!("{instruction}\nEsta é uma edição do rascunho acima, não uma continuação: conserve os fatos e os interlocutores escritos nele; não acrescente acontecimentos, decisões, falas ou conhecimento.")}));
    repair["messages"] = json!(messages);
    repair
}

pub fn score(a: &Value, limit: usize, pct: f64) -> f64 {
    (a["dialoguePct"].as_f64().unwrap_or(0.0) - pct).abs() / 100.0
        + a["minCharacters"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(a["characters"].as_u64().unwrap_or(0)) as f64
            / limit as f64
        // overflow above the cap also lowers the score: a fix that removes the overflow must be accepted
        + a["characters"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(a["maxCharacters"].as_u64().unwrap_or(limit as u64)) as f64
            / limit as f64
        + if a["languageOk"] == true { 0.0 } else { 2.0 }
        + if a["monologueOk"] == false {
            1.0 + a["longestMonologueCharacters"].as_f64().unwrap_or(0.0)
                / a["maxMonologueCharacters"].as_f64().unwrap_or(1.0).max(1.0)
        } else {
            0.0
        }
}

pub fn accept_repair(
    before: &Value,
    after: &Value,
    candidate: &str,
    finish_reason: Option<&str>,
    size: usize,
    pct: f64,
    settings: &Value,
    server_capped: bool,
) -> bool {
    let hard = rules::reply_bounds(settings).2 as u64;
    let within_hard = after["characters"].as_u64().unwrap_or(u64::MAX) <= hard;
    let no_size_regression = before["sizeOk"] != true || after["sizeOk"] == true;
    let no_monologue_regression = before["monologueOk"] != true || after["monologueOk"] == true;
    let improved = score(after, size, pct) < score(before, size, pct)
        || (server_capped && after["ok"] == true);
    !candidate.is_empty()
        && after["languageOk"] == true
        && within_hard
        && no_size_regression
        && no_monologue_regression
        && finish_reason.is_none_or(|r| !matches!(r, "length" | "max_tokens"))
        && improved
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_ratio_and_language() {
        let good = format!(
            "Mira: {}",
            "Vamos examinar o endereço da carta antes de sair. ".repeat(9)
        );
        assert_eq!(assess(&good, 500, 100.0)["ok"], true);
        assert_eq!(assess("*Ela abre a carta.*", 500, 0.0)["ratioOk"], true);
        assert_eq!(
            assess(
                "Mira: You said that she was there with the letter.",
                500,
                100.0
            )["languageOk"],
            false
        );
        assert_eq!(
            assess("Mira: **Olá** *Ela abre a carta.*", 500, 100.0)["ratioOk"],
            false
        );
    }

    #[test]
    fn accept_repair_keeps_real_improvements_without_regressing_passing_checks() {
        let settings = json!({
            "botChars": 1500,
            "dialoguePct": 70,
            "monologueChars": 300,
            "lengthMode": "maximum"
        });
        // Draft was narration-heavy (25% speech) with a 500-char monologue.
        let before = json!({
            "characters": 1400,
            "minCharacters": 0,
            "maxCharacters": 1500,
            "dialoguePct": 25.0,
            "sizeOk": true,
            "ratioOk": false,
            "languageOk": true,
            "monologueOk": false,
            "maxMonologueCharacters": 300,
            "longestMonologueCharacters": 500,
            "ok": false
        });
        // Rewrite hits 70% speech and trims monologue from 500 -> 315 (still slightly >300, under hard cap).
        let improved_after = json!({
            "characters": 1350,
            "minCharacters": 0,
            "maxCharacters": 1500,
            "dialoguePct": 70.0,
            "sizeOk": true,
            "ratioOk": true,
            "languageOk": true,
            "monologueOk": false,
            "maxMonologueCharacters": 300,
            "longestMonologueCharacters": 315,
            "ok": false
        });
        assert!(accept_repair(
            &before,
            &improved_after,
            "Lia: Texto ajustado.",
            Some("stop"),
            1500,
            70.0,
            &settings,
            false
        ));

        // Regressing an already-passing monologueOk or sizeOk, exceeding hard, or failing languageOk is rejected.
        let mut regressed_size = improved_after.clone();
        regressed_size["sizeOk"] = json!(false);
        assert!(!accept_repair(
            &before,
            &regressed_size,
            "Lia: Texto.",
            Some("stop"),
            1500,
            70.0,
            &settings,
            false
        ));

        let mut over_hard = improved_after.clone();
        over_hard["characters"] = json!(1501);
        assert!(!accept_repair(
            &before,
            &over_hard,
            "Lia: Texto.",
            Some("stop"),
            1500,
            70.0,
            &settings,
            false
        ));

        let mut bad_lang = improved_after.clone();
        bad_lang["languageOk"] = json!(false);
        assert!(!accept_repair(
            &before,
            &bad_lang,
            "Lia: English text.",
            Some("stop"),
            1500,
            70.0,
            &settings,
            false
        ));

        let passing_before = json!({
            "characters": 1400,
            "minCharacters": 0,
            "maxCharacters": 1500,
            "dialoguePct": 25.0,
            "sizeOk": true,
            "ratioOk": false,
            "languageOk": true,
            "monologueOk": true,
            "maxMonologueCharacters": 300,
            "longestMonologueCharacters": 200,
            "ok": false
        });
        assert!(!accept_repair(
            &passing_before,
            &improved_after,
            "Lia: Texto.",
            Some("stop"),
            1500,
            70.0,
            &settings,
            false
        ));

        // A server-capped draft rewritten to ok=true is accepted.
        let mut ok_after = improved_after.clone();
        ok_after["monologueOk"] = json!(true);
        ok_after["longestMonologueCharacters"] = json!(220);
        ok_after["ok"] = json!(true);
        assert!(accept_repair(
            &ok_after,
            &ok_after,
            "Lia: Texto curto.",
            Some("stop"),
            1500,
            70.0,
            &settings,
            true
        ));
    }

    #[test]
    fn reconciles_monologue_cap_with_dialogue_ratio_in_single_speaker_scenes() {
        let settings = json!({
            "botChars": 1500,
            "dialoguePct": 70,
            "monologueChars": 300,
            "lengthMode": "maximum"
        });
        assert_eq!(effective_monologue_limit(&settings, Some(1)), 1050);
        assert_eq!(effective_monologue_limit(&settings, Some(4)), 300);

        let single_turn = turn_instruction_for(&settings, Some(1));
        assert!(single_turn.contains("1050"));
        assert!(single_turn.contains("Cumpra a proporção configurada"));
        assert!(!single_turn.contains("Reduza o texto antes de aumentar falas"));

        // 1-speaker narration-heavy draft (~15% speech) should not be told to condense speech.
        let narration_heavy = format!(
            "*{}*\n\nLia: {}",
            "Ela observa o corredor silencioso com calma. ".repeat(26),
            "Você chegou mais cedo hoje, podemos conversar agora. ".repeat(4)
        );
        let before = assess_settings(&narration_heavy, 1500, 70.0, &settings);
        assert_eq!(before["uniqueSpeakers"], 1);
        assert_eq!(before["monologueOk"], true);
        assert_eq!(before["ratioOk"], false);
        let repair_prompt = instruction_settings(&before, 70.0, &settings);
        assert!(!repair_prompt.contains("condense as falas que excederem o limite"));
        assert!(repair_prompt.contains("substitua o excesso de narração"));
        assert!(repair_prompt.contains("1050"));

        // Rewriting to ~70% speech (~980 chars) passes both ratioOk and monologueOk and is accepted.
        let balanced = format!(
            "*{}*\n\nLia: {}",
            "Ela apoia os braços na mesa e respira fundo. ".repeat(9),
            "Eu estava esperando você voltar para resolver isso logo, sem rodeios. ".repeat(14)
        );
        let after = assess_settings(&balanced, 1500, 70.0, &settings);
        assert_eq!(after["uniqueSpeakers"], 1);
        assert_eq!(after["ratioOk"], true);
        assert_eq!(after["monologueOk"], true);
        assert_eq!(after["ok"], true);
        assert!(accept_repair(
            &before,
            &after,
            &balanced,
            Some("stop"),
            1500,
            70.0,
            &settings,
            false
        ));

        // Slightly above target (~75% speech, still inside 70 ± 10%) is not falsely rejected on monologueOk.
        let slightly_talkative = format!(
            "*{}*\n\nLia: {}",
            "Ela inclina a cabeça e sorri devagar. ".repeat(9),
            "Eu estava esperando você voltar para resolver isso logo, sem rodeios. ".repeat(15)
        );
        let after_high = assess_settings(&slightly_talkative, 1500, 70.0, &settings);
        assert_eq!(after_high["ratioOk"], true);
        assert_eq!(after_high["monologueOk"], true);
        assert_eq!(after_high["ok"], true);
        assert!(accept_repair(
            &before,
            &after_high,
            &slightly_talkative,
            Some("stop"),
            1500,
            70.0,
            &settings,
            false
        ));

        // Wall-of-dialogue (>90% speech) still fails both ratioOk and monologueOk and asks to condense speech.
        let wall = format!(
            "*Ela respira.*\n\nLia: {}",
            "Eu continuo falando sem parar sobre tudo o que aconteceu ontem à noite. ".repeat(19)
        );
        let wall_a = assess_settings(&wall, 1500, 70.0, &settings);
        assert_eq!(wall_a["ratioOk"], false);
        assert_eq!(wall_a["monologueOk"], false);
        let wall_prompt = instruction_settings(&wall_a, 70.0, &settings);
        assert!(wall_prompt.contains("condense as falas que excederem o limite"));
        assert!(wall_prompt.contains("falas em excesso"));

        // Multi-speaker scenes keep the configured 300-char monologue ceiling in maximum mode.
        let multi = format!(
            "Lia: {}\n\nMara: Tudo bem, vou checar lá fora.",
            "Precisamos decidir agora mesmo o que fazer com a carta. ".repeat(9)
        );
        let multi_a = assess_settings(&multi, 1500, 70.0, &settings);
        assert_eq!(multi_a["uniqueSpeakers"], 2);
        assert_eq!(multi_a["maxMonologueCharacters"], 300);
        assert_eq!(multi_a["monologueOk"], false);

        // Target mode scales per-speaker cap across available speakers.
        let target_settings = json!({
            "botChars": 2000,
            "dialoguePct": 70,
            "monologueChars": 300,
            "lengthMode": "target"
        });
        assert_eq!(effective_monologue_limit(&target_settings, Some(1)), 1400);
        assert_eq!(effective_monologue_limit(&target_settings, Some(2)), 700);
    }
}
