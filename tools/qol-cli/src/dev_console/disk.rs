use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qol_dev_build::target_cache::{
    format_bytes, path_bytes, prunable_target_bytes, prune_cargo_target_dir, SWEPT_CACHE_CEILING,
};
use ratatui::layout::Rect;
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span};
use ratatui::Frame;

pub(super) use qol_dev_build::scan_ledger::{
    resolve_bytes, resolve_dir_by_children, ScanLedger, SubtreeRecord,
};

use super::activity::Activity;
use super::render_util::{accent, now_unix_ms, relative_age, NavigationOverflow};
use super::{Dash, View};

pub(super) struct DiskPanel {
    pub(super) last: Option<DiskReport>,
    pub(super) last_at_ms: Option<u64>,
    pub(super) scan: Option<DiskScan>,
    pub(super) verify: Option<DiskScan>,
    pub(super) error: Option<String>,
}

impl DiskPanel {
    pub(super) fn new() -> Self {
        Self {
            last: None,
            last_at_ms: None,
            scan: None,
            verify: None,
            error: None,
        }
    }
}

pub(super) struct DiskScan {
    pub(super) rx: Receiver<Result<(DiskReport, ScanLedger), String>>,
    progress: Arc<Mutex<String>>,
    started_at_ms: u64,
    phase: &'static str,
}

impl DiskScan {
    fn progress_step(&self) -> String {
        self.progress
            .lock()
            .map(|step| step.clone())
            .unwrap_or_default()
    }

    pub(super) fn activity(&self, now_ms: u64) -> Activity {
        Activity {
            title: "disk",
            phase: self.phase.to_string(),
            detail: self.progress_step(),
            elapsed: Duration::from_millis(now_ms.saturating_sub(self.started_at_ms)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct DiskReport {
    pub(super) rows: Vec<DiskRow>,
}

impl DiskReport {
    pub(super) fn target_total(&self) -> Option<u64> {
        self.rows
            .iter()
            .find(|row| row.label == TARGET_TOTAL_LABEL)
            .and_then(|row| row.bytes)
    }

    fn cleanable_total(&self) -> u64 {
        self.rows
            .iter()
            .filter(|row| row.cleanable)
            .filter_map(|row| row.bytes)
            .sum()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct DiskRow {
    pub(super) label: String,
    pub(super) detail: String,
    pub(super) bytes: Option<u64>,
    pub(super) cleanable: bool,
}

const TARGET_TOTAL_LABEL: &str = "target";
const WORKTREE_LABEL_PREFIX: &str = "worktree · ";
const PRUNABLE_LABEL: &str = "target · prunable";
const CLEANABLE_ATTENTION: u64 = 10 * 1024 * 1024 * 1024;
const PRUNABLE_LEDGER_KEY: &str = "__qol_dev_disk_prunable__";
const VERIFY_TARGET_MAX_AGE_MS: u64 = 6 * 60 * 60 * 1000;
const VERIFY_LONG_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;
const MIN_VISIBLE_DURATION: Duration = Duration::from_millis(700);

pub(super) fn open_disk(dash: &mut Dash) {
    dash.view = View::Disk;
    dash.scroll_offset = 0;
    if dash.disk.last.is_none() {
        start_disk_scan(dash);
    }
}

pub(super) fn start_disk_scan(dash: &mut Dash) {
    if dash.disk.scan.is_some() {
        return;
    }
    dash.disk.verify = None;
    let running = dash.running_worktree.clone();
    dash.disk.scan = Some(spawn_disk_worker("scanning", move |progress| {
        scan_disk_usage(&running, progress)
    }));
}

pub(super) fn start_disk_cleanup(dash: &mut Dash) {
    if let Some(scan) = &dash.disk.scan {
        if scan.phase == "cleaning" {
            dash.notice = Some((
                Instant::now(),
                format!("disk {} · re-arm once it finishes", scan.phase),
            ));
            return;
        }
    }
    dash.disk.verify = None;
    let running = dash.running_worktree.clone();
    dash.disk.scan = Some(spawn_disk_worker("cleaning", move |progress| {
        cleanup_disk_usage(&running, progress)
    }));
}

pub(super) fn spawn_disk_worker(
    phase: &'static str,
    work: impl FnOnce(&Arc<Mutex<String>>) -> Result<(DiskReport, ScanLedger), String> + Send + 'static,
) -> DiskScan {
    let (tx, rx) = channel();
    let progress = Arc::new(Mutex::new(String::new()));
    let worker_progress = Arc::clone(&progress);
    std::thread::spawn(move || {
        lower_worker_priority();
        let started = Instant::now();
        let outcome = work(&worker_progress);
        let elapsed = started.elapsed();
        if elapsed < MIN_VISIBLE_DURATION {
            std::thread::sleep(MIN_VISIBLE_DURATION - elapsed);
        }
        let _ = tx.send(outcome);
    });
    DiskScan {
        rx,
        progress,
        started_at_ms: now_unix_ms(),
        phase,
    }
}

fn lower_worker_priority() {
    #[cfg(target_os = "macos")]
    {
        const IOPOL_TYPE_DISK: libc::c_int = 0;
        const IOPOL_SCOPE_THREAD: libc::c_int = 1;
        const IOPOL_THROTTLE: libc::c_int = 3;
        unsafe extern "C" {
            fn setiopolicy_np(
                scope: libc::c_int,
                policy: libc::c_int,
                value: libc::c_int,
            ) -> libc::c_int;
        }
        unsafe {
            libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0);
            setiopolicy_np(IOPOL_TYPE_DISK, IOPOL_SCOPE_THREAD, IOPOL_THROTTLE);
        }
    }
}

pub(super) fn apply_disk_outcome(
    dash: &mut Dash,
    outcome: Result<(DiskReport, ScanLedger), String>,
) {
    match outcome {
        Ok((report, ledger)) => {
            let at_ms = now_unix_ms();
            persist_report(&dash.running_worktree, &report, &ledger, at_ms);
            dash.disk.last = Some(report);
            dash.disk.last_at_ms = Some(at_ms);
            dash.disk.error = None;
        }
        Err(error) => dash.disk.error = Some(error),
    }
}

#[derive(serde::Serialize)]
struct CachedDiskReport<'a> {
    scanned_at_ms: u64,
    rows: &'a [DiskRow],
    ledger: &'a ScanLedger,
}

#[derive(serde::Deserialize)]
struct CachedDiskReportOwned {
    scanned_at_ms: u64,
    rows: Vec<DiskRow>,
    #[serde(default)]
    ledger: ScanLedger,
}

fn cache_path(running: &Path) -> PathBuf {
    running
        .join("target")
        .join("qol-dev")
        .join("disk-report.json")
}

pub(super) fn persist_report(running: &Path, report: &DiskReport, ledger: &ScanLedger, at_ms: u64) {
    let Ok(serialized) = serde_json::to_string(&CachedDiskReport {
        scanned_at_ms: at_ms,
        rows: &report.rows,
        ledger,
    }) else {
        return;
    };
    let path = cache_path(running);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, serialized);
}

pub(super) fn load_cached_report(running: &Path) -> Option<(DiskReport, u64, ScanLedger)> {
    let serialized = std::fs::read_to_string(cache_path(running)).ok()?;
    let cached: CachedDiskReportOwned = serde_json::from_str(&serialized).ok()?;
    Some((
        DiskReport { rows: cached.rows },
        cached.scanned_at_ms,
        cached.ledger,
    ))
}

fn set_progress(progress: &Arc<Mutex<String>>, step: &str) {
    if let Ok(mut current) = progress.lock() {
        *current = step.to_string();
    }
}

fn scan_disk_usage(
    running: &Path,
    progress: &Arc<Mutex<String>>,
) -> Result<(DiskReport, ScanLedger), String> {
    let now_ms = now_unix_ms();
    let mut ledger = ScanLedger::new();
    let report = usage_rows(running, &mut ledger, now_ms, 0, 0, progress)?;
    Ok((report, ledger))
}

pub(super) fn verify_disk_usage(
    running: &Path,
    mut ledger: ScanLedger,
    progress: &Arc<Mutex<String>>,
) -> Result<(DiskReport, ScanLedger), String> {
    let now_ms = now_unix_ms();
    let report = usage_rows(
        running,
        &mut ledger,
        now_ms,
        VERIFY_TARGET_MAX_AGE_MS,
        VERIFY_LONG_MAX_AGE_MS,
        progress,
    )?;
    Ok((report, ledger))
}

fn usage_rows(
    running: &Path,
    ledger: &mut ScanLedger,
    now_ms: u64,
    target_max_age_ms: u64,
    long_max_age_ms: u64,
    progress: &Arc<Mutex<String>>,
) -> Result<DiskReport, String> {
    let target = running.join("target");
    let mut rows = Vec::new();
    set_progress(progress, "target roots");
    let (bucket_rows, any_target_rescan) = if target.exists() {
        target_bucket_rows(&target, ledger, now_ms, target_max_age_ms)
    } else {
        (target_root_rows(&target), false)
    };
    rows.extend(bucket_rows);
    set_progress(progress, "stale caches");
    let prunable = if any_target_rescan {
        let value = prunable_target_bytes(&target);
        ledger.records.insert(
            PRUNABLE_LEDGER_KEY.to_string(),
            SubtreeRecord {
                bytes: value,
                sig: None,
                scanned_at_ms: now_ms,
            },
        );
        value
    } else {
        ledger
            .records
            .get(PRUNABLE_LEDGER_KEY)
            .map(|record| record.bytes)
            .unwrap_or_else(|| {
                let value = prunable_target_bytes(&target);
                ledger.records.insert(
                    PRUNABLE_LEDGER_KEY.to_string(),
                    SubtreeRecord {
                        bytes: value,
                        sig: None,
                        scanned_at_ms: now_ms,
                    },
                );
                value
            })
    };
    rows.push(DiskRow {
        label: PRUNABLE_LABEL.to_string(),
        detail: format!(
            "the doctor auto-prunes debug caches to a {} ceiling",
            format_bytes(SWEPT_CACHE_CEILING)
        ),
        bytes: Some(prunable),
        cleanable: true,
    });
    let mut deep = |path: &Path| path_bytes(path);
    for worktree in qol_dev_build::tray::list_worktrees(running) {
        set_progress(progress, &format!("worktree {}", worktree.branch));
        let live = worktree.path == running;
        let target_path = worktree.path.join("target");
        let bytes = target_path
            .exists()
            .then(|| resolve_bytes(&target_path, ledger, now_ms, target_max_age_ms, &mut deep).0);
        rows.push(DiskRow {
            label: format!("{WORKTREE_LABEL_PREFIX}{}", worktree.branch),
            detail: format!(
                "{}{}",
                target_path.display(),
                if live { " · live target" } else { "" }
            ),
            bytes,
            cleanable: !live,
        });
    }
    set_progress(progress, "sccache");
    rows.push(resolved_optional_row(
        "sccache",
        sccache_dir(),
        ledger,
        now_ms,
        long_max_age_ms,
        &mut deep,
    ));
    set_progress(progress, "cargo home");
    rows.push(resolved_optional_row(
        "cargo home",
        cargo_home(),
        ledger,
        now_ms,
        long_max_age_ms,
        &mut deep,
    ));
    Ok(DiskReport { rows })
}

fn cleanup_disk_usage(
    running: &Path,
    progress: &Arc<Mutex<String>>,
) -> Result<(DiskReport, ScanLedger), String> {
    let mut freed = 0;
    let mut failures = Vec::new();
    for worktree in qol_dev_build::tray::list_worktrees(running) {
        if worktree.path == running {
            continue;
        }
        let target = worktree.path.join("target");
        if !target.exists() {
            continue;
        }
        set_progress(progress, &format!("worktree {}", worktree.branch));
        let bytes = path_bytes(&target);
        match std::fs::remove_dir_all(&target) {
            Ok(()) => freed += bytes,
            Err(error) => {
                failures.push(format!(
                    "worktree {}: {} ({error})",
                    worktree.branch,
                    target.display()
                ));
                freed += bytes.saturating_sub(path_bytes(&target));
            }
        }
    }
    set_progress(progress, "stale caches");
    let target = running.join("target");
    if target.exists() {
        let prunable = prunable_target_bytes(&target);
        match prune_cargo_target_dir(&target) {
            Ok(()) => freed += prunable,
            Err(error) => {
                failures.push(format!("stale caches: {error}"));
                freed += prunable.saturating_sub(prunable_target_bytes(&target));
            }
        }
    }
    let (mut report, ledger) = scan_disk_usage(running, progress)?;
    report.rows.insert(0, cleanup_row(freed, &failures));
    Ok((report, ledger))
}

fn cleanup_row(freed: u64, failures: &[String]) -> DiskRow {
    let mut detail = "worktree targets removed · stale caches pruned".to_string();
    if !failures.is_empty() {
        detail.push_str(&format!(" · could not remove: {}", failures.join("; ")));
    }
    DiskRow {
        label: "cleanup · freed".to_string(),
        detail,
        bytes: Some(freed),
        cleanable: false,
    }
}

pub(super) fn target_root_rows(target: &Path) -> Vec<DiskRow> {
    let buckets = [
        ("debug", "live build cache"),
        ("qol-dev", "development builds and runtime generations"),
        ("qol-env", "sandbox guest payloads"),
    ];
    if !target.exists() {
        let mut rows = vec![DiskRow {
            label: TARGET_TOTAL_LABEL.to_string(),
            detail: target.display().to_string(),
            bytes: None,
            cleanable: false,
        }];
        rows.extend(buckets.iter().map(|(name, detail)| DiskRow {
            label: format!("target · {name}"),
            detail: detail.to_string(),
            bytes: None,
            cleanable: false,
        }));
        return rows;
    }
    let mut other = 0;
    if let Ok(entries) = std::fs::read_dir(target) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if buckets
                .iter()
                .any(|(bucket, _)| name.to_string_lossy() == *bucket)
            {
                continue;
            }
            other += path_bytes(&entry.path());
        }
    }
    let mut total = other;
    let mut rows: Vec<DiskRow> = buckets
        .iter()
        .map(|(name, detail)| {
            let bytes = path_bytes(&target.join(name));
            total += bytes;
            DiskRow {
                label: format!("target · {name}"),
                detail: detail.to_string(),
                bytes: Some(bytes),
                cleanable: false,
            }
        })
        .collect();
    rows.push(DiskRow {
        label: "target · other".to_string(),
        detail: "secondary build roots".to_string(),
        bytes: Some(other),
        cleanable: false,
    });
    rows.insert(
        0,
        DiskRow {
            label: TARGET_TOTAL_LABEL.to_string(),
            detail: target.display().to_string(),
            bytes: Some(total),
            cleanable: false,
        },
    );
    rows
}

fn target_bucket_rows(
    target: &Path,
    ledger: &mut ScanLedger,
    now_ms: u64,
    max_age_ms: u64,
) -> (Vec<DiskRow>, bool) {
    let buckets = [
        ("debug", "live build cache"),
        ("qol-dev", "development builds and runtime generations"),
        ("qol-env", "sandbox guest payloads"),
    ];
    let mut deep = |path: &Path| path_bytes(path);
    let mut other = 0;
    let mut any_rescan = false;
    if let Ok(entries) = std::fs::read_dir(target) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if buckets
                .iter()
                .any(|(bucket, _)| name.to_string_lossy() == *bucket)
            {
                continue;
            }
            let (bytes, rescanned) =
                resolve_bytes(&entry.path(), ledger, now_ms, max_age_ms, &mut deep);
            other += bytes;
            any_rescan |= rescanned;
        }
    }
    let mut total = other;
    let mut rows = Vec::new();
    for (name, detail) in buckets {
        let path = target.join(name);
        let (bytes, rescanned) = if name == "debug" {
            resolve_dir_by_children(&path, ledger, now_ms, max_age_ms, &mut deep)
        } else {
            resolve_bytes(&path, ledger, now_ms, max_age_ms, &mut deep)
        };
        any_rescan |= rescanned;
        total += bytes;
        rows.push(DiskRow {
            label: format!("target · {name}"),
            detail: detail.to_string(),
            bytes: Some(bytes),
            cleanable: false,
        });
    }
    rows.push(DiskRow {
        label: "target · other".to_string(),
        detail: "secondary build roots".to_string(),
        bytes: Some(other),
        cleanable: false,
    });
    rows.insert(
        0,
        DiskRow {
            label: TARGET_TOTAL_LABEL.to_string(),
            detail: target.display().to_string(),
            bytes: Some(total),
            cleanable: false,
        },
    );
    (rows, any_rescan)
}

fn resolved_optional_row(
    label: &str,
    path: Option<PathBuf>,
    ledger: &mut ScanLedger,
    now_ms: u64,
    max_age_ms: u64,
    deep: &mut dyn FnMut(&Path) -> u64,
) -> DiskRow {
    let Some(path) = path else {
        return DiskRow {
            label: label.to_string(),
            detail: "location unknown".to_string(),
            bytes: None,
            cleanable: false,
        };
    };
    let bytes = path
        .exists()
        .then(|| resolve_bytes(&path, ledger, now_ms, max_age_ms, deep).0);
    DiskRow {
        label: label.to_string(),
        detail: path.display().to_string(),
        bytes,
        cleanable: false,
    }
}

fn sccache_dir() -> Option<PathBuf> {
    std::env::var_os("SCCACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|cache| cache.join("sccache")))
}

fn cargo_home() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".cargo")))
}

pub(super) fn disk_status(panel: &DiskPanel, now_ms: u64) -> (Color, Vec<Span<'static>>) {
    if let Some(scan) = &panel.scan {
        return (Color::Yellow, vec![scan.phase.fg(Color::Yellow)]);
    }
    if let Some(verify) = &panel.verify {
        return (Color::Yellow, vec![verify.phase.fg(Color::Yellow)]);
    }
    if let Some(error) = &panel.error {
        return (
            Color::Red,
            vec![
                "scan failed".fg(Color::Red).bold(),
                format!(" · {error}").fg(Color::DarkGray),
            ],
        );
    }
    let Some(report) = &panel.last else {
        return (accent(), vec!["enter to scan".fg(Color::DarkGray)]);
    };
    let total = report
        .target_total()
        .map(format_bytes)
        .unwrap_or_else(|| "no target dir".to_string());
    let mut value = vec![format!("{total} target").fg(Color::DarkGray)];
    let cleanable = report.cleanable_total();
    let color = if cleanable >= CLEANABLE_ATTENTION {
        Color::Yellow
    } else {
        accent()
    };
    if cleanable > 0 {
        value.push(format!(" · {} cleanable", format_bytes(cleanable)).fg(color));
    }
    if let Some(at) = panel.last_at_ms {
        value.push(format!(" · {}", relative_age(now_ms, at)).fg(Color::DarkGray));
    }
    (color, value)
}

pub(super) fn draw_disk(frame: &mut Frame, dash: &mut Dash, area: Rect) -> NavigationOverflow {
    let lines = disk_view_lines(&dash.disk);
    super::stream_view::draw_top_anchored(frame, dash, area, lines)
}

pub(super) fn disk_view_lines(panel: &DiskPanel) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(error) = &panel.error {
        lines.push(Line::from(vec![
            "  ✗ ".fg(Color::Red).bold(),
            error.clone().into(),
        ]));
    }
    let Some(report) = &panel.last else {
        if panel.scan.is_none() && lines.is_empty() {
            lines.push(Line::from(
                "  no scan yet · press enter".fg(Color::DarkGray),
            ));
        }
        return lines;
    };
    lines.extend(report.rows.iter().map(disk_row_line));
    lines
}

fn disk_row_line(row: &DiskRow) -> Line<'static> {
    let size = match row.bytes {
        Some(bytes) => format_bytes(bytes),
        None => "-".to_string(),
    };
    Line::from(vec![
        format!("  {:<22}", row.label).fg(Color::White),
        format!("{size:>10}").fg(Color::White).bold(),
        format!("  {}", row.detail).fg(Color::DarkGray),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_console::dash::HealthSnapshot;
    use crate::dev_console::key_bindings::Action;
    use crate::dev_console::session::apply_health;
    use std::collections::BTreeMap;
    use std::time::Duration;

    fn span_text(spans: &[Span<'static>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    fn report(rows: Vec<DiskRow>) -> DiskReport {
        DiskReport { rows }
    }

    fn row(label: &str, bytes: Option<u64>) -> DiskRow {
        DiskRow {
            label: label.to_string(),
            detail: "detail".to_string(),
            bytes,
            cleanable: label.starts_with(WORKTREE_LABEL_PREFIX) || label == PRUNABLE_LABEL,
        }
    }

    #[test]
    fn disk_status_covers_panel_states() {
        let now = 1_000_000_000;
        let scanned = DiskPanel {
            last: Some(report(vec![row("target", Some(3 * 1024 * 1024 * 1024))])),
            last_at_ms: Some(now - 15_000),
            scan: None,
            verify: None,
            error: None,
        };
        let scanning = DiskPanel {
            scan: Some(never_finishing_scan()),
            ..DiskPanel::new()
        };
        let verifying = DiskPanel {
            verify: Some(scan_with_phase("verifying")),
            ..DiskPanel::new()
        };
        let failed = DiskPanel {
            error: Some("boom".to_string()),
            ..DiskPanel::new()
        };
        let cleanable = DiskPanel {
            last: Some(report(vec![
                row("target", Some(3 * 1024 * 1024 * 1024)),
                row("worktree · big", Some(20 * 1024 * 1024 * 1024)),
            ])),
            last_at_ms: Some(now - 15_000),
            scan: None,
            verify: None,
            error: None,
        };
        let cases = [
            (DiskPanel::new(), accent(), "enter to scan"),
            (scanning, Color::Yellow, "scanning"),
            (verifying, Color::Yellow, "verifying"),
            (failed, Color::Red, "scan failed · boom"),
            (scanned, accent(), "3.0 GiB target · 15s ago"),
            (
                cleanable,
                Color::Yellow,
                "3.0 GiB target · 20.0 GiB cleanable · 15s ago",
            ),
        ];
        for (panel, expected_color, expected_text) in cases {
            let (color, spans) = disk_status(&panel, now);
            assert_eq!(color, expected_color, "text: {expected_text}");
            assert_eq!(span_text(&spans), expected_text);
        }
    }

    fn scan_with_phase(phase: &'static str) -> DiskScan {
        let (tx, rx) = channel();
        std::mem::forget(tx);
        DiskScan {
            rx,
            progress: Arc::new(Mutex::new(String::new())),
            started_at_ms: 0,
            phase,
        }
    }

    fn never_finishing_scan() -> DiskScan {
        scan_with_phase("scanning")
    }

    fn progress() -> Arc<Mutex<String>> {
        Arc::new(Mutex::new(String::new()))
    }

    fn write_tree(dir: &Path, bytes: usize) {
        std::fs::create_dir_all(dir).expect("dirs");
        std::fs::write(dir.join("payload.bin"), vec![0; bytes]).expect("file");
    }

    fn git_worktree(root: &Path, branch: &str) -> PathBuf {
        let dir = root.join("worktrees").join("main").join(branch);
        std::fs::create_dir_all(&dir).expect("worktree dir");
        let status = std::process::Command::new("git")
            .args(["init", "-q", "-b", branch])
            .current_dir(&dir)
            .status()
            .expect("git init");
        assert!(status.success(), "git init for {branch}");
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"probe\"\n").expect("manifest");
        let status = std::process::Command::new("git")
            .args(["add", "Cargo.toml"])
            .current_dir(&dir)
            .status()
            .expect("git add");
        assert!(status.success(), "git add for {branch}");
        let status = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@test.local",
                "commit",
                "-q",
                "-m",
                "init",
            ])
            .current_dir(&dir)
            .status()
            .expect("git commit");
        assert!(status.success(), "git commit for {branch}");
        dir
    }

    fn row_for<'a>(report: &'a DiskReport, branch: &str) -> &'a DiskRow {
        let label = format!("{WORKTREE_LABEL_PREFIX}{branch}");
        report
            .rows
            .iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| panic!("missing worktree row for {branch}"))
    }

    #[test]
    fn cleanup_row_reports_freed_bytes_and_failure_reasons() {
        let clean = cleanup_row(2 * 1024 * 1024, &[]);
        assert_eq!(clean.label, "cleanup · freed");
        assert_eq!(clean.bytes, Some(2 * 1024 * 1024));
        assert!(!clean.detail.contains("could not remove"));

        let reasons = vec![
            "worktree feat-x: /t/worktrees/main/feat-x/target (Permission denied)".to_string(),
        ];
        let partial = cleanup_row(0, &reasons);
        assert!(
            partial.detail.contains("Permission denied"),
            "failure reasons must be visible: {}",
            partial.detail
        );
        assert!(
            !partial.detail.contains("could not be removed"),
            "the bare failure count is replaced by reasons"
        );
    }

    #[test]
    fn apply_disk_outcome_persists_a_cache_that_loads_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut dash = Dash::new(Vec::new());
        dash.running_worktree = dir.path().to_path_buf();
        let expected = report(vec![row("target", Some(5 * 1024 * 1024))]);
        let mut ledger = ScanLedger::new();
        ledger.records.insert(
            "seed".to_string(),
            SubtreeRecord {
                bytes: 7,
                sig: None,
                scanned_at_ms: 11,
            },
        );

        apply_disk_outcome(&mut dash, Ok((expected.clone(), ledger)));

        assert_eq!(dash.disk.last, Some(expected.clone()));
        let (cached, at_ms, cached_ledger) =
            load_cached_report(dir.path()).expect("cache file written");
        assert_eq!(cached, expected, "the persisted report must round-trip");
        assert!(at_ms > 0, "the cached timestamp is the scan time");
        assert_eq!(
            dash.disk.last_at_ms,
            Some(at_ms),
            "the panel timestamp matches the persisted one"
        );
        let seeded = cached_ledger
            .records
            .get("seed")
            .expect("ledger round-trips");
        assert_eq!(seeded.bytes, 7);
        assert_eq!(seeded.scanned_at_ms, 11);
    }

    #[test]
    fn armed_enter_replaces_an_in_flight_scan_instead_of_dropping_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut dash = Dash::new(Vec::new());
        dash.running_worktree = dir.path().to_path_buf();
        dash.disk.scan = Some(never_finishing_scan());

        start_disk_cleanup(&mut dash);

        let scan = dash.disk.scan.as_ref().expect("cleanup worker spawned");
        assert_eq!(
            scan.phase, "cleaning",
            "armed enter must start cleanup, not wait on the scan"
        );
        assert!(
            dash.notice.is_none(),
            "replacing the scan must not show the deferral notice"
        );
    }

    #[test]
    fn live_scans_surface_as_the_disk_activity() {
        let mut dash = Dash::new(Vec::new());
        dash.disk.scan = Some(never_finishing_scan());
        let activity = dash.activity().expect("live scan announces");
        assert_eq!(activity.title, "disk");
        assert_eq!(activity.phase, "scanning");
    }

    #[test]
    fn target_root_rows_bucket_protected_roots_and_sum_the_rest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path();
        let files = [
            ("debug/a.bin", 3u64),
            ("qol-dev/runtime/gen", 5),
            ("qol-env/lane/img", 7),
            ("release/lib.rlib", 11),
            ("cargo-timings/t.html", 13),
        ];
        for (rel, len) in files {
            let path = target.join(rel);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
            std::fs::write(&path, vec![0; len as usize]).expect("file");
        }

        let rows = target_root_rows(target);

        let cases = [
            ("target", Some(39)),
            ("target · debug", Some(3)),
            ("target · qol-dev", Some(5)),
            ("target · qol-env", Some(7)),
            ("target · other", Some(24)),
        ];
        assert_eq!(rows.len(), cases.len());
        for (label, bytes) in cases {
            let row = rows
                .iter()
                .find(|row| row.label == label)
                .unwrap_or_else(|| panic!("missing row: {label}"));
            assert_eq!(row.bytes, bytes, "label: {label}");
        }
    }

    #[test]
    fn target_root_rows_report_missing_target_without_sizes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rows = target_root_rows(&dir.path().join("does-not-exist"));
        assert!(rows.iter().all(|row| row.bytes.is_none()));
        assert_eq!(rows[0].label, "target");
    }

    #[test]
    fn disk_scroll_reveals_rows_below_the_viewport() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Disk;
        dash.keys_hidden = true;
        dash.disk.last = Some(report(
            (0..20)
                .map(|index| row(&format!("bucket-{index}"), Some(index * 1024)))
                .collect(),
        ));

        let top_rows = super::super::testkit::render_rows_at(&mut dash, 110, 14);
        assert!(
            top_rows.iter().any(|line| line.contains("bucket-0")),
            "the first rows must render at the top"
        );
        assert!(
            !top_rows.iter().any(|line| line.contains("bucket-19")),
            "rows below the viewport must not render initially"
        );

        for _ in 0..20 {
            super::super::session::apply_action(&mut dash, Action::ScrollDown, false);
        }
        let scrolled_rows = super::super::testkit::render_rows_at(&mut dash, 110, 14);
        assert!(
            scrolled_rows.iter().any(|line| line.contains("bucket-19")),
            "down keys must reveal the last row"
        );
        assert!(
            !scrolled_rows.iter().any(|line| line.contains("bucket-0")),
            "down keys must hide the first row"
        );

        for _ in 0..20 {
            super::super::session::apply_action(&mut dash, Action::ScrollUp, false);
        }
        let top_rows = super::super::testkit::render_rows_at(&mut dash, 110, 14);
        assert!(
            top_rows.iter().any(|line| line.contains("bucket-0")),
            "up keys must return to the first row"
        );
    }

    #[test]
    fn copy_count_entry_reveals_the_tail_it_will_copy() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Disk;
        dash.keys_hidden = true;
        dash.disk.last = Some(report(
            (0..20)
                .map(|index| row(&format!("bucket-{index}"), Some(index * 1024)))
                .collect(),
        ));
        dash.copying = true;
        dash.copy_count = "3".to_string();

        let rows = super::super::testkit::render_rows_at(&mut dash, 110, 14);
        assert!(
            rows.iter().any(|line| line.contains("bucket-19")),
            "entering a copy count must scroll the tail into view"
        );
    }

    #[test]
    fn disk_row_line_aligns_sizes_and_marks_missing_paths() {
        let cases = [
            (row("target", Some(2 * 1024 * 1024)), "2 MiB"),
            (row("sccache", None), "-"),
        ];
        for (input, expected_size) in cases {
            let text = disk_row_line(&input).to_string();
            assert!(
                text.contains(expected_size),
                "line: {text:?} expected: {expected_size}"
            );
            assert!(text.contains(&input.label), "line: {text:?}");
        }
    }

    #[test]
    fn disk_view_lines_show_empty_state_error_and_report() {
        assert_eq!(
            disk_view_lines(&DiskPanel::new())[0].to_string(),
            "  no scan yet · press enter"
        );

        let failed = DiskPanel {
            error: Some("no repo".to_string()),
            ..DiskPanel::new()
        };
        assert!(disk_view_lines(&failed)[0].to_string().contains("no repo"));

        let scanned = DiskPanel {
            last: Some(report(vec![row("target", Some(1024 * 1024))])),
            ..DiskPanel::new()
        };
        let lines = disk_view_lines(&scanned);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].to_string().contains("target"));
    }

    #[test]
    fn armed_enter_while_cleaning_waits_with_a_notice() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut dash = Dash::new(Vec::new());
        dash.running_worktree = dir.path().to_path_buf();
        dash.disk.scan = Some(scan_with_phase("cleaning"));

        start_disk_cleanup(&mut dash);

        assert_eq!(
            dash.disk.scan.as_ref().expect("scan").phase,
            "cleaning",
            "a second cleanup is not stacked onto the running one"
        );
        let notice = dash.notice.as_ref().map(|(_, message)| message.as_str());
        assert!(
            notice.is_some_and(|message| message.contains("re-arm once it finishes")),
            "notice: {notice:?}"
        );
    }

    #[test]
    fn scan_marks_the_running_worktree_target_as_not_cleanable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let running = git_worktree(root, "feat-running");
        let idle = git_worktree(root, "feat-idle");
        write_tree(&running.join("target"), 4);
        write_tree(&idle.join("target"), 8);

        let (report, ledger) = scan_disk_usage(&running, &progress()).expect("scan");

        assert!(!ledger.records.is_empty(), "a full scan seeds the ledger");
        let running_row = row_for(&report, "feat-running");
        assert!(
            !running_row.cleanable,
            "the live worktree target must not advertise as cleanable"
        );
        assert!(running_row.detail.contains("live target"));
        let idle_row = row_for(&report, "feat-idle");
        assert!(idle_row.cleanable, "idle worktree targets stay cleanable");
        assert_eq!(
            report.cleanable_total(),
            12,
            "cleanable counts idle worktrees plus the live target's prunable roots"
        );
    }

    #[test]
    fn cleanup_removes_idle_worktree_targets_prunes_the_main_target_and_rescans() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let running = git_worktree(root, "feat-running");
        let idle = git_worktree(root, "feat-idle");
        write_tree(&running.join("target"), 4);
        write_tree(&idle.join("target"), 8);

        let (report, _) = cleanup_disk_usage(&running, &progress()).expect("cleanup");

        assert!(
            running.join("target").exists(),
            "the running worktree target must survive the cleanup"
        );
        assert!(
            !idle.join("target").exists(),
            "idle worktree targets are removed"
        );
        assert!(
            !running.join("target").join("payload.bin").exists(),
            "prunable main-target roots are removed"
        );
        assert_eq!(
            report.cleanable_total(),
            0,
            "the post-clean rescan must show a reduced cleanable figure"
        );
        let freed = report.rows[0]
            .bytes
            .expect("cleanup row reports freed bytes");
        assert_eq!(freed, 12, "freed counts idle targets plus pruned roots");
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_reports_prune_failure_reasons_and_partial_freed_bytes() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let running = git_worktree(root, "feat-running");
        let release = running.join("target").join("release");
        write_tree(&release, 16);
        std::fs::write(release.join("held.rlib"), b"x").expect("held file");
        std::fs::set_permissions(&release, std::fs::Permissions::from_mode(0o555))
            .expect("lock perms");
        write_tree(&running.join("target").join("sandbox"), 8);

        let (report, _) = cleanup_disk_usage(&running, &progress()).expect("cleanup");

        std::fs::set_permissions(&release, std::fs::Permissions::from_mode(0o755))
            .expect("unlock perms");
        assert!(
            !running.join("target").join("sandbox").exists(),
            "unlocked entries still prune past the locked one"
        );
        let row = &report.rows[0];
        assert!(
            row.detail.contains("release") && row.detail.contains("Permission denied"),
            "the failure reason must be visible: {}",
            row.detail
        );
        assert_eq!(
            row.bytes,
            Some(8),
            "freed counts only what was actually removed"
        );
    }

    #[test]
    fn verify_skips_deep_walks_on_an_unchanged_tree() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let running = git_worktree(root, "feat-running");
        write_tree(&running.join("target/debug"), 4);
        write_tree(&running.join("target/qol-dev"), 8);

        let (seed_report, seed_ledger) = scan_disk_usage(&running, &progress()).expect("seed");
        assert!(
            !seed_ledger.records.is_empty(),
            "the seed scan records every root"
        );
        let before: BTreeMap<String, u64> = seed_ledger
            .records
            .iter()
            .map(|(key, record)| (key.clone(), record.scanned_at_ms))
            .collect();

        let (verify_report, verify_ledger) =
            verify_disk_usage(&running, seed_ledger, &progress()).expect("verify");

        assert_eq!(
            verify_report.rows, seed_report.rows,
            "an unchanged tree verifies to the same rows"
        );
        for (key, record) in &verify_ledger.records {
            assert_eq!(
                *before.get(key).expect("verify adds no new records"),
                record.scanned_at_ms,
                "unchanged bucket {key} must not rescan"
            );
        }
    }

    #[test]
    fn verify_rescans_only_the_changed_bucket() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let running = git_worktree(root, "feat-running");
        write_tree(&running.join("target/debug"), 4);
        write_tree(&running.join("target/qol-dev"), 8);
        let (seed_report, seed_ledger) = scan_disk_usage(&running, &progress()).expect("seed");
        let before: BTreeMap<String, (u64, u64)> = seed_ledger
            .records
            .iter()
            .map(|(key, record)| (key.clone(), (record.bytes, record.scanned_at_ms)))
            .collect();

        write_tree(&running.join("target/debug/extra"), 16);

        let (verify_report, verify_ledger) =
            verify_disk_usage(&running, seed_ledger, &progress()).expect("verify");

        let mut rescanned = 0;
        for (key, record) in &verify_ledger.records {
            if key.as_str() == PRUNABLE_LEDGER_KEY {
                continue;
            }
            match before.get(key) {
                Some((seed_bytes, seed_at)) if seed_bytes == &record.bytes => {
                    assert_eq!(
                        *seed_at, record.scanned_at_ms,
                        "unchanged bucket {key} must not rescan"
                    );
                }
                _ => rescanned += 1,
            }
        }
        assert!(rescanned >= 1, "the added child must rescan");
        let bytes_of = |label: &str| {
            verify_report
                .rows
                .iter()
                .find(|row| row.label == label)
                .and_then(|row| row.bytes)
        };
        let seed_bytes_of = |label: &str| {
            seed_report
                .rows
                .iter()
                .find(|row| row.label == label)
                .and_then(|row| row.bytes)
        };
        assert_eq!(bytes_of("target · debug"), Some(4 + 16));
        assert_eq!(bytes_of("target"), Some(4 + 8 + 16));
        assert_eq!(
            bytes_of("target · qol-dev"),
            seed_bytes_of("target · qol-dev"),
            "the untouched bucket keeps its cached figure"
        );
    }

    #[test]
    fn manual_scan_drops_an_in_flight_verify_without_waiting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut dash = Dash::new(Vec::new());
        dash.running_worktree = dir.path().to_path_buf();
        let (tx, rx) = channel();
        dash.disk.verify = Some(DiskScan {
            rx,
            progress: Arc::new(Mutex::new(String::new())),
            started_at_ms: 0,
            phase: "verifying",
        });

        start_disk_scan(&mut dash);

        assert!(
            dash.disk.scan.is_some(),
            "a manual scan starts while a verify runs"
        );
        assert!(
            dash.disk.verify.is_none(),
            "the verify slot is cleared so its late result cannot apply"
        );
        assert!(
            tx.send(Ok((report(vec![]), ScanLedger::new()))).is_err(),
            "the dropped receiver rejects the late verify result"
        );
    }

    #[test]
    fn armed_cleanup_drops_an_in_flight_verify() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut dash = Dash::new(Vec::new());
        dash.running_worktree = dir.path().to_path_buf();
        dash.disk.verify = Some(scan_with_phase("verifying"));

        start_disk_cleanup(&mut dash);

        let scan = dash.disk.scan.as_ref().expect("cleanup worker spawned");
        assert_eq!(scan.phase, "cleaning");
        assert!(dash.disk.verify.is_none(), "cleanup replaces the verify");
    }

    #[test]
    fn cached_boot_starts_a_verify_that_refreshes_and_persists_the_cache() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let running = git_worktree(root, "feat-running");
        write_tree(&running.join("target/debug"), 4);
        let (report, ledger) = scan_disk_usage(&running, &progress()).expect("seed scan");
        let at_ms = 987_654;
        persist_report(&running, &report, &ledger, at_ms);

        let mut dash = Dash::new(Vec::new());
        dash.running_worktree = running.clone();
        dash.disk_scan_pending = true;
        apply_health(
            &mut dash,
            HealthSnapshot {
                api: true,
                web: true,
            },
        );

        assert!(
            dash.disk.scan.is_none(),
            "a cached report must not spawn a full scan"
        );
        let verify = dash
            .disk
            .verify
            .as_ref()
            .expect("cached boot spawns a verify");
        assert_eq!(verify.phase, "verifying");
        assert_eq!(
            dash.disk.last_at_ms,
            Some(at_ms),
            "cached figures show immediately"
        );

        let outcome = verify
            .rx
            .recv_timeout(Duration::from_secs(10))
            .expect("verify completes");
        let cache_len = std::fs::metadata(running.join("target/qol-dev/disk-report.json"))
            .expect("cache file")
            .len();
        dash.disk.verify = None;
        apply_disk_outcome(&mut dash, outcome);

        let (cached, verified_at, cached_ledger) =
            load_cached_report(&running).expect("cache rewritten");
        assert!(verified_at > at_ms, "the verify refreshes the timestamp");
        let bytes_of = |rows: &[DiskRow], label: &str| {
            rows.iter()
                .find(|row| row.label == label)
                .and_then(|row| row.bytes)
        };
        assert_eq!(
            bytes_of(&cached.rows, "target · debug"),
            Some(4),
            "the untouched bucket keeps its cached figure"
        );
        assert_eq!(
            bytes_of(&cached.rows, "target · qol-dev"),
            Some(cache_len),
            "the verify measures the cache file written into qol-dev"
        );
        assert_eq!(bytes_of(&cached.rows, "target"), Some(4 + cache_len));
        assert_eq!(
            bytes_of(&cached.rows, "worktree · feat-running"),
            Some(4 + cache_len)
        );
        assert!(bytes_of(&cached.rows, "target · prunable") == Some(0));
        assert!(
            !cached_ledger.records.is_empty(),
            "the persisted cache carries the ledger"
        );
        assert_eq!(dash.disk.last, Some(cached));
        assert_eq!(dash.disk.last_at_ms, Some(verified_at));
    }

    #[test]
    fn old_cache_without_a_ledger_field_still_loads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = dir.path().join("target/qol-dev/disk-report.json");
        std::fs::create_dir_all(cache.parent().expect("parent")).expect("dirs");
        std::fs::write(
            &cache,
            r#"{"scanned_at_ms":123,"rows":[{"label":"target","detail":"c","bytes":5,"cleanable":false}]}"#,
        )
        .expect("cache file");

        let (report, at_ms, ledger) = load_cached_report(dir.path()).expect("old cache loads");

        assert_eq!(at_ms, 123);
        assert!(ledger.records.is_empty(), "the ledger defaults to empty");
        assert_eq!(report.target_total(), Some(5));
    }
}
