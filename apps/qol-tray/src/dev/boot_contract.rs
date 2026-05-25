//! Single owner of the boot-target invariant: dev/active-worktree.txt and the
//! OS autostart artifact must agree on which binary should boot qol-tray.
//! Spec: docs/superpowers/specs/2026-05-24-boot-target-drift-fix-design.md.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BootTarget {
    Worktree { branch: String, binary: PathBuf },
    Fallback { binary: PathBuf },
}

impl BootTarget {
    pub fn binary(&self) -> &std::path::Path {
        match self {
            BootTarget::Worktree { binary, .. } | BootTarget::Fallback { binary } => binary,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DriftEvent {
    SelectionMissingFromWorktreeList { branch: String },
    SelectionBinaryNotBuilt { branch: String, expected: PathBuf },
    AutostartTargetDisagrees { actual: PathBuf, expected: PathBuf },
    IgnoredDevMarker { branch: String },
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct HealReport {
    pub events: Vec<DriftEvent>,
    pub actions: Vec<HealAction>,
    pub failures: Vec<HealFailure>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HealAction {
    ClearedSelection { branch: String },
    WroteAutostart { binary: PathBuf },
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HealFailure {
    WriteAutostartFailed { binary: PathBuf, error: String },
    ClearSelectionFailed { error: String },
}

#[derive(Clone, Debug)]
pub struct SetSelectedReport {
    pub target: BootTarget,
    pub cleared_selection: bool,
    pub wrote_autostart: bool,
}

use std::collections::HashMap;
use std::path::Path;

/// Lists git worktrees as a branch -> root-directory map.
pub trait WorktreeLister: Send + Sync {
    fn list(&self) -> HashMap<String, PathBuf>;
}

/// Probes whether a file exists at the given path.
pub trait BinaryProbe: Send + Sync {
    fn exists(&self, path: &Path) -> bool;
}

pub struct GitWorktreeLister;
impl WorktreeLister for GitWorktreeLister {
    fn list(&self) -> HashMap<String, PathBuf> {
        #[cfg(not(feature = "dev"))]
        {
            return HashMap::new();
        }
        #[cfg(feature = "dev")]
        {
            let stdout = std::process::Command::new("git")
                .args(["worktree", "list", "--porcelain"])
                .current_dir(crate::paths::repo_root_from_manifest_dir())
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            parse_branch_map(&stdout)
        }
    }
}

fn parse_branch_map(porcelain: &str) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();
    for block in porcelain.split("\n\n") {
        let mut wt_path: Option<&str> = None;
        let mut branch: Option<&str> = None;
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                wt_path = Some(p);
            }
            if let Some(b) = line.strip_prefix("branch refs/heads/") {
                branch = Some(b);
            }
        }
        if let (Some(p), Some(b)) = (wt_path, branch) {
            map.insert(b.to_string(), PathBuf::from(p));
        }
    }
    map
}

pub struct FsBinaryProbe;
impl BinaryProbe for FsBinaryProbe {
    fn exists(&self, path: &Path) -> bool {
        path.is_file()
    }
}

#[cfg(test)]
pub(crate) struct InMemoryWorktreeLister {
    pub map: HashMap<String, PathBuf>,
}

#[cfg(test)]
impl WorktreeLister for InMemoryWorktreeLister {
    fn list(&self) -> HashMap<String, PathBuf> {
        self.map.clone()
    }
}

#[cfg(test)]
pub(crate) struct InMemoryBinaryProbe {
    pub existing: std::collections::HashSet<PathBuf>,
}

#[cfg(test)]
impl BinaryProbe for InMemoryBinaryProbe {
    fn exists(&self, path: &Path) -> bool {
        self.existing.contains(path)
    }
}

/// Reads `dev/active-worktree.txt` (dev only), walks the worktree list, checks
/// whether the expected binary is built, reads the autostart artifact, and
/// returns the resolved BootTarget plus every drift event observed.
pub fn resolve(
    env: &dyn crate::installer::BootEnvironment,
    config_dir: &Path,
    lister: &dyn WorktreeLister,
    probe: &dyn BinaryProbe,
) -> (BootTarget, Vec<DriftEvent>) {
    let canonical = env.canonical_binary().unwrap_or_default();
    let autostart = env.read_autostart_target().ok().flatten();
    let mut events = Vec::new();

    let marker = read_marker(config_dir);

    if !env.honors_dev_selection() {
        if let Some(branch) = marker.clone() {
            events.push(DriftEvent::IgnoredDevMarker { branch });
        }
        push_autostart_drift_if_any(&autostart, &canonical, &mut events);
        return (BootTarget::Fallback { binary: canonical }, events);
    }

    let Some(branch) = marker else {
        push_autostart_drift_if_any(&autostart, &canonical, &mut events);
        return (BootTarget::Fallback { binary: canonical }, events);
    };

    let worktrees = lister.list();
    let Some(worktree_dir) = worktrees.get(&branch).cloned() else {
        events.push(DriftEvent::SelectionMissingFromWorktreeList {
            branch: branch.clone(),
        });
        push_autostart_drift_if_any(&autostart, &canonical, &mut events);
        return (BootTarget::Fallback { binary: canonical }, events);
    };

    let expected_binary = worktree_dir
        .join("target")
        .join("debug")
        .join(crate::installer::binary_filename());
    if !probe.exists(&expected_binary) {
        events.push(DriftEvent::SelectionBinaryNotBuilt {
            branch,
            expected: expected_binary,
        });
        push_autostart_drift_if_any(&autostart, &canonical, &mut events);
        return (BootTarget::Fallback { binary: canonical }, events);
    }

    push_autostart_drift_if_any(&autostart, &expected_binary, &mut events);
    (
        BootTarget::Worktree {
            branch,
            binary: expected_binary,
        },
        events,
    )
}

fn read_marker(config_dir: &Path) -> Option<String> {
    let path = config_dir.join("dev").join("active-worktree.txt");
    std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn push_autostart_drift_if_any(
    actual: &Option<PathBuf>,
    expected: &Path,
    events: &mut Vec<DriftEvent>,
) {
    let actual_path = match actual {
        Some(a) => a.clone(),
        None => {
            events.push(DriftEvent::AutostartTargetDisagrees {
                actual: PathBuf::new(),
                expected: expected.to_path_buf(),
            });
            return;
        }
    };
    if !paths_equal(&actual_path, expected) {
        events.push(DriftEvent::AutostartTargetDisagrees {
            actual: actual_path,
            expected: expected.to_path_buf(),
        });
    }
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    let canon_a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let canon_b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    canon_a == canon_b
}

/// Atomic-ish two-step write: persist `active-worktree.txt` (or clear) and write
/// the autostart artifact to the resolved binary. Idempotent: only writes if the
/// state would actually change. Returns SetSelectedReport flagging which
/// side-effects ran so callers (heal) can record precise actions.
pub fn set_selected_worktree(
    env: &dyn crate::installer::BootEnvironment,
    config_dir: &Path,
    branch: Option<&str>,
    lister: &dyn WorktreeLister,
    probe: &dyn BinaryProbe,
) -> anyhow::Result<SetSelectedReport> {
    let prior_marker = read_marker(config_dir);
    let new_marker = if env.honors_dev_selection() {
        branch.map(str::to_string)
    } else {
        None
    };
    let prior_matches_new = prior_marker.as_deref() == new_marker.as_deref();
    let cleared_selection = prior_marker.is_some() && new_marker.is_none();
    if !prior_matches_new {
        crate::dev::linking::set_active_worktree_branch(config_dir, new_marker.as_deref())
            .map_err(anyhow::Error::msg)?;
    }

    let (target, _events) = resolve(env, config_dir, lister, probe);
    let current = env.read_autostart_target().ok().flatten();
    let mut wrote_autostart = false;
    let desired = target.binary();
    let current_matches = current.as_deref().is_some_and(|c| paths_equal(c, desired));
    if !current_matches {
        env.write_autostart_target(desired)?;
        wrote_autostart = true;
    }

    Ok(SetSelectedReport {
        target,
        cleared_selection,
        wrote_autostart,
    })
}

/// Runs resolve, observes drift events, applies the healing matrix:
/// - If selection-missing or ignored-dev-marker: clear marker, then write autostart.
/// - Else if autostart drift only: keep marker, write autostart.
/// - Else (only SelectionBinaryNotBuilt + autostart already aligned to fallback): no-op.
pub fn heal_drift_on_startup(
    env: &dyn crate::installer::BootEnvironment,
    config_dir: &Path,
    lister: &dyn WorktreeLister,
    probe: &dyn BinaryProbe,
) -> HealReport {
    let (_target, events) = resolve(env, config_dir, lister, probe);
    let mut report = HealReport {
        events: events.clone(),
        actions: Vec::new(),
        failures: Vec::new(),
    };
    if events.is_empty() {
        return report;
    }

    let clear_selection = events.iter().any(|e| {
        matches!(
            e,
            DriftEvent::SelectionMissingFromWorktreeList { .. }
                | DriftEvent::IgnoredDevMarker { .. }
        )
    });
    let autostart_disagrees = events
        .iter()
        .any(|e| matches!(e, DriftEvent::AutostartTargetDisagrees { .. }));

    if !(clear_selection || autostart_disagrees) {
        return report;
    }

    let target_branch = if clear_selection {
        None
    } else {
        read_marker(config_dir)
    };

    match set_selected_worktree(env, config_dir, target_branch.as_deref(), lister, probe) {
        Ok(set_report) => {
            if set_report.cleared_selection {
                let branch = events
                    .iter()
                    .find_map(|e| match e {
                        DriftEvent::SelectionMissingFromWorktreeList { branch }
                        | DriftEvent::IgnoredDevMarker { branch } => Some(branch.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                report.actions.push(HealAction::ClearedSelection { branch });
            }
            if set_report.wrote_autostart {
                report.actions.push(HealAction::WroteAutostart {
                    binary: set_report.target.binary().to_path_buf(),
                });
            }
        }
        Err(e) => {
            let binary = env.canonical_binary().unwrap_or_default();
            report.failures.push(HealFailure::WriteAutostartFailed {
                binary,
                error: e.to_string(),
            });
        }
    }

    report
}

#[cfg(test)]
mod seam_tests {
    use super::*;

    #[test]
    fn parse_branch_map_simple() {
        let porcelain = "worktree /a\nHEAD abc\nbranch refs/heads/x\n\nworktree /b\nHEAD def\nbranch refs/heads/y\n\n";
        let m = parse_branch_map(porcelain);
        assert_eq!(m.get("x"), Some(&PathBuf::from("/a")));
        assert_eq!(m.get("y"), Some(&PathBuf::from("/b")));
    }

    #[test]
    fn parse_branch_map_ignores_detached() {
        let porcelain =
            "worktree /a\nHEAD abc\ndetached\n\nworktree /b\nHEAD def\nbranch refs/heads/y\n\n";
        let m = parse_branch_map(porcelain);
        assert!(!m.contains_key("(no branch)"));
        assert_eq!(m.get("y"), Some(&PathBuf::from("/b")));
    }

    #[test]
    fn in_memory_lister_returns_seeded_map() {
        let lister = InMemoryWorktreeLister {
            map: [("main".to_string(), PathBuf::from("/repo"))]
                .into_iter()
                .collect(),
        };
        let m = lister.list();
        assert_eq!(m.get("main"), Some(&PathBuf::from("/repo")));
    }

    #[test]
    fn in_memory_probe_hits_and_misses() {
        let probe = InMemoryBinaryProbe {
            existing: [PathBuf::from("/bin/qol-tray")].into_iter().collect(),
        };
        assert!(probe.exists(Path::new("/bin/qol-tray")));
        assert!(!probe.exists(Path::new("/bin/missing")));
    }
}

#[cfg(test)]
mod resolve_dev_tests {
    use super::*;
    use crate::installer::boot_environment::InMemoryBootEnvironment;
    use std::collections::HashSet;
    use tempfile::TempDir;

    fn dev_env(canonical: PathBuf, autostart: Option<PathBuf>) -> InMemoryBootEnvironment {
        let env = InMemoryBootEnvironment::new(canonical, true);
        if let Some(a) = autostart {
            env.with_autostart(a)
        } else {
            env
        }
    }

    fn lister(pairs: &[(&str, &str)]) -> InMemoryWorktreeLister {
        let map = pairs
            .iter()
            .map(|(b, p)| ((*b).to_string(), PathBuf::from(p)))
            .collect();
        InMemoryWorktreeLister { map }
    }

    fn probe(exists: &[&str]) -> InMemoryBinaryProbe {
        InMemoryBinaryProbe {
            existing: exists.iter().map(PathBuf::from).collect::<HashSet<_>>(),
        }
    }

    fn write_marker(dir: &Path, value: &str) {
        std::fs::create_dir_all(dir.join("dev")).unwrap();
        std::fs::write(dir.join("dev/active-worktree.txt"), value).unwrap();
    }

    fn binary_in_worktree(wt_path: &str) -> String {
        format!(
            "{}/target/debug/{}",
            wt_path,
            crate::installer::binary_filename()
        )
    }

    #[test]
    fn dev_unset_returns_fallback() {
        let tmp = TempDir::new().unwrap();
        let canonical = PathBuf::from("/main/qol-tray");
        let env = dev_env(canonical.clone(), Some(canonical.clone()));
        let (target, events) = resolve(&env, tmp.path(), &lister(&[]), &probe(&[]));
        assert_eq!(target, BootTarget::Fallback { binary: canonical });
        assert!(events.is_empty());
    }

    #[test]
    fn dev_set_valid_built_aligned_is_clean() {
        let tmp = TempDir::new().unwrap();
        write_marker(tmp.path(), "feat-x");
        let wt = "/wt";
        let binary_path = binary_in_worktree(wt);
        let env = dev_env(
            PathBuf::from("/main/qol-tray"),
            Some(PathBuf::from(&binary_path)),
        );
        let (target, events) = resolve(
            &env,
            tmp.path(),
            &lister(&[("feat-x", wt)]),
            &probe(&[&binary_path]),
        );
        assert_eq!(
            target,
            BootTarget::Worktree {
                branch: "feat-x".to_string(),
                binary: PathBuf::from(&binary_path),
            }
        );
        assert!(events.is_empty());
    }

    #[test]
    fn dev_set_valid_built_autostart_drift() {
        let tmp = TempDir::new().unwrap();
        write_marker(tmp.path(), "feat-x");
        let wt = "/wt";
        let binary_path = binary_in_worktree(wt);
        let env = dev_env(
            PathBuf::from("/main/qol-tray"),
            Some(PathBuf::from("/elsewhere")),
        );
        let (target, events) = resolve(
            &env,
            tmp.path(),
            &lister(&[("feat-x", wt)]),
            &probe(&[&binary_path]),
        );
        assert!(matches!(target, BootTarget::Worktree { .. }));
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            DriftEvent::AutostartTargetDisagrees { .. }
        ));
    }

    #[test]
    fn dev_set_branch_missing() {
        let tmp = TempDir::new().unwrap();
        write_marker(tmp.path(), "ghost");
        let canonical = PathBuf::from("/main/qol-tray");
        let env = dev_env(canonical.clone(), Some(canonical.clone()));
        let (target, events) = resolve(&env, tmp.path(), &lister(&[]), &probe(&[]));
        assert_eq!(target, BootTarget::Fallback { binary: canonical });
        assert!(matches!(
            events[0],
            DriftEvent::SelectionMissingFromWorktreeList { .. }
        ));
    }

    #[test]
    fn dev_set_binary_not_built() {
        let tmp = TempDir::new().unwrap();
        write_marker(tmp.path(), "feat-x");
        let canonical = PathBuf::from("/main/qol-tray");
        let env = dev_env(canonical.clone(), Some(canonical.clone()));
        let (target, events) =
            resolve(&env, tmp.path(), &lister(&[("feat-x", "/wt")]), &probe(&[]));
        assert_eq!(target, BootTarget::Fallback { binary: canonical });
        assert!(matches!(
            events[0],
            DriftEvent::SelectionBinaryNotBuilt { .. }
        ));
    }
}

#[cfg(test)]
mod resolve_prod_tests {
    use super::*;
    use crate::installer::boot_environment::InMemoryBootEnvironment;
    use tempfile::TempDir;

    fn prod_env(canonical: PathBuf, autostart: Option<PathBuf>) -> InMemoryBootEnvironment {
        let env = InMemoryBootEnvironment::new(canonical, false);
        if let Some(a) = autostart {
            env.with_autostart(a)
        } else {
            env
        }
    }

    fn empty_lister() -> InMemoryWorktreeLister {
        InMemoryWorktreeLister {
            map: Default::default(),
        }
    }

    fn empty_probe() -> InMemoryBinaryProbe {
        InMemoryBinaryProbe {
            existing: Default::default(),
        }
    }

    #[test]
    fn prod_no_marker_aligned_is_clean() {
        let tmp = TempDir::new().unwrap();
        let canonical = PathBuf::from("/install/qol-tray");
        let env = prod_env(canonical.clone(), Some(canonical.clone()));
        let (target, events) = resolve(&env, tmp.path(), &empty_lister(), &empty_probe());
        assert_eq!(target, BootTarget::Fallback { binary: canonical });
        assert!(events.is_empty());
    }

    #[test]
    fn prod_marker_present_emits_ignored() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("dev")).unwrap();
        std::fs::write(tmp.path().join("dev/active-worktree.txt"), "feat-x").unwrap();
        let canonical = PathBuf::from("/install/qol-tray");
        let env = prod_env(canonical.clone(), Some(canonical.clone()));
        let (_target, events) = resolve(&env, tmp.path(), &empty_lister(), &empty_probe());
        assert_eq!(
            events,
            vec![DriftEvent::IgnoredDevMarker {
                branch: "feat-x".to_string()
            }]
        );
    }

    #[test]
    fn prod_marker_present_autostart_drift() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("dev")).unwrap();
        std::fs::write(tmp.path().join("dev/active-worktree.txt"), "feat-x").unwrap();
        let canonical = PathBuf::from("/install/qol-tray");
        let env = prod_env(canonical.clone(), Some(PathBuf::from("/elsewhere")));
        let (_target, events) = resolve(&env, tmp.path(), &empty_lister(), &empty_probe());
        assert!(events
            .iter()
            .any(|e| matches!(e, DriftEvent::IgnoredDevMarker { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, DriftEvent::AutostartTargetDisagrees { .. })));
    }
}

#[cfg(test)]
mod set_tests {
    use super::*;
    use crate::installer::boot_environment::InMemoryBootEnvironment;
    use crate::installer::BootEnvironment;
    use tempfile::TempDir;

    fn binary_in_worktree(wt_path: &str) -> String {
        format!(
            "{}/target/debug/{}",
            wt_path,
            crate::installer::binary_filename()
        )
    }

    #[test]
    fn set_is_idempotent_when_aligned() {
        let tmp = TempDir::new().unwrap();
        let wt = "/wt";
        let binary_path = binary_in_worktree(wt);
        std::fs::create_dir_all(tmp.path().join("dev")).unwrap();
        std::fs::write(tmp.path().join("dev/active-worktree.txt"), "feat-x").unwrap();
        let env = InMemoryBootEnvironment::new(PathBuf::from("/main"), true)
            .with_autostart(PathBuf::from(&binary_path));
        let lister = InMemoryWorktreeLister {
            map: [("feat-x".to_string(), PathBuf::from(wt))]
                .into_iter()
                .collect(),
        };
        let probe = InMemoryBinaryProbe {
            existing: [PathBuf::from(&binary_path)].into_iter().collect(),
        };
        let r = set_selected_worktree(&env, tmp.path(), Some("feat-x"), &lister, &probe).unwrap();
        assert!(!r.cleared_selection);
        assert!(!r.wrote_autostart);
    }

    #[test]
    fn set_writes_autostart_when_drifted() {
        let tmp = TempDir::new().unwrap();
        let env = InMemoryBootEnvironment::new(PathBuf::from("/main"), true)
            .with_autostart(PathBuf::from("/other"));
        let r = set_selected_worktree(
            &env,
            tmp.path(),
            None,
            &InMemoryWorktreeLister {
                map: Default::default(),
            },
            &InMemoryBinaryProbe {
                existing: Default::default(),
            },
        )
        .unwrap();
        assert!(!r.cleared_selection);
        assert!(r.wrote_autostart);
        assert_eq!(
            env.read_autostart_target().unwrap(),
            Some(PathBuf::from("/main"))
        );
    }

    #[test]
    fn set_clears_marker_when_present() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("dev")).unwrap();
        std::fs::write(tmp.path().join("dev/active-worktree.txt"), "feat-x").unwrap();
        let canonical = PathBuf::from("/main");
        let env =
            InMemoryBootEnvironment::new(canonical.clone(), true).with_autostart(canonical.clone());
        let r = set_selected_worktree(
            &env,
            tmp.path(),
            None,
            &InMemoryWorktreeLister {
                map: Default::default(),
            },
            &InMemoryBinaryProbe {
                existing: Default::default(),
            },
        )
        .unwrap();
        assert!(r.cleared_selection);
        assert!(!r.wrote_autostart);
    }

    #[test]
    fn set_prod_ignores_branch_arg() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("dev")).unwrap();
        std::fs::write(tmp.path().join("dev/active-worktree.txt"), "feat-x").unwrap();
        let canonical = PathBuf::from("/install");
        let env = InMemoryBootEnvironment::new(canonical.clone(), false)
            .with_autostart(canonical.clone());
        let r = set_selected_worktree(
            &env,
            tmp.path(),
            Some("ignored"),
            &InMemoryWorktreeLister {
                map: Default::default(),
            },
            &InMemoryBinaryProbe {
                existing: Default::default(),
            },
        )
        .unwrap();
        assert!(r.cleared_selection);
        assert!(!r.wrote_autostart);
    }

    #[test]
    fn set_propagates_writer_error() {
        let tmp = TempDir::new().unwrap();
        let mut env = InMemoryBootEnvironment::new(PathBuf::from("/main"), true);
        env.fail_write = true;
        let result = set_selected_worktree(
            &env,
            tmp.path(),
            None,
            &InMemoryWorktreeLister {
                map: Default::default(),
            },
            &InMemoryBinaryProbe {
                existing: Default::default(),
            },
        );
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use crate::installer::boot_environment::InMemoryBootEnvironment;
    use crate::installer::BootEnvironment;
    use proptest::prelude::*;
    use std::collections::HashSet;
    use tempfile::TempDir;

    fn arb_binary_set() -> impl Strategy<Value = HashSet<PathBuf>> {
        proptest::collection::hash_set("/[a-z]{1,8}/[a-z]{1,8}", 0..5)
            .prop_map(|s| s.into_iter().map(PathBuf::from).collect())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn heal_implies_runnable_autostart(
            marker in proptest::option::of("[a-z]{1,8}"),
            autostart in proptest::option::of("/[a-z]{1,8}"),
            binaries in arb_binary_set(),
            honors_dev in any::<bool>(),
            canonical in "/[a-z]{1,8}/main",
        ) {
            let tmp = TempDir::new().unwrap();
            if let Some(b) = &marker {
                std::fs::create_dir_all(tmp.path().join("dev")).unwrap();
                std::fs::write(tmp.path().join("dev/active-worktree.txt"), b).unwrap();
            }
            let canonical_pb = PathBuf::from(&canonical);
            let mut binaries_with_canonical = binaries.clone();
            binaries_with_canonical.insert(canonical_pb.clone());

            let mut env = InMemoryBootEnvironment::new(canonical_pb.clone(), honors_dev);
            if let Some(a) = autostart {
                env = env.with_autostart(PathBuf::from(a));
            }
            let lister = InMemoryWorktreeLister { map: Default::default() };
            let probe = InMemoryBinaryProbe { existing: binaries_with_canonical.clone() };
            let report = heal_drift_on_startup(&env, tmp.path(), &lister, &probe);

            if report.failures.is_empty() {
                if let Some(after) = env.read_autostart_target().unwrap() {
                    prop_assert!(probe.exists(&after) || after == canonical_pb,
                        "post-heal autostart {} not runnable", after.display());
                }
            }
        }

        #[test]
        fn prod_never_keeps_dev_marker(
            marker in proptest::option::of("[a-z]{1,8}"),
            canonical in "/[a-z]{1,8}/install",
        ) {
            let tmp = TempDir::new().unwrap();
            if let Some(b) = &marker {
                std::fs::create_dir_all(tmp.path().join("dev")).unwrap();
                std::fs::write(tmp.path().join("dev/active-worktree.txt"), b).unwrap();
            }
            let canonical_pb = PathBuf::from(&canonical);
            let env = InMemoryBootEnvironment::new(canonical_pb.clone(), false)
                .with_autostart(canonical_pb.clone());
            let lister = InMemoryWorktreeLister { map: Default::default() };
            let probe = InMemoryBinaryProbe { existing: Default::default() };
            let report = heal_drift_on_startup(&env, tmp.path(), &lister, &probe);

            if report.failures.is_empty() {
                let marker_after = std::fs::read_to_string(tmp.path().join("dev/active-worktree.txt"))
                    .ok()
                    .filter(|s| !s.trim().is_empty());
                prop_assert!(marker_after.is_none(),
                    "prod must not retain dev marker; saw {:?}", marker_after);
            }
        }
    }
}

#[cfg(test)]
mod heal_tests {
    use super::*;
    use crate::installer::boot_environment::InMemoryBootEnvironment;
    use tempfile::TempDir;

    fn empty_lister() -> InMemoryWorktreeLister {
        InMemoryWorktreeLister {
            map: Default::default(),
        }
    }

    fn empty_probe() -> InMemoryBinaryProbe {
        InMemoryBinaryProbe {
            existing: Default::default(),
        }
    }

    fn write_marker(dir: &Path, value: &str) {
        std::fs::create_dir_all(dir.join("dev")).unwrap();
        std::fs::write(dir.join("dev/active-worktree.txt"), value).unwrap();
    }

    #[test]
    fn heal_no_drift_is_noop() {
        let tmp = TempDir::new().unwrap();
        let canonical = PathBuf::from("/main");
        let env =
            InMemoryBootEnvironment::new(canonical.clone(), true).with_autostart(canonical.clone());
        let r = heal_drift_on_startup(&env, tmp.path(), &empty_lister(), &empty_probe());
        assert!(r.events.is_empty());
        assert!(r.actions.is_empty());
        assert!(r.failures.is_empty());
    }

    #[test]
    fn heal_clears_ghost_selection_and_writes_autostart() {
        let tmp = TempDir::new().unwrap();
        write_marker(tmp.path(), "ghost");
        let canonical = PathBuf::from("/main");
        let env = InMemoryBootEnvironment::new(canonical.clone(), true)
            .with_autostart(PathBuf::from("/ghost-binary"));
        let r = heal_drift_on_startup(&env, tmp.path(), &empty_lister(), &empty_probe());
        assert!(r
            .events
            .iter()
            .any(|e| matches!(e, DriftEvent::SelectionMissingFromWorktreeList { .. })));
        assert!(r
            .events
            .iter()
            .any(|e| matches!(e, DriftEvent::AutostartTargetDisagrees { .. })));
        assert!(r
            .actions
            .iter()
            .any(|a| matches!(a, HealAction::ClearedSelection { .. })));
        assert!(r.actions.iter().any(|a| matches!(
            a,
            HealAction::WroteAutostart { binary } if binary == &canonical
        )));
        assert!(r.failures.is_empty());
    }

    #[test]
    fn heal_records_failure_when_writer_errors() {
        let tmp = TempDir::new().unwrap();
        write_marker(tmp.path(), "ghost");
        let mut env = InMemoryBootEnvironment::new(PathBuf::from("/main"), true);
        env.fail_write = true;
        let r = heal_drift_on_startup(&env, tmp.path(), &empty_lister(), &empty_probe());
        assert!(!r.failures.is_empty());
    }

    #[test]
    fn heal_prod_clears_leftover_marker_autostart_aligned() {
        let tmp = TempDir::new().unwrap();
        write_marker(tmp.path(), "feat-x");
        let canonical = PathBuf::from("/install");
        let env = InMemoryBootEnvironment::new(canonical.clone(), false)
            .with_autostart(canonical.clone());
        let r = heal_drift_on_startup(&env, tmp.path(), &empty_lister(), &empty_probe());
        assert!(r
            .events
            .iter()
            .any(|e| matches!(e, DriftEvent::IgnoredDevMarker { .. })));
        assert!(r
            .actions
            .iter()
            .any(|a| matches!(a, HealAction::ClearedSelection { .. })));
        assert!(!r
            .actions
            .iter()
            .any(|a| matches!(a, HealAction::WroteAutostart { .. })));
    }
}
