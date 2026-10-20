use crate::{Error, Result, s};
use serde_json::{Value, json};
use std::sync::LazyLock;

/// Version of the preset definitions; every generation records it.
pub const PRESET_VERSION: u32 = 11;
/// Retired presets: old conversations that use them still open and generate with the current default preset.
const RETIRED_PRESETS: &[&str] = &["space-bunny"];
/// Routes that must not be exposed or selected (e.g. argon is excluded by request; space-bunny is retired).
const RETIRED_ROUTE_MARKS: &[&str] = &[
    "space-bunny",
    "argon",
    "nvidia/z-ai/glm-5.2",
];
pub fn selectable_route(route: &str) -> bool {
    !route.trim().is_empty() && route.len() <= 512 && !route.chars().any(char::is_control)
        && !RETIRED_ROUTE_MARKS.iter().any(|m| route.to_lowercase().contains(m))
}
static PRESETS: LazyLock<Vec<Value>> =
    LazyLock::new(|| serde_json::from_str(include_str!("presets.json")).unwrap());
pub fn by_id(id: &str) -> Value {
    PRESETS
        .iter()
        .find(|p| p["id"] == id)
        .unwrap_or_else(|| PRESETS.last().unwrap())
        .clone()
}
/// Compiled `match`/`exclude` patterns of each preset (presets.json), in file order. A route belongs to the first
/// preset with a matching pattern and no matching exclusion; anything else is the generic preset.
static MATCHERS: LazyLock<Vec<(String, Vec<regex::Regex>, Vec<regex::Regex>)>> =
    LazyLock::new(|| {
        let compile = |v: &Value| -> Vec<regex::Regex> {
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p.as_str())
                .map(|p| regex::Regex::new(p).expect("invalid pattern in presets.json"))
                .collect()
        };
        PRESETS
            .iter()
            .filter(|p| p["match"].is_array())
            .map(|p| (s(p, "id"), compile(&p["match"]), compile(&p["exclude"])))
            .collect()
    });
/// Preset of a route, matched on the lowercase route (the provider prefix, if any, is just part of the text).
pub fn for_route(route: &str) -> Value {
    if route.starts_with("mobile-api:") {
        return by_id("mobile-api");
    }
    let r = route.to_lowercase();
    let id = MATCHERS
        .iter()
        .find(|(_, m, x)| m.iter().any(|re| re.is_match(&r)) && !x.iter().any(|re| re.is_match(&r)))
        .map_or("generic", |(id, _, _)| id.as_str());
    by_id(id)
}
/// Effective preset route: old routes listed in `routeAliases` (e.g. Gemini medium/high) map to the current route.
pub fn alias_route(preset: &Value, route: &str) -> String {
    preset["routeAliases"][route]
        .as_str()
        .unwrap_or(route)
        .to_string()
}
/// Normalizes the (route, preset, effort) stored in a conversation to what the app accepts today.
/// A retired preset or route falls back to the default preset (first in the list, with its default route and effort);
/// replaced routes become the current route and presets without effort drop an old effort. Read-only:
/// the stored record is not changed.
pub fn migrate(preset_id: &str, route_id: &str, effort: &Value) -> (String, String, Value) {
    let lower = route_id.to_lowercase();
    if RETIRED_PRESETS.contains(&preset_id) || RETIRED_ROUTE_MARKS.iter().any(|m| lower.contains(m))
    {
        let d = &PRESETS[0];
        return (
            d["preferredRoutes"][0].as_str().unwrap_or_default().into(),
            d["id"].as_str().unwrap_or_default().into(),
            d["defaultEffort"].clone(),
        );
    }
    let preset = by_id(preset_id);
    let no_effort = preset["efforts"].as_array().is_some_and(Vec::is_empty);
    (
        alias_route(&preset, route_id),
        preset_id.to_string(),
        if no_effort {
            Value::Null
        } else {
            effort.clone()
        },
    )
}
/// Effective effort: when the route already sets the level (one route per level, e.g. Gemini), the route wins; otherwise the
/// valid request or the preset default.
pub fn effort_for(preset: &Value, route: &str, requested: &Value) -> Value {
    if let Some(e) = preset["effortByRoute"][route].as_str() {
        return json!(e);
    }
    let valid = requested.as_str().is_some_and(|r| {
        preset["efforts"]
            .as_array()
            .is_some_and(|all| all.iter().any(|e| e == r))
    });
    if valid {
        requested.clone()
    } else {
        preset["defaultEffort"].clone()
    }
}
pub fn public(ids: &[String], validation: &Value) -> Vec<Value> {
    let has_server_routes = ids.iter().any(|id| id != "mobile-local" && !id.starts_with("mobile-api:"));
    PRESETS
        .iter()
        .map(|p| {
            let mut out = p.clone();
            // routes listed by the providers + known direct routes served without being listed in /models
            let mut route_ids: Vec<String> = ids
                .iter()
                .filter(|id| {
                    for_route(id)["id"] == p["id"]
                        && p["routeAliases"][id.as_str()].is_null()
                        && selectable_route(id)
                })
                .cloned()
                .collect();
            if has_server_routes && !s(p, "id").starts_with("mobile-") {
                for u in p["unlistedRoutes"].as_array().into_iter().flatten() {
                    if let Some(u) = u.as_str().filter(|u| !route_ids.iter().any(|r| r == u)) {
                        route_ids.push(u.to_string());
                    }
                }
            }
            if let Some(grp) = p["groupRoute"].as_str() {
                if let Some(pos) = route_ids.iter().position(|r| r == grp) {
                    let item = route_ids.remove(pos);
                    route_ids.insert(0, item);
                }
            }
            let routes: Vec<_> = route_ids
                .iter()
                .map(|id| json!({"id":id,"validation":validation["routes"][id]}))
                .collect();
            out["available"] = if ids.is_empty() {
                Value::Null
            } else {
                json!(!routes.is_empty())
            };
            out["routes"] = json!(routes);
            if p["id"] == "generic" {
                out["label"] = json!("Outros modelos do servidor");
                out["selectable"] = json!(!route_ids.is_empty());
            }
            out
        })
        .filter(|p| p["id"] != "generic" || p["selectable"] == true)
        .collect()
}
pub fn resolve(body: &Value, current: Option<&Value>) -> Result<Value> {
    resolve_available(body, current, &[])
}
/// Unknown model families remain selectable only when advertised by a configured provider.
/// They use the generic request contract, without invented effort/thinking controls.
pub fn resolve_available(body: &Value, current: Option<&Value>, available: &[String]) -> Result<Value> {
    let default = std::env::var("DEFAULT_MODEL").unwrap_or("cbai/deepseek-v4.1-flash".into());
    let route = body["routeId"]
        .as_str()
        .or(body["model"].as_str())
        .or(current.and_then(|c| c["routeId"].as_str()))
        .unwrap_or(&default)
        .trim();
    let preset = if let Some(id) = body["presetId"].as_str() {
        if !PRESETS.iter().any(|p| p["id"] == id) {
            return Err(Error::new(400, "Preset desconhecido."));
        }
        let p = by_id(id);
        if id != "generic" && for_route(route)["id"] != id {
            return Err(Error::new(400, "Rota não pertence ao preset."));
        }
        p
    } else {
        for_route(route)
    };
    if preset["id"] == "generic"
        && !current.is_some_and(|c| c["routeId"] == route && c["presetId"] == "generic")
        && !(selectable_route(route) && available.iter().any(|id| id == route))
    {
        return Err(Error::new(
            400,
            "Escolha um modelo publicado pelos provedores configurados.",
        ));
    }
    let route = alias_route(&preset, route);
    let route = route.as_str();
    let requested = body.get("effort").unwrap_or(&Value::Null);
    let requested = if requested.is_null() {
        current
            .filter(|c| c["presetId"] == preset["id"] && c["routeId"] == route)
            .map(|c| c["effort"].clone())
            .unwrap_or(Value::Null)
    } else {
        requested.clone()
    };
    Ok(
        json!({"routeId":route,"presetId":preset["id"],"effort":effort_for(&preset, route, &requested)}),
    )
}

pub struct Plan {
    pub body: Value,
    pub controls: Value,
    pub capacity: Value,
    pub system_mode: String,
}
/// Inference-only plan; the stored route, preset and effort stay unchanged.
pub fn mobile_local_plan(conv: &Value, settings: &Value) -> Plan {
    let mut local = conv.clone();
    local["routeId"] = json!("mobile-local");
    local["presetId"] = json!("mobile-local");
    local["effort"] = Value::Null;
    let mut plan = request(&local, vec![], &json!({}));
    plan.controls["mobileCompact"] = json!(true);
    apply_length(&mut plan, settings);
    plan
}
/// Without a choice stored in the conversation, the preset default applies (`thinkingDefault`; GLM 5.2 starts
/// without thinking). Only a stored boolean changes the default.
pub fn thinking_enabled(preset: &Value, settings: &Value) -> bool {
    if preset["thinkingCanDisable"] != true {
        return true;
    }
    settings["thinkingEnabled"]
        .as_bool()
        .unwrap_or(preset["thinkingDefault"] != false)
}
/// The API cap includes text and, on some routes, thinking. The reserve is a planning
/// margin, not a separate reasoning control at the provider.
pub fn apply_length(plan: &mut Plan, settings: &Value) -> Value {
    let chars = crate::rules::reply_size(settings) as u64;
    // In "target" mode the API budget covers the hard cut (135%), so the provider never stops a reply that is
    // still inside the acceptable range.
    let (minimum, stretch, hard) = crate::rules::reply_bounds(settings);
    let visible = (hard as u64).div_ceil(3);
    let text_budget = (visible * 6).div_ceil(5);
    let effort = plan.controls["requestedEffort"].as_str();
    let thinking = if plan.controls["thinkingEnabled"] == false {
        0
    } else {
        match effort {
            // no effort chosen: only the minimum margin the preset declares (Gemini cannot turn thinking off)
            None | Some("none" | "off") => plan.controls["reasoningFloor"].as_u64().unwrap_or(0),
            Some("low") => 2048,
            Some("medium") => 4096,
            _ => 8192,
        }
    };
    let max_output = plan.capacity["maxOutput"].as_u64().unwrap_or(8192);
    let total = (text_budget + thinking).min(max_output);
    plan.body["max_tokens"] = json!(total);
    plan.controls["sentParams"]["max_tokens"] = json!(total);
    let length = json!({"maxCharacters":chars,"minCharacters":minimum,"rangeMaxCharacters":stretch,"hardCharacters":hard,"mode":if settings["lengthMode"]=="target"{"target"}else{"maximum"},"visibleTokens":visible,"textTokenBudget":text_budget.min(total),"reasoningReserve":total.saturating_sub(text_budget),"apiMaxTokens":total,"thinkingEnabled":plan.controls["thinkingEnabled"],"method":"estimated","monologueChars":settings["monologueChars"].as_u64().unwrap_or(300),"turnPacing":settings["turnPacing"].as_str().unwrap_or("interactive")});
    plan.controls["length"] = length.clone();
    length
}
pub fn default_safety_settings() -> Value {
    json!([
        {
            "category": "HARM_CATEGORY_HARASSMENT",
            "threshold": "BLOCK_NONE"
        },
        {
            "category": "HARM_CATEGORY_HATE_SPEECH",
            "threshold": "BLOCK_NONE"
        },
        {
            "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT",
            "threshold": "BLOCK_NONE"
        },
        {
            "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
            "threshold": "BLOCK_NONE"
        }
    ])
}
pub fn unrestricted_safety_enabled(conv: &Value) -> bool {
    if std::env::var("MC_UNRESTRICTED_SAFETY").as_deref() == Ok("0") {
        return false;
    }
    conv["settings"]["unrestrictedSafety"]
        .as_bool()
        .unwrap_or(true)
}
pub fn request(conv: &Value, messages: Vec<Value>, validation: &Value) -> Plan {
    let preset = by_id(&s(conv, "presetId"));
    let route = alias_route(&preset, &s(conv, "routeId"));
    let rv = &validation["routes"][&route];
    let effort = &effort_for(&preset, &route, &conv["effort"]);
    let thinking_enabled = thinking_enabled(&preset, &conv["settings"]);
    let shape = rv["shape"].as_str().unwrap_or("reasoning_effort");
    let mut body = json!({"model":route,"messages":messages,"stream":true,"stream_options":{"include_usage":true}});
    let unrestricted = unrestricted_safety_enabled(conv);
    if unrestricted {
        let safety = conv["settings"]["safetySettings"]
            .as_array()
            .cloned()
            .map(Value::Array)
            .unwrap_or_else(default_safety_settings);
        body["safetySettings"] = safety.clone();
        body["safety_settings"] = safety;
    }
    if !thinking_enabled {
        // 9Router converts none into the native control: disabled on DeepSeek
        // and between_tools on Sonnet 5.5. The chosen effort is kept.
        // GLM 5.2 (cbai) is the opposite: any reasoning_effort, including none, turns thinking ON;
        // with no control at all it answers directly (measured through 9Router on 2026-10-12). Hence "omit".
        if preset["thinkingOffShape"] != "omit" {
            body["reasoning_effort"] = json!("none");
        }
    } else if !effort.is_null() {
        match shape {
            "reasoning_object" => body["reasoning"] = json!({"effort":effort}),
            "output_config" => body["output_config"] = json!({"effort":effort}),
            "claude_adaptive" => {
                body["output_config"] = json!({"effort":effort});
                body["thinking"] = json!({"type":"adaptive"});
            }
            "deepseek_thinking" => {
                body["thinking"] = json!({"type":"enabled"});
                body["reasoning_effort"] = effort.clone();
            }
            _ => body["reasoning_effort"] = effort.clone(),
        }
    }
    let temperature_sent = preset["sampling"] == "temperature" && conv["temperature"].is_number();
    if temperature_sent {
        body["temperature"] = conv["temperature"].clone();
        let top_p = conv["settings"]["top_p"].as_f64().unwrap_or(0.95);
        body["top_p"] = json!(top_p);
    }
    let max_output = rv["maxOutput"]
        .as_u64()
        .unwrap_or(preset["maxOutput"].as_u64().unwrap_or(8192));
    body["max_tokens"] = json!(
        preset["providerReserve"]
            .as_u64()
            .unwrap_or(8192)
            .min(max_output)
    );
    let sent: serde_json::Map<String, Value> = body
        .as_object()
        .unwrap()
        .iter()
        .filter(|(k, _)| {
            matches!(
                k.as_str(),
                "reasoning_effort" | "reasoning" | "output_config" | "thinking" | "max_tokens"
            )
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let confirmation = if effort.is_null() {
        "not_applicable"
    } else {
        rv["efforts"][effort.as_str().unwrap_or_default()]["verdict"]
            .as_str()
            .or(rv["control"].as_str())
            .unwrap_or("not_validated")
    };
    let mut controls = json!({"family":preset["family"],"requestedEffort":effort,"shape":if effort.is_null(){Value::Null}else{json!(shape)},"sentParams":sent,"temperatureSent":temperature_sent,"confirmation":confirmation});
    controls["thinkingEnabled"] = json!(thinking_enabled);
    controls["thinkingCanDisable"] = json!(preset["thinkingCanDisable"] == true);
    controls["thinkingOffType"] = preset["thinkingOffType"].clone();
    controls["sampling"] = preset["sampling"].clone();
    controls["usesTemperature"] = json!(preset["sampling"] == "temperature");
    if let Some(floor) = preset["reasoningFloor"].as_u64() {
        controls["reasoningFloor"] = json!(floor);
    }
    controls["unrestrictedSafety"] = json!(unrestricted);
    Plan {
        body,
        controls,
        capacity: json!({"contextWindow":rv["contextWindow"].as_u64().unwrap_or(preset["contextWindow"].as_u64().unwrap_or(128000)),"maxOutput":max_output,"source":if rv["maxOutput"].is_number(){json!("validated")}else{preset["capacitySource"].clone()}}),
        system_mode: {
            let r = route.to_lowercase();
            let is_cbai_or_deepseek = r.starts_with("cbai/")
                || r.contains("cbai")
                || r.contains("codebuddy")
                || r.contains("deepseek")
                || preset["family"] == "deepseek"
                || (preset["family"] == "glm" && r.contains("cbai"));
            if conv["settings"]["rulesPlacement"] == "first_turn"
                || rv["systemRole"] == "dropped"
                || preset["systemMode"] == "user_prefix"
                || is_cbai_or_deepseek
            {
                "user_prefix"
            } else {
                "system"
            }
            .into()
        },
    }
}
