//! Conservative card/name parsing, shared by the immediate cast and local extraction.
use crate::{rules, s};
use regex::Regex;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::LazyLock};

pub fn clean(text: &str, max: usize) -> String {
    rules::clip(&text.split_whitespace().collect::<Vec<_>>().join(" "), max).to_owned()
}
pub fn name(text: &str) -> Option<String> {
    static VALID: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\p{Lu}[\p{L}\p{M}'’.\- ]*$").unwrap());
    let name = clean(&text.replace(['*', '_', '"', '“', '”', '`'], ""), 100);
    if !(2..=40).contains(&rules::utf16_len(&name)) || !VALID.is_match(&name) {
        return None;
    }
    let key = rules::key(&name);
    if "ele ela eles elas voce eu nos ninguem todos alguem narrador usuario user personagem o a um uma tudo nada".split_whitespace().any(|n|n==key) {return None;}
    if name.split_whitespace().skip(1).any(|word| {
        !word.chars().next().is_some_and(char::is_uppercase)
            && !"de da do das dos di du van von la le el al bin ibn"
                .split_whitespace()
                .any(|p| p == word.to_lowercase())
    }) {
        return None;
    }
    Some(name)
}
pub fn known(text: &str, max: usize) -> Option<String> {
    static UNKNOWN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)^(n/a|na|nenhum|nenhuma|desconhecid[oa]|incert[oa]|indefinid[oa]|n[aã]o (informad|especificad|definid|mencionad)[oa]|momento incerto|sem informa[cç][aã]o)(?:$|[^\p{L}])").unwrap()
    });
    let text = clean(text, max);
    (!text.is_empty() && !UNKNOWN.is_match(&text)).then_some(text)
}
struct Header {
    name: String,
    title: Option<String>,
    paren: Option<String>,
    words: usize,
}
static PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\(([^)]*)\)\s*$").unwrap());
static TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(prof(?:essor|essora)?\.?ª?\.?|profª\.?|dra?\.|dr\.|sra?\.|sr\.|srta\.|diretora?|vice-diretora?|madre|irm[ãa]o?|padre|pastor|capit[ãa]o?|tenente|general|comandante|lorde|lady|dona|dom|mestre|mestra|doutora?|senhora?)\s+").unwrap()
});
static NON_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)academia|instituto|escola|col[ée]gio|universidade|cidade|reino|imp[ée]rio|mundo|sistema|ranking|tipos|hierarquia|magias?\b|ra[çc]as|linha|leis|diretrizes|cen[áa]rio|regras|instala[çc]|alojament|sal[ãa]o|biblioteca|jardim|arena|sala\b|refeit|p[áa]tio|catedral|igreja|templo|castelo|torre|floresta|vila|bairro|pa[íi]s|na[çc][ãa]o|guerra|evento|cap[íi]tulo|parte\b|se[çc][ãa]o").unwrap()
});
static SECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)personage|elenco|corpo docente|docente|funcion[áa]ri|professor|magos|alunos|estudantes|npcs?\b|characters?|\bcast\b|staff|membros|equipe|fam[íi]lia|tripula|integrantes|externos|secund[áa]ri|coadjuvante|antagonista|vil[õo]es|aliados").unwrap()
});
const FIELDS: &str = "nome name personagem character personalidade aparencia idade genero cenario historia historico regras regra mundo gostos habilidades habilidade poderes poder familia relacoes relacao objetivo objetivos contexto descricao observacao observacoes notas nota exemplo exemplos formato estilo tom resumo local ambiente epoca dados gerais geral aptidao aptidoes origem perfil comportamento condicao fisico fisica tracos habitos hobbies profissao funcao cargo papel classe raca especie sangue magia magias armas equipamento backstory appearance personality skills background info informacoes status atributos medidas altura peso curiosidades segredos segredo motivacao medos fraquezas forcas";
fn header(text: &str) -> Option<Header> {
    let mut text = clean(&text.replace(['*', '_', '#', '>', '"', '“', '”'], ""), 200);
    let paren = PAREN.captures(&text).map(|c| c[1].to_owned());
    text = PAREN.replace(&text, "").trim().to_owned();
    let title = TITLE.find(&text).map(|m| m.as_str().trim().to_owned());
    text = TITLE.replace(&text, "").trim().to_owned();
    let name = name(&text)?;
    let words = name.split_whitespace().count();
    if words > 4
        || NON_NAME.is_match(&name)
        || name
            .split_whitespace()
            .any(|w| FIELDS.split_whitespace().any(|f| f == rules::key(w)))
    {
        return None;
    }
    Some(Header {
        name,
        title,
        paren,
        words,
    })
}
fn add(
    out: &mut Vec<Value>,
    map: &mut HashMap<String, usize>,
    h: Header,
    kind: &str,
    first: &str,
    persona: &str,
) -> Option<usize> {
    let key = rules::key(&h.name);
    if key == rules::key(persona) {
        return None;
    }
    let idx = *map.entry(key).or_insert_with(|| {
        let idx = out.len();
        out.push(json!({"name":h.name,"aliases":[],"parts":[],"kind":kind}));
        idx
    });
    if let Some(title) = h.title {
        let words = h.name.split_whitespace().collect::<Vec<_>>();
        for word in [words[0], words[words.len() - 1]] {
            let alias = json!(format!("{title} {word}"));
            let aliases = out[idx]["aliases"].as_array_mut().unwrap();
            if !aliases.contains(&alias) {
                aliases.push(alias);
            }
        }
    }
    if let Some(paren) = h.paren {
        out[idx]["parts"].as_array_mut().unwrap().push(json!(paren));
    }
    if !first.is_empty() {
        out[idx]["parts"].as_array_mut().unwrap().push(json!(first));
    }
    Some(idx)
}
pub fn from_card(text: &str, persona: &str) -> Vec<Value> {
    static YOU: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\bvoc[eê]\s+(?:é|e|interpreta|ser[áa])\s+(?:a\s+|o\s+)?(\p{Lu}[\p{L}\p{M}'’-]+(?:\s+\p{Lu}[\p{L}\p{M}'’-]+)?)").unwrap()
    });
    static FREE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(\p{Lu}[\p{L}\p{M}'’-]+(?:\s+\p{Lu}[\p{L}\p{M}'’-]+)?)\s*:\s*(.{12,})$")
            .unwrap()
    });
    let (mut out, mut map) = (Vec::<Value>::new(), HashMap::new());
    let (mut in_chars, mut saw_section, mut kind, mut current) =
        (false, false, "npc", None::<usize>);
    for raw in text.lines() {
        let line = raw
            .trim_start_matches(|c: char| c.is_whitespace() || "-*•#>".contains(c))
            .trim();
        if line.is_empty() {
            continue;
        }
        let colon = line.find(':');
        let standalone = colon.is_none_or(|c| c == line.len() - 1);
        if standalone {
            let line = line.trim_end_matches(':').trim();
            if let Some(c) = YOU.captures(line) {
                if let Some(h) = header(&c[1]) {
                    current = add(&mut out, &mut map, h, "main", line, persona);
                }
                continue;
            }
            let core = PAREN.replace(line, "");
            let short = core.split_whitespace().count() <= 8 && !core.ends_with(['.', '!', '?']);
            if short && SECTION.is_match(line) {
                in_chars = true;
                saw_section = true;
                kind = if rules::key(line).contains("principa")
                    || rules::key(line).contains("protagonista")
                {
                    "main"
                } else {
                    "npc"
                };
                current = None;
                continue;
            }
            if in_chars
                && short
                && let Some(h) = header(line)
                && (h.words >= 1 || h.title.is_some() || h.paren.is_some())
            {
                current = add(&mut out, &mut map, h, kind, "", persona);
                continue;
            }
            if short {
                in_chars = SECTION.is_match(line);
                current = None;
                continue;
            }
            if let Some(idx) = current {
                out[idx]["parts"].as_array_mut().unwrap().push(json!(line));
            }
        } else {
            let (label, rest) = line.split_at(colon.unwrap());
            let rest = rest[1..].trim();
            if ["nome", "name", "personagem", "character"].contains(&rules::key(label).as_str()) {
                if let Some(h) = header(rest.split([',', ';']).next().unwrap_or_default()) {
                    let kind = if out.is_empty() { "main" } else { "npc" };
                    current = add(&mut out, &mut map, h, kind, "", persona);
                }
                continue;
            }
            if in_chars {
                if let Some(h) = header(label)
                    && (h.words >= 1 || h.title.is_some() || h.paren.is_some())
                {
                    current = add(&mut out, &mut map, h, kind, rest, persona);
                    continue;
                }
                if let Some(idx) = current {
                    out[idx]["parts"].as_array_mut().unwrap().push(json!(line));
                }
            }
        }
    }
    if !saw_section {
        for raw in text.lines() {
            if let Some(c) = FREE
                .captures(raw.trim_start_matches(|c: char| c.is_whitespace() || "-*•".contains(c)))
                && text.matches(&c[1]).count() >= 2
                && let Some(h) = header(&c[1])
            {
                add(&mut out, &mut map, h, "npc", &c[2], persona);
            }
        }
    }
    let mut first = HashMap::<String, usize>::new();
    let mut last = HashMap::<String, usize>::new();
    for c in &out {
        let name = s(c, "name");
        let w = name.split_whitespace().collect::<Vec<_>>();
        *first.entry(rules::key(w[0])).or_default() += 1;
        if w.len() > 1 {
            *last.entry(rules::key(w[w.len() - 1])).or_default() += 1;
        }
    }
    out.into_iter()
        .take(40)
        .map(|mut c| {
            let name = s(&c, "name");
            let w = name.split_whitespace().collect::<Vec<_>>();
            let aliases = c["aliases"].as_array_mut().unwrap();
            if w.len() > 1 {
                for (word, count) in [
                    (w[0], first.get(&rules::key(w[0]))),
                    (w[w.len() - 1], last.get(&rules::key(w[w.len() - 1]))),
                ] {
                    if count == Some(&1) && !aliases.contains(&json!(word)) {
                        aliases.push(json!(word));
                    }
                }
            }
            aliases.retain(|a| rules::key(a.as_str().unwrap_or_default()) != rules::key(&name));
            aliases.truncate(8);
            c["description"] = json!(clean(
                &c["parts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" · "),
                600
            ));
            c.as_object_mut().unwrap().remove("parts");
            c
        })
        .collect()
}
