use crate::s;
use regex::Regex;
use serde_json::{Value, json};
use std::{collections::HashSet, sync::LazyLock};

static SPEECH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*\*{0,2}\s*(\p{Lu}[\p{L}\p{M}'’. -]{1,59}?)\s*\*{0,2}\s*:\s*(.*)$").unwrap()
});
static QUOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"["“]([^"”\n]{12,300})["”]"#).unwrap());
static MIMIC: LazyLock<Regex> = LazyLock::new(|| Regex::new("(?i)imit|cit|repet|voz").unwrap());

pub fn sources(pairs: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for pair in pairs.iter().skip(pairs.len().saturating_sub(3)) {
        if let Some(identified) = pair["sources"].as_array() {
            out.extend(
                identified
                    .iter()
                    .skip(identified.len().saturating_sub(8))
                    .cloned(),
            );
        } else {
            if pair["user"].is_string() {
                out.push(json!({"id":pair["id"],"role":"user","text":pair["user"]}));
            }
            if pair["assistant"].as_str().is_some_and(|t| !t.is_empty()) {
                out.push(json!({"id":pair["versions"].as_array().and_then(|a|a.last()).cloned().unwrap_or_else(||json!(format!("{}:assistant",s(pair,"id")))),"role":"assistant","text":pair["assistant"]}));
            }
        }
    }
    out.split_off(out.len().saturating_sub(8))
}

fn original_speech_contains(body: &str, quote: &str) -> bool {
    let (mut quoted, mut action, mut parens, mut brackets) = (false, false, 0usize, 0usize);
    for (at, c) in body.char_indices() {
        if !quoted && !action && parens == 0 && brackets == 0 && body[at..].starts_with(quote) {
            return true;
        }
        match c {
            '"' => quoted = !quoted,
            '“' => quoted = true,
            '”' => quoted = false,
            '*' if !quoted => action = !action,
            '(' if !quoted => parens += 1,
            ')' if !quoted => parens = parens.saturating_sub(1),
            '[' if !quoted => brackets += 1,
            ']' if !quoted => brackets = brackets.saturating_sub(1),
            _ => {}
        }
    }
    false
}
/// Whole speech in quotes (`Name: "text"` or `Name: “text”`): counts as the speech without quotes. Only when the outer
/// quotes are the only ones in the span; inner quotes (a quotation inside the speech) keep the body as is.
fn unquoted(body: &str) -> &str {
    let body = body.trim();
    let mut chars = body.chars();
    if let (Some(first @ ('"' | '“')), Some(last @ ('"' | '”'))) = (chars.next(), chars.next_back())
        && body.len() > first.len_utf8() + last.len_utf8()
    {
        let inner = &body[first.len_utf8()..body.len() - last.len_utf8()];
        if !inner.contains(['"', '“', '”']) {
            return inner;
        }
    }
    body
}
pub fn quotes(text: &str, sources: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for m in QUOTE.captures_iter(text) {
        let quote = &m[1];
        let mut hits = Vec::new();
        for source in sources.iter().skip(sources.len().saturating_sub(8)) {
            for line in source["text"].as_str().unwrap_or_default().lines() {
                if let Some(sp) = SPEECH.captures(line)
                    && original_speech_contains(unquoted(&sp[2]), quote)
                {
                    hits.push(json!({"sourceId":source["id"],"sourceRole":source["role"],"originalSpeaker":sp[1].trim(),"originalEvidence":line.trim(),"quote":quote}));
                }
            }
        }
        if hits.is_empty()
            || hits
                .iter()
                .map(|h| s(h, "originalSpeaker"))
                .collect::<HashSet<_>>()
                .len()
                != 1
        {
            continue;
        }
        let full = m.get(0).unwrap();
        let before = text[..full.start()]
            .rsplit('\n')
            .next()
            .unwrap_or_default()
            .trim_end_matches('\r');
        let speaker = SPEECH.captures(before).map(|sp| sp[1].trim().to_owned());
        let after = crate::rules::clip(&text[full.end()..], 120);
        let mut hit = hits.pop().unwrap();
        hit["citingSpeaker"] = json!(speaker);
        hit["citingEvidence"] = json!(format!("{}{after}", full.as_str()));
        hit["referent"] = Value::Null;
        hit["use"] = json!(if MIMIC.is_match(after) {
            "citation_or_imitation"
        } else {
            "exact_reference"
        });
        out.push(hit);
        if out.len() == 4 {
            break;
        }
    }
    out
}
pub fn block(text: &str, sources: &[Value]) -> String {
    let quotes = quotes(text, sources);
    if quotes.is_empty() {
        String::new()
    } else {
        format!(
            "[PROVENIÊNCIA DE CITAÇÕES RECENTES — correspondência exata; sem inferir pronomes]\n{}\nA origem é originalSpeaker; citar/imitar não cria outra declaração. Comentários fora das aspas são separados; referentes não explicitados permanecem desconhecidos.",
            json!(quotes)
        )
    }
}
/// Plain-language reading of the current message (`quoteReading`): only when a quoted sentence exactly
/// matches a line from the history. Without an exact match nothing is asserted.
pub fn reading(text: &str, sources: &[Value]) -> String {
    let quotes = quotes(text, sources);
    if quotes.is_empty() {
        return String::new();
    }
    let mut lines = vec![String::from(
        "[LEITURA DA MENSAGEM ACIMA — conferida por correspondência exata com o histórico]",
    )];
    for q in &quotes {
        let quote = s(q, "quote");
        let quote = if quote.chars().count() > 120 {
            format!("{}...", quote.chars().take(117).collect::<String>())
        } else {
            quote
        };
        let speaker = s(q, "originalSpeaker");
        lines.push(format!(
            "- A frase entre aspas “{quote}” foi dita originalmente por {speaker} (histórico acima). Na mensagem acima ela é apenas CITADA/IMITADA por quem a escreveu: não é uma fala nova de ninguém nem uma declaração nova sobre {speaker}."
        ));
    }
    lines.push(String::from(
        "O que está fora das aspas é comentário de quem escreveu a mensagem. Não atribua a ninguém uma fala, apelido, insulto ou ato que não esteja escrito acima ou no histórico.",
    ));
    lines.join("\n")
}
pub fn source(id: &str, role: &str, text: &str, author: Option<&str>) -> String {
    format!(
        "[INÍCIO DA FONTE id={id} role={role}{}; interlocutores definidos SOMENTE pelo texto, não pelo role]\n{text}\n[FIM DA FONTE id={id}]",
        author
            .map(|a| format!(" autor_da_entrada={a}"))
            .unwrap_or_default()
    )
}
