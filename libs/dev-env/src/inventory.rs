use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::registry::ResolvedEnvironment;
use crate::report::{read_report, ReportKind, RunConcern, RunSummary};

#[derive(Clone, Debug)]
pub struct EnvironmentSnapshot {
    pub resolved: ResolvedEnvironment,
    pub runs: Vec<RunSummary>,
}

impl EnvironmentSnapshot {
    pub fn latest_run(&self) -> Option<&RunSummary> {
        self.runs.first()
    }

    pub fn latest_session(&self) -> Option<&RunSummary> {
        self.session_runs().next()
    }

    pub fn session_runs(&self) -> impl Iterator<Item = &RunSummary> {
        self.runs.iter().filter(|run| run.kind.is_session())
    }

    pub fn lane_runs(&self) -> impl Iterator<Item = &RunSummary> {
        self.runs.iter().filter(|run| run.kind.is_lane())
    }

    pub fn live_runs(&self) -> impl Iterator<Item = &RunSummary> {
        self.lane_runs().filter(|run| run.status.is_active())
    }

    pub fn attention_runs(&self) -> impl Iterator<Item = (&RunSummary, RunConcern)> {
        self.runs.iter().filter_map(|run| {
            run.concern()
                .filter(|concern| concern.requires_attention())
                .map(|concern| (run, concern))
        })
    }

    pub fn live_lane_count(&self) -> u64 {
        self.live_runs().map(|_| 1_u64).sum()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InventoryIssue {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Inventory {
    pub environments: Vec<EnvironmentSnapshot>,
    pub flows: Vec<RunSummary>,
    pub unassigned_runs: Vec<RunSummary>,
    pub issues: Vec<InventoryIssue>,
}

pub fn scan_inventory(environments: &[ResolvedEnvironment]) -> Inventory {
    let mut report_paths = BTreeSet::new();
    let mut issues = Vec::new();
    for run_root in environments
        .iter()
        .filter_map(|environment| environment.run_root.as_deref())
        .collect::<BTreeSet<_>>()
    {
        collect_reports(run_root, &mut report_paths, &mut issues);
    }
    let mut by_environment = BTreeMap::<String, Vec<RunSummary>>::new();
    let mut flows = Vec::new();
    let mut unassigned_runs = Vec::new();
    for path in report_paths {
        let report = match read_report(&path) {
            Ok(Some(report)) => report,
            Ok(None) => continue,
            Err(error) => {
                issues.push(InventoryIssue {
                    path,
                    message: format!("{error:#}"),
                });
                continue;
            }
        };
        let summary = report.summary();
        if matches!(summary.kind, ReportKind::FlowFanout) {
            flows.push(summary.clone());
        }
        let Some(environment_id) = summary.environment_id.clone() else {
            unassigned_runs.push(summary);
            continue;
        };
        by_environment
            .entry(environment_id)
            .or_default()
            .push(summary);
    }
    let mut snapshots = environments
        .iter()
        .cloned()
        .map(|resolved| {
            let mut runs = by_environment
                .remove(&resolved.definition.id)
                .unwrap_or_default();
            sort_runs(&mut runs);
            EnvironmentSnapshot { resolved, runs }
        })
        .collect::<Vec<_>>();
    for runs in by_environment.into_values() {
        unassigned_runs.extend(runs);
    }
    snapshots.sort_by(|left, right| {
        left.resolved
            .definition
            .id
            .cmp(&right.resolved.definition.id)
    });
    sort_runs(&mut flows);
    sort_runs(&mut unassigned_runs);
    issues.sort_by(|left, right| left.path.cmp(&right.path));
    Inventory {
        environments: snapshots,
        flows,
        unassigned_runs,
        issues,
    }
}

fn collect_reports(
    run_root: &Path,
    report_paths: &mut BTreeSet<PathBuf>,
    issues: &mut Vec<InventoryIssue>,
) {
    collect_child_reports(&run_root.join("cases"), report_paths, issues);
    collect_child_reports(&run_root.join("flows"), report_paths, issues);
    let entries = match fs::read_dir(run_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            issues.push(InventoryIssue {
                path: run_root.to_path_buf(),
                message: format!("failed to read run root: {error}"),
            });
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                issues.push(InventoryIssue {
                    path: run_root.to_path_buf(),
                    message: format!("failed to read run entry: {error}"),
                });
                continue;
            }
        };
        if matches!(entry.file_name().to_str(), Some("cases" | "flows")) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                issues.push(InventoryIssue {
                    path: entry.path(),
                    message: format!("failed to inspect run entry: {error}"),
                });
                continue;
            }
        };
        if file_type.is_dir() {
            report_paths.insert(entry.path().join("report.json"));
        }
    }
}

fn collect_child_reports(
    root: &Path,
    report_paths: &mut BTreeSet<PathBuf>,
    issues: &mut Vec<InventoryIssue>,
) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            issues.push(InventoryIssue {
                path: root.to_path_buf(),
                message: format!("failed to read run collection: {error}"),
            });
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                issues.push(InventoryIssue {
                    path: root.to_path_buf(),
                    message: format!("failed to read run entry: {error}"),
                });
                continue;
            }
        };
        match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => {
                report_paths.insert(entry.path().join("report.json"));
            }
            Ok(_) => {}
            Err(error) => issues.push(InventoryIssue {
                path: entry.path(),
                message: format!("failed to inspect run entry: {error}"),
            }),
        }
    }
}

fn sort_runs(runs: &mut [RunSummary]) {
    runs.sort_by(|left, right| {
        right
            .observed_at_unix_ms()
            .cmp(&left.observed_at_unix_ms())
            .then_with(|| left.run_id.cmp(&right.run_id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{
        BootDefinition, EnvironmentDefinition, ImageDefinition, MountDefinition, ResolutionState,
    };
    use serde_json::{json, Value};
    use tempfile::tempdir;

    fn environment(id: &str, run_root: &Path) -> ResolvedEnvironment {
        ResolvedEnvironment {
            definition: EnvironmentDefinition {
                id: id.to_string(),
                name: id.to_string(),
                family: "linux".to_string(),
                backend: "qemu".to_string(),
                image: ImageDefinition {
                    kind: "qcow2".to_string(),
                    base: PathBuf::from("base.qcow2"),
                    recommended_size_gb: 1,
                    arch: Some("x86_64".to_string()),
                    firmware: Some("bios".to_string()),
                },
                boot: BootDefinition {
                    memory_mb: 512,
                    cpus: 1,
                    display: "none".to_string(),
                },
                mounts: MountDefinition { workspace: false },
                capabilities: BTreeMap::new(),
                source: PathBuf::from("definition.toml"),
            },
            state: ResolutionState::Ready,
            image_path: Some(PathBuf::from("base.qcow2")),
            verified_image: None,
            run_root: Some(run_root.to_path_buf()),
            messages: Vec::new(),
        }
    }

    fn write_report(path: &Path, value: Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }

    #[test]
    fn scans_only_canonical_run_locations_and_orders_newest_first() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let id = "linux/mint";
        write_report(
            &root.join("batch-old/report.json"),
            json!({
                "kind": "environment-batch",
                "run_id": "old",
                "status": "stopped",
                "environment": { "id": id },
                "started_at_unix_ms": 10,
                "finished_at_unix_ms": 20,
                "teardown": { "status": "complete" },
                "runs": []
            }),
        );
        write_report(
            &root.join("flows/new/report.json"),
            json!({
                "kind": "flow-fanout",
                "run_id": "new",
                "status": "running",
                "environment": { "id": id },
                "started_at_unix_ms": 30,
                "workflow": { "repeat": 2 },
                "lanes": []
            }),
        );
        write_report(
            &root.join("nested/ignored/report.json"),
            json!({
                "kind": "environment",
                "run_id": "ignored",
                "status": "running",
                "environment": { "id": id }
            }),
        );
        let inventory = scan_inventory(&[environment(id, root)]);
        let snapshot = &inventory.environments[0];
        assert_eq!(
            snapshot
                .runs
                .iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            vec!["new", "old"]
        );
        assert_eq!(
            snapshot
                .session_runs()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            vec!["new", "old"]
        );
        assert!(snapshot.lane_runs().next().is_none());
        assert_eq!(snapshot.live_lane_count(), 0);
        assert!(inventory.issues.is_empty());
    }

    #[test]
    fn active_environment_batch_and_child_count_as_one_live_lane() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let id = "linux/foo";
        write_report(
            &root.join("batch/report.json"),
            json!({
                "kind": "environment-batch",
                "run_id": "batch",
                "status": "running",
                "environment": { "id": id },
                "started_at_unix_ms": 10,
                "resources": { "requested_lanes": 1 },
                "runs": [{ "run_id": "lane" }]
            }),
        );
        write_report(
            &root.join("cases/lane/report.json"),
            json!({
                "kind": "environment",
                "run_id": "lane",
                "status": "running",
                "environment": { "id": id },
                "started_at_unix_ms": 20
            }),
        );

        let inventory = scan_inventory(&[environment(id, root)]);
        let snapshot = &inventory.environments[0];

        assert_eq!(
            snapshot.latest_session().map(|run| run.run_id.as_str()),
            Some("batch")
        );
        assert_eq!(
            snapshot
                .live_runs()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            vec!["lane"]
        );
        assert_eq!(snapshot.live_lane_count(), 1);
    }

    #[test]
    fn active_flow_fanout_counts_only_active_child_lanes() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let id = "linux/foo";
        write_report(
            &root.join("flows/fanout/report.json"),
            json!({
                "kind": "flow-fanout",
                "run_id": "fanout",
                "status": "running",
                "environment": { "id": id },
                "started_at_unix_ms": 10,
                "workflow": { "repeat": 3 },
                "lanes": []
            }),
        );
        for (run_id, status, started_at) in [
            ("lane-a", "running", 20),
            ("lane-b", "stopping", 30),
            ("lane-c", "pass", 40),
        ] {
            write_report(
                &root.join(format!("cases/{run_id}/report.json")),
                json!({
                    "kind": "flow",
                    "run_id": run_id,
                    "status": status,
                    "environment": { "id": id },
                    "started_at_unix_ms": started_at,
                    "teardown": {}
                }),
            );
        }

        let inventory = scan_inventory(&[environment(id, root)]);
        let snapshot = &inventory.environments[0];

        assert_eq!(
            snapshot
                .live_runs()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            vec!["lane-b", "lane-a"]
        );
        assert_eq!(snapshot.live_lane_count(), 2);
    }

    #[test]
    fn malformed_reports_become_visible_issues_without_hiding_other_runs() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("bad")).unwrap();
        fs::write(root.join("bad/report.json"), b"not json").unwrap();
        write_report(
            &root.join("good/report.json"),
            json!({
                "kind": "environment-batch",
                "run_id": "good",
                "status": "running",
                "environment": { "id": "linux/mint" }
            }),
        );
        let inventory = scan_inventory(&[environment("linux/mint", root)]);
        assert_eq!(inventory.environments[0].runs.len(), 1);
        assert_eq!(inventory.environments[0].runs[0].run_id, "good");
        assert_eq!(inventory.issues.len(), 1);
        assert!(inventory.issues[0].message.contains("failed to parse"));
    }

    #[test]
    fn attention_contains_unresolved_cleanup_not_terminal_failure_history() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let id = "linux/mint";
        write_report(
            &root.join("failed/report.json"),
            json!({
                "kind": "environment-batch",
                "run_id": "failed-clean",
                "status": "failed",
                "environment": { "id": id },
                "started_at_unix_ms": 10,
                "finished_at_unix_ms": 20,
                "launch": { "count": 1 },
                "runs": [{ "run_id": "lane-a" }],
                "teardown": {
                    "status": "complete",
                    "lanes": [{
                        "run_id": "lane-a",
                        "status": "pass",
                        "verification": "verified-cleanup",
                        "report_status": "stopped",
                        "stop_error": null
                    }]
                }
            }),
        );
        write_report(
            &root.join("stopped/report.json"),
            json!({
                "kind": "environment-batch",
                "run_id": "stopped-dirty",
                "status": "stopped",
                "environment": { "id": id },
                "started_at_unix_ms": 30,
                "finished_at_unix_ms": 40
            }),
        );

        let inventory = scan_inventory(&[environment(id, root)]);
        let snapshot = &inventory.environments[0];
        assert_eq!(
            snapshot
                .attention_runs()
                .map(|(run, concern)| (run.run_id.as_str(), concern))
                .collect::<Vec<_>>(),
            vec![("stopped-dirty", RunConcern::UnresolvedCleanup)]
        );
        assert_eq!(
            snapshot.runs[1].concern(),
            Some(RunConcern::HistoricalFailure)
        );
    }

    #[test]
    fn ignores_non_directory_lock_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("admission.lock"), b"").unwrap();
        let inventory = scan_inventory(&[environment("linux/mint", dir.path())]);
        assert!(inventory.environments[0].runs.is_empty());
        assert!(inventory.issues.is_empty());
    }
}
