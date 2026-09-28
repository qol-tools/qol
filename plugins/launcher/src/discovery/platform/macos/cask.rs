use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

const CASKROOMS: [&str; 2] = ["/opt/homebrew/Caskroom", "/usr/local/Caskroom"];
const CATALOGUE: &str = "Library/Caches/Homebrew/api/cask.jws.json";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Cask {
    pub token: String,
    pub summary: Option<String>,
    pub website: Option<String>,
    pub installed: Option<SystemTime>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Listing {
    summary: Option<String>,
    website: Option<String>,
}

#[derive(Deserialize)]
struct Signed {
    payload: String,
}

#[derive(Deserialize)]
struct Entry {
    token: String,
    desc: Option<String>,
    homepage: Option<String>,
}

#[derive(Deserialize)]
struct Receipt {
    time: Option<u64>,
}

pub(super) fn owner_of(app: &Path) -> Option<Cask> {
    static OWNERS: OnceLock<HashMap<PathBuf, (PathBuf, String)>> = OnceLock::new();
    static LISTINGS: OnceLock<HashMap<String, Listing>> = OnceLock::new();
    let owners = OWNERS.get_or_init(|| {
        CASKROOMS
            .iter()
            .flat_map(|room| app_owners(Path::new(room)))
            .collect()
    });
    let (room, token) = owners.get(app)?;
    let listing = LISTINGS
        .get_or_init(|| {
            let installed: HashSet<&str> =
                owners.values().map(|(_, token)| token.as_str()).collect();
            std::env::var_os("HOME")
                .and_then(|home| fs::read_to_string(Path::new(&home).join(CATALOGUE)).ok())
                .map(|text| listings(&text, &installed))
                .unwrap_or_default()
        })
        .get(token)
        .cloned()
        .unwrap_or_default();
    Some(Cask {
        token: token.clone(),
        summary: listing.summary,
        website: listing.website,
        installed: installed_at(&room.join(token)),
    })
}

fn app_owners(room: &Path) -> Vec<(PathBuf, (PathBuf, String))> {
    let mut owners = Vec::new();
    for token in children(room) {
        let Some(name) = token.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        for version in children(&token).filter(|version| !is_hidden(version)) {
            for link in children(&version)
                .filter(|link| link.extension().is_some_and(|extension| extension == "app"))
            {
                if let Ok(target) = fs::read_link(&link) {
                    owners.push((version.join(target), (room.to_path_buf(), name.to_owned())));
                }
            }
        }
    }
    owners
}

fn children(dir: &Path) -> impl Iterator<Item = PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

fn listings(text: &str, installed: &HashSet<&str>) -> HashMap<String, Listing> {
    let Ok(signed) = serde_json::from_str::<Signed>(text) else {
        return HashMap::new();
    };
    let Ok(entries) = serde_json::from_str::<Vec<Entry>>(&signed.payload) else {
        return HashMap::new();
    };
    let present = |value: Option<String>| {
        value
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    entries
        .into_iter()
        .filter(|entry| installed.contains(entry.token.as_str()))
        .map(|entry| {
            (
                entry.token,
                Listing {
                    summary: present(entry.desc),
                    website: present(entry.homepage),
                },
            )
        })
        .collect()
}

fn installed_at(cask: &Path) -> Option<SystemTime> {
    let text = fs::read_to_string(cask.join(".metadata/INSTALL_RECEIPT.json")).ok()?;
    let seconds = serde_json::from_str::<Receipt>(&text).ok()?.time?;
    Some(UNIX_EPOCH + Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owners_follow_the_caskroom_links_to_the_installed_app() {
        let temp = tempfile::tempdir().unwrap();
        let room = temp.path().join("Caskroom");
        let version = room.join("discord/0.0.376");
        fs::create_dir_all(&version).unwrap();
        fs::create_dir_all(room.join("discord/.metadata/0.0.376")).unwrap();
        std::os::unix::fs::symlink("/Applications/Discord.app", version.join("Discord.app"))
            .unwrap();

        assert_eq!(
            app_owners(&room),
            vec![(
                PathBuf::from("/Applications/Discord.app"),
                (room.clone(), "discord".to_owned())
            )]
        );
    }

    #[test]
    fn listings_keep_only_installed_casks_from_the_signed_payload() {
        let payload = serde_json::to_string(
            r#"[{"token":"discord","desc":"Voice and text chat software","homepage":"https://discord.com/","version":"1"},
                {"token":"zoom","desc":"Video calls","homepage":" "}]"#,
        )
        .unwrap();
        let text = format!(r#"{{"payload":{payload},"signatures":[]}}"#);

        let found = listings(&text, &HashSet::from(["discord"]));

        assert_eq!(
            found,
            HashMap::from([(
                "discord".to_owned(),
                Listing {
                    summary: Some("Voice and text chat software".to_owned()),
                    website: Some("https://discord.com/".to_owned()),
                }
            )])
        );
    }

    #[test]
    fn install_time_comes_from_the_receipt() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".metadata")).unwrap();
        fs::write(
            temp.path().join(".metadata/INSTALL_RECEIPT.json"),
            r#"{"time":1770924123,"arch":"arm64"}"#,
        )
        .unwrap();

        assert_eq!(
            installed_at(temp.path()),
            Some(UNIX_EPOCH + Duration::from_secs(1_770_924_123))
        );
    }
}
