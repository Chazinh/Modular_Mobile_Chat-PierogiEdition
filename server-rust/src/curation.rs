//! Memory curation (memory × context management), same logic as the former Node implementation.
//! Group selection is deterministic (near-duplicate memories); the local model only decides whether they are the same
//! fact and writes ONE sentence summarizing them. A guard rejects any text that invents something outside the group.
use crate::rules;
use serde_json::{Value, json};
use std::collections::HashSet;

pub const MIN_ACTIVE: i64 = 60;
pub const MIN_NEW: i64 = 20;
pub const CLUSTER_MIN_SIMILARITY: f64 = 0.55;
pub const MERGED_MAX: usize = 400;

pub fn schema() -> Value {
    json!({"type":"object","properties":{"decisions":{"type":"array","items":{"type":"object","properties":{
        "group":{"type":"integer"},"action":{"type":"string","enum":["merge","keep_all"]},
        "keep_id":{"type":"string"},"merged":{"type":"string"}},"required":["group","action"],"additionalProperties":false}}},
        "required":["decisions"],"additionalProperties":false})
}

fn text(m: &Value, key: &str) -> String {
    m[key].as_str().unwrap_or_default().to_owned()
}

/// Groups of near-duplicate memories of the same type and subject (at most 6 groups of up to 5 memories).
pub fn build_clusters(memories: &[Value]) -> Vec<Vec<Value>> {
    let (max, max_size) = (6, 5);
    let prepared = memories
        .iter()
        .map(|m| {
            (
                m["id"].as_str().unwrap_or_default(),
                m["kind"].as_str().unwrap_or_default(),
                rules::key(m["subject"].as_str().unwrap_or_default()),
                rules::tokens(m["content"].as_str().unwrap_or_default()),
            )
        })
        .collect::<Vec<_>>();
    let mut used = HashSet::new();
    let mut out = Vec::new();
    for (i, a) in prepared.iter().enumerate() {
        if out.len() >= max {
            break;
        }
        if used.contains(a.0) {
            continue;
        }
        let mut group = vec![i];
        for (j, b) in prepared.iter().enumerate().skip(i + 1) {
            if group.len() >= max_size {
                break;
            }
            if used.contains(b.0) || b.1 != a.1 || b.2 != a.2 {
                continue;
            }
            if group.iter().all(|&g| {
                let tokens = &prepared[g].3;
                let union = tokens.union(&b.3).count();
                let similarity = if union == 0 {
                    0.0
                } else {
                    tokens.intersection(&b.3).count() as f64 / union as f64
                };
                similarity >= CLUSTER_MIN_SIMILARITY
            }) {
                group.push(j);
            }
        }
        if group.len() > 1 {
            for &g in &group {
                used.insert(prepared[g].0);
            }
            out.push(group.into_iter().map(|g| memories[g].clone()).collect());
        }
    }
    out
}

pub fn messages(clusters: &[Vec<Value>]) -> Vec<Value> {
    let body = clusters
        .iter()
        .enumerate()
        .map(|(i, group)| {
            format!(
                "GRUPO {}:\n{}",
                i + 1,
                group
                    .iter()
                    .map(|m| format!("- id={}: {}", text(m, "id"), text(m, "content")))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    vec![
        json!({"role":"system","content":"Você faz a curadoria da memória de uma conversa com personagens para economizar contexto. Cada GRUPO reúne memórias parecidas. Se forem o MESMO fato, responda action=\"merge\", escolha keep_id (uma das ids do grupo) e escreva em \"merged\" UMA frase curta em português que preserve tudo o que importa, SEM acrescentar nada que não esteja nas memórias. Se forem fatos diferentes, responda action=\"keep_all\". Na dúvida, keep_all. Responda somente JSON."}),
        json!({"role":"user","content":body}),
    ]
}

/// Safe decisions only: [{keepId, dropIds, merged}]. Everything else is ignored.
pub fn sanitize(raw: &Value, clusters: &[Vec<Value>]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for d in raw["decisions"].as_array().into_iter().flatten() {
        let Some(idx) = d["group"]
            .as_i64()
            .and_then(|g| g.checked_sub(1))
            .and_then(|g| usize::try_from(g).ok())
        else {
            continue;
        };
        let Some(group) = clusters.get(idx) else {
            continue;
        };
        if seen.contains(&idx) || d["action"] != "merge" {
            continue;
        }
        let Some(keep) = d["keep_id"]
            .as_str()
            .filter(|k| group.iter().any(|m| m["id"] == *k))
        else {
            continue;
        };
        let merged = d["merged"]
            .as_str()
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let len = merged.chars().count();
        if !(12..=MERGED_MAX).contains(&len) {
            continue;
        }
        let union: HashSet<String> = group
            .iter()
            .flat_map(|m| rules::tokens(&text(m, "content")))
            .collect();
        let mt = rules::tokens(&merged);
        if mt.is_empty() || union.is_empty() {
            continue;
        }
        let shared = mt.iter().filter(|t| union.contains(*t)).count() as f64;
        if shared / (mt.len() as f64) < 0.85 || shared / (union.len() as f64) < 0.6 {
            continue;
        }
        seen.insert(idx);
        out.push(json!({
            "keepId": keep,
            "dropIds": group.iter().filter(|m| m["id"] != keep).map(|m| m["id"].clone()).collect::<Vec<_>>(),
            "merged": merged,
        }));
    }
    out
}
