use std::collections::BTreeMap;

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CaskToken(String);

impl CaskToken {
    fn parse(value: &str) -> Option<Self> {
        let valid = !value.is_empty()
            && value
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric())
            && value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '+' | '.' | '_' | '-')
            });
        valid.then(|| Self(value.to_string()))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub(super) enum CaskStatus {
    Managed(CaskToken),
    NotManaged,
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub(super) enum BasenameOwner {
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
    let mut basenames = Vec::new();
    for artifact in &cask.artifacts {
        let Some(apps) = artifact.get("app").and_then(|value| value.as_array()) else {
            continue;
        };
        let targets = apps
            .iter()
            .filter_map(|entry| {
                entry
                    .as_object()?
                    .get("target")?
                    .as_str()
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        if targets.is_empty() {
            basenames.extend(
                apps.iter()
                    .filter_map(|entry| entry.as_str().map(str::to_string)),
            );
        } else {
            basenames.extend(targets);
        }
    }
    basenames
}

pub(super) fn parse_cask_map(json: &str) -> Result<BTreeMap<String, BasenameOwner>> {
    let info: BrewInfo = serde_json::from_str(json)?;
    let mut map = BTreeMap::new();
    for cask in &info.casks {
        let Some(token) = CaskToken::parse(&cask.token) else {
            continue;
        };
        for basename in app_basenames(cask) {
            match map.get(&basename) {
                None => {
                    map.insert(basename, BasenameOwner::One(token.clone()));
                }
                Some(_) => {
                    map.insert(basename, BasenameOwner::Many);
                }
            }
        }
    }
    Ok(map)
}

fn cask_status_for(
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
                .filter(|basename| basename.as_str() == target_basename)
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

#[derive(Debug, Clone)]
pub(super) struct CaskIndex(BTreeMap<String, BasenameOwner>);

impl CaskIndex {
    pub(super) fn from_map(map: BTreeMap<String, BasenameOwner>) -> Self {
        Self(map)
    }

    pub(super) fn classify(
        &self,
        target_basename: &str,
        inventory_basenames: &[String],
    ) -> CaskStatus {
        cask_status_for(target_basename, &self.0, inventory_basenames)
    }
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
            !map.contains_key("Code - Insiders.app"),
            "source name not keyed when target present"
        );
        assert!(!map.contains_key("X.otf"), "non-app artifact skipped");
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
    fn cask_index_classifies_via_map() {
        let index = CaskIndex::from_map(parse_cask_map(FIXTURE).unwrap());
        let one = vec!["Discord.app".to_string()];
        assert!(matches!(
            index.classify("Discord.app", &one),
            CaskStatus::Managed(_)
        ));
        assert!(matches!(
            index.classify("Firefox.app", &one),
            CaskStatus::NotManaged
        ));
    }
}
