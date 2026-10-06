use std::collections::BTreeMap;

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
