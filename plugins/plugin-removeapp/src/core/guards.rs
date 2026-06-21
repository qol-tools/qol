use std::collections::BTreeMap;

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaskToken(String);

impl CaskToken {
    pub fn parse(s: &str) -> Option<CaskToken> {
        let ok = !s.is_empty()
            && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '_' | '-'));
        ok.then(|| CaskToken(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug)]
pub enum CaskStatus {
    Managed(CaskToken),
    NotManaged,
    Unavailable(String),
}

#[derive(Debug)]
pub struct Guards {
    pub running: bool,
    pub cask: CaskStatus,
}

#[derive(Debug)]
pub enum BasenameOwner {
    One(CaskToken),
    Many,
}

#[derive(Deserialize)]
struct BrewInfo {
    casks: Vec<BrewCask>,
}

#[derive(Deserialize)]
struct BrewCask {
    token: String,
    #[serde(default)]
    artifacts: Vec<serde_json::Value>,
}

fn app_basenames(cask: &BrewCask) -> Vec<String> {
    let mut out = Vec::new();
    for artifact in &cask.artifacts {
        let Some(apps) = artifact.get("app").and_then(|a| a.as_array()) else {
            continue;
        };
        let targets: Vec<String> = apps
            .iter()
            .filter_map(|e| e.as_object()?.get("target")?.as_str().map(str::to_string))
            .collect();
        if targets.is_empty() {
            out.extend(apps.iter().filter_map(|e| e.as_str().map(str::to_string)));
        } else {
            out.extend(targets);
        }
    }
    out
}

pub fn parse_cask_map(json: &str) -> Result<BTreeMap<String, BasenameOwner>> {
    let info: BrewInfo = serde_json::from_str(json)?;
    let mut map: BTreeMap<String, BasenameOwner> = BTreeMap::new();
    for cask in &info.casks {
        let Some(token) = CaskToken::parse(&cask.token) else {
            continue;
        };
        for base in app_basenames(cask) {
            match map.get(&base) {
                None => {
                    map.insert(base, BasenameOwner::One(token.clone()));
                }
                Some(_) => {
                    map.insert(base, BasenameOwner::Many);
                }
            }
        }
    }
    Ok(map)
}

pub fn cask_status_for(
    target_basename: &str,
    map: &BTreeMap<String, BasenameOwner>,
    inventory_basenames: &[String],
) -> CaskStatus {
    match map.get(target_basename) {
        None => CaskStatus::NotManaged,
        Some(BasenameOwner::Many) => {
            CaskStatus::Unavailable(format!("{target_basename}: multiple casks share this name"))
        }
        Some(BasenameOwner::One(token)) => {
            let shared = inventory_basenames
                .iter()
                .filter(|b| b.as_str() == target_basename)
                .count();
            if shared > 1 {
                CaskStatus::Unavailable(format!(
                    "{target_basename}: {shared} installed apps share this name"
                ))
            } else {
                CaskStatus::Managed(token.clone())
            }
        }
    }
}

pub fn sanitize_stderr(raw: &[u8], cap: usize) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        if c == '\n' || c == '\t' || !c.is_control() {
            out.push(c);
        }
    }
    let end = (0..=cap.min(out.len()))
        .rev()
        .find(|&i| out.is_char_boundary(i))
        .unwrap_or(0);
    out.truncate(end);
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{"casks":[
      {"token":"discord","artifacts":[{"app":["Discord.app"]}]},
      {"token":"hiddenbar","artifacts":[{"app":["Hidden Bar.app"]}]},
      {"token":"vscode","artifacts":[{"app":["Code - Insiders.app",{"target":"Code.app"}]}]},
      {"token":"font-x","artifacts":[{"font":["X.otf"]}]},
      {"token":"dup-a","artifacts":[{"app":["Same.app"]}]},
      {"token":"dup-b","artifacts":[{"app":["Same.app"]}]}
    ]}"#;

    #[test]
    fn cask_token_parse_rejects_illegal_and_leading_dash() {
        assert!(CaskToken::parse("google-chrome").is_some());
        assert!(CaskToken::parse("font-jetbrains-mono").is_some());
        assert!(CaskToken::parse("-evil").is_none());
        assert!(CaskToken::parse("a b").is_none());
        assert!(CaskToken::parse("").is_none());
    }

    #[test]
    fn cask_map_resolves_target_skips_non_app_and_marks_collisions() {
        let map = parse_cask_map(FIXTURE).unwrap();
        assert!(matches!(
            map.get("Discord.app"),
            Some(BasenameOwner::One(_))
        ));
        assert!(
            matches!(map.get("Code.app"), Some(BasenameOwner::One(_))),
            "target: resolved"
        );
        assert!(
            map.get("Code - Insiders.app").is_none(),
            "source name not keyed when target present"
        );
        assert!(map.get("X.otf").is_none(), "non-app artifact skipped");
        assert!(
            matches!(map.get("Same.app"), Some(BasenameOwner::Many)),
            "two casks collide"
        );
    }

    #[test]
    fn cask_status_classifies_managed_notmanaged_unavailable() {
        let map = parse_cask_map(FIXTURE).unwrap();
        let one = vec!["Discord.app".to_string()];
        let two = vec!["Discord.app".to_string(), "Discord.app".to_string()];
        assert!(matches!(
            cask_status_for("Discord.app", &map, &one),
            CaskStatus::Managed(_)
        ));
        assert!(matches!(
            cask_status_for("Firefox.app", &map, &one),
            CaskStatus::NotManaged
        ));
        assert!(matches!(
            cask_status_for("Same.app", &map, &one),
            CaskStatus::Unavailable(_)
        ));
        assert!(matches!(
            cask_status_for("Discord.app", &map, &two),
            CaskStatus::Unavailable(_)
        ));
        assert!(parse_cask_map("not json").is_err());
    }

    #[test]
    fn sanitize_stderr_strips_control_and_caps() {
        let raw = b"\x1b[31merror\x1b[0m\x07 happened";
        let out = sanitize_stderr(raw, 64);
        assert_eq!(out, "error happened");
        assert_eq!(sanitize_stderr(b"abcdef", 3), "abc");
    }
}
