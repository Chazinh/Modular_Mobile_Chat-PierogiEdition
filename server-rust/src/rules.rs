use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, sync::LazyLock};
use unicode_normalization::UnicodeNormalization;
pub const USER_LIMIT: usize = 6000;
pub const BOT_LIMIT: usize = 8000;
/// Largest stored model reply: the biggest selectable size (BOT_LIMIT) plus the 35% an approximate target may use.
pub const REPLY_STORE_LIMIT: usize = BOT_LIMIT * TARGET_CEILING_PCT / 100;
/// "Approximate target": the chosen size is the centre, not a ceiling. The reply should land between 85% and
/// 125% of it and is only cut at 135%, for every size (2,000 → 1,700–2,500, cut at 2,700; 8,000 → cut at 10,800).
pub const TARGET_MIN_PCT: usize = 85;
pub const TARGET_STRETCH_PCT: usize = 125;
pub const TARGET_CEILING_PCT: usize = 135;
/// Configured reply size (500–BOT_LIMIT).
pub fn reply_size(settings: &Value) -> usize {
    (settings["botChars"].as_u64().unwrap_or(BOT_LIMIT as u64) as usize).clamp(500, BOT_LIMIT)
}
pub fn target_mode(settings: &Value) -> bool {
    settings["lengthMode"] == "target"
}
fn scaled(size: usize, pct: usize) -> usize {
    (size * pct).div_ceil(100)
}
/// (minimum, upper end of the acceptable range, hard cut) in UTF-16 units.
pub fn reply_bounds(settings: &Value) -> (usize, usize, usize) {
    let size = reply_size(settings);
    if target_mode(settings) {
        (scaled(size, TARGET_MIN_PCT), scaled(size, TARGET_STRETCH_PCT), scaled(size, TARGET_CEILING_PCT))
    } else {
        (0, size, size)
    }
}
/// A reply cut by the hard limit ends at the last complete sentence (or paragraph) instead of mid-word.
/// Keeps the text untouched when no boundary is found in its second half.
pub fn trim_to_sentence(t: &str) -> &str {
    let mut best = None;
    let mut chars = t.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|(_, n)| *n);
        let closes = matches!(c, '.' | '!' | '?' | '…' | '*' | ')' | '"' | '”' | '»');
        if (closes && next.is_none_or(char::is_whitespace)) || (c == '\n' && next == Some('\n')) {
            best = Some(i + c.len_utf8());
        }
    }
    match best {
        Some(end) if end * 2 >= t.len() => t[..end].trim_end(),
        _ => t,
    }
}
pub fn utf16_len(t: &str) -> usize {
    t.encode_utf16().count()
}
pub fn byte_at(t: &str, offset: usize) -> usize {
    let mut units = 0;
    for (b, c) in t.char_indices() {
        if units + c.len_utf16() > offset {
            return b;
        }
        units += c.len_utf16();
    }
    t.len()
}
pub fn clip(t: &str, units: usize) -> &str {
    &t[..byte_at(t, units)]
}
// Legitimate client offsets always fall on character boundaries. Rejecting half an emoji avoids duplication.
pub fn suffix(t: &str, units: usize) -> Option<&str> {
    let b = byte_at(t, units);
    if utf16_len(&t[..b]) == units {
        Some(&t[b..])
    } else {
        None
    }
}
pub fn chunks(t: &str, size: usize, overlap: usize) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut a = 0usize;
    while a < t.len() {
        let mut b = a;
        let mut end = start;
        // Walk only the next fragment. Re-scanning UTF-16 prefixes here made
        // multi-megabyte legacy imports quadratic in the number of fragments.
        for c in t[a..].chars() {
            if end - start + c.len_utf16() > size.max(2) {
                break;
            }
            b += c.len_utf8();
            end += c.len_utf16();
        }
        if b < t.len() {
            let seg = &t[a..b];
            if let Some(at) = seg.rfind('\n') {
                let len = utf16_len(&seg[..at]) + 1;
                if len as f64 > size as f64 * 0.6 {
                    end = start + len;
                    b = a + at + 1;
                }
            }
        }
        out.push((start, end, t[a..b].to_owned()));
        if b == t.len() {
            break;
        }
        let raw = (start + 1).max(end.saturating_sub(overlap.min(size / 2)));
        // Round UP inside a surrogate pair, and advance both cursors locally.
        // At most half of the current fragment is revisited for overlap.
        for c in t[a..b].chars() {
            if start >= raw {
                break;
            }
            a += c.len_utf8();
            start += c.len_utf16();
        }
    }
    out
}
pub fn hash(t: &str) -> String {
    format!("{:x}", Sha256::digest(t.as_bytes()))
}
pub fn fold(t: &str) -> String {
    t.nfd()
        .filter(|c| !('\u{0300}'..='\u{036f}').contains(c))
        .collect::<String>()
        .to_lowercase()
}
pub fn key(t: &str) -> String {
    fold(t).trim().to_owned()
}
pub fn fact_key(t: &str) -> String {
    static SEPARATORS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\p{L}\p{N}]+").unwrap());
    SEPARATORS.replace_all(&fold(t), " ").trim().to_owned()
}
static PIECE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{L}\p{N}]+|[^\s\p{L}\p{N}]|\n").unwrap());
pub fn estimate(t: &str) -> usize {
    PIECE
        .find_iter(t)
        .map(|m| {
            let p = m.as_str();
            if p == "\n" {
                0.5
            } else if p.chars().next().is_some_and(char::is_alphanumeric) {
                let l = utf16_len(p);
                if l <= 3 { 1.0 } else { (l as f64 / 3.6).ceil() }
            } else {
                1.0
            }
        })
        .sum::<f64>()
        .ceil() as usize
}
pub fn budget(t: &str, calibration: Option<f64>) -> usize {
    budget_raw(estimate(t), calibration)
}
pub fn budget_raw(raw: usize, calibration: Option<f64>) -> usize {
    let ratio = calibration
        .filter(|c| c.is_finite() && *c != 0.0)
        .unwrap_or(1.0)
        .clamp(0.5, 2.0);
    (raw as f64 * ratio * 1.2).ceil() as usize
}
pub fn estimate_messages(messages: &[Value]) -> usize {
    messages
        .iter()
        .map(|m| estimate(m["content"].as_str().unwrap_or_default()) + 4)
        .sum::<usize>()
        + 3
}
static WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new("[a-z0-9]{3,}").unwrap());
const STOP: &str = "de da do das dos e o a os as um uma uns umas em no na nos nas para por com sem que se ao aos como mais mas ou foi era ser tem tinha seu sua seus suas ele ela eles elas isso isto esse essa este esta lhe lhes me te nos vos ja nao sim muito pouco entre sobre ate apos";
pub fn tokens(t: &str) -> HashSet<String> {
    WORDS
        .find_iter(&fold(t))
        .map(|x| x.as_str().to_owned())
        .filter(|x| !STOP.split_whitespace().any(|y| y == x))
        .collect()
}
pub fn overlap(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        0.0
    } else {
        a.intersection(b).count() as f64 / a.len().min(b.len()) as f64
    }
}
pub fn jaccard(a: &str, b: &str) -> f64 {
    let a = tokens(a);
    let b = tokens(b);
    let u = a.union(&b).count();
    if u == 0 {
        0.0
    } else {
        a.intersection(&b).count() as f64 / u as f64
    }
}
pub fn fts(t: &str, any: bool) -> String {
    static TERMS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z0-9]{2,}").unwrap());
    let folded = fold(t);
    let mut seen = HashSet::new();
    let terms: Vec<_> = TERMS
        .find_iter(&folded)
        .map(|m| m.as_str())
        .filter(|x| seen.insert(*x))
        .take(if any { 64 } else { 24 })
        .collect();
    terms
        .iter()
        .enumerate()
        .map(|(i, x)| {
            format!(
                "\"{x}\"{}",
                if !any && i + 1 == terms.len() {
                    "*"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join(if any { " OR " } else { " " })
}
pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return 0.0;
    }
    let (mut d, mut x, mut y) = (0.0, 0.0, 0.0);
    for (&a, &b) in a.iter().zip(b) {
        let a = f64::from(a);
        let b = f64::from(b);
        d += a * b;
        x += a * a;
        y += b * b;
    }
    if x * y == 0.0 {
        0.0
    } else {
        d / (x * y).sqrt()
    }
}
pub fn vector_blob(v: &[f32]) -> crate::Result<Vec<u8>> {
    if v.is_empty() || v.len() > 8192 || v.iter().any(|x| !x.is_finite()) {
        return Err(crate::Error::new(422, "Vetor de memória inválido."));
    }
    Ok(v.iter().flat_map(|x| x.to_le_bytes()).collect())
}
pub fn decode_vector(blob: &[u8], dim: usize) -> Option<Vec<f32>> {
    if dim == 0 || dim > 8192 || blob.len() != dim.checked_mul(4)? {
        return None;
    }
    let v: Vec<f32> = blob
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    if v.iter().all(|x| x.is_finite()) {
        Some(v)
    } else {
        None
    }
}

#[derive(Default)]
pub struct ThinkFilter {
    buf: String,
    inside: bool,
    pub hidden: usize,
}
impl ThinkFilter {
    pub fn push(&mut self, chunk: &str) -> String {
        self.buf.push_str(chunk);
        let mut out = String::new();
        loop {
            let tags = if self.inside {
                &["</think>", "</thinking>"]
            } else {
                &["<think>", "<thinking>"]
            };
            let found = tags
                .iter()
                .filter_map(|t| self.buf.find(t).map(|i| (i, t.len())))
                .min_by_key(|x| x.0);
            if let Some((i, l)) = found {
                if self.inside {
                    self.hidden += utf16_len(&self.buf[..i]);
                } else {
                    out.push_str(&self.buf[..i]);
                }
                self.buf.drain(..i + l);
                if self.inside {
                    self.buf = self.buf.trim_start().to_owned();
                }
                self.inside = !self.inside;
                continue;
            }
            let keep = tags
                .iter()
                .flat_map(|t| 1..t.len())
                .filter(|&n| {
                    n <= self.buf.len()
                        && self.buf.is_char_boundary(self.buf.len() - n)
                        && tags
                            .iter()
                            .any(|t| t.starts_with(&self.buf[self.buf.len() - n..]))
                })
                .max()
                .unwrap_or(0);
            let end = self.buf.len() - keep;
            if self.inside {
                self.hidden += utf16_len(&self.buf[..end]);
            } else {
                out.push_str(&self.buf[..end]);
            }
            self.buf.drain(..end);
            return out;
        }
    }
    pub fn flush(&mut self) -> String {
        if self.inside {
            self.hidden += utf16_len(&self.buf);
            self.buf.clear();
            String::new()
        } else {
            std::mem::take(&mut self.buf)
        }
    }
}
pub fn length_hint(t: &str, present: usize, cap: usize) -> Value {
    let text = fold(t);
    let mut why = Vec::new();
    if estimate(t) > 300 {
        why.push("entrada longa");
    }
    if present >= 3 {
        why.push("vários personagens");
    }
    for (pat, label) in [
        (
            r"\b(vou para|vamos para|entro|entrar|saio|sair|chego|chegar|subo|desco|viajo|volto para|vou ate|nos mudamos)",
            "mudança de local",
        ),
        (
            r"\b(briga|brigar|discut|grit|chor|raiva|odeio|trai|ciume|ciumes|magoa|ameac|desculpa|medo|sofr|saudade|termin)",
            "conflito emocional",
        ),
        (
            r"\b(segredo|confess|revel|verdade|descobr|admit|contar a verdade|mentir|menti)",
            "revelação",
        ),
        (
            r"\b(ataca|atacar|golpe|luta|lutar|corre|correr|pula|pular|empurr|soco|chute|espada|arma|tiro|persegu|foge|fugir|agarr|derrub)",
            "ação física",
        ),
    ] {
        if Regex::new(pat).unwrap().is_match(&text) {
            why.push(label);
        }
    }
    if text.matches('?').count() >= 2
        || Regex::new(r"[.!?]\s").unwrap().find_iter(&text).count() >= 4
    {
        why.push("várias perguntas/ações");
    }
    let table = [650, 850, 1100, 1500, 1900, 2300, 2700];
    json!({"score":why.len(),"visibleTokens":cap.min(table[why.len().min(6)]),"reasons":why})
}
