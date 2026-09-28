use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::super::super::details::Package;

const DPKG_DIR: &str = "/var/lib/dpkg";
const KB: u64 = 1_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Record {
    pub package: Package,
    pub summary: Option<String>,
    pub long: Option<String>,
    pub website: Option<String>,
    pub maintainer: Option<String>,
}

pub(super) fn owner_of(desktop: &Path) -> Option<Record> {
    static OWNERS: OnceLock<HashMap<PathBuf, String>> = OnceLock::new();
    let dpkg = Path::new(DPKG_DIR);
    let list = OWNERS.get_or_init(|| desktop_owners(dpkg)).get(desktop)?;
    let name = list.split(':').next().unwrap_or(list);
    let status = fs::read_to_string(dpkg.join("status")).ok()?;
    let mut record = status_record(&status, name)?;
    record.package.installed = fs::metadata(dpkg.join("info").join(format!("{list}.list")))
        .and_then(|metadata| metadata.modified())
        .ok();
    Some(record)
}

fn desktop_owners(dpkg: &Path) -> HashMap<PathBuf, String> {
    let Ok(entries) = fs::read_dir(dpkg.join("info")) else {
        return HashMap::new();
    };
    let mut owners = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(list) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".list"))
        else {
            continue;
        };
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        for line in content.lines().filter(|line| line.ends_with(".desktop")) {
            owners.insert(PathBuf::from(line), list.to_owned());
        }
    }
    owners
}

fn status_record(status: &str, name: &str) -> Option<Record> {
    let stanza = status.split("\n\n").find(|stanza| {
        stanza
            .lines()
            .any(|line| line.strip_prefix("Package: ") == Some(name))
            && stanza
                .lines()
                .any(|line| line.starts_with("Status: ") && line.ends_with(" installed"))
    })?;
    let field = |key: &str| {
        stanza
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix(": "))
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let (summary, long) = description(stanza);
    Some(Record {
        package: Package {
            name: name.to_owned(),
            version: field("Version").map(upstream_version),
            size: field("Installed-Size")
                .and_then(|size| size.parse::<u64>().ok())
                .map(|size| size * KB),
            installed: None,
        },
        summary,
        long,
        website: field("Homepage").map(str::to_owned),
        maintainer: field("Maintainer").and_then(person),
    })
}

fn description(stanza: &str) -> (Option<String>, Option<String>) {
    let mut lines = stanza
        .lines()
        .skip_while(|line| !line.starts_with("Description: "));
    let Some(summary) = lines
        .next()
        .and_then(|line| line.strip_prefix("Description: "))
    else {
        return (None, None);
    };
    let body: Vec<&str> = lines
        .take_while(|line| line.starts_with(' '))
        .map(str::trim)
        .take_while(|line| *line != ".")
        .collect();
    let long = (!body.is_empty()).then(|| body.join(" "));
    (Some(summary.trim().to_owned()), long)
}

fn upstream_version(version: &str) -> String {
    let version = version.split_once(':').map_or(version, |(_, rest)| rest);
    version
        .split(['-', '+', '~'])
        .next()
        .unwrap_or(version)
        .to_owned()
}

fn person(field: &str) -> Option<String> {
    let name = field.split('<').next().unwrap_or(field).trim();
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "Package: xed-common\nStatus: install ok installed\nVersion: 3.8.9+zena\n\nPackage: xed\nStatus: install ok installed\nInstalled-Size: 1248\nMaintainer: Linux Mint <root@linuxmint.com>\nVersion: 3.8.9+zena\nDescription: Text editor\n Xed is a small, but powerful text editor.\n It supports Unicode.\n .\n A second paragraph.\nHomepage: https://github.com/linuxmint/xed\n\nPackage: gone\nStatus: deinstall ok config-files\nVersion: 1.0\n";

    #[test]
    fn status_records_read_the_installed_stanza() {
        let record = status_record(STATUS, "xed").expect("xed");
        assert_eq!(record.package.name, "xed");
        assert_eq!(record.package.version.as_deref(), Some("3.8.9"));
        assert_eq!(record.package.size, Some(1_248_000));
        assert_eq!(record.summary.as_deref(), Some("Text editor"));
        assert_eq!(
            record.long.as_deref(),
            Some("Xed is a small, but powerful text editor. It supports Unicode.")
        );
        assert_eq!(
            record.website.as_deref(),
            Some("https://github.com/linuxmint/xed")
        );
        assert_eq!(record.maintainer.as_deref(), Some("Linux Mint"));
        assert!(status_record(STATUS, "gone").is_none());
        assert!(status_record(STATUS, "missing").is_none());
    }

    #[test]
    fn versions_drop_the_epoch_and_the_distribution_suffix() {
        assert_eq!(upstream_version("3.52.0+mint2+xia"), "3.52.0");
        assert_eq!(upstream_version("1:2.4.1-3ubuntu2"), "2.4.1");
        assert_eq!(upstream_version("390-1ubuntu3"), "390");
        assert_eq!(upstream_version("1.0.4"), "1.0.4");
    }

    #[test]
    fn owners_map_each_desktop_entry_to_its_list() {
        let dir = tempfile::TempDir::new().unwrap();
        let info = dir.path().join("info");
        fs::create_dir(&info).unwrap();
        fs::write(
            info.join("xed.list"),
            "/usr/bin/xed\n/usr/share/applications/org.x.editor.desktop\n",
        )
        .unwrap();
        fs::write(
            info.join("libfoo:amd64.list"),
            "/usr/share/applications/foo.desktop\n",
        )
        .unwrap();
        let owners = desktop_owners(dir.path());
        assert_eq!(owners.len(), 2);
        assert_eq!(
            owners
                .get(Path::new("/usr/share/applications/org.x.editor.desktop"))
                .map(String::as_str),
            Some("xed")
        );
        assert_eq!(
            owners
                .get(Path::new("/usr/share/applications/foo.desktop"))
                .map(String::as_str),
            Some("libfoo:amd64")
        );
    }
}
