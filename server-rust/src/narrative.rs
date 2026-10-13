//! Immutable narrative boundaries with UTF-16 source offsets and calendar evidence.
use crate::{n, rules, s};
use chrono::Datelike;
use regex::Regex;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::LazyLock};
static SPEECH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\*{0,2}[\p{L}][^:*\n]{0,60}:\s").unwrap());
static REPORTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"flashback|lembr|record|relat|contou|disse|citac|no passado|sonh|imagin|hipotet")
        .unwrap()
});
static SENTENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^.!?]+(?:[.!?]+|$)").unwrap());
static NEXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:no |o |ao |na |a )?dia seguinte\b").unwrap());
const AMOUNT: &str =
    r"(\d{1,4}|um|uma|dois|duas|tres|quatro|cinco|seis|sete|oito|nove|dez|onze|doze)";
const UNIT: &str = r"(dias?|semanas?|mes(?:es)?|anos?)";
static ADVANCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^(?:(?:apos|depois de|passados?|passadas?)\s+{AMOUNT}\s+{UNIT}\b|{AMOUNT}\s+{UNIT}\s+(?:depois|se passaram|se passa(?:m)?|passa(?:m)?|mais tarde)\b)")).unwrap()
});
static ANCHOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"\bha\s*{AMOUNT}\s+{UNIT}\b|\b{AMOUNT}\s+{UNIT}\s+atras\b"
    ))
    .unwrap()
});
static DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:em |no dia |dia |data:\s*)?(\d{4}-\d{2}-\d{2}|\d{1,2}/\d{1,2}/\d{4})\b")
        .unwrap()
});
fn quantity(t: &str) -> i64 {
    t.parse().unwrap_or(match t {
        "um" | "uma" => 1,
        "dois" | "duas" => 2,
        "tres" => 3,
        "quatro" => 4,
        "cinco" => 5,
        "seis" => 6,
        "sete" => 7,
        "oito" => 8,
        "nove" => 9,
        "dez" => 10,
        "onze" => 11,
        "doze" => 12,
        _ => 0,
    })
}
fn event(kind: &str, amount: i64, unit: &str, evidence: &str) -> Value {
    let (days, months) = if unit.starts_with("mes") {
        (0, amount)
    } else if unit.starts_with("ano") {
        (0, amount * 12)
    } else {
        (amount * if unit.starts_with("semana") { 7 } else { 1 }, 0)
    };
    json!({"kind":kind,"days":days,"months":months,"evidence":evidence})
}
/// What is searched in the text: time skips/dates in the chat, "N months ago" anchors from the sheet or chat, the sheet's absolute date.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Chat,
    Card,
    ChatAnchor,
    CardDate,
}
fn absolute(clean: &str, raw: &str) -> Option<Value> {
    let d = &DATE.captures(clean)?[1];
    let format = if d.contains('/') {
        "%d/%m/%Y"
    } else {
        "%Y-%m-%d"
    };
    let date = chrono::NaiveDate::parse_from_str(d, format).ok()?;
    Some(json!({"kind":"absolute","date":date.to_string(),"evidence":raw}))
}
pub fn evidence(text: &str, card: bool) -> Vec<Value> {
    scan(text, if card { Mode::Card } else { Mode::Chat })
}
/// "N months ago" anchors said in a chat message (the sheet uses `evidence(_, true)`).
pub fn chat_anchors(text: &str) -> Vec<Value> {
    scan(text, Mode::ChatAnchor)
}
/// First absolute date written in the sheet (e.g. `Data: 2024-01-31`), used as the timeline start date.
pub fn card_date(prompt: &str) -> Option<Value> {
    scan(prompt, Mode::CardDate)
        .first()
        .map(|e| e["date"].clone())
}
fn scan(text: &str, mode: Mode) -> Vec<Value> {
    let card = matches!(mode, Mode::Card | Mode::CardDate);
    let mut out = Vec::new();
    let mut line_offset = 0;
    for line in text.split_inclusive('\n') {
        let clean_line = line.trim_start().trim_start_matches(['*', '_', '#']).trim();
        if line.trim_start().starts_with(['>', '"', '“', '\'', '‘'])
            || !card && SPEECH.is_match(line) && !rules::fold(clean_line).starts_with("data:")
        {
            line_offset += line.len();
            continue;
        }
        for found in SENTENCE.find_iter(line) {
            let raw = found.as_str();
            let clean = raw.trim_start().trim_start_matches(['*', '_', '#']).trim();
            let t = rules::fold(clean);
            let start = rules::utf16_len(&text[..line_offset + found.start()]);
            let end = rules::utf16_len(&text[..line_offset + found.end()]);
            let mut events = Vec::new();
            if mode == Mode::CardDate {
                // in the sheet only the labeled date counts ("Data: ..."); "Em 12/05/1990 ela nasceu" is not the scene start
                if t.starts_with("data:") {
                    events.extend(absolute(clean, raw));
                }
            } else if mode == Mode::Card || mode == Mode::ChatAnchor && !REPORTED.is_match(&t) {
                for m in ANCHOR.captures_iter(&t) {
                    let a = m.get(1).or(m.get(3)).unwrap().as_str();
                    let u = m.get(2).or(m.get(4)).unwrap().as_str();
                    events.push(event("anchor", quantity(a), u, raw));
                }
            } else if mode == Mode::Chat && !REPORTED.is_match(&t) {
                if NEXT.is_match(&t) {
                    events.push(event("advance", 1, "dia", raw));
                } else if let Some(m) = ADVANCE.captures(&t) {
                    events.push(event(
                        "advance",
                        quantity(m.get(1).or(m.get(3)).unwrap().as_str()),
                        m.get(2).or(m.get(4)).unwrap().as_str(),
                        raw,
                    ));
                } else {
                    events.extend(absolute(clean, raw));
                }
            }
            for mut event in events {
                event["start"] = json!(start);
                event["end"] = json!(end);
                out.push(event);
            }
        }
        line_offset += line.len();
    }
    out
}
fn move_date(date: Value, days: i64, months: i64) -> Value {
    let Some(date) = date
        .as_str()
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
    else {
        return Value::Null;
    };
    date.checked_add_months(chrono::Months::new(months as u32))
        .filter(|d| d.day() == date.day())
        .and_then(|d| d.checked_add_signed(chrono::Duration::days(days)))
        .map(|d| json!(d.to_string()))
        .unwrap_or(Value::Null)
}
/// Records the chat anchors that came before `before` (UTF-16) with the elapsed time at that point.
fn take_anchors(
    pending: &mut Vec<Value>,
    before: Option<i64>,
    source_id: &Value,
    elapsed: (i64, i64),
    chat: &mut Vec<(Value, i64, i64)>,
) {
    while pending
        .last()
        .is_some_and(|a| before.is_none_or(|b| n(a, "start") < b))
    {
        let mut a = pending.pop().unwrap();
        a["sourceId"] = source_id.clone();
        chat.push((a, elapsed.0, elapsed.1));
    }
}
pub fn derive_snapshots(prompt: &str, sources: &[Value], seed: Option<&Value>) -> Vec<Value> {
    let anchors = evidence(prompt, true);
    let (mut days, mut months) = (
        seed.map(|s| n(s, "days")).unwrap_or(0),
        seed.map(|s| n(s, "months")).unwrap_or(0),
    );
    // without a seed, the start date comes from the sheet ("Data: 2024-01-31"); with a seed, the seed wins
    let mut date = seed
        .map(|s| s["date"].clone())
        .unwrap_or_else(|| card_date(prompt).unwrap_or(Value::Null));
    // anchors said in the chat: (anchor, days, months already elapsed when it was said); the seed carries them with the elapsed time
    let mut chat: Vec<(Value, i64, i64)> = seed
        .into_iter()
        .flat_map(|s| {
            s["anchors"]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |a| (s, a))
        })
        .filter(|(_, a)| a["sourceId"].is_string())
        .map(|(s, a)| {
            let base = |unit: &str, elapsed: &str| n(a, unit) + n(s, unit) - n(a, elapsed);
            (
                a.clone(),
                base("days", "elapsedDays"),
                base("months", "elapsedMonths"),
            )
        })
        .collect();
    let mut snapshots = Vec::new();
    for source in sources {
        let (mut advances, mut boundaries, mut user_events, mut previous_assistant) = (
            Vec::new(),
            Vec::new(),
            Vec::<Value>::new(),
            Vec::<Value>::new(),
        );
        let mut previous_text = String::new();
        for part in source["parts"].as_array().into_iter().flatten() {
            let text = s(part, "text");
            let events = evidence(&text, false);
            let mut pending = if part["role"] == "user" {
                chat_anchors(&text)
            } else {
                Vec::new()
            };
            pending.reverse();
            let mut start = 0;
            let mut state = json!({"days":days,"months":months,"date":date});
            for (index, ev) in events.iter().enumerate() {
                take_anchors(
                    &mut pending,
                    Some(n(ev, "start")),
                    &part["id"],
                    (days, months),
                    &mut chat,
                );
                let same = |e: &Value| {
                    e["kind"] == ev["kind"]
                        && e["days"] == ev["days"]
                        && e["months"] == ev["months"]
                        && e["date"] == ev["date"]
                };
                let exact_repeat = !previous_text.trim().is_empty()
                    && text.trim().starts_with(previous_text.trim());
                let reiterated = part["role"] == "assistant"
                    && (user_events.get(index).is_some_and(same)
                        || exact_repeat && previous_assistant.get(index).is_some_and(same));
                if n(ev, "start") > start {
                    boundaries.push(json!({"sourceId":part["id"],"role":part["role"],"start":start,"end":ev["start"],"days":state["days"],"months":state["months"],"date":state["date"]}));
                }
                if !reiterated {
                    if ev["kind"] == "advance" {
                        days += n(ev, "days");
                        months += n(ev, "months");
                        date = move_date(date, n(ev, "days"), n(ev, "months"));
                    } else {
                        date = ev["date"].clone();
                    }
                    state = json!({"days":days,"months":months,"date":date});
                }
                let mut event = ev.clone();
                event["sourceId"] = part["id"].clone();
                event["disposition"] = json!(if reiterated { "reiteration" } else { "advance" });
                event["elapsedDays"] = json!(days);
                event["elapsedMonths"] = json!(months);
                event["currentDate"] = date.clone();
                advances.push(event);
                start = n(ev, "start");
            }
            take_anchors(&mut pending, None, &part["id"], (days, months), &mut chat);
            if rules::utf16_len(&text) > start as usize {
                boundaries.push(json!({"sourceId":part["id"],"role":part["role"],"start":start,"end":rules::utf16_len(&text),"days":state["days"],"months":state["months"],"date":state["date"]}));
            }
            if part["role"] == "user" {
                user_events.extend(events);
            } else {
                previous_assistant = events;
                previous_text = text;
                user_events.clear();
            }
        }
        let parts = source["parts"].as_array().cloned().unwrap_or_default();
        // sheet: relative to the scene's present; chat: relative to the moment it was said
        let elapsed = anchors
            .iter()
            .map(|a| (a, 0, 0))
            .chain(chat.iter().map(|(a, d, m)| (a, *d, *m)))
            .map(|(a, base_days, base_months)| {
                let mut a = a.clone();
                a["elapsedDays"] = json!(n(&a, "days") + days - base_days);
                a["elapsedMonths"] = json!(n(&a, "months") + months - base_months);
                a
            })
            .collect::<Vec<_>>();
        snapshots.push(json!({"interactionId":source["id"],"seq":source["seq"],"days":days,"months":months,"date":date,"advances":advances,"boundaries":boundaries,
            "versionIds":parts.iter().filter(|p|p["role"]=="assistant").map(|p|p["id"].clone()).collect::<Vec<_>>(),"sources":parts.iter().map(|p|json!({"id":p["id"],"role":p["role"]})).collect::<Vec<_>>(),"scenes":source["scenes"].as_array().cloned().unwrap_or_default(),"anchors":elapsed}));
    }
    snapshots
}
pub fn view(prompt: &str, snapshots: &[Value]) -> Value {
    let mut daily = Vec::<Value>::new();
    let mut index = HashMap::new();
    for snapshot in snapshots {
        for b in snapshot["boundaries"].as_array().into_iter().flatten() {
            let key = format!(
                "{}:{}:{}",
                b["months"],
                b["days"],
                b["date"].as_str().unwrap_or_default()
            );
            let idx=*index.entry(key.clone()).or_insert_with(||{daily.push(json!({"key":key,"days":b["days"],"months":b["months"],"date":b["date"],"blocks":[]}));daily.len()-1});
            let mut block = b.clone();
            block["interactionId"] = snapshot["interactionId"].clone();
            block["seq"] = snapshot["seq"].clone();
            daily[idx]["blocks"].as_array_mut().unwrap().push(block);
        }
    }
    json!({"version":3,"basis":"narrative","anchors":evidence(prompt,true),"current":snapshots.last(),"snapshots":snapshots,"days":daily})
}
pub fn derive(prompt: &str, sources: &[Value]) -> Value {
    view(prompt, &derive_snapshots(prompt, sources, None))
}
