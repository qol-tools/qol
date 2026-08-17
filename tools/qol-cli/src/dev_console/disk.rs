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
use super::render_util::{accent, now_unix_ms, relative_age, NavigationOverflow};
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
            .filter(|row| row.cleanable)
            .filter_map(|row| row.bytes)
            .sum()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    let running = dash.running_worktree.clone();
    dash.disk.scan = Some(spawn_disk_worker("cleaning", move |progress| {
        cleanup_disk_usage(&running, progress)
    }));
}

fn spawn_disk_worker(
    phase: &'static str,
    work: impl FnOnce(&Arc<Mutex<String>>) -> Result<DiskReport, String> + Send + 'static,
) -> DiskScan {
    let (tx, rx) = channel();
    let progress = Arc::new(Mutex::new(String::new()));
    let worker_progress = Arc::clone(&progress);
    std::thread::spawn(move || {
        lower_worker_priority();
        let _ = tx.send(work(&worker_progress));
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

fn scan_disk_usage(running: &Path, progress: &Arc<Mutex<String>>) -> Result<DiskReport, String> {
    let target = running.join("target");
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
        cleanable: true,
    });
    for worktree in qol_dev_build::tray::list_worktrees(running) {
        set_progress(progress, &format!("worktree {}", worktree.branch));
        let live = worktree.path == running;
        rows.push(measured_row(
            format!("{WORKTREE_LABEL_PREFIX}{}", worktree.branch),
            &worktree.path.join("target"),
            !live,
            if live { " · live target" } else { "" },
        ));
    }
    set_progress(progress, "sccache");
    rows.push(optional_dir_row("sccache", sccache_dir()));
    set_progress(progress, "cargo home");
    rows.push(optional_dir_row("cargo home", cargo_home()));
    Ok(DiskReport { rows })
}

fn cleanup_disk_usage(running: &Path, progress: &Arc<Mutex<String>>) -> Result<DiskReport, String> {
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
    let mut report = scan_disk_usage(running, progress)?;
    report.rows.insert(0, cleanup_row(freed, &failures));
    Ok(report)
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

fn measured_row(label: String, path: &Path, cleanable: bool, suffix: &str) -> DiskRow {
    let bytes = path.exists().then(|| path_bytes(path));
    DiskRow {
        label,
        detail: format!("{}{}", path.display(), suffix),
        bytes,
        cleanable,
    }
}

fn optional_dir_row(label: &str, path: Option<PathBuf>) -> DiskRow {
    let Some(path) = path else {
        return DiskRow {
            label: label.to_string(),
            detail: "location unknown".to_string(),
            bytes: None,
            cleanable: false,
        };
    };
    measured_row(label.to_string(), &path, false, "")
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

        let report = scan_disk_usage(&running, &progress()).expect("scan");

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

        let report = cleanup_disk_usage(&running, &progress()).expect("cleanup");

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

        let report = cleanup_disk_usage(&running, &progress()).expect("cleanup");

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
}
