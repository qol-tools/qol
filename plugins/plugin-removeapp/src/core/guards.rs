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
    use super::*;

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
