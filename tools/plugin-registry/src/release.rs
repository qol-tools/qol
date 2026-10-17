use qol_platform::Os;
use std::collections::BTreeMap;

const HOST_ID_PREFIX: &str = "qol-tray-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTag {
    pub tag: String,
    pub plugin_id: String,
    pub version: String,
    pub number: (u64, u64, u64),
}

impl ReleaseTag {
    pub fn parse(tag: &str) -> Option<Self> {
        let (plugin_id, version) = tag.rsplit_once("-v")?;
        let mut parts = version.split('.').map(|part| {
            (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse::<u64>().ok())
                .flatten()
        });
        let number = (parts.next()??, parts.next()??, parts.next()??);
        if parts.next().is_some() || plugin_id.is_empty() {
            return None;
        }
        Some(Self {
            tag: tag.to_string(),
            plugin_id: plugin_id.to_string(),
            version: version.to_string(),
            number,
        })
    }

    pub fn is_host(&self) -> bool {
        self.plugin_id.starts_with(HOST_ID_PREFIX)
    }

    pub fn host_system(&self) -> Option<&str> {
        let os = self.plugin_id.strip_prefix(HOST_ID_PREFIX)?;
        Os::from_name(os).map(Os::name)
    }
}

pub fn is_host_tag(tag: &str) -> bool {
    ReleaseTag::parse(tag).is_some_and(|release| release.host_system().is_some())
}

pub fn declared_id(plugin_toml: &str) -> Option<String> {
    declared(plugin_toml, "id")
}

pub fn declared_version(plugin_toml: &str) -> Option<String> {
    declared(plugin_toml, "version")
}

fn declared(plugin_toml: &str, field: &str) -> Option<String> {
    let table: toml::Table = toml::from_str(plugin_toml).ok()?;
    table
        .get("plugin")?
        .get(field)?
        .as_str()
        .map(str::to_string)
}

pub fn newest_per_plugin<'a>(
    tags: impl IntoIterator<Item = &'a str>,
    keep: usize,
) -> Vec<ReleaseTag> {
    let mut grouped: BTreeMap<String, Vec<ReleaseTag>> = BTreeMap::new();
    for release in tags.into_iter().filter_map(ReleaseTag::parse) {
        grouped
            .entry(release.plugin_id.clone())
            .or_default()
            .push(release);
    }
    grouped
        .into_values()
        .flat_map(|mut releases| {
            releases.sort_by_key(|release| std::cmp::Reverse(release.number));
            releases.truncate(keep);
            releases
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    type Parsed = Option<(&'static str, &'static str, (u64, u64, u64))>;

    #[test]
    fn parses_plugin_release_tags_only() {
        let cases: &[(&str, Parsed)] = &[
            (
                "qol-alt-tab-v0.89.1",
                Some(("qol-alt-tab", "0.89.1", (0, 89, 1))),
            ),
            (
                "qol-voice-v0.39.0",
                Some(("qol-voice", "0.39.0", (0, 39, 0))),
            ),
            ("qol-shot-v1.2", None),
            ("qol-shot-v1.2.3.4", None),
            ("qol-shot-v1.2.x", None),
            ("qol-shot-v1..3", None),
            ("-v1.2.3", None),
            ("sha256-abc.sig", None),
            ("not-a-release", None),
        ];
        for (tag, expected) in cases {
            let parsed = ReleaseTag::parse(tag)
                .map(|release| (release.plugin_id, release.version, release.number));
            let expected =
                expected.map(|(id, version, number)| (id.to_string(), version.to_string(), number));
            assert_eq!(parsed, expected, "{tag}");
        }
    }

    #[test]
    fn host_tags_name_a_known_os() {
        let cases: &[(&str, bool, Option<&str>)] = &[
            ("qol-tray-linux-v3.101.4", true, Some(Os::Linux.name())),
            ("qol-tray-macos-v3.101.4", true, Some(Os::Macos.name())),
            ("qol-tray-windows-v3.101.4", true, Some(Os::Windows.name())),
            ("qol-tray-freebsd-v3.101.4", true, None),
            ("qol-tray-v3.82.0", false, None),
            ("qol-shot-v1.0.0", false, None),
        ];
        for (tag, host, os) in cases {
            let release = ReleaseTag::parse(tag).unwrap();
            assert_eq!(release.is_host(), *host, "{tag}");
            assert_eq!(release.host_system(), *os, "{tag}");
            assert_eq!(is_host_tag(tag), os.is_some(), "{tag}");
        }
        for os in Os::ALL.map(Os::name) {
            let tag = qol_plugin_index::host_tag(os, "1.2.3");
            assert_eq!(ReleaseTag::parse(&tag).unwrap().host_system(), Some(os));
        }
        assert!(!is_host_tag("index"));
    }

    #[test]
    fn reads_the_declared_id_and_version() {
        let manifest = "[plugin]\nid = \"qol-shot\"\nversion = \"1.2.3\"\n";
        assert_eq!(declared_id(manifest).as_deref(), Some("qol-shot"));
        assert_eq!(declared_version(manifest).as_deref(), Some("1.2.3"));
        assert_eq!(declared_id("not toml ["), None);
    }

    #[test]
    fn keeps_the_newest_versions_of_each_plugin_by_number() {
        let tags = [
            "qol-alt-tab-v0.9.0",
            "qol-alt-tab-v0.10.0",
            "qol-alt-tab-v0.2.0",
            "qol-shot-v1.0.0",
            "qol-tray-v3.82.0",
            "sha256-abc.sig",
        ];
        let kept: Vec<String> = newest_per_plugin(tags, 2)
            .into_iter()
            .map(|release| release.tag)
            .collect();
        assert_eq!(
            kept,
            [
                "qol-alt-tab-v0.10.0",
                "qol-alt-tab-v0.9.0",
                "qol-shot-v1.0.0",
                "qol-tray-v3.82.0"
            ]
        );
    }
}
