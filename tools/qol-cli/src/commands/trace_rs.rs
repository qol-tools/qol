use anyhow::{bail, Context, Result};
use chrono::{Local, TimeZone};
use ratatui::crossterm::event::{
    self, Event as TerminalEvent, KeyCode, KeyEventKind, KeyModifiers,
};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, IsTerminal, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::mem::MaybeUninit;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_LOG_FILE: &str = qol_conventions::TRACE_LOG_PATH;
const REPLAY_GAP_MS: u64 = 120;
const REVERT_WINDOW_MS: u64 = 200;
const TAIL_FLUSH_AFTER: Duration = Duration::from_millis(80);
const TAIL_IDLE_SLEEP: Duration = Duration::from_millis(10);

const COLOR_RESET: &str = "\x1b[0m";
const COLOR_HEADER: &str = "\x1b[1;36m";
const COLOR_TIME: &str = "\x1b[2m";
const COLOR_DIM: &str = "\x1b[2m";
const COLOR_OK: &str = "\x1b[1;32m";
const COLOR_WARN: &str = "\x1b[1;33m";
const COLOR_FAIL: &str = "\x1b[1;31m";
const COLOR_FOCUS: &str = "\x1b[1;33m";
const COLOR_AMC: &str = "\x1b[1;35m";
const COLOR_OPACITY: &str = "\x1b[1;36m";
const COLOR_HOTKEY: &str = "\x1b[1;35m";

const ANOMALY_MARKERS: [&str; 7] = [
    "MISDIRECTED",
    "FOCUS FAILURE",
    "SUPERSEDED",
    "DIVERGENCE",
    "Timed out",
    "THRASH",
    "REVERT",
];

mod cli;
mod format;
mod model;
mod parse;
mod runner;
mod tail_input;
use cli::Args;
use format::{
    active_status, churn_suffix, format_group, format_opacity, format_python_float,
    format_timestamp, increment_count, increment_ordered_count, latency_color, opacity_eq,
    path_suffix, percentile, sorted_counts, truncate_chars, winact_outcome_color,
};
use model::{
    parse_raw_line, Event, GhostWindow, OpacityClassification, OpacityWaste, OpacityWrite,
    PendingActivation, RawLine, TraceStats,
};
use parse::{
    arrow_status, bracket_field, ewmh_payload, field, first_quoted, hide_opacity,
    launcher_pos_size, launcher_window, parse_at_bounds, parse_ghost_window,
    parse_monitor_bounds_debug, parse_python_cycle, parse_python_ghost_dump,
    parse_qol_title_origin, parse_xrandr_geometry_line, quoted_field, reason, reason_suffix,
    sequence, show_opacity, title_contains_match, tuple_field,
};
use runner::TraceRunner;
use tail_input::{detail_control_hint, DetailToggleInput, TailControl};

pub(crate) fn run(args: &[OsString]) -> Result<()> {
    run_as("trace-rs", args)
}

pub(crate) fn run_as(command_name: &str, args: &[OsString]) -> Result<()> {
    let args = Args::parse_for(command_name, args)?;
    let path = log_file();
    if let Some(mark) = args.mark.as_deref() {
        write_mark(&path, mark)?;
        return Ok(());
    }
    if !path.is_file() {
        bail!("trace log file {} does not exist yet", path.display());
    }
    TraceRunner::new(args, path).run()
}

pub(super) fn log_file() -> PathBuf {
    std::env::var_os("QOL_TRACE_LOG_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_LOG_FILE))
}

pub(super) fn write_mark(path: &PathBuf, message: &str) -> Result<()> {
    let ts = now_ms();
    let pid = std::process::id();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open trace log {}", path.display()))?;
    writeln!(file, "{ts} pid={pid} MARK message=\"{message}\"")?;
    println!("Injected marker: {message}");
    Ok(())
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}
