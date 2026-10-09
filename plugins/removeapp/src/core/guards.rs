use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageManager {
    Homebrew,
    Apt,
    Flatpak,
    Windows,
}

impl PackageManager {
    pub fn label(self) -> &'static str {
        match self {
            PackageManager::Homebrew => "Homebrew",
            PackageManager::Apt => "APT",
            PackageManager::Flatpak => "Flatpak",
            PackageManager::Windows => "Windows",
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
        let valid = match manager {
            PackageManager::Windows => valid_registry_key(id),
            _ => valid_package_id(manager, id),
        };
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

fn valid_package_id(manager: PackageManager, id: &str) -> bool {
    !id.is_empty()
        && id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '+' | '.' | '_' | '-')
                || (c == ':' && manager == PackageManager::Apt)
        })
}

fn valid_registry_key(id: &str) -> bool {
    !id.trim().is_empty()
        && id.len() <= 255
        && !id.contains('\\')
        && !id.chars().any(char::is_control)
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn windows_package_ids_are_single_registry_key_names() {
        let cases = [
            ("{23170F69-40C1-2702-2301-000001000000}", true),
            ("Mozilla Firefox 128.0 (x64 en-US)", true),
            ("Steam App 570", true),
            ("", false),
            ("   ", false),
            ("Uninstall\\Other", false),
            ("bad\nname", false),
        ];
        for (id, expected) in cases {
            assert_eq!(
                ManagedPackage::parse(PackageManager::Windows, id, PackageScope::System).is_some(),
                expected,
                "{id:?}"
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
