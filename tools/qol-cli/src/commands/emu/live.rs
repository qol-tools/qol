use anyhow::{anyhow, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct LiveRun {
    pub(crate) run_dir: PathBuf,
    pub(crate) qmp_port: u16,
}

pub(crate) fn find(runs_root: &Path, id: &str) -> Result<LiveRun> {
    let entries = fs::read_dir(runs_root).map_err(|_| no_live_run(id))?;
    let mut best: Option<(u64, LiveRun)> = None;
    for entry in entries.flatten() {
        let run_dir = entry.path();
        let Ok(content) = fs::read_to_string(run_dir.join("report.json")) else {
            continue;
        };
        let Ok(report) = serde_json::from_str::<Value>(&content) else {
            continue;
        };
        let Some((started_at, qmp_port)) = running_report(&report, id) else {
            continue;
        };
        let newer = best
            .as_ref()
            .is_none_or(|(best_started, _)| started_at > *best_started);
        if newer {
            best = Some((started_at, LiveRun { run_dir, qmp_port }));
        }
    }
    best.map(|(_, live)| live).ok_or_else(|| no_live_run(id))
}

fn no_live_run(id: &str) -> anyhow::Error {
    anyhow!("no running emu `{id}`; start one with `qol emu up {id}`")
}

fn running_report(report: &Value, id: &str) -> Option<(u64, u16)> {
    if report.get("environment")?.get("id")?.as_str()? != id {
        return None;
    }
    if report.get("status")?.as_str()? != "running" {
        return None;
    }
    let started_at = report.get("started_at_unix_ms")?.as_u64()?;
    let port = u16::try_from(report.get("qmp")?.get("port")?.as_u64()?).ok()?;
    Some((started_at, port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn running_report_filters_id_status_and_port() {
        let running = json!({
            "environment": {"id": "foo"},
            "status": "running",
            "started_at_unix_ms": 10u64,
            "qmp": {"port": 4444},
        });
        let finished = json!({
            "environment": {"id": "foo"},
            "status": "pass",
            "started_at_unix_ms": 20u64,
            "qmp": {"port": 4445},
        });
        let other_id = json!({
            "environment": {"id": "bar"},
            "status": "running",
            "started_at_unix_ms": 30u64,
            "qmp": {"port": 4446},
        });
        let cases = [
            (&running, Some((10u64, 4444u16))),
            (&finished, None),
            (&other_id, None),
        ];
        for (report, expected) in cases {
            assert_eq!(running_report(report, "foo"), expected, "report: {report}");
        }
    }

    #[test]
    fn find_picks_newest_running_run() {
        let root = std::env::temp_dir().join(format!("qol-emu-live-{}", std::process::id()));
        let write = |dir: &str, report: serde_json::Value| {
            let run_dir = root.join(dir);
            fs::create_dir_all(&run_dir).unwrap();
            fs::write(run_dir.join("report.json"), report.to_string()).unwrap();
        };
        write(
            "foo-10",
            json!({"environment": {"id": "foo"}, "status": "running",
                   "started_at_unix_ms": 10u64, "qmp": {"port": 4444}}),
        );
        write(
            "foo-20",
            json!({"environment": {"id": "foo"}, "status": "running",
                   "started_at_unix_ms": 20u64, "qmp": {"port": 5555}}),
        );
        let live = find(&root, "foo").unwrap();
        assert_eq!(live.qmp_port, 5555);
        assert_eq!(live.run_dir, root.join("foo-20"));
        assert!(find(&root, "bar").is_err(), "bar has no running run");
        fs::remove_dir_all(&root).unwrap();
    }
}
