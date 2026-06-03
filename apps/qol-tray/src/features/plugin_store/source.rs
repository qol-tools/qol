use crate::features::plugin_store::release_assets::{resolve_asset_pattern, PlatformTarget};
use crate::plugins::manifest::BinaryDependency;
use crate::version::normalize_semver_tag;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PluginSource {
    pub(crate) name: String,
    pub(crate) repo: String,
    pub(crate) git_ref: String,
}

impl PluginSource {
    pub(crate) fn new(
        name: impl Into<String>,
        repo: impl Into<String>,
        git_ref: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            repo: repo.into(),
            git_ref: git_ref.into(),
        }
    }

    pub(crate) fn repo_clone_url(&self) -> String {
        if let Some(path) = self.repo.strip_prefix("file://") {
            return path.to_string();
        }
        if self.repo.starts_with('/') {
            return self.repo.clone();
        }
        format!("https://github.com/{}.git", self.repo)
    }

    pub(crate) fn plugin_subdir_html_url(&self, plugin_id: &str) -> String {
        format!(
            "https://github.com/{}/tree/{}/plugins/{}",
            self.repo, self.git_ref, plugin_id
        )
    }

    pub(crate) fn tree_api_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/git/trees/{}?recursive=1",
            self.repo, self.git_ref
        )
    }

    pub(crate) fn releases_api_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/releases?per_page=100",
            self.repo
        )
    }

    pub(crate) fn manifest_raw_url(&self, plugin_id: &str) -> String {
        format!(
            "https://raw.githubusercontent.com/{}/{}/plugins/{}/plugin.toml",
            self.repo, self.git_ref, plugin_id
        )
    }

    pub(crate) fn plugin_release_tag(&self, plugin_id: &str, version: &str) -> String {
        format!("{}-v{}", plugin_id, version)
    }
}

pub(super) fn builtin_sources() -> Vec<PluginSource> {
    if let Some(override_sources) = test_sources_override() {
        return override_sources;
    }
    default_builtin_sources()
}

fn default_builtin_sources() -> Vec<PluginSource> {
    vec![PluginSource::new("core", "qol-tools/qol", "main")]
}

pub(crate) fn resolve_source_for_plugin(_plugin_id: &str) -> Option<PluginSource> {
    builtin_sources().into_iter().next()
}

#[cfg(test)]
fn test_sources_override() -> Option<Vec<PluginSource>> {
    test_seam::current()
}

#[cfg(not(test))]
fn test_sources_override() -> Option<Vec<PluginSource>> {
    None
}

#[cfg(test)]
pub(crate) mod test_seam {
    use super::PluginSource;
    use std::sync::{Mutex, OnceLock};

    fn slot() -> &'static Mutex<Option<Vec<PluginSource>>> {
        static SLOT: OnceLock<Mutex<Option<Vec<PluginSource>>>> = OnceLock::new();
        SLOT.get_or_init(|| Mutex::new(None))
    }

    pub(crate) fn current() -> Option<Vec<PluginSource>> {
        slot().lock().expect("test source seam poisoned").clone()
    }

    pub(crate) struct OverrideGuard {
        _private: (),
    }

    impl Drop for OverrideGuard {
        fn drop(&mut self) {
            *slot().lock().expect("test source seam poisoned") = None;
        }
    }

    pub(crate) fn install(sources: Vec<PluginSource>) -> OverrideGuard {
        *slot().lock().expect("test source seam poisoned") = Some(sources);
        OverrideGuard { _private: () }
    }
}

const PLUGINS_DIR: &str = "plugins";

pub(super) fn plugin_id_from_tree_path(path: &str) -> Option<&str> {
    let mut parts = path.split('/');
    if parts.next()? != PLUGINS_DIR {
        return None;
    }
    let dir = parts.next()?;
    if parts.next()? != "plugin.toml" {
        return None;
    }
    if parts.next().is_some() {
        return None;
    }
    if !is_safe_plugin_dir(dir) {
        return None;
    }
    Some(dir)
}

fn is_safe_plugin_dir(dir: &str) -> bool {
    !dir.is_empty()
        && dir.len() <= 128
        && dir
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
        && !dir.starts_with('-')
        && !dir.starts_with('.')
        && !dir.contains("..")
}

pub(crate) fn version_from_plugin_tag(tag: &str, plugin_id: &str) -> Option<String> {
    let prefix = format!("{}-v", plugin_id);
    let suffix = tag.strip_prefix(&prefix)?;
    normalize_semver_tag(suffix)
}

pub(crate) fn select_release_tag<'a, I>(release_tags: I, plugin_id: &str) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    release_tags
        .into_iter()
        .find(|tag| version_from_plugin_tag(tag, plugin_id).is_some())
}

pub(super) fn required_release_asset_names(
    binaries: &[BinaryDependency],
    target: PlatformTarget,
) -> Vec<String> {
    binaries
        .iter()
        .map(|binary| resolve_asset_pattern(&binary.pattern, target))
        .collect()
}

pub(super) fn fold_collected_plugins<I>(
    buckets: I,
) -> Vec<crate::features::plugin_store::github::PluginMetadata>
where
    I: IntoIterator<Item = Vec<crate::features::plugin_store::github::PluginMetadata>>,
{
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for bucket in buckets {
        for plugin in bucket {
            if !seen.insert(plugin.id.clone()) {
                log::info!(
                    "Skipping duplicate plugin id {:?} from later source",
                    plugin.id
                );
                continue;
            }
            out.push(plugin);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::plugin_store::github::PluginMetadata;

    fn meta(id: &str, repo_url: &str) -> PluginMetadata {
        PluginMetadata {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            version: "1.0.0".to_string(),
            repo_url: repo_url.to_string(),
            platforms: None,
        }
    }

    #[test]
    fn builtin_sources_has_one_core_entry() {
        let sources = builtin_sources();
        assert_eq!(sources.len(), 1, "v1 ships one source: {:?}", sources);
        let core = &sources[0];
        assert_eq!(core.name, "core");
        assert_eq!(core.repo, "qol-tools/qol");
        assert_eq!(core.git_ref, "main");
    }

    #[test]
    fn repo_clone_url_resolves_owner_repo_to_https_and_local_paths_verbatim() {
        let cases: &[(&str, &str)] = &[
            ("qol-tools/qol", "https://github.com/qol-tools/qol.git"),
            (
                "some-org/some-repo",
                "https://github.com/some-org/some-repo.git",
            ),
            ("/tmp/local-fixture-repo", "/tmp/local-fixture-repo"),
            ("/var/folders/x/local repo", "/var/folders/x/local repo"),
            ("file:///tmp/local-fixture", "/tmp/local-fixture"),
            ("file:///abs/path/with spaces", "/abs/path/with spaces"),
        ];
        for (repo, expected) in cases {
            let s = PluginSource::new("test", *repo, "main");
            assert_eq!(s.repo_clone_url(), *expected, "repo: {:?}", repo);
        }
    }

    #[tokio::test]
    async fn builtin_sources_returns_test_override_when_installed() {
        let _env = crate::test_support::env_lock().lock().await;
        let custom = vec![PluginSource::new(
            "fixture",
            "/tmp/some-fixture-repo",
            "main",
        )];
        let _guard = test_seam::install(custom.clone());
        assert_eq!(builtin_sources(), custom);
        assert_eq!(
            resolve_source_for_plugin("plugin-x"),
            Some(custom[0].clone())
        );
    }

    #[tokio::test]
    async fn builtin_sources_falls_back_to_default_after_override_dropped() {
        let _env = crate::test_support::env_lock().lock().await;
        {
            let _guard = test_seam::install(vec![PluginSource::new(
                "fixture",
                "/tmp/some-fixture-repo",
                "main",
            )]);
            assert_eq!(
                builtin_sources().first().map(|s| s.name.as_str()),
                Some("fixture")
            );
        }
        let sources = builtin_sources();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name, "core");
        assert_eq!(sources[0].repo, "qol-tools/qol");
    }

    #[test]
    fn plugin_source_url_helpers() {
        let s = PluginSource::new("core", "qol-tools/qol", "main");
        assert_eq!(s.repo_clone_url(), "https://github.com/qol-tools/qol.git");
        assert_eq!(
            s.plugin_subdir_html_url("plugin-alt-tab"),
            "https://github.com/qol-tools/qol/tree/main/plugins/plugin-alt-tab"
        );
        assert_eq!(
            s.tree_api_url(),
            "https://api.github.com/repos/qol-tools/qol/git/trees/main?recursive=1"
        );
        assert_eq!(
            s.releases_api_url(),
            "https://api.github.com/repos/qol-tools/qol/releases?per_page=100"
        );
        assert_eq!(
            s.manifest_raw_url("plugin-alt-tab"),
            "https://raw.githubusercontent.com/qol-tools/qol/main/plugins/plugin-alt-tab/plugin.toml"
        );
        assert_eq!(
            s.plugin_release_tag("plugin-alt-tab", "1.2.3"),
            "plugin-alt-tab-v1.2.3"
        );
    }

    #[test]
    fn plugin_id_from_tree_path_table() {
        let cases: &[(&str, Option<&str>)] = &[
            ("plugins/plugin-alt-tab/plugin.toml", Some("plugin-alt-tab")),
            (
                "plugins/plugin-launcher/plugin.toml",
                Some("plugin-launcher"),
            ),
            ("plugins/task-runner/plugin.toml", Some("task-runner")),
            ("plugins/p/plugin.toml", Some("p")),
            ("plugins/plugin-alt-tab/src/plugin.toml", None),
            ("plugins/plugin-alt-tab/Cargo.toml", None),
            ("plugin-alt-tab/plugin.toml", None),
            ("plugins/plugin-alt-tab", None),
            ("plugins/plugin.toml", None),
            ("", None),
            ("plugins//plugin.toml", None),
            ("plugins/-leading/plugin.toml", None),
            ("plugins/.hidden/plugin.toml", None),
            ("plugins/has..dots/plugin.toml", None),
            ("plugins/UPPER/plugin.toml", None),
            ("apps/qol-tray/plugin.toml", None),
            ("libs/qol-config/plugin.toml", None),
        ];
        for (path, expected) in cases {
            assert_eq!(
                plugin_id_from_tree_path(path),
                *expected,
                "path: {:?}",
                path
            );
        }
    }

    #[test]
    fn version_from_plugin_tag_strips_id_prefix() {
        let cases: &[(&str, &str, Option<&str>)] = &[
            ("plugin-alt-tab-v1.2.3", "plugin-alt-tab", Some("1.2.3")),
            ("plugin-launcher-v0.1.0", "plugin-launcher", Some("0.1.0")),
            (
                "plugin-alt-tab-v1.2.3-beta.1",
                "plugin-alt-tab",
                Some("1.2.3-beta.1"),
            ),
            ("qol-tray-v1.2.3", "plugin-alt-tab", None),
            ("plugin-launcher-v0.1.0", "plugin-alt-tab", None),
            ("plugin-alt-tab-1.2.3", "plugin-alt-tab", None),
            ("plugin-alt-tab-vNOTSEMVER", "plugin-alt-tab", None),
            ("plugin-alt-tabv1.2.3", "plugin-alt-tab", None),
        ];
        for (tag, id, expected) in cases {
            assert_eq!(
                version_from_plugin_tag(tag, id).as_deref(),
                *expected,
                "tag: {:?}, id: {:?}",
                tag,
                id
            );
        }
    }

    #[test]
    fn select_release_tag_picks_newest_first_matching_prefix() {
        let tags = [
            "qol-tray-v3.10.1",
            "plugin-launcher-v0.4.0",
            "plugin-alt-tab-v1.2.3",
            "plugin-alt-tab-v1.2.2",
            "plugin-launcher-v0.3.9",
        ];

        let alt_tab = select_release_tag(tags.iter().copied(), "plugin-alt-tab");
        assert_eq!(
            alt_tab,
            Some("plugin-alt-tab-v1.2.3"),
            "must pick the first (newest) matching tag, not the second-newest"
        );

        let launcher = select_release_tag(tags.iter().copied(), "plugin-launcher");
        assert_eq!(
            launcher,
            Some("plugin-launcher-v0.4.0"),
            "host releases like qol-tray-vN must not match plugin selection"
        );

        let missing = select_release_tag(tags.iter().copied(), "plugin-keyremap");
        assert_eq!(missing, None);
    }

    #[test]
    fn select_release_tag_rejects_non_semver_suffix() {
        let tags = ["plugin-alt-tab-vNOTSEMVER", "plugin-alt-tab-v1.0.0"];
        let got = select_release_tag(tags.iter().copied(), "plugin-alt-tab");
        assert_eq!(
            got,
            Some("plugin-alt-tab-v1.0.0"),
            "non-semver suffixes must be skipped, not chosen"
        );
    }

    #[test]
    fn select_release_tag_rejects_plugin_id_collision() {
        let tags = ["plugin-alt-tab-extra-v1.0.0", "plugin-alt-tab-v0.1.0"];
        let got = select_release_tag(tags.iter().copied(), "plugin-alt-tab");
        assert_eq!(
            got,
            Some("plugin-alt-tab-v0.1.0"),
            "an id-prefix that isn't followed by '-v' must not be accepted"
        );
    }

    #[test]
    fn fold_collected_plugins_first_source_wins_on_id_collision() {
        let source_a = vec![
            meta("plugin-alt-tab", "https://github.com/qol-tools/qol"),
            meta("plugin-launcher", "https://github.com/qol-tools/qol"),
        ];
        let source_b = vec![
            meta("plugin-alt-tab", "https://github.com/some-fork/qol"),
            meta("plugin-extra", "https://github.com/some-fork/qol"),
        ];

        let merged = fold_collected_plugins([source_a, source_b]);

        let ids: Vec<&str> = merged.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["plugin-alt-tab", "plugin-launcher", "plugin-extra"],
            "first-source-wins must preserve source-A's alt-tab and append source-B's unique entries"
        );

        let alt_tab = merged.iter().find(|m| m.id == "plugin-alt-tab").unwrap();
        assert_eq!(
            alt_tab.repo_url, "https://github.com/qol-tools/qol",
            "first-source-wins must keep source-A's repo_url, not source-B's"
        );
    }

    #[test]
    fn required_release_asset_names_maps_each_binary() {
        let binaries = vec![
            BinaryDependency {
                name: "alt-tab".to_string(),
                repo: "legacy/qol-tools".to_string(),
                pattern: "alt-tab-{os}-{arch}".to_string(),
            },
            BinaryDependency {
                name: "launcher".to_string(),
                repo: "legacy/qol-tools".to_string(),
                pattern: "launcher-{os}-{arch}".to_string(),
            },
        ];
        let names = required_release_asset_names(
            &binaries,
            PlatformTarget::current().expect("test runs on a supported host"),
        );
        assert_eq!(names.len(), 2);
        assert!(
            names[0].starts_with("alt-tab-"),
            "expected alt-tab- prefix, got {:?}",
            names[0]
        );
        assert!(
            names[1].starts_with("launcher-"),
            "expected launcher- prefix, got {:?}",
            names[1]
        );
    }
}
