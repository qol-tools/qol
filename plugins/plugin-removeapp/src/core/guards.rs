use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageManager {
    Homebrew,
    Apt,
    Flatpak,
}

impl PackageManager {
    pub fn label(self) -> &'static str {
        match self {
            PackageManager::Homebrew => "Homebrew",
            PackageManager::Apt => "APT",
            PackageManager::Flatpak => "Flatpak",
        }
    }

    pub fn action_key(self) -> &'static str {
        match self {
            PackageManager::Homebrew => "b",
            PackageManager::Apt | PackageManager::Flatpak => "u",
        }
    }

    pub fn action_key_label(self) -> &'static str {
        match self {
            PackageManager::Homebrew => "B",
            PackageManager::Apt | PackageManager::Flatpak => "U",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageScope {
    User,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ManagedPackage {
    manager: PackageManager,
    id: String,
    scope: PackageScope,
}

impl ManagedPackage {
    pub fn parse(manager: PackageManager, id: &str, scope: PackageScope) -> Option<ManagedPackage> {
        let valid = !id.is_empty()
            && id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
            && id.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(c, '+' | '.' | '_' | '-')
                    || (c == ':' && manager == PackageManager::Apt)
            });
        valid.then(|| ManagedPackage {
            manager,
            id: id.to_string(),
            scope,
        })
    }

    pub fn manager(&self) -> PackageManager {
        self.manager
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn scope(&self) -> PackageScope {
        self.scope
    }
}

#[derive(Debug, Clone)]
pub enum PackageStatus {
    Managed(ManagedPackage),
    NotManaged,
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub struct PackageIndex {
    statuses: BTreeMap<PathBuf, PackageStatus>,
    fallback: PackageStatus,
}

impl Default for PackageIndex {
    fn default() -> Self {
        Self::absent()
    }
}

impl PackageIndex {
    pub fn absent() -> PackageIndex {
        PackageIndex {
            statuses: BTreeMap::new(),
            fallback: PackageStatus::NotManaged,
        }
    }

    pub fn unavailable(reason: impl Into<String>) -> PackageIndex {
        PackageIndex {
            statuses: BTreeMap::new(),
            fallback: PackageStatus::Unavailable(reason.into()),
        }
    }

    pub fn insert(&mut self, path: PathBuf, status: PackageStatus) {
        self.statuses.insert(path, status);
    }

    pub fn classify(&self, path: &Path) -> PackageStatus {
        self.statuses
            .get(path)
            .cloned()
            .unwrap_or_else(|| self.fallback.clone())
    }
}

#[derive(Debug, Clone)]
pub struct Guards {
    pub running: bool,
    pub package: PackageStatus,
}

#[cfg(target_os = "macos")]
mod cask {
    use super::*;
    use anyhow::Result;
    use serde::Deserialize;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct CaskToken(String);

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

    #[derive(Debug, Clone)]
    pub(crate) enum CaskStatus {
        Managed(CaskToken),
        NotManaged,
        Unavailable(String),
    }

    #[derive(Debug, Clone)]
    pub(crate) enum BasenameOwner {
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

    pub(crate) fn parse_cask_map(json: &str) -> Result<BTreeMap<String, BasenameOwner>> {
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

    pub(crate) fn cask_status_for(
        target_basename: &str,
        map: &BTreeMap<String, BasenameOwner>,
        inventory_basenames: &[String],
    ) -> CaskStatus {
        match map.get(target_basename) {
            None => CaskStatus::NotManaged,
            Some(BasenameOwner::Many) => CaskStatus::Unavailable(format!(
                "{target_basename}: multiple casks share this name"
            )),
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

    #[derive(Debug, Clone)]
    pub(crate) struct CaskIndex(BTreeMap<String, BasenameOwner>);

    impl CaskIndex {
        pub fn from_map(map: BTreeMap<String, BasenameOwner>) -> CaskIndex {
            CaskIndex(map)
        }

        pub fn classify(
            &self,
            target_basename: &str,
            inventory_basenames: &[String],
        ) -> CaskStatus {
            cask_status_for(target_basename, &self.0, inventory_basenames)
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) use cask::{parse_cask_map, CaskIndex, CaskStatus};

pub(crate) fn sanitize_stderr(raw: &[u8], cap: usize) -> String {
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
    #[cfg(target_os = "macos")]
    use super::cask::{
        cask_status_for, parse_cask_map, BasenameOwner, CaskIndex, CaskStatus, CaskToken,
    };
    use super::*;

    #[cfg(target_os = "macos")]
    const FIXTURE: &str = r#"{"casks":[
      {"token":"discord","artifacts":[{"app":["Discord.app"]}]},
      {"token":"hiddenbar","artifacts":[{"app":["Hidden Bar.app"]}]},
      {"token":"vscode","artifacts":[{"app":["Code - Insiders.app",{"target":"Code.app"}]}]},
      {"token":"font-x","artifacts":[{"font":["X.otf"]}]},
      {"token":"dup-a","artifacts":[{"app":["Same.app"]}]},
      {"token":"dup-b","artifacts":[{"app":["Same.app"]}]}
    ]}"#;

    #[cfg(target_os = "macos")]
    #[test]
    fn cask_token_parse_rejects_illegal_and_leading_dash() {
        assert!(CaskToken::parse("google-chrome").is_some());
        assert!(CaskToken::parse("font-jetbrains-mono").is_some());
        assert!(CaskToken::parse("-evil").is_none());
        assert!(CaskToken::parse("a b").is_none());
        assert!(CaskToken::parse("").is_none());
    }

    #[cfg(target_os = "macos")]
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

    #[cfg(target_os = "macos")]
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

    #[cfg(target_os = "macos")]
    #[test]
    fn cask_index_classifies_via_map() {
        let idx = CaskIndex::from_map(parse_cask_map(FIXTURE).unwrap());
        let one = vec!["Discord.app".to_string()];
        assert!(matches!(
            idx.classify("Discord.app", &one),
            CaskStatus::Managed(_)
        ));
        assert!(matches!(
            idx.classify("Firefox.app", &one),
            CaskStatus::NotManaged
        ));
    }

    #[test]
    fn sanitize_stderr_strips_control_and_caps() {
        let raw = b"\x1b[31merror\x1b[0m\x07 happened";
        let out = sanitize_stderr(raw, 64);
        assert_eq!(out, "error happened");
        assert_eq!(sanitize_stderr(b"abcdef", 3), "abc");
    }

    #[test]
    fn managed_package_rejects_option_shaped_and_shell_shaped_ids() {
        assert!(
            ManagedPackage::parse(PackageManager::Apt, "firefox:amd64", PackageScope::System)
                .is_some()
        );
        assert!(ManagedPackage::parse(
            PackageManager::Flatpak,
            "org.example.Widget",
            PackageScope::User
        )
        .is_some());
        assert!(
            ManagedPackage::parse(PackageManager::Homebrew, "foo:bar", PackageScope::User)
                .is_none()
        );
        assert!(ManagedPackage::parse(
            PackageManager::Flatpak,
            "org.example:Widget",
            PackageScope::User
        )
        .is_none());
        for invalid in ["", "-rf", "two words", "foo;bar", "foo/bar"] {
            assert!(
                ManagedPackage::parse(PackageManager::Apt, invalid, PackageScope::System).is_none(),
                "invalid id accepted: {invalid:?}"
            );
        }
    }

    #[test]
    fn package_index_classifies_by_exact_launcher_path() {
        let path = PathBuf::from("/usr/share/applications/firefox.desktop");
        let package =
            ManagedPackage::parse(PackageManager::Apt, "firefox", PackageScope::System).unwrap();
        let mut index = PackageIndex::absent();
        index.insert(path.clone(), PackageStatus::Managed(package));

        assert!(matches!(index.classify(&path), PackageStatus::Managed(_)));
        assert!(matches!(
            index.classify(Path::new("/tmp/firefox.desktop")),
            PackageStatus::NotManaged
        ));
    }
}
