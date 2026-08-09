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

use super::activity::Activity;
use super::log_pane::clamp_offset;
use super::render_util::{
    accent, list_capacity, now_unix_ms, relative_age, view_content, NavigationOverflow,
};
use super::{Dash, View};

pub(super) struct DiskPanel {
    pub(super) last: Option<DiskReport>,
    pub(super) last_at_ms: Option<u64>,
    pub(super) scan: Option<DiskScan>,
    pub(super) error: Option<String>,
}

impl DiskPanel {
    pub(super) fn new() -> Self {
        Self {
            last: None,
            last_at_ms: None,
            scan: None,
            error: None,
        }
    }
}

pub(super) struct DiskScan {
    pub(super) rx: Receiver<Result<DiskReport, String>>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
            .filter(|row| {
                row.label.starts_with(WORKTREE_LABEL_PREFIX) || row.label == PRUNABLE_LABEL
            })
            .filter_map(|row| row.bytes)
            .sum()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DiskRow {
    pub(super) label: String,
    pub(super) detail: String,
    pub(super) bytes: Option<u64>,
}

const TARGET_TOTAL_LABEL: &str = "target";
const WORKTREE_LABEL_PREFIX: &str = "worktree · ";
const PRUNABLE_LABEL: &str = "target · prunable";
const CLEANABLE_ATTENTION: u64 = 10 * 1024 * 1024 * 1024;

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
    dash.disk.scan = Some(spawn_disk_scan());
}

pub(super) fn start_disk_cleanup(dash: &mut Dash) {
    if let Some(scan) = &dash.disk.scan {
        dash.notice = Some((
            Instant::now(),
            format!("disk {} · re-arm once it finishes", scan.phase),
        ));
        return;
    }
    let keep = dash.running_worktree.clone();
    dash.disk.scan = Some(spawn_disk_worker("cleaning", move |progress| {
        cleanup_disk_usage(progress, &keep)
    }));
}

fn spawn_disk_scan() -> DiskScan {
    spawn_disk_worker("scanning", scan_disk_usage)
}

fn spawn_disk_worker(
    phase: &'static str,
    work: impl FnOnce(&Arc<Mutex<String>>) -> Result<DiskReport, String> + Send + 'static,
) -> DiskScan {
    let (tx, rx) = channel();
    let progress = Arc::new(Mutex::new(String::new()));
    let worker_progress = Arc::clone(&progress);
    std::thread::spawn(move || {
        let _ = tx.send(work(&worker_progress));
    });
    DiskScan {
        rx,
        progress,
        started_at_ms: now_unix_ms(),
        phase,
    }
}

pub(super) fn apply_disk_outcome(dash: &mut Dash, outcome: Result<DiskReport, String>) {
    match outcome {
        Ok(report) => {
            dash.disk.last = Some(report);
            dash.disk.last_at_ms = Some(now_unix_ms());
            dash.disk.error = None;
        }
        Err(error) => dash.disk.error = Some(error),
    }
}

fn set_progress(progress: &Arc<Mutex<String>>, step: &str) {
    if let Ok(mut current) = progress.lock() {
        *current = step.to_string();
    }
}

fn scan_disk_usage(progress: &Arc<Mutex<String>>) -> Result<DiskReport, String> {
    let root = crate::workspace::repo_root().map_err(|error| format!("{error:#}"))?;
    let target = root.join("target");
    let mut rows = Vec::new();
    set_progress(progress, "target roots");
    rows.extend(target_root_rows(&target));
    set_progress(progress, "stale caches");
    rows.push(DiskRow {
        label: PRUNABLE_LABEL.to_string(),
        detail: format!(
            "the doctor auto-prunes debug caches to a {} ceiling",
            format_bytes(SWEPT_CACHE_CEILING)
        ),
        bytes: Some(prunable_target_bytes(&target)),
    });
    for worktree in qol_dev_build::tray::list_worktrees(&root) {
        set_progress(progress, &format!("worktree {}", worktree.branch));
        rows.push(measured_row(
            format!("{WORKTREE_LABEL_PREFIX}{}", worktree.branch),
            &worktree.path.join("target"),
        ));
    }
    set_progress(progress, "sccache");
    rows.push(optional_dir_row("sccache", sccache_dir()));
    set_progress(progress, "cargo home");
    rows.push(optional_dir_row("cargo home", cargo_home()));
    Ok(DiskReport { rows })
}

fn cleanup_disk_usage(progress: &Arc<Mutex<String>>, keep: &Path) -> Result<DiskReport, String> {
    let root = crate::workspace::repo_root().map_err(|error| format!("{error:#}"))?;
    let mut freed = 0;
    let mut failed = 0;
    for worktree in qol_dev_build::tray::list_worktrees(&root) {
        if worktree.path.starts_with(keep) || keep.starts_with(&worktree.path) {
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
            Err(_) => failed += 1,
        }
    }
    set_progress(progress, "stale caches");
    let target = root.join("target");
    let prunable = prunable_target_bytes(&target);
    match prune_cargo_target_dir(&target) {
        Ok(()) => freed += prunable,
        Err(_) => failed += 1,
    }
    let mut report = scan_disk_usage(progress)?;
    report.rows.insert(0, cleanup_row(freed, failed));
    Ok(report)
}

fn cleanup_row(freed: u64, failed: usize) -> DiskRow {
    let mut detail = "worktree targets removed · stale caches pruned".to_string();
    if failed > 0 {
        detail.push_str(&format!(" · {failed} could not be removed"));
    }
    DiskRow {
        label: "cleanup · freed".to_string(),
        detail,
        bytes: Some(freed),
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
        }];
        rows.extend(buckets.iter().map(|(name, detail)| DiskRow {
            label: format!("target · {name}"),
            detail: detail.to_string(),
            bytes: None,
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
            }
        })
        .collect();
    rows.push(DiskRow {
        label: "target · other".to_string(),
        detail: "secondary build roots".to_string(),
        bytes: Some(other),
    });
    rows.insert(
        0,
        DiskRow {
            label: TARGET_TOTAL_LABEL.to_string(),
            detail: target.display().to_string(),
            bytes: Some(total),
        },
    );
    rows
}

fn measured_row(label: String, path: &Path) -> DiskRow {
    let bytes = path.exists().then(|| path_bytes(path));
    DiskRow {
        label,
        detail: path.display().to_string(),
        bytes,
    }
}

fn optional_dir_row(label: &str, path: Option<PathBuf>) -> DiskRow {
    let Some(path) = path else {
        return DiskRow {
            label: label.to_string(),
            detail: "location unknown".to_string(),
            bytes: None,
        };
    };
    measured_row(label.to_string(), &path)
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
    let total = lines.len();
    let height = list_capacity(area.height);
    dash.log_height = height;
    dash.scroll_offset = clamp_offset(total, height, dash.scroll_offset);
    let start = dash.scroll_offset;
    view_content(
        frame,
        area,
        lines.into_iter().skip(start).take(height).collect(),
    );
    NavigationOverflow::from_window(start, height, total)
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
    use crate::dev_console::key_bindings::Action;

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
        }
    }

    #[test]
    fn disk_status_covers_panel_states() {
        let now = 1_000_000_000;
        let scanned = DiskPanel {
            last: Some(report(vec![row("target", Some(3 * 1024 * 1024 * 1024))])),
            last_at_ms: Some(now - 15_000),
            scan: None,
            error: None,
        };
        let scanning = DiskPanel {
            scan: Some(never_finishing_scan()),
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
            error: None,
        };
        let cases = [
            (DiskPanel::new(), accent(), "enter to scan"),
            (scanning, Color::Yellow, "scanning"),
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

    fn never_finishing_scan() -> DiskScan {
        let (tx, rx) = channel();
        std::mem::forget(tx);
        DiskScan {
            rx,
            progress: Arc::new(Mutex::new(String::new())),
            started_at_ms: 0,
            phase: "scanning",
        }
    }

    #[test]
    fn cleanup_row_reports_freed_bytes_and_failures() {
        let clean = cleanup_row(2 * 1024 * 1024, 0);
        assert_eq!(clean.label, "cleanup · freed");
        assert_eq!(clean.bytes, Some(2 * 1024 * 1024));
        assert!(!clean.detail.contains("could not be removed"));

        let partial = cleanup_row(0, 2);
        assert!(partial.detail.contains("2 could not be removed"));
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
}
