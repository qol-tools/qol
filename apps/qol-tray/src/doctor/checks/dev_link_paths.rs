use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use crate::plugins::registry::{self, Registry, SlotSource};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const ID: &str = "dev_link_paths";

pub(super) struct DevLinkPathsCheck;

impl DoctorCheck for DevLinkPathsCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Dev-link paths", CheckCategory::DevBuild)
            .group(&["dev-loop"])
            .dev_only()
    }

    fn run(&self, ctx: &DoctorContext) -> CheckReport {
        let registry = match ctx.registry() {
            Ok(registry) => registry,
            Err(error) => {
                return CheckReport::ok(format!("could not read plugin registry: {error}"));
            }
        };

        let findings = collect_findings(registry, &fs_manifest_probe, &fs_subplugins_probe);

        if findings.is_empty() {
            return CheckReport::ok("no dev-link path corruption detected".to_string());
        }

        let fixes = findings
            .iter()
            .filter_map(|finding| finding.fix_action())
            .collect::<Vec<_>>();
        CheckReport::warn(format_message(&findings), ID, fixes)
    }
}

pub(crate) fn relocate_dev_link(
    config_dir: &Path,
    plugin_id: &str,
    to: &Path,
) -> Result<(), String> {
    let mut reg = registry::load_registry(config_dir)?;
    let Some(entry) = reg.entries.iter_mut().find(|e| e.id == plugin_id) else {
        return Err(format!("registry entry for {plugin_id} not found"));
    };
    let new_source = match &entry.active.source {
        SlotSource::DevLink { .. } => SlotSource::DevLink {
            origin_path: to.to_path_buf(),
        },
        SlotSource::WorktreeLink { branch, .. } => SlotSource::WorktreeLink {
            origin_path: to.to_path_buf(),
            branch: branch.clone(),
        },
        SlotSource::ReleaseAsset => {
            return Err(format!(
                "registry entry for {plugin_id} is not a live-source slot; refusing to relocate"
            ));
        }
    };
    entry.active.path = to.to_path_buf();
    entry.active.source = new_source;
    registry::save_registry(config_dir, &reg)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ManifestStatus {
    Missing,
    NoManifest,
    WithId(String),
    Unparseable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Finding {
    NoManifest {
        plugin_id: String,
        path: PathBuf,
        resolution: Resolution,
    },
    Missing {
        plugin_id: String,
        path: PathBuf,
    },
    IdMismatch {
        plugin_id: String,
        path: PathBuf,
        found: String,
    },
    Unparseable {
        plugin_id: String,
        path: PathBuf,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    Relocate(PathBuf),
    Ambiguous(Vec<PathBuf>),
    NoMatch,
}

impl Finding {
    fn fix_action(&self) -> Option<FixAction> {
        match self {
            Finding::NoManifest {
                plugin_id,
                resolution: Resolution::Relocate(to),
                ..
            } => Some(FixAction::RelocateDevLink {
                plugin_id: plugin_id.clone(),
                to: to.clone(),
            }),
            _ => None,
        }
    }
}

fn is_live_source(source: &SlotSource) -> bool {
    match source {
        SlotSource::DevLink { .. } | SlotSource::WorktreeLink { .. } => true,
        SlotSource::ReleaseAsset => false,
    }
}

pub(crate) fn collect_findings(
    registry: &Registry,
    manifest_probe: &dyn Fn(&Path) -> ManifestStatus,
    subplugins_probe: &dyn Fn(&Path) -> Vec<(String, PathBuf)>,
) -> Vec<Finding> {
    registry
        .entries
        .iter()
        .filter_map(|entry| {
            if !is_live_source(&entry.active.source) {
                return None;
            }
            classify(
                &entry.id,
                &entry.active.path,
                manifest_probe,
                subplugins_probe,
            )
        })
        .collect()
}

fn classify(
    plugin_id: &str,
    path: &Path,
    manifest_probe: &dyn Fn(&Path) -> ManifestStatus,
    subplugins_probe: &dyn Fn(&Path) -> Vec<(String, PathBuf)>,
) -> Option<Finding> {
    match manifest_probe(path) {
        ManifestStatus::Missing => Some(Finding::Missing {
            plugin_id: plugin_id.to_string(),
            path: path.to_path_buf(),
        }),
        ManifestStatus::WithId(found) if found == plugin_id => None,
        ManifestStatus::WithId(found) => Some(Finding::IdMismatch {
            plugin_id: plugin_id.to_string(),
            path: path.to_path_buf(),
            found,
        }),
        ManifestStatus::Unparseable => Some(Finding::Unparseable {
            plugin_id: plugin_id.to_string(),
            path: path.to_path_buf(),
        }),
        ManifestStatus::NoManifest => Some(Finding::NoManifest {
            plugin_id: plugin_id.to_string(),
            path: path.to_path_buf(),
            resolution: resolve_no_manifest(plugin_id, path, manifest_probe, subplugins_probe),
        }),
    }
}

fn resolve_no_manifest(
    plugin_id: &str,
    path: &Path,
    manifest_probe: &dyn Fn(&Path) -> ManifestStatus,
    subplugins_probe: &dyn Fn(&Path) -> Vec<(String, PathBuf)>,
) -> Resolution {
    let direct = path.join("plugins").join(plugin_id);
    if matches!(manifest_probe(&direct), ManifestStatus::WithId(ref id) if id == plugin_id) {
        return Resolution::Relocate(direct);
    }
    let matches: Vec<PathBuf> = subplugins_probe(path)
        .into_iter()
        .filter_map(|(found_id, sub_path)| (found_id == plugin_id).then_some(sub_path))
        .collect();
    match matches.len() {
        0 => Resolution::NoMatch,
        1 => Resolution::Relocate(matches.into_iter().next().expect("len checked")),
        _ => Resolution::Ambiguous(matches),
    }
}

fn fs_manifest_probe(path: &Path) -> ManifestStatus {
    if !path.exists() {
        return ManifestStatus::Missing;
    }
    if !path.is_dir() {
        return ManifestStatus::Missing;
    }
    let manifest_path = path.join("plugin.toml");
    if !manifest_path.is_file() {
        return ManifestStatus::NoManifest;
    }
    read_plugin_id(&manifest_path).map_or(ManifestStatus::Unparseable, ManifestStatus::WithId)
}

fn fs_subplugins_probe(path: &Path) -> Vec<(String, PathBuf)> {
    let plugins_dir = path.join("plugins");
    let Ok(entries) = std::fs::read_dir(&plugins_dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let sub = entry.path();
        if !sub.is_dir() {
            continue;
        }
        let manifest_path = sub.join("plugin.toml");
        if !manifest_path.is_file() {
            continue;
        }
        if let Some(id) = read_plugin_id(&manifest_path) {
            out.push((id, sub));
        }
    }
    out
}

#[derive(Deserialize)]
struct ManifestSlice {
    plugin: PluginIdSlice,
}

#[derive(Deserialize)]
struct PluginIdSlice {
    id: Option<String>,
}

fn read_plugin_id(manifest_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(manifest_path).ok()?;
    let slice: ManifestSlice = toml::from_str(&content).ok()?;
    slice.plugin.id
}

fn format_message(findings: &[Finding]) -> String {
    let mut by_kind: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for finding in findings {
        match finding {
            Finding::NoManifest {
                plugin_id,
                path,
                resolution,
            } => {
                let label = match resolution {
                    Resolution::Relocate(to) => {
                        format!("{plugin_id} ({} -> {})", path.display(), to.display())
                    }
                    Resolution::Ambiguous(candidates) => format!(
                        "{plugin_id} ({}: ambiguous candidates: {})",
                        path.display(),
                        candidates
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    Resolution::NoMatch => format!(
                        "{plugin_id} ({}: no matching plugin subdir found)",
                        path.display()
                    ),
                };
                by_kind
                    .entry("dev-link path has no plugin.toml")
                    .or_default()
                    .push(label);
            }
            Finding::Missing { plugin_id, path } => by_kind
                .entry("dev-link path missing")
                .or_default()
                .push(format!("{plugin_id} ({})", path.display())),
            Finding::IdMismatch {
                plugin_id,
                path,
                found,
            } => by_kind
                .entry("dev-link plugin.toml id mismatch")
                .or_default()
                .push(format!(
                    "{plugin_id} ({}: manifest declares {found})",
                    path.display()
                )),
            Finding::Unparseable { plugin_id, path } => by_kind
                .entry("dev-link plugin.toml unparseable")
                .or_default()
                .push(format!("{plugin_id} ({})", path.display())),
        }
    }
    let parts: Vec<String> = by_kind
        .into_iter()
        .map(|(kind, items)| format!("{kind}: {}", items.join(", ")))
        .collect();
    format!("dev-link path corruption detected — {}", parts.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::registry::{Entry, Slot};
    use std::collections::HashMap;

    fn devlink(id: &str, path: &str) -> Entry {
        Entry {
            id: id.into(),
            active: Slot {
                path: PathBuf::from(path),
                source: SlotSource::DevLink {
                    origin_path: PathBuf::from(path),
                },
            },
            fallback: None,
        }
    }

    fn worktree_link(id: &str, path: &str, branch: &str) -> Entry {
        Entry {
            id: id.into(),
            active: Slot {
                path: PathBuf::from(path),
                source: SlotSource::WorktreeLink {
                    origin_path: PathBuf::from(path),
                    branch: branch.into(),
                },
            },
            fallback: None,
        }
    }

    fn release(id: &str, path: &str) -> Entry {
        Entry {
            id: id.into(),
            active: Slot {
                path: PathBuf::from(path),
                source: SlotSource::ReleaseAsset,
            },
            fallback: None,
        }
    }

    fn registry_with(entries: Vec<Entry>) -> Registry {
        Registry {
            version: registry::CURRENT_REGISTRY_VERSION,
            entries,
        }
    }

    fn map_probe(map: HashMap<PathBuf, ManifestStatus>) -> impl Fn(&Path) -> ManifestStatus {
        move |p: &Path| map.get(p).cloned().unwrap_or(ManifestStatus::Missing)
    }

    fn empty_subprobe() -> impl Fn(&Path) -> Vec<(String, PathBuf)> {
        |_: &Path| Vec::new()
    }

    #[test]
    fn healthy_devlink_yields_no_findings() {
        let registry = registry_with(vec![devlink("plugin-foo", "/ws/plugins/plugin-foo")]);
        let mut probe = HashMap::new();
        probe.insert(
            PathBuf::from("/ws/plugins/plugin-foo"),
            ManifestStatus::WithId("plugin-foo".into()),
        );
        let findings = collect_findings(&registry, &map_probe(probe), &empty_subprobe());
        assert!(findings.is_empty(), "got: {findings:?}");
    }

    #[test]
    fn release_asset_entries_are_ignored() {
        let registry = registry_with(vec![release("plugin-foo", "/installed/plugin-foo")]);
        let probe = HashMap::new();
        let findings = collect_findings(&registry, &map_probe(probe), &empty_subprobe());
        assert!(findings.is_empty(), "got: {findings:?}");
    }

    #[test]
    fn missing_path_reports_missing_no_fix() {
        let registry = registry_with(vec![devlink("plugin-foo", "/gone")]);
        let mut probe = HashMap::new();
        probe.insert(PathBuf::from("/gone"), ManifestStatus::Missing);
        let findings = collect_findings(&registry, &map_probe(probe), &empty_subprobe());
        assert_eq!(
            findings,
            vec![Finding::Missing {
                plugin_id: "plugin-foo".into(),
                path: PathBuf::from("/gone"),
            }]
        );
        assert!(findings[0].fix_action().is_none());
    }

    #[test]
    fn id_mismatch_warns_but_no_auto_fix() {
        let registry = registry_with(vec![devlink("plugin-foo", "/other/plugin-bar")]);
        let mut probe = HashMap::new();
        probe.insert(
            PathBuf::from("/other/plugin-bar"),
            ManifestStatus::WithId("plugin-bar".into()),
        );
        let findings = collect_findings(&registry, &map_probe(probe), &empty_subprobe());
        assert_eq!(
            findings,
            vec![Finding::IdMismatch {
                plugin_id: "plugin-foo".into(),
                path: PathBuf::from("/other/plugin-bar"),
                found: "plugin-bar".into(),
            }]
        );
        assert!(findings[0].fix_action().is_none());
    }

    #[test]
    fn no_manifest_resolves_via_direct_plugins_subdir() {
        let registry = registry_with(vec![devlink("plugin-foo", "/workspace")]);
        let mut probe = HashMap::new();
        probe.insert(PathBuf::from("/workspace"), ManifestStatus::NoManifest);
        probe.insert(
            PathBuf::from("/workspace/plugins/plugin-foo"),
            ManifestStatus::WithId("plugin-foo".into()),
        );
        let findings = collect_findings(&registry, &map_probe(probe), &empty_subprobe());
        assert_eq!(
            findings,
            vec![Finding::NoManifest {
                plugin_id: "plugin-foo".into(),
                path: PathBuf::from("/workspace"),
                resolution: Resolution::Relocate(PathBuf::from("/workspace/plugins/plugin-foo")),
            }]
        );
        assert_eq!(
            findings[0].fix_action(),
            Some(FixAction::RelocateDevLink {
                plugin_id: "plugin-foo".into(),
                to: PathBuf::from("/workspace/plugins/plugin-foo"),
            })
        );
    }

    #[test]
    fn no_manifest_resolves_via_shallow_scan_when_direct_subdir_absent() {
        let registry = registry_with(vec![devlink("plugin-foo", "/workspace")]);
        let mut probe = HashMap::new();
        probe.insert(PathBuf::from("/workspace"), ManifestStatus::NoManifest);
        let sub_probe = |p: &Path| {
            if p == Path::new("/workspace") {
                vec![(
                    "plugin-foo".to_string(),
                    PathBuf::from("/workspace/apps/plugin-foo"),
                )]
            } else {
                Vec::new()
            }
        };
        let findings = collect_findings(&registry, &map_probe(probe), &sub_probe);
        assert_eq!(
            findings,
            vec![Finding::NoManifest {
                plugin_id: "plugin-foo".into(),
                path: PathBuf::from("/workspace"),
                resolution: Resolution::Relocate(PathBuf::from("/workspace/apps/plugin-foo")),
            }]
        );
    }

    #[test]
    fn no_manifest_with_multiple_matches_is_ambiguous_no_fix() {
        let registry = registry_with(vec![devlink("plugin-foo", "/workspace")]);
        let mut probe = HashMap::new();
        probe.insert(PathBuf::from("/workspace"), ManifestStatus::NoManifest);
        let sub_probe = |p: &Path| {
            if p == Path::new("/workspace") {
                vec![
                    (
                        "plugin-foo".into(),
                        PathBuf::from("/workspace/a/plugin-foo"),
                    ),
                    (
                        "plugin-foo".into(),
                        PathBuf::from("/workspace/b/plugin-foo"),
                    ),
                ]
            } else {
                Vec::new()
            }
        };
        let findings = collect_findings(&registry, &map_probe(probe), &sub_probe);
        match &findings[..] {
            [Finding::NoManifest {
                resolution: Resolution::Ambiguous(candidates),
                ..
            }] => {
                assert_eq!(candidates.len(), 2);
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(findings[0].fix_action().is_none());
    }

    #[test]
    fn no_manifest_with_no_subdir_match_yields_no_match_no_fix() {
        let registry = registry_with(vec![devlink("plugin-foo", "/workspace")]);
        let mut probe = HashMap::new();
        probe.insert(PathBuf::from("/workspace"), ManifestStatus::NoManifest);
        let findings = collect_findings(&registry, &map_probe(probe), &empty_subprobe());
        assert_eq!(
            findings,
            vec![Finding::NoManifest {
                plugin_id: "plugin-foo".into(),
                path: PathBuf::from("/workspace"),
                resolution: Resolution::NoMatch,
            }]
        );
        assert!(findings[0].fix_action().is_none());
    }

    #[test]
    fn shallow_scan_only_runs_when_direct_subdir_missing() {
        let registry = registry_with(vec![devlink("plugin-foo", "/workspace")]);
        let mut probe = HashMap::new();
        probe.insert(PathBuf::from("/workspace"), ManifestStatus::NoManifest);
        probe.insert(
            PathBuf::from("/workspace/plugins/plugin-foo"),
            ManifestStatus::WithId("plugin-foo".into()),
        );
        use std::cell::Cell;
        let calls = Cell::new(0_usize);
        let sub_probe = |_: &Path| {
            calls.set(calls.get() + 1);
            Vec::new()
        };
        let findings = collect_findings(&registry, &map_probe(probe), &sub_probe);
        assert!(matches!(
            findings[0],
            Finding::NoManifest {
                resolution: Resolution::Relocate(_),
                ..
            }
        ));
        assert_eq!(
            calls.get(),
            0,
            "shallow scan must not run when direct plugins/<id> subdir resolves"
        );
    }

    #[test]
    fn unparseable_manifest_warns_without_auto_fix() {
        let registry = registry_with(vec![devlink("plugin-foo", "/path")]);
        let mut probe = HashMap::new();
        probe.insert(PathBuf::from("/path"), ManifestStatus::Unparseable);
        let findings = collect_findings(&registry, &map_probe(probe), &empty_subprobe());
        assert_eq!(
            findings,
            vec![Finding::Unparseable {
                plugin_id: "plugin-foo".into(),
                path: PathBuf::from("/path"),
            }]
        );
        assert!(findings[0].fix_action().is_none());
    }

    #[test]
    fn message_groups_findings_by_kind() {
        let findings = vec![
            Finding::NoManifest {
                plugin_id: "plugin-a".into(),
                path: PathBuf::from("/ws"),
                resolution: Resolution::Relocate(PathBuf::from("/ws/plugins/plugin-a")),
            },
            Finding::Missing {
                plugin_id: "plugin-b".into(),
                path: PathBuf::from("/gone"),
            },
            Finding::IdMismatch {
                plugin_id: "plugin-c".into(),
                path: PathBuf::from("/x"),
                found: "plugin-z".into(),
            },
        ];
        let message = format_message(&findings);
        assert!(
            message.contains(
                "dev-link path has no plugin.toml: plugin-a (/ws -> /ws/plugins/plugin-a)"
            ),
            "actual: {message}"
        );
        assert!(
            message.contains("dev-link path missing: plugin-b (/gone)"),
            "actual: {message}"
        );
        assert!(
            message.contains(
                "dev-link plugin.toml id mismatch: plugin-c (/x: manifest declares plugin-z)"
            ),
            "actual: {message}"
        );
    }

    #[test]
    fn read_plugin_id_extracts_kebab_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        let manifest = tmp.path().join("plugin.toml");
        std::fs::write(&manifest, "[plugin]\nid = \"plugin-foo\"\nname = \"Foo\"\n").unwrap();
        assert_eq!(read_plugin_id(&manifest).as_deref(), Some("plugin-foo"));
    }

    #[test]
    fn read_plugin_id_returns_none_when_id_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let manifest = tmp.path().join("plugin.toml");
        std::fs::write(&manifest, "[plugin]\nname = \"Foo\"\n").unwrap();
        assert_eq!(read_plugin_id(&manifest), None);
    }

    #[test]
    fn read_plugin_id_returns_none_for_unparseable_toml() {
        let tmp = tempfile::TempDir::new().unwrap();
        let manifest = tmp.path().join("plugin.toml");
        std::fs::write(&manifest, "not valid toml [[[").unwrap();
        assert_eq!(read_plugin_id(&manifest), None);
    }

    #[test]
    fn fs_manifest_probe_classifies_real_directories() {
        let tmp = tempfile::TempDir::new().unwrap();
        let missing = tmp.path().join("absent");
        assert_eq!(fs_manifest_probe(&missing), ManifestStatus::Missing);

        let no_manifest_dir = tmp.path().join("no-manifest");
        std::fs::create_dir(&no_manifest_dir).unwrap();
        assert_eq!(
            fs_manifest_probe(&no_manifest_dir),
            ManifestStatus::NoManifest
        );

        let with_id_dir = tmp.path().join("with-id");
        std::fs::create_dir(&with_id_dir).unwrap();
        std::fs::write(
            with_id_dir.join("plugin.toml"),
            "[plugin]\nid = \"plugin-foo\"\nname = \"Foo\"\n",
        )
        .unwrap();
        assert_eq!(
            fs_manifest_probe(&with_id_dir),
            ManifestStatus::WithId("plugin-foo".into())
        );

        let bad_dir = tmp.path().join("bad");
        std::fs::create_dir(&bad_dir).unwrap();
        std::fs::write(bad_dir.join("plugin.toml"), "not toml [[[").unwrap();
        assert_eq!(fs_manifest_probe(&bad_dir), ManifestStatus::Unparseable);
    }

    #[test]
    fn fs_subplugins_probe_finds_id_matches_in_plugins_subdirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plugins = tmp.path().join("plugins");
        std::fs::create_dir(&plugins).unwrap();
        for (subdir, id) in [
            ("plugin-foo", "plugin-foo"),
            ("plugin-bar", "plugin-bar"),
            ("renamed-on-disk", "plugin-baz"),
        ] {
            let sub = plugins.join(subdir);
            std::fs::create_dir(&sub).unwrap();
            std::fs::write(
                sub.join("plugin.toml"),
                format!("[plugin]\nid = \"{id}\"\nname = \"X\"\n"),
            )
            .unwrap();
        }
        std::fs::create_dir(plugins.join("no-manifest-dir")).unwrap();

        let mut out = fs_subplugins_probe(tmp.path());
        out.sort();
        let mut want = vec![
            ("plugin-bar".to_string(), plugins.join("plugin-bar")),
            ("plugin-baz".to_string(), plugins.join("renamed-on-disk")),
            ("plugin-foo".to_string(), plugins.join("plugin-foo")),
        ];
        want.sort();
        assert_eq!(out, want);
    }

    #[test]
    fn relocate_dev_link_rewrites_path_and_origin() {
        let tmp = tempfile::TempDir::new().unwrap();
        let reg = registry_with(vec![devlink("plugin-foo", "/workspace")]);
        registry::save_registry(tmp.path(), &reg).unwrap();

        relocate_dev_link(
            tmp.path(),
            "plugin-foo",
            Path::new("/workspace/plugins/plugin-foo"),
        )
        .unwrap();

        let loaded = registry::load_registry(tmp.path()).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(
            loaded.entries[0].active.path,
            PathBuf::from("/workspace/plugins/plugin-foo")
        );
        match &loaded.entries[0].active.source {
            SlotSource::DevLink { origin_path } => {
                assert_eq!(origin_path, Path::new("/workspace/plugins/plugin-foo"))
            }
            other => panic!("expected DevLink, got {other:?}"),
        }
    }

    #[test]
    fn relocate_dev_link_is_idempotent_when_path_already_correct() {
        let tmp = tempfile::TempDir::new().unwrap();
        let reg = registry_with(vec![devlink("plugin-foo", "/workspace/plugins/plugin-foo")]);
        registry::save_registry(tmp.path(), &reg).unwrap();

        relocate_dev_link(
            tmp.path(),
            "plugin-foo",
            Path::new("/workspace/plugins/plugin-foo"),
        )
        .unwrap();

        let loaded = registry::load_registry(tmp.path()).unwrap();
        assert_eq!(loaded, reg);
    }

    #[test]
    fn relocate_dev_link_refuses_to_touch_release_asset_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let reg = registry_with(vec![release("plugin-foo", "/installed/plugin-foo")]);
        registry::save_registry(tmp.path(), &reg).unwrap();

        let err = relocate_dev_link(
            tmp.path(),
            "plugin-foo",
            Path::new("/workspace/plugins/plugin-foo"),
        )
        .expect_err("must refuse to mutate ReleaseAsset entries");
        assert!(err.contains("not a live-source slot"), "actual: {err}");

        let loaded = registry::load_registry(tmp.path()).unwrap();
        assert_eq!(loaded, reg);
    }

    #[test]
    fn relocate_dev_link_errors_when_plugin_id_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let reg = registry_with(vec![devlink("plugin-bar", "/some/path")]);
        registry::save_registry(tmp.path(), &reg).unwrap();

        let err = relocate_dev_link(tmp.path(), "plugin-foo", Path::new("/anywhere"))
            .expect_err("must refuse to invent entries");
        assert!(err.contains("not found"), "actual: {err}");

        let loaded = registry::load_registry(tmp.path()).unwrap();
        assert_eq!(loaded, reg);
    }

    #[test]
    fn worktree_link_entries_are_also_classified_as_live_sources() {
        let registry = registry_with(vec![worktree_link("plugin-foo", "/workspace", "feat-x")]);
        let mut probe = HashMap::new();
        probe.insert(PathBuf::from("/workspace"), ManifestStatus::NoManifest);
        probe.insert(
            PathBuf::from("/workspace/plugins/plugin-foo"),
            ManifestStatus::WithId("plugin-foo".into()),
        );
        let findings = collect_findings(&registry, &map_probe(probe), &empty_subprobe());
        assert_eq!(
            findings,
            vec![Finding::NoManifest {
                plugin_id: "plugin-foo".into(),
                path: PathBuf::from("/workspace"),
                resolution: Resolution::Relocate(PathBuf::from("/workspace/plugins/plugin-foo")),
            }],
            "WorktreeLink must be checked the same way as DevLink",
        );
    }

    #[test]
    fn relocate_preserves_worktree_link_variant_and_branch() {
        let tmp = tempfile::TempDir::new().unwrap();
        let reg = registry_with(vec![worktree_link("plugin-foo", "/workspace", "feat-x")]);
        registry::save_registry(tmp.path(), &reg).unwrap();

        relocate_dev_link(
            tmp.path(),
            "plugin-foo",
            Path::new("/workspace/plugins/plugin-foo"),
        )
        .unwrap();

        let loaded = registry::load_registry(tmp.path()).unwrap();
        assert_eq!(
            loaded.entries[0].active.path,
            PathBuf::from("/workspace/plugins/plugin-foo")
        );
        match &loaded.entries[0].active.source {
            SlotSource::WorktreeLink {
                origin_path,
                branch,
            } => {
                assert_eq!(origin_path, Path::new("/workspace/plugins/plugin-foo"));
                assert_eq!(branch, "feat-x", "branch must survive relocation");
            }
            other => panic!("expected WorktreeLink, got {other:?}"),
        }
    }

    #[test]
    fn relocate_dev_link_does_not_disturb_other_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let reg = registry_with(vec![
            devlink("plugin-foo", "/workspace"),
            devlink("plugin-bar", "/workspace/plugins/plugin-bar"),
            release("plugin-baz", "/installed/plugin-baz"),
        ]);
        registry::save_registry(tmp.path(), &reg).unwrap();

        relocate_dev_link(
            tmp.path(),
            "plugin-foo",
            Path::new("/workspace/plugins/plugin-foo"),
        )
        .unwrap();

        let loaded = registry::load_registry(tmp.path()).unwrap();
        assert_eq!(loaded.entries.len(), 3);
        let by_id: HashMap<_, _> = loaded.entries.iter().map(|e| (e.id.clone(), e)).collect();
        assert_eq!(
            by_id["plugin-foo"].active.path,
            PathBuf::from("/workspace/plugins/plugin-foo")
        );
        assert_eq!(
            by_id["plugin-bar"].active.path,
            PathBuf::from("/workspace/plugins/plugin-bar")
        );
        assert!(matches!(
            by_id["plugin-baz"].active.source,
            SlotSource::ReleaseAsset
        ));
        assert_eq!(
            by_id["plugin-baz"].active.path,
            PathBuf::from("/installed/plugin-baz")
        );
    }
}
