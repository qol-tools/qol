use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

pub const ALIAS_CAP: usize = 4;
pub const CONCEPT_ALIASES_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/concept-aliases.json"
));

#[derive(Default)]
pub struct AliasMap {
    map: HashMap<String, Vec<String>>,
}

impl AliasMap {
    pub fn get(&self, term: &str) -> Option<&[String]> {
        self.map.get(term).map(Vec::as_slice)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn insert(&mut self, term: String, expansions: Vec<String>) {
        self.map.insert(term, expansions);
    }
}

fn term_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-z0-9]{2,}$").expect("term regex"))
}

pub fn embedded() -> AliasMap {
    if matches!(
        std::env::var("QOL_MEMORY_ALIASES_DISABLE").as_deref(),
        Ok("1")
    ) {
        return AliasMap::default();
    }
    match load(CONCEPT_ALIASES_JSON) {
        Ok(map) => map,
        Err(err) => {
            eprintln!("concept-aliases: load failed: {err}; using empty alias map");
            AliasMap::default()
        }
    }
}

pub fn load(json: &str) -> anyhow::Result<AliasMap> {
    let raw: Value = serde_json::from_str(json)?;
    if raw.get("schema").and_then(Value::as_u64) != Some(1) {
        anyhow::bail!("schema must be 1");
    }
    let mut alias_map = AliasMap::default();
    match raw.get("aliases") {
        None | Some(Value::Null) => Ok(alias_map),
        Some(Value::Object(entries)) => {
            for (term, exps) in entries {
                let Some(list) = exps.as_array() else {
                    anyhow::bail!("alias \"{term}\" must map to an array of terms");
                };
                let mut flat: Vec<String> = Vec::new();
                'flattening: for exp in list {
                    let Value::String(text) = exp else {
                        anyhow::bail!("alias expansions must be strings");
                    };
                    for token in crate::text::tokens(text) {
                        if flat.len() >= ALIAS_CAP {
                            break 'flattening;
                        }
                        flat.push(token);
                    }
                }
                alias_map.insert(term.clone(), flat);
            }
            Ok(alias_map)
        }
        Some(_) => anyhow::bail!("aliases must be an object of term -> term arrays"),
    }
}

pub fn validate(json: &str) -> Vec<String> {
    let raw: Value = match serde_json::from_str(json) {
        Ok(raw) => raw,
        Err(err) => return vec![format!("unreadable: {err}")],
    };
    let mut errors: Vec<String> = Vec::new();
    if raw.get("schema").and_then(Value::as_u64) != Some(1) {
        errors.push(format!("schema must be 1, found {}", schema_display(&raw)));
    }
    match raw.get("aliases") {
        Some(Value::Object(entries)) => {
            for (term, exps) in entries {
                if !term_re().is_match(term) {
                    errors.push(format!("alias term \"{term}\" is not a valid token"));
                }
                let Some(list) = exps.as_array() else {
                    errors.push(format!("alias \"{term}\" must map to an array of terms"));
                    continue;
                };
                for exp in list {
                    match exp.as_str() {
                        None => errors.push(format!("alias \"{term}\" has a non-string expansion")),
                        Some(expansion) => {
                            if !term_re().is_match(expansion) {
                                errors.push(format!(
                                    "alias \"{term}\" expansion \"{expansion}\" is not a valid token"
                                ));
                            }
                        }
                    }
                }
            }
        }
        _ => errors.push("aliases must be an object of term -> term arrays".to_string()),
    }
    errors
}

fn schema_display(raw: &Value) -> String {
    if raw.is_null() {
        return "null".to_string();
    }
    match raw.get("schema") {
        None => "undefined".to_string(),
        Some(value) => match value {
            Value::Null | Value::Bool(_) | Value::Number(_) => value.to_string(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        },
    }
}

pub fn expand_tokens(list: &[String], alias_map: &AliasMap) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(list.len());
    for token in list {
        match alias_map.get(token) {
            Some(exps) => out.extend(exps.iter().cloned()),
            None => out.push(token.clone()),
        }
    }
    out
}

pub fn expand_tokens_keep(list: &[String], alias_map: &AliasMap) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(list.len());
    for token in list {
        out.push(token.clone());
        if let Some(exps) = alias_map.get(token) {
            out.extend(exps.iter().cloned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_flattens_in_order_and_stops_at_cap() {
        let map = load(
            r#"{"schema":1,"aliases":{
                "m4a1":["bspace","clip","caf","dba"],
                "five":["aa bb cc dd ee"],
                "multi":["uno dos","tres"],
                "stems":["running","wanted"]
            }}"#,
        )
        .unwrap();
        assert_eq!(map.len(), 4);
        assert_eq!(
            map.get("m4a1").unwrap(),
            ["bspace", "clip", "caf", "dba"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(map.get("five").unwrap(), vec!["aa", "bb", "cc", "dd"]);
        assert_eq!(map.get("multi").unwrap(), vec!["uno", "dos", "tre"]);
        assert_eq!(map.get("stems").unwrap(), vec!["runn", "want"]);
        assert!(map.get("missing").is_none());
    }

    #[test]
    fn load_rejects_bad_schema_and_shapes() {
        assert!(load(r#"{"schema":2}"#).is_err());
        assert!(load("{}").is_err());
        assert!(load("not json").is_err());
        assert!(load(r#"{"schema":1,"aliases":{"t":"not-an-array"}}"#).is_err());
        let empty_ok = load(r#"{"schema":1}"#).unwrap();
        assert!(empty_ok.is_empty());
        assert_eq!(empty_ok.len(), 0);
    }

    #[test]
    fn embedded_asset_loads_or_env_disables() {
        let map = embedded();
        match std::env::var("QOL_MEMORY_ALIASES_DISABLE").as_deref() {
            Ok("1") => assert!(map.is_empty()),
            _ => {
                assert_eq!(map.len(), 4);
                assert_eq!(
                    map.get("m4a1").unwrap(),
                    vec!["bspace", "clip", "caf", "dba"]
                );
            }
        }
    }

    #[test]
    fn validate_port_messages() {
        assert!(validate(CONCEPT_ALIASES_JSON).is_empty());
        assert!(validate(r#"{"schema":1,"aliases":{"notebook":["notes","records"]}}"#).is_empty());

        assert_eq!(
            validate(r#"{"schema":2,"aliases":{}}"#),
            vec!["schema must be 1, found 2"]
        );
        assert_eq!(
            validate("{}"),
            vec![
                "schema must be 1, found undefined",
                "aliases must be an object of term -> term arrays"
            ]
        );
        assert_eq!(
            validate("null"),
            vec![
                "schema must be 1, found null",
                "aliases must be an object of term -> term arrays"
            ]
        );
        assert_eq!(
            validate(r#"{"schema":"1"}"#),
            vec![
                "schema must be 1, found 1",
                "aliases must be an object of term -> term arrays"
            ]
        );
        assert_eq!(
            validate(
                r#"{"schema":1,"aliases":{"BAD KEY":["ok"],"numeral":5,"nos":["no",7,"UPPER"],"shape":"thing"}}"#
            ),
            vec![
                "alias term \"BAD KEY\" is not a valid token",
                "alias \"numeral\" must map to an array of terms",
                "alias \"nos\" has a non-string expansion",
                "alias \"nos\" expansion \"UPPER\" is not a valid token",
                "alias \"shape\" must map to an array of terms"
            ]
        );
        assert!(validate("{oops")[0].starts_with("unreadable: "));
    }

    #[test]
    fn expand_tokens_replace_and_keep() {
        let map =
            load(r#"{"schema":1,"aliases":{"m4a1":["bspace","clip"],"july":["idle"]}}"#).unwrap();
        let q = ["m4a1", "alpha", "july"];
        let owned: Vec<String> = q.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            expand_tokens(&owned, &map),
            vec!["bspace", "clip", "alpha", "idle"]
        );
        assert_eq!(
            expand_tokens_keep(&owned, &map),
            vec!["m4a1", "bspace", "clip", "alpha", "july", "idle"]
        );
        assert!(expand_tokens(&[], &map).is_empty());
        let unknown = vec!["zzz".to_string()];
        assert_eq!(expand_tokens(&unknown, &map), unknown);
        assert_eq!(expand_tokens_keep(&unknown, &map), unknown);
        let dead_map = load(r#"{"schema":1,"aliases":{"dead":[]}}"#).unwrap();
        let dead = vec!["dead".to_string()];
        assert!(expand_tokens(&dead, &dead_map).is_empty());
        assert_eq!(expand_tokens_keep(&dead, &dead_map), dead);
    }
}
