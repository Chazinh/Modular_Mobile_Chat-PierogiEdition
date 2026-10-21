use crate::rules;
use regex::Regex;
use serde_json::{Value, json};
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

pub fn assess_settings(text: &str, limit: usize, pct: f64, settings: &Value) -> Value {
    let mut a = assess(text, limit, pct);
    if settings["lengthMode"] != "target" {
        a["minCharacters"] = json!(0);
        a["sizeOk"] = json!(!text.trim().is_empty() && rules::utf16_len(text) <= limit);
    } else {
        // `limit` is the centre of the approximate target; the acceptable range reaches 125% of it.
        let stretch = (limit * rules::TARGET_STRETCH_PCT).div_ceil(100);
        let chars = rules::utf16_len(text);
        a["maxCharacters"] = json!(stretch);
        a["sizeOk"] =
            json!(chars >= a["minCharacters"].as_u64().unwrap_or(0) as usize && chars <= stretch);
    }
    let monologue_limit = settings["monologueChars"].as_u64().unwrap_or(300) as usize;
    let longest = longest_monologue(text);
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
    let limit = settings["monologueChars"].as_u64().unwrap_or(300);
    let speech = if limit == 0 {
        "Falas sem limite individual, respeitando o teto total.".into()
    } else {
        format!(
            "No máximo {limit} unidades UTF-16 de falas seguidas do mesmo personagem, contando todos os trechos mesmo separados por ações ou quebras de parágrafo. Não divida um monólogo em várias linhas para contornar o limite; reduza a fala e devolva espaço ao usuário."
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
            "Ritmo interativo: cada personagem relevante reage de forma natural à entrada atual, permitindo apartes, comentários paralelos e interações cruzadas breves entre si sem monopolizar a cena nem encadear acontecimentos excessivos antes da resposta do usuário. Termine numa pausa natural, sem forçar uma pergunta, e devolva a vez."
        }
    };
    format!(
        "{speech} {pacing} Reduza o texto antes de aumentar falas só para cumprir a proporção; preserve a agência do usuário."
    )
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
        "[AJUSTE DA RESPOSTA ANTERIOR — devolva somente a narrativa final]\nReescreva a resposta anterior preservando os acontecimentos, autoria, personalidade, conhecimento e agência do usuário. Não repita a entrada do usuário nem invente escolhas por ele. Escreva integralmente em português do Brasil (pt-BR), sem alternar idiomas. {size} O volume de texto deve conter aproximadamente {pct}% falas e {}% narração. {format} A versão anterior tem {} caracteres e {}% falas. Ajuste o volume dos blocos, não só a quantidade de parágrafos. Não mostre explicações nem comentários sobre esta revisão.",
        100.0 - pct,
        before["characters"],
        before["dialoguePct"]
    )
}

pub fn instruction_settings(before: &Value, pct: f64, settings: &Value) -> String {
    format!(
        "{} {} A maior sequência de fala anterior tinha {} unidades UTF-16; condense as falas que excederem o limite.",
        instruction(before, pct),
        turn_instruction(settings),
        before["longestMonologueCharacters"]
    )
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
}
