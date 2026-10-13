use crate::{rules, s};
use regex::Regex;
use serde_json::{Value, json};

pub fn heuristic(text: &str, persona: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if persona.is_empty() {
        return out;
    }
    let speech = Regex::new(&format!(
        r"(?i)^\s*\*{{0,2}}\s*{}\s*\*{{0,2}}\s*:",
        regex::escape(persona)
    ))
    .unwrap();
    let subject = Regex::new(&format!(r"(?i)^\W*{}\s+\p{{Ll}}", regex::escape(persona))).unwrap();
    let other = Regex::new(r"^\s*\*{0,2}\s*\p{Lu}[^:*\n]{0,40}\s*\*{0,2}\s*:\s*(.*)$").unwrap();
    let italic = Regex::new(r"\*([^*]+)\*").unwrap();
    let sentences = Regex::new(r"[.!?…]\s+|\n+").unwrap();
    let (mut spoke, mut acted) = (false, false);
    for line in text.lines() {
        if speech.is_match(line) {
            if !spoke {
                out.push(json!({"kind":"agency","detail":format!("Fala atribuída a {persona}, controlado só pelo usuário."),"evidence":rules::clip(line.trim(),200)}));
                spoke = true;
            }
            continue;
        }
        let pieces = if let Some(m) = other.captures(line) {
            italic
                .captures_iter(m.get(1).unwrap().as_str())
                .map(|m| m[1].to_owned())
                .collect::<Vec<_>>()
        } else {
            vec![line.replace('*', "")]
        };
        for piece in pieces {
            for sent in sentences.split(&piece) {
                if !acted && subject.is_match(sent.trim()) {
                    out.push(json!({"kind":"agency","detail":format!("Ação/pensamento/decisão de {persona} narrada pela IA."),"evidence":rules::clip(sent.trim(),200)}));
                    acted = true;
                }
            }
        }
    }
    out
}
pub fn messages(input: &Value, text: &str, prefix: &str, secrets: &[String]) -> Vec<Value> {
    let system = include_str!("review-prompt.txt").replace(
        "{{persona}}",
        input["conv"]["persona"].as_str().unwrap_or("o usuário"),
    );
    let sources = input["recentSources"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let user = format!(
        "CENA ANTERIOR (referência possivelmente atrasada):\n{}\nTEMPO ANTERIOR:\n{}\nQUEM NÃO SABE O QUÊ:\n{}\nHISTÓRICO ANTERIOR RELEVANTE:\n{}\nMENSAGEM ATUAL DO USUÁRIO (canônica):\n{}\n{}\nPRELÚDIO JÁ OCORRIDO:\n{}\nRESPOSTA A REVISAR:\n{}",
        input["scene"],
        input["timeline"]["current"],
        secrets.join("\n"),
        s(input, "previous"),
        s(input, "userText"),
        crate::provenance::block(&s(input, "userText"), &sources),
        prefix,
        text
    );
    vec![
        json!({"role":"system","content":system}),
        json!({"role":"user","content":user}),
    ]
}
pub fn supported(raw: &Value, text: &str, preceding: &str, secrets: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    let subject=Regex::new(r"^\s*[*_]*\s*([\p{Lu}][\p{L}'’-]*(?:\s+[\p{Lu}][\p{L}'’-]*)*)\s+(?:est|fic|perm|n[aã]o|sent|levant|deit|entr|sai|volt|caminh|cheg|seg|continu|foi|[ée]|se\b)").unwrap();
    let restriction=Regex::new(r"(?i)pres[oa]|im[oó]vel|imobil|amarr|trancad|impedid|n[aã]o (?:podia|pode|consegue|conseguia)|sem (?:poder|sair|mover)|durante todo|ao mesmo tempo|simultaneamente").unwrap();
    let simultaneous = Regex::new(r"(?i)ao mesmo tempo|simultaneamente").unwrap();
    let release = Regex::new(
        r"(?i)^(?:foi\s+)?(?:libertad\w*|solt\w*|desamarr\w*|destranc\w*|se\s+libert\w*)",
    )
    .unwrap();
    let state = Regex::new(
        r"(?i)^(?:estava|está|ficou|fica|permaneceu|permanece|foi|continua|continuou|é)\b",
    )
    .unwrap();
    for kind in ["agency", "knowledge", "continuity"] {
        for f in raw[kind].as_array().into_iter().flatten().take(3) {
            let quote = s(f, "quote");
            if quote.is_empty() || !text.contains(&quote) {
                continue;
            }
            let old = s(f, "previousQuote");
            if kind != "agency" {
                if old.is_empty() {
                    continue;
                }
                if kind == "knowledge" {
                    if !secrets.iter().any(|s| s.contains(&old)) {
                        continue;
                    }
                } else {
                    let previous = format!("{preceding}\n{}", &text[..text.find(&quote).unwrap()]);
                    let Some(at) = previous.rfind(&old) else {
                        continue;
                    };
                    let Some(a) = subject.captures(&old) else {
                        continue;
                    };
                    let Some(b) = subject.captures(&quote) else {
                        continue;
                    };
                    if a[1] != b[1] {
                        continue;
                    }
                    if !restriction.is_match(&old) && !simultaneous.is_match(&quote) {
                        continue;
                    }
                    let events = Regex::new(&format!(
                        r"(?i)(?:^|[^\p{{L}}\p{{N}}]){}\s+([^.!?\n]*)",
                        regex::escape(&a[1])
                    ))
                    .unwrap();
                    let mut released = false;
                    for event in events.captures_iter(&previous[at + old.len()..]) {
                        if release.is_match(&event[1]) {
                            released = true;
                        } else if state.is_match(&event[1]) && restriction.is_match(&event[1]) {
                            released = false;
                        }
                    }
                    if released && !simultaneous.is_match(&quote) {
                        continue;
                    }
                }
            }
            out.push(json!({"kind":kind,"detail":rules::clip(&s(f,"reason"),200),"evidence":rules::clip(&quote,2000),"previousQuote":old}));
        }
    }
    out
}
pub fn supported_with_sources(
    raw: &Value,
    text: &str,
    preceding: &str,
    secrets: &[String],
    input: &Value,
) -> Vec<Value> {
    let mut out = supported(raw, text, preceding, secrets);
    let sources = input["recentSources"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let origins = crate::provenance::quotes(&s(input, "userText"), &sources);
    // Authorship evidence: every field must be a NON-empty string (not missing, blank, nor another JSON type).
    let filled = |flag: &Value, key: &str| flag[key].as_str().is_some_and(|v| !v.trim().is_empty());
    for flag in raw["attribution"].as_array().into_iter().flatten().take(3) {
        if ![
            "quote",
            "sourceQuote",
            "previousQuote",
            "originalSpeaker",
            "claimedSpeaker",
        ]
        .iter()
        .all(|key| filled(flag, key))
        {
            continue;
        }
        let quote = s(flag, "quote");
        let old = s(flag, "previousQuote");
        if !text.contains(&quote) {
            continue;
        }
        let origin = origins.iter().find(|o| {
            o["quote"] == flag["sourceQuote"]
                && o["originalSpeaker"] == flag["originalSpeaker"]
                && s(o, "originalEvidence").contains(&old)
        });
        if let Some(origin) = origin
            && flag["claimedSpeaker"] != origin["originalSpeaker"]
        {
            out.push(json!({"kind":"continuity","detail":format!("Autoria: {}",rules::clip(&s(flag,"reason"),180)),"evidence":quote,"previousQuote":old}));
        }
    }
    out
}
