//! Disposable model-facing views. Stored memories and identified text sources stay intact.
use crate::{provenance, s};
use serde_json::{Value, json};
use std::collections::HashSet;

fn pick(value: &Value, fields: &[&str]) -> Value {
    Value::Object(
        fields
            .iter()
            .filter_map(|&field| {
                value
                    .get(field)
                    .filter(|v| !v.is_null())
                    .map(|v| (field.to_owned(), v.clone()))
            })
            .collect(),
    )
}

fn items(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn source_fields(value: &Value, out: &mut Value) {
    // These are functional citation identities/offsets, unlike memory/cache UUIDs.
    if value["sourceId"].as_str().is_some_and(|id| !id.is_empty()) {
        for field in [
            "sourceId",
            "sourceRole",
            "role",
            "canonicalSpeaker",
            "start",
            "end",
            "offsets",
            "evidence",
        ] {
            if let Some(v) = value.get(field).filter(|v| !v.is_null()) {
                out[field] = v.clone();
            }
        }
    }
}

fn memory(value: &Value) -> Value {
    let mut out = pick(value, &["kind", "subject", "content", "visibility"]);
    if out.get("content").is_none() {
        if let Some(text) = value.get("text") {
            out["content"] = text.clone();
        }
    }
    if out.get("kind").is_none() && value["type"].as_str().is_some_and(|t| t != "memory") {
        out["kind"] = value["type"].clone();
    }
    if value["knowledge"].is_array() {
        out["knowledge"] = json!(
            items(&value["knowledge"])
                .iter()
                .map(|k| {
                    let mut row = pick(k, &["name", "state", "belief", "via", "evidence"]);
                    if row.get("name").is_none() && k["who"].is_string() {
                        row["name"] = k["who"].clone();
                    }
                    source_fields(k, &mut row);
                    row
                })
                .collect::<Vec<_>>()
        );
    }
    if value["attribution"].is_object() {
        let a = &value["attribution"];
        let mut attribution = pick(
            a,
            &[
                "quote",
                "originalSpeaker",
                "originalEvidence",
                "citingSpeaker",
                "citingEvidence",
                "referent",
                "use",
            ],
        );
        source_fields(a, &mut attribution);
        out["attribution"] = attribution;
    }
    source_fields(value, &mut out);
    out
}

fn memories<'a>(value: &'a Value, seen: &mut HashSet<&'a str>) -> Value {
    json!(
        items(value)
            .iter()
            .filter(|m| {
                m["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .is_none_or(|id| seen.insert(id))
            })
            .map(memory)
            .collect::<Vec<_>>()
    )
}

fn scene(value: &Value) -> Value {
    if !value.is_object() {
        return value.clone();
    }
    let mut out = pick(
        value,
        &[
            "location",
            "time",
            "mood",
            "present",
            "objects",
            "positions",
            "locations",
            "holding",
        ],
    );
    if value["characters"].is_array() {
        out["characters"] = json!(
            items(&value["characters"])
                .iter()
                .map(|c| pick(c, &["name", "location", "position", "holding"]))
                .collect::<Vec<_>>()
        );
    }
    out
}

fn time(value: &Value) -> Value {
    if !value.is_object() {
        return value.clone();
    }
    let mut out = pick(value, &["days", "months", "date"]);
    for field in ["anchors", "advances"] {
        if value[field].is_array() {
            out[field] = json!(
                items(&value[field])
                    .iter()
                    .map(|a| {
                        pick(
                            a,
                            &[
                                "kind",
                                "days",
                                "months",
                                "date",
                                "elapsedDays",
                                "elapsedMonths",
                                "evidence",
                            ],
                        )
                    })
                    .collect::<Vec<_>>()
            );
        }
    }
    if value["scenes"].is_array() {
        out["scenes"] = json!(
            items(&value["scenes"])
                .iter()
                .map(scene)
                .collect::<Vec<_>>()
        );
    }
    out
}

/// Full desktop extraction context; compact mode and retry truncation remain with the caller.
pub fn desktop(
    input: &Value,
    current_scene: &Value,
    related: &Value,
    retrieval: &Value,
    text: &str,
    previous: &str,
    sources: &[Value],
) -> String {
    let mut seen = HashSet::new();
    let related = memories(related, &mut seen);
    let recovered = memories(&retrieval["candidates"], &mut seen);
    format!(
        "PERSONA: {}\nCENA ANTERIOR: {}\nTEMPO: {}\nFATOS ANTERIORES: {related}\nRECUPERAÇÃO: {recovered}\nTHREADS: {}\nHISTÓRICO ANTERIOR RELEVANTE (fontes identificadas):\n{previous}\n{}\nTEXTO DO TURNO (fragmento integral):\n{text}",
        input["conv"]["persona"],
        scene(current_scene),
        time(&input["timeline"]["current"]),
        input["threads"],
        provenance::block(&s(input, "userText"), sources),
    )
}
