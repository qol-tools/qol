use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Padding, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use serde::{Deserialize, Serialize};

use crate::commands::emu::{
    emu_config_path, emu_dir, emu_scan, newest_run_detail, EnvironmentStatus, ImageCandidate,
    LastRun, ResolveState, RunDetail,
};
use crate::dev_server::{
    fetch_workspace_plugins, health_ok, post_recompile_current, post_reload_plugins,
    probe_endpoints, toggle_dev_link, web_ok, EndpointStatus, LinkToggle, WorkspacePlugin,
    WEBSITE_URL,
};
use crate::host_facade;
use crate::poller::Poller;

const LOG_CAP: usize = 2000;
const TICK: Duration = Duration::from_millis(150);
const HEALTH_PROBE_INTERVAL: Duration = Duration::from_secs(2);
const EMU_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const LINKS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const DOCTOR_BASE_INTERVAL: Duration = Duration::from_secs(10);
const DOCTOR_CAP_INTERVAL: Duration = Duration::from_secs(60);
const ENDPOINTS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const STOP_GRACE: Duration = Duration::from_secs(5);
const CRASH_TAIL: usize = 40;
const ACK_TTL: Duration = Duration::from_secs(6);
const FILTER_PANEL_MIN_WIDTH: u16 = 32;
const FILTER_PANEL_MAX_WIDTH: u16 = 78;
const FILTER_BRICK_GAP: usize = 1;
const FILTER_BRICK_CHROME: usize = 4;

pub(crate) enum SessionEnd {
    ChildExited(ExitStatus),
    UserQuit,
    ReloadRequested,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Action {
    ToggleView,
    ToggleKeys,
    ToggleArm,
    FeatureFlags,
    Rebuild,
    Doctor,
    ToggleTraceDetails,
    ToggleTraceRate,
    Back,
    Activate,
    Dive,
    Quit,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    Follow,
    Filter,
    Copy,
    OpenEmuDir,
    OpenCurrentLogFolder,
    OpenCurrentLogEditor,
    OpenCurrentLogRaw,
    ToggleArch,
    Confirm,
    Ignore,
}

fn preserves_arm(action: Action) -> bool {
    matches!(
        action,
        Action::ScrollUp
            | Action::ScrollDown
            | Action::PageUp
            | Action::PageDown
            | Action::Dive
            | Action::Back
            | Action::ToggleView
            | Action::ToggleKeys
            | Action::Follow
    )
}

#[derive(Clone, Copy)]
struct KeyHint {
    key: &'static str,
    desc: &'static str,
}

#[derive(Clone, Copy)]
struct KeyStroke {
    code: KeyCode,
    mods: KeyModifiers,
}

impl KeyStroke {
    fn plain(code: KeyCode) -> Self {
        Self {
            code,
            mods: KeyModifiers::NONE,
        }
    }

    fn ctrl(c: char) -> Self {
        Self {
            code: KeyCode::Char(c),
            mods: KeyModifiers::CONTROL,
        }
    }

    fn matches(self, code: KeyCode, mods: KeyModifiers) -> bool {
        if self.code != code {
            return false;
        }
        normalized_mods(mods) == self.mods
    }
}

#[derive(Clone)]
struct KeyBinding {
    hint: KeyHint,
    action: Action,
    strokes: Vec<KeyStroke>,
}

impl KeyBinding {
    fn matches(&self, code: KeyCode, mods: KeyModifiers) -> bool {
        self.strokes.iter().any(|stroke| stroke.matches(code, mods))
    }
}

fn normalized_mods(mut mods: KeyModifiers) -> KeyModifiers {
    mods.remove(KeyModifiers::SHIFT);
    mods
}

fn binding(
    key: &'static str,
    desc: &'static str,
    action: Action,
    strokes: Vec<KeyStroke>,
) -> KeyBinding {
    KeyBinding {
        hint: KeyHint { key, desc },
        action,
        strokes,
    }
}

fn char_binding(key: &'static str, desc: &'static str, action: Action, c: char) -> KeyBinding {
    let mut strokes = vec![KeyStroke::plain(KeyCode::Char(c))];
    let upper = c.to_ascii_uppercase();
    if upper != c {
        strokes.push(KeyStroke::plain(KeyCode::Char(upper)));
    }
    binding(key, desc, action, strokes)
}

fn global_action_bindings(armed: bool) -> Vec<KeyBinding> {
    let ctrl_r_desc = if armed {
        "reload qol dev"
    } else {
        "rebuild tray+plugins"
    };
    vec![
        binding(
            "ctrl+r",
            ctrl_r_desc,
            Action::Rebuild,
            vec![KeyStroke::ctrl('r')],
        ),
        char_binding("l", "logs", Action::ToggleView, 'l'),
        char_binding("k", "keys", Action::ToggleKeys, 'k'),
        binding(
            "ctrl+f",
            "feature flags",
            Action::FeatureFlags,
            vec![KeyStroke::ctrl('f')],
        ),
        binding(
            "q / ctrl+c",
            "quit",
            Action::Quit,
            vec![KeyStroke::plain(KeyCode::Char('q')), KeyStroke::ctrl('c')],
        ),
    ]
}

fn context_action_bindings(dash: &Dash) -> Vec<KeyBinding> {
    match dash.view {
        View::Dashboard => vec![
            binding(
                "↑/↓",
                "move",
                Action::ScrollUp,
                vec![KeyStroke::plain(KeyCode::Up)],
            ),
            binding(
                "↑/↓",
                "move",
                Action::ScrollDown,
                vec![KeyStroke::plain(KeyCode::Down)],
            ),
            binding(
                "enter",
                "act on row",
                Action::Activate,
                vec![KeyStroke::plain(KeyCode::Enter)],
            ),
            binding(
                "→ / ←",
                "dive · back",
                Action::Dive,
                vec![KeyStroke::plain(KeyCode::Right)],
            ),
            binding(
                "→ / ←",
                "dive · back",
                Action::Back,
                vec![
                    KeyStroke::plain(KeyCode::Left),
                    KeyStroke::plain(KeyCode::Esc),
                ],
            ),
            binding(
                "space",
                "arm, then enter",
                Action::ToggleArm,
                vec![KeyStroke::plain(KeyCode::Char(' '))],
            ),
            char_binding("d", "doctor", Action::Doctor, 'd'),
        ],
        View::Emu => vec![
            binding(
                "↑/↓",
                "select emu",
                Action::ScrollUp,
                vec![KeyStroke::plain(KeyCode::Up)],
            ),
            binding(
                "↑/↓",
                "select emu",
                Action::ScrollDown,
                vec![KeyStroke::plain(KeyCode::Down)],
            ),
            binding(
                "enter",
                "boot · stop",
                Action::Activate,
                vec![KeyStroke::plain(KeyCode::Enter)],
            ),
            binding(
                "→",
                "detail · log",
                Action::Dive,
                vec![KeyStroke::plain(KeyCode::Right)],
            ),
            binding(
                "space",
                "arm: checks",
                Action::ToggleArm,
                vec![KeyStroke::plain(KeyCode::Char(' '))],
            ),
            char_binding("o", "open emu dir", Action::OpenEmuDir, 'o'),
            char_binding("t", "set arch", Action::ToggleArch, 't'),
            char_binding("a", "add image", Action::Confirm, 'a'),
            binding(
                "←",
                "back",
                Action::Back,
                vec![
                    KeyStroke::plain(KeyCode::Left),
                    KeyStroke::plain(KeyCode::Esc),
                ],
            ),
        ],
        View::Logs => stream_view_bindings(false, true, false),
        View::Trace => stream_view_bindings(true, true, dash.trace_rate.is_realtime()),
        View::EmuDetail => stream_view_bindings(false, false, false),
        View::Doctor => vec![
            char_binding("d", "refresh checks", Action::Doctor, 'd'),
            binding(
                "space",
                "raw output",
                Action::ToggleArm,
                vec![KeyStroke::plain(KeyCode::Char(' '))],
            ),
            binding(
                "↑/↓",
                "scroll",
                Action::ScrollUp,
                vec![KeyStroke::plain(KeyCode::Up)],
            ),
            binding(
                "↑/↓",
                "scroll",
                Action::ScrollDown,
                vec![KeyStroke::plain(KeyCode::Down)],
            ),
            binding(
                "←",
                "back",
                Action::Back,
                vec![
                    KeyStroke::plain(KeyCode::Left),
                    KeyStroke::plain(KeyCode::Esc),
                ],
            ),
        ],
        View::Plugins => vec![
            binding(
                "↑/↓",
                "move",
                Action::ScrollUp,
                vec![KeyStroke::plain(KeyCode::Up)],
            ),
            binding(
                "↑/↓",
                "move",
                Action::ScrollDown,
                vec![KeyStroke::plain(KeyCode::Down)],
            ),
            binding(
                "enter",
                "link/unlink",
                Action::Activate,
                vec![KeyStroke::plain(KeyCode::Enter)],
            ),
            binding(
                "←",
                "back",
                Action::Back,
                vec![
                    KeyStroke::plain(KeyCode::Left),
                    KeyStroke::plain(KeyCode::Esc),
                ],
            ),
        ],
        View::Endpoints => vec![
            binding(
                "↑/↓",
                "scroll",
                Action::ScrollUp,
                vec![KeyStroke::plain(KeyCode::Up)],
            ),
            binding(
                "↑/↓",
                "scroll",
                Action::ScrollDown,
                vec![KeyStroke::plain(KeyCode::Down)],
            ),
            binding(
                "←",
                "back",
                Action::Back,
                vec![
                    KeyStroke::plain(KeyCode::Left),
                    KeyStroke::plain(KeyCode::Esc),
                ],
            ),
        ],
    }
}

fn stream_view_bindings(trace: bool, log_resource: bool, trace_realtime: bool) -> Vec<KeyBinding> {
    let mut bindings = vec![
        binding(
            "↑/↓",
            "scroll",
            Action::ScrollUp,
            vec![KeyStroke::plain(KeyCode::Up)],
        ),
        binding(
            "↑/↓",
            "scroll",
            Action::ScrollDown,
            vec![KeyStroke::plain(KeyCode::Down)],
        ),
        binding(
            "pgup/pgdn",
            "page",
            Action::PageUp,
            vec![KeyStroke::plain(KeyCode::PageUp)],
        ),
        binding(
            "pgup/pgdn",
            "page",
            Action::PageDown,
            vec![KeyStroke::plain(KeyCode::PageDown)],
        ),
        binding(
            "f / end",
            "follow tail",
            Action::Follow,
            vec![
                KeyStroke::plain(KeyCode::Char('f')),
                KeyStroke::plain(KeyCode::Char('F')),
                KeyStroke::plain(KeyCode::End),
            ],
        ),
        binding(
            "/",
            "filter",
            Action::Filter,
            vec![KeyStroke::plain(KeyCode::Char('/'))],
        ),
        char_binding("c", "copy last N", Action::Copy, 'c'),
    ];
    if log_resource {
        bindings.push(char_binding(
            "o",
            "open folder",
            Action::OpenCurrentLogFolder,
            'o',
        ));
        bindings.push(char_binding(
            "e",
            "open in editor",
            Action::OpenCurrentLogEditor,
            'e',
        ));
        if trace {
            bindings.push(char_binding(
                "r",
                "open raw",
                Action::OpenCurrentLogRaw,
                'r',
            ));
        }
    }
    if trace {
        bindings.push(binding(
            "space",
            "arm: reload",
            Action::ToggleArm,
            vec![KeyStroke::plain(KeyCode::Char(' '))],
        ));
        bindings.push(char_binding(
            "d",
            "details",
            Action::ToggleTraceDetails,
            'd',
        ));
        bindings.push(char_binding(
            "s",
            if trace_realtime {
                "rate (realtime)"
            } else {
                "rate (relaxed)"
            },
            Action::ToggleTraceRate,
            's',
        ));
    }
    bindings.push(binding(
        "←",
        "back",
        Action::Back,
        vec![
            KeyStroke::plain(KeyCode::Left),
            KeyStroke::plain(KeyCode::Esc),
        ],
    ));
    bindings
}

fn action_for(dash: &Dash, code: KeyCode, mods: KeyModifiers) -> Action {
    global_action_bindings(dash.armed)
        .into_iter()
        .chain(context_action_bindings(dash))
        .find(|binding| binding.matches(code, mods))
        .map(|binding| binding.action)
        .unwrap_or(Action::Ignore)
}

fn unique_hints(bindings: Vec<KeyBinding>) -> Vec<KeyHint> {
    let mut hints = Vec::new();
    for binding in bindings {
        if hints
            .iter()
            .any(|hint: &KeyHint| hint.key == binding.hint.key && hint.desc == binding.hint.desc)
        {
            continue;
        }
        hints.push(binding.hint);
    }
    hints
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TraceRate {
    #[default]
    Relaxed,
    Realtime,
}

impl TraceRate {
    fn is_realtime(self) -> bool {
        matches!(self, Self::Realtime)
    }

    fn toggled(self) -> Self {
        match self {
            Self::Relaxed => Self::Realtime,
            Self::Realtime => Self::Relaxed,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Relaxed => "rate: relaxed",
            Self::Realtime => "rate: realtime",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FilterStrategy {
    Include,
    Exclude,
}

impl FilterStrategy {
    fn symbol(self) -> &'static str {
        match self {
            Self::Include => "+",
            Self::Exclude => "-",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Include => Color::Green,
            Self::Exclude => Color::Red,
        }
    }

    fn cycle(self) -> Self {
        match self {
            Self::Include => Self::Exclude,
            Self::Exclude => Self::Include,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
struct LogFilter {
    strategy: FilterStrategy,
    text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterScope {
    Logs,
    Trace,
    Emu,
}

fn filter_scope(view: View) -> Option<FilterScope> {
    match view {
        View::Logs => Some(FilterScope::Logs),
        View::Trace => Some(FilterScope::Trace),
        View::EmuDetail => Some(FilterScope::Emu),
        View::Dashboard | View::Doctor | View::Plugins | View::Emu | View::Endpoints => None,
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ViewFilters {
    #[serde(default)]
    logs: Vec<LogFilter>,
    #[serde(default)]
    trace: Vec<LogFilter>,
    #[serde(default)]
    emu: Vec<LogFilter>,
}

impl ViewFilters {
    fn for_view(&self, view: View) -> &[LogFilter] {
        match filter_scope(view) {
            Some(FilterScope::Logs) => &self.logs,
            Some(FilterScope::Trace) => &self.trace,
            Some(FilterScope::Emu) => &self.emu,
            None => &[],
        }
    }

    fn for_view_mut(&mut self, view: View) -> Option<&mut Vec<LogFilter>> {
        match filter_scope(view) {
            Some(FilterScope::Logs) => Some(&mut self.logs),
            Some(FilterScope::Trace) => Some(&mut self.trace),
            Some(FilterScope::Emu) => Some(&mut self.emu),
            None => None,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConsoleState {
    #[serde(default)]
    filters: ViewFilters,
    #[serde(default)]
    trace_details: bool,
    #[serde(default)]
    trace_rate: TraceRate,
    #[serde(default)]
    keys_hidden: bool,
    #[serde(default)]
    feature_flags: Vec<String>,
}

fn console_state_path() -> Option<PathBuf> {
    qol_config::config_dir().map(|dir| dir.join("dev/console.json"))
}

fn load_console_state() -> ConsoleState {
    let Some(path) = console_state_path() else {
        return ConsoleState::default();
    };
    let Ok(contents) = fs::read_to_string(&path) else {
        return ConsoleState::default();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

fn save_console_state(state: &ConsoleState) {
    let Some(path) = console_state_path() else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(json) = serde_json::to_string_pretty(state) else {
        return;
    };
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    if fs::write(&tmp, json).is_err() {
        return;
    }
    let _ = fs::rename(&tmp, &path);
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct PickerBrick {
    index: usize,
    row: usize,
    x: usize,
    width: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum PickerMove {
    Left,
    Right,
    Up,
    Down,
}

fn line_matches_filters(line: &str, filters: &[LogFilter]) -> bool {
    let mut has_include = false;
    let mut included = false;
    for filter in filters {
        if filter.text.is_empty() {
            continue;
        }
        let matched = line.contains(&filter.text);
        match filter.strategy {
            FilterStrategy::Exclude if matched => return false,
            FilterStrategy::Include => {
                has_include = true;
                included |= matched;
            }
            FilterStrategy::Exclude => {}
        }
    }
    !has_include || included
}

#[derive(Debug, PartialEq, Eq)]
enum FilterState {
    Closed,
    Managing,
    Editing {
        index: Option<usize>,
        draft: String,
        strategy: FilterStrategy,
    },
}

impl FilterState {
    fn is_active(&self) -> bool {
        !matches!(self, Self::Closed)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum FeatureFlag {}

struct FeatureFlagDef {
    flag: FeatureFlag,
    id: &'static str,
    label: &'static str,
}

const FEATURE_FLAGS: &[FeatureFlagDef] = &[];

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum TraceRenderer {
    Rust,
}

impl TraceRenderer {
    fn name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
        }
    }

    fn missing_hint(self) -> &'static str {
        match self {
            Self::Rust => "could not exec current qol binary",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Default)]
struct FeatureFlags {
    enabled: Vec<FeatureFlag>,
}

impl FeatureFlags {
    fn enabled(&self, flag: FeatureFlag) -> bool {
        self.enabled.contains(&flag)
    }

    fn ids(&self) -> Vec<String> {
        FEATURE_FLAGS
            .iter()
            .filter(|def| self.enabled(def.flag))
            .map(|def| def.id.to_string())
            .collect()
    }

    fn from_ids(ids: &[String]) -> Self {
        let enabled = FEATURE_FLAGS
            .iter()
            .filter(|def| ids.iter().any(|id| id == def.id))
            .map(|def| def.flag)
            .collect();
        Self { enabled }
    }
}

#[derive(Debug, PartialEq, Eq, Default)]
struct FeatureFlagPanel {
    open: bool,
    selected: usize,
    layout_width: usize,
}

impl FeatureFlagPanel {
    fn is_active(&self) -> bool {
        self.open
    }
}

struct LogRing {
    lines: VecDeque<String>,
    collapse: bool,
    last_key: Option<String>,
    repeat: usize,
}

impl LogRing {
    fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            collapse: false,
            last_key: None,
            repeat: 0,
        }
    }

    fn collapsing() -> Self {
        Self {
            collapse: true,
            ..Self::new()
        }
    }

    fn set_collapse(&mut self, on: bool) {
        self.collapse = on;
        self.last_key = None;
        self.repeat = 0;
    }

    fn push(&mut self, line: String) {
        if self.collapse && self.try_collapse(&line) {
            return;
        }
        if self.lines.len() == LOG_CAP {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    fn try_collapse(&mut self, line: &str) -> bool {
        let key = collapse_key(line);
        if self.repeat > 0 && self.last_key.as_deref() == Some(key.as_str()) {
            if let Some(slot) = self.lines.back_mut() {
                self.repeat += 1;
                *slot = format!("{line}{}", repeat_badge(self.repeat));
                return true;
            }
        }
        self.last_key = Some(key);
        self.repeat = 1;
        false
    }

    fn len(&self) -> usize {
        self.lines.len()
    }
}

fn repeat_badge(count: usize) -> String {
    format!("\u{1b}[2m (\u{d7}{count})\u{1b}[0m")
}

fn collapse_key(line: &str) -> String {
    let plain = strip_ansi(line);
    let trimmed = plain.trim();
    trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
        .map(|(_, tail)| tail.trim().to_string())
        .unwrap_or_else(|| trimmed.to_string())
}

fn window_start(len: usize, height: usize, offset: usize) -> usize {
    len.saturating_sub(height.saturating_add(offset))
}

fn clamp_offset(len: usize, height: usize, offset: usize) -> usize {
    offset.min(len.saturating_sub(height))
}

struct OwnedSource {
    child: Child,
    rx: Receiver<String>,
}

struct LogPane {
    ring: LogRing,
    source: Option<OwnedSource>,
}

impl LogPane {
    fn new() -> Self {
        Self {
            ring: LogRing::new(),
            source: None,
        }
    }

    fn collapsing() -> Self {
        Self {
            ring: LogRing::collapsing(),
            source: None,
        }
    }

    fn set_collapse(&mut self, on: bool) {
        self.ring.set_collapse(on);
    }

    fn attach(&mut self, child: Child, rx: Receiver<String>) {
        self.source = Some(OwnedSource { child, rx });
    }

    fn is_live(&self) -> bool {
        self.source.is_some()
    }

    fn push(&mut self, line: String) {
        self.ring.push(line);
    }

    fn len(&self) -> usize {
        self.ring.len()
    }

    fn drain(&mut self, keep: impl Fn(&str) -> bool) {
        let Some(source) = self.source.as_ref() else {
            return;
        };
        let mut received = Vec::new();
        while let Ok(line) = source.rx.try_recv() {
            received.push(line);
        }
        for line in received {
            if keep(&line) {
                self.ring.push(line);
            }
        }
    }

    fn stop(&mut self) {
        if let Some(mut source) = self.source.take() {
            let _ = source.child.kill();
            let _ = source.child.wait();
        }
    }

    fn replay(path: &Path) -> Self {
        let mut ring = LogRing::new();
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                ring.push(line.to_string());
            }
        }
        Self { ring, source: None }
    }

    fn poll_finished(&mut self, keep: impl Fn(&str) -> bool) -> bool {
        let mut received = Vec::new();
        let mut exited = None;
        if let Some(source) = self.source.as_mut() {
            while let Ok(line) = source.rx.try_recv() {
                received.push(line);
            }
            if let Ok(Some(status)) = source.child.try_wait() {
                while let Ok(line) = source.rx.try_recv() {
                    received.push(line);
                }
                exited = Some(status);
            }
        }
        for line in received {
            if keep(&line) {
                self.ring.push(line);
            }
        }
        match exited {
            Some(status) => {
                self.ring.push(emu_run_line("done", &status.to_string()));
                self.source = None;
                true
            }
            None => false,
        }
    }

    fn stop_graceful(&mut self) {
        let Some(mut source) = self.source.take() else {
            return;
        };
        let deadline = Instant::now() + STOP_GRACE;
        while Instant::now() < deadline {
            if matches!(source.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = source.child.kill();
        let _ = source.child.wait();
    }
}

struct DevLogFile {
    path: PathBuf,
    writer: Option<BufWriter<File>>,
}

impl DevLogFile {
    fn create() -> Option<Self> {
        let primary = dev_log_dir();
        create_dev_log_file_in(&primary).or_else(|| {
            let fallback = std::env::temp_dir().join("qol-tray/logs");
            create_dev_log_file_in(&fallback)
        })
    }

    #[cfg(test)]
    fn path_only(path: PathBuf) -> Self {
        Self { path, writer: None }
    }

    fn write_line(&mut self, line: &str) {
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        let _ = writeln!(writer, "{line}");
        let _ = writer.flush();
    }
}

fn dev_log_dir() -> PathBuf {
    core_log_dir()
}

fn create_dev_log_file_in(dir: &Path) -> Option<DevLogFile> {
    fs::create_dir_all(dir).ok()?;
    let path = dir.join(dev_log_file_name());
    let writer = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
        .map(BufWriter::new)?;
    Some(DevLogFile {
        path,
        writer: Some(writer),
    })
}

fn dev_log_file_name() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("qol-dev-{ts}-{}.log", std::process::id())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Dashboard,
    Logs,
    Doctor,
    Plugins,
    Emu,
    EmuDetail,
    Trace,
    Endpoints,
}

#[derive(Clone, Copy)]
enum Row {
    Tray,
    Web,
    Plugins,
    Emu,
    Doctor,
    Logs,
    Trace,
}

const ROWS: [Row; 7] = [
    Row::Tray,
    Row::Web,
    Row::Plugins,
    Row::Emu,
    Row::Doctor,
    Row::Logs,
    Row::Trace,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Health {
    Checking,
    Up,
    Down,
}

struct HealthSnapshot {
    api: bool,
    web: bool,
}

type EmuScanResult = Result<(Vec<EnvironmentStatus>, Vec<ImageCandidate>), String>;

struct Probes {
    health: Poller<HealthSnapshot>,
    emu: Poller<EmuScanResult>,
    links: Poller<Result<Vec<WorkspacePlugin>, String>>,
    doctor: Poller<Result<DoctorRun, String>>,
    endpoints: Option<Poller<Vec<EndpointStatus>>>,
}

impl Probes {
    fn spawn() -> Self {
        Self {
            health: Poller::spawn(HEALTH_PROBE_INTERVAL, || HealthSnapshot {
                api: health_ok(),
                web: web_ok(),
            }),
            emu: Poller::spawn(EMU_REFRESH_INTERVAL, || {
                emu_scan().map_err(|error| format!("{error:#}"))
            }),
            links: Poller::spawn(LINKS_REFRESH_INTERVAL, || {
                fetch_workspace_plugins().map_err(|error| format!("{error:#}"))
            }),
            doctor: spawn_doctor_probe(),
            endpoints: None,
        }
    }
}

fn spawn_doctor_probe() -> Poller<Result<DoctorRun, String>> {
    Poller::spawn_adaptive(
        DOCTOR_BASE_INTERVAL,
        DOCTOR_CAP_INTERVAL,
        run_doctor_prebuilt,
    )
}

#[derive(Default)]
struct Pokes {
    emu: bool,
    links: bool,
    doctor: bool,
}

fn flush_pokes(dash: &mut Dash, probes: &Probes) {
    if std::mem::take(&mut dash.pokes.emu) {
        probes.emu.poke();
    }
    if std::mem::take(&mut dash.pokes.links) {
        probes.links.poke();
    }
    if std::mem::take(&mut dash.pokes.doctor) {
        probes.doctor.poke();
    }
}

enum EndpointsState {
    Probing,
    Done(Vec<EndpointStatus>),
}

enum EmuState {
    Probing,
    Done(Vec<EnvironmentStatus>),
    Failed(String),
}

enum LinksState {
    Unknown,
    Live(Vec<WorkspacePlugin>),
    Unreachable,
}

struct EmuDetail {
    id: String,
    info: Vec<Line<'static>>,
    replay: Option<LogPane>,
}

#[derive(Clone, Copy, PartialEq)]
struct DoctorReport {
    ok: usize,
    warn: usize,
    error: usize,
    crash: usize,
}

impl DoctorReport {
    fn divergences(&self) -> usize {
        self.warn + self.error + self.crash
    }
}

struct DoctorPanel {
    last: Option<DoctorRun>,
    last_at_ms: Option<u64>,
    manual: Option<(DoctorMode, Receiver<Result<DoctorRun, String>>)>,
    error: Option<String>,
}

#[derive(Clone, PartialEq)]
struct DoctorRun {
    report: DoctorReport,
    lines: Vec<String>,
}

enum RebuildState {
    Idle,
    Requested(Instant),
    Failed(String),
}

enum Reload {
    Idle,
    Running { child: Child, rx: Receiver<String> },
}

enum ReloadOutcome {
    Pending,
    Ready,
}

struct Dash {
    view: View,
    logs: LogPane,
    log_file: Option<DevLogFile>,
    scroll_offset: usize,
    health: Health,
    web: Health,
    endpoints: EndpointsState,
    started: Instant,
    rebuild: RebuildState,
    plugin_reload: RebuildState,
    plugin_names: Vec<String>,
    emu: EmuState,
    active_runs: HashMap<String, LogPane>,
    emu_detail: Option<EmuDetail>,
    emu_cursor: usize,
    emu_candidates: Vec<ImageCandidate>,
    log_height: usize,
    cursor: usize,
    plugin_cursor: usize,
    doctor: DoctorPanel,
    trace: LogPane,
    trace_unavailable: bool,
    trace_details: bool,
    trace_rate: TraceRate,
    features: FeatureFlags,
    feature_panel: FeatureFlagPanel,
    boot_rx: Option<Receiver<String>>,
    keys_hidden: bool,
    filters: ViewFilters,
    filter_index: usize,
    filter_layout_width: usize,
    filter_state: FilterState,
    state_dirty: bool,
    copy_count: String,
    copying: bool,
    notice: Option<(Instant, String)>,
    armed: bool,
    reload: Reload,
    pokes: Pokes,
    links: LinksState,
}

impl Dash {
    fn new(plugin_names: Vec<String>) -> Self {
        Self {
            view: View::Dashboard,
            logs: LogPane::new(),
            log_file: None,
            scroll_offset: 0,
            health: Health::Checking,
            web: Health::Checking,
            endpoints: EndpointsState::Probing,
            started: Instant::now(),
            rebuild: RebuildState::Idle,
            plugin_reload: RebuildState::Idle,
            plugin_names,
            emu: EmuState::Probing,
            active_runs: HashMap::new(),
            emu_detail: None,
            emu_cursor: 0,
            emu_candidates: Vec::new(),
            log_height: 0,
            cursor: 0,
            plugin_cursor: 0,
            doctor: DoctorPanel {
                last: None,
                last_at_ms: None,
                manual: None,
                error: None,
            },
            trace: LogPane::collapsing(),
            trace_unavailable: false,
            trace_details: false,
            trace_rate: TraceRate::default(),
            features: FeatureFlags::default(),
            feature_panel: FeatureFlagPanel {
                layout_width: default_filter_layout_width(),
                ..FeatureFlagPanel::default()
            },
            boot_rx: None,
            keys_hidden: false,
            filters: ViewFilters::default(),
            filter_index: 0,
            filter_layout_width: default_filter_layout_width(),
            filter_state: FilterState::Closed,
            state_dirty: false,
            copy_count: String::new(),
            copying: false,
            notice: None,
            armed: false,
            reload: Reload::Idle,
            pokes: Pokes::default(),
            links: LinksState::Unknown,
        }
    }

    fn start_log_file(&mut self) {
        self.log_file = DevLogFile::create();
    }

    fn push_log(&mut self, line: impl Into<String>) {
        let line = line.into();
        if let Some(log_file) = self.log_file.as_mut() {
            log_file.write_line(&line);
        }
        self.logs.push(line);
    }

    fn is_reloading(&self) -> bool {
        matches!(self.reload, Reload::Running { .. })
    }

    fn start_doctor(&mut self, mode: DoctorMode) {
        self.doctor.manual = Some((mode, spawn_doctor(mode)));
    }

    fn active_filters(&self) -> &[LogFilter] {
        self.filters.for_view(self.view)
    }

    fn mark_state_dirty(&mut self) {
        self.state_dirty = true;
    }

    fn apply_state(&mut self, state: ConsoleState) {
        self.filters = state.filters;
        self.trace_details = state.trace_details;
        self.trace_rate = state.trace_rate;
        self.trace.set_collapse(!self.trace_rate.is_realtime());
        self.keys_hidden = state.keys_hidden;
        self.features = FeatureFlags::from_ids(&state.feature_flags);
        self.state_dirty = false;
    }

    fn to_state(&self) -> ConsoleState {
        ConsoleState {
            filters: self.filters.clone(),
            trace_details: self.trace_details,
            trace_rate: self.trace_rate,
            keys_hidden: self.keys_hidden,
            feature_flags: self.features.ids(),
        }
    }

    fn close_filters(&mut self) {
        self.filter_index = 0;
        self.filter_state = FilterState::Closed;
    }

    fn open_filter_manager(&mut self) {
        self.filter_state = FilterState::Managing;
        let len = self.active_filters().len();
        self.filter_index = self.filter_index.min(len.saturating_sub(1));
    }

    fn move_filter(&mut self, direction: PickerMove) {
        let width = self.filter_layout_width;
        let active = self.active_filters();
        let layout = filter_brick_layout(active, width);
        let len = active.len();
        move_picker_selection(&mut self.filter_index, len, direction, &layout);
    }

    fn start_filter_add(&mut self) {
        self.filter_state = FilterState::Editing {
            index: None,
            draft: String::new(),
            strategy: FilterStrategy::Include,
        };
    }

    fn start_filter_edit(&mut self) {
        let Some(filter) = self.active_filters().get(self.filter_index).cloned() else {
            return;
        };
        self.filter_state = FilterState::Editing {
            index: Some(self.filter_index),
            draft: filter.text,
            strategy: filter.strategy,
        };
    }

    fn save_filter_draft(&mut self) {
        let (index, strategy, text) = match &self.filter_state {
            FilterState::Editing {
                index,
                draft,
                strategy,
            } => (*index, *strategy, draft.trim().to_string()),
            _ => return,
        };
        if text.is_empty() {
            return;
        }
        let filter = LogFilter { strategy, text };
        let view = self.view;
        let Some(filters) = self.filters.for_view_mut(view) else {
            return;
        };
        let new_index = match index {
            Some(i) => match filters.get_mut(i) {
                Some(slot) => {
                    *slot = filter;
                    i
                }
                None => return,
            },
            None => {
                filters.push(filter);
                filters.len().saturating_sub(1)
            }
        };
        self.filter_index = new_index;
        self.filter_state = FilterState::Managing;
        self.mark_state_dirty();
    }

    fn delete_selected_filter(&mut self) {
        let view = self.view;
        let Some(filters) = self.filters.for_view_mut(view) else {
            return;
        };
        if filters.is_empty() {
            return;
        }
        let index = self.filter_index.min(filters.len() - 1);
        filters.remove(index);
        let len = filters.len();
        self.filter_index = index.min(len.saturating_sub(1));
        self.mark_state_dirty();
    }

    fn trace_details_enabled(&self) -> bool {
        self.trace_details
    }

    fn trace_renderer(&self) -> TraceRenderer {
        TraceRenderer::Rust
    }

    fn toggle_feature_flags_panel(&mut self) {
        if self.feature_panel.is_active() {
            self.feature_panel.open = false;
            return;
        }
        self.filter_state = FilterState::Closed;
        self.copying = false;
        self.armed = false;
        self.feature_panel.open = true;
        self.feature_panel.selected = self
            .feature_panel
            .selected
            .min(FEATURE_FLAGS.len().saturating_sub(1));
    }

    fn move_feature_flag(&mut self, direction: PickerMove) {
        let layout = feature_flag_brick_layout(self.feature_panel.layout_width);
        move_picker_selection(
            &mut self.feature_panel.selected,
            FEATURE_FLAGS.len(),
            direction,
            &layout,
        );
    }

    fn toggle_selected_feature_flag(&mut self) {
        let Some(def) = FEATURE_FLAGS.get(self.feature_panel.selected) else {
            return;
        };
        toggle_feature_flag(def.flag);
    }
}

pub(crate) fn run_session(
    child: &mut Child,
    verbose: bool,
    plugins: Vec<String>,
    boot: Option<Receiver<String>>,
) -> Result<SessionEnd> {
    let lines = spawn_forwarders(child);
    if verbose || !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return plain_session(child, &lines, boot);
    }
    let mut probes = Probes::spawn();
    let mut dash = Dash::new(plugins);
    dash.apply_state(load_console_state());
    dash.start_log_file();
    dash.boot_rx = boot;
    start_trace(&mut dash);
    let mut terminal = ratatui::init();
    let result = tui_session(&mut terminal, child, &lines, &mut probes, &mut dash);
    ratatui::restore();
    if let Ok(SessionEnd::ChildExited(status)) = &result {
        if !status.success() {
            print_crash_tail(&dash.logs.ring);
        }
    }
    result
}

fn print_crash_tail(logs: &LogRing) {
    let start = logs.len().saturating_sub(CRASH_TAIL);
    for line in logs.lines.iter().skip(start) {
        eprintln!("{line}");
    }
}

fn plain_session(
    child: &mut Child,
    lines: &Receiver<String>,
    boot: Option<Receiver<String>>,
) -> Result<SessionEnd> {
    loop {
        if let Some(rx) = boot.as_ref() {
            while let Ok(line) = rx.try_recv() {
                println!("{line}");
            }
        }
        match lines.recv_timeout(TICK) {
            Ok(line) => println!("{line}"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                let status = child
                    .wait()
                    .context("failed waiting for qol-tray dev process")?;
                return Ok(SessionEnd::ChildExited(status));
            }
        }
        if let Some(status) = try_wait(child)? {
            while let Ok(line) = lines.try_recv() {
                println!("{line}");
            }
            return Ok(SessionEnd::ChildExited(status));
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum KeyOutcome {
    Quit,
    Reload,
    Handled,
}

fn handle_key(dash: &mut Dash, code: KeyCode, mods: KeyModifiers) -> KeyOutcome {
    if is_feature_flags_shortcut(code, mods) {
        dash.toggle_feature_flags_panel();
        return KeyOutcome::Handled;
    }
    if dash.feature_panel.is_active() {
        edit_feature_flags(dash, code);
        return KeyOutcome::Handled;
    }
    if dash.filter_state.is_active() {
        edit_filters(dash, code);
        return KeyOutcome::Handled;
    }
    if dash.copying {
        edit_copy(dash, code);
        return KeyOutcome::Handled;
    }
    if dash.armed && code == KeyCode::Esc {
        dash.armed = false;
        return KeyOutcome::Handled;
    }
    let modified = dash.armed;
    let action = action_for(dash, code, mods);
    match action {
        Action::Quit => KeyOutcome::Quit,
        Action::Rebuild if modified => {
            dash.armed = false;
            KeyOutcome::Reload
        }
        action => {
            apply_action(dash, action, modified);
            if modified && !preserves_arm(action) {
                dash.armed = false;
            }
            KeyOutcome::Handled
        }
    }
}

fn tui_session(
    terminal: &mut DefaultTerminal,
    child: &mut Child,
    lines: &Receiver<String>,
    probes: &mut Probes,
    dash: &mut Dash,
) -> Result<SessionEnd> {
    loop {
        while let Ok(line) = lines.try_recv() {
            dash.push_log(line);
        }
        if let Some(snapshot) = probes.health.latest() {
            apply_health(dash, snapshot);
        }
        match (dash.view == View::Endpoints, probes.endpoints.is_some()) {
            (true, false) => {
                probes.endpoints = Some(Poller::spawn(ENDPOINTS_REFRESH_INTERVAL, probe_endpoints));
            }
            (false, true) => probes.endpoints = None,
            (true, true) | (false, false) => {}
        }
        if let Some(results) = probes.endpoints.as_ref().and_then(|poller| poller.latest()) {
            dash.endpoints = EndpointsState::Done(results);
        }
        if let Some(outcome) = probes.emu.latest() {
            dash.emu = match outcome {
                Ok((statuses, candidates)) => {
                    dash.emu_candidates = candidates;
                    EmuState::Done(statuses)
                }
                Err(error) => EmuState::Failed(error),
            };
        }
        if let Some(outcome) = probes.links.latest() {
            dash.links = match outcome {
                Ok(links) => LinksState::Live(links),
                Err(_) => LinksState::Unreachable,
            };
        }
        let manual_outcome = dash
            .doctor
            .manual
            .as_ref()
            .and_then(|(_, rx)| rx.try_recv().ok());
        if let Some(outcome) = manual_outcome {
            dash.doctor.manual = None;
            apply_doctor_outcome(dash, outcome);
            probes.doctor = spawn_doctor_probe();
        } else if dash.doctor.manual.is_none() {
            if let Some(outcome) = probes.doctor.latest() {
                apply_doctor_outcome(dash, outcome);
            }
        } else {
            let _ = probes.doctor.latest();
        }
        dash.trace.drain(|_| true);
        drain_boot(dash);
        drain_emu_runs(dash);
        if let ReloadOutcome::Ready = poll_reload(dash) {
            persist_if_dirty(dash);
            stop_trace(dash);
            stop_emu_runs(dash);
            stop_child(child)?;
            return Ok(SessionEnd::ReloadRequested);
        }
        if let Some(status) = try_wait(child)? {
            while let Ok(line) = lines.try_recv() {
                dash.push_log(line);
            }
            stop_trace(dash);
            stop_emu_runs(dash);
            return Ok(SessionEnd::ChildExited(status));
        }
        flush_pokes(dash, probes);
        terminal.draw(|frame| draw(frame, dash))?;
        if let Some((code, mods)) = poll_key()? {
            match handle_key(dash, code, mods) {
                KeyOutcome::Quit => {
                    persist_if_dirty(dash);
                    stop_trace(dash);
                    stop_emu_runs(dash);
                    stop_child(child)?;
                    return Ok(SessionEnd::UserQuit);
                }
                KeyOutcome::Reload => start_reload(dash),
                KeyOutcome::Handled => {}
            }
        }
        persist_if_dirty(dash);
    }
}

fn persist_if_dirty(dash: &mut Dash) {
    if dash.state_dirty {
        save_console_state(&dash.to_state());
        dash.state_dirty = false;
    }
}

fn drain_boot(dash: &mut Dash) {
    let mut received = Vec::new();
    if let Some(rx) = dash.boot_rx.as_ref() {
        while let Ok(line) = rx.try_recv() {
            received.push(line);
        }
    }
    for line in received {
        dash.push_log(line);
    }
}

fn is_feature_flags_shortcut(code: KeyCode, mods: KeyModifiers) -> bool {
    KeyStroke::ctrl('f').matches(code, mods)
}

fn edit_feature_flags(dash: &mut Dash, code: KeyCode) {
    match code {
        KeyCode::Left => dash.move_feature_flag(PickerMove::Left),
        KeyCode::Right => dash.move_feature_flag(PickerMove::Right),
        KeyCode::Up => dash.move_feature_flag(PickerMove::Up),
        KeyCode::Down => dash.move_feature_flag(PickerMove::Down),
        KeyCode::Enter | KeyCode::Char(' ') => dash.toggle_selected_feature_flag(),
        KeyCode::Esc => dash.feature_panel.open = false,
        _ => {}
    }
}

fn edit_filters(dash: &mut Dash, code: KeyCode) {
    if let FilterState::Editing {
        draft, strategy, ..
    } = &mut dash.filter_state
    {
        match code {
            KeyCode::Char(c) => draft.push(c),
            KeyCode::Backspace => {
                draft.pop();
            }
            KeyCode::Up | KeyCode::Down => *strategy = (*strategy).cycle(),
            KeyCode::Enter => dash.save_filter_draft(),
            KeyCode::Esc => dash.filter_state = FilterState::Managing,
            _ => {}
        }
        return;
    }
    match code {
        KeyCode::Left => dash.move_filter(PickerMove::Left),
        KeyCode::Right => dash.move_filter(PickerMove::Right),
        KeyCode::Up => dash.move_filter(PickerMove::Up),
        KeyCode::Down => dash.move_filter(PickerMove::Down),
        KeyCode::Enter => dash.start_filter_add(),
        KeyCode::Char('e') | KeyCode::Char('E') => dash.start_filter_edit(),
        KeyCode::Char('d') | KeyCode::Char('D') => dash.delete_selected_filter(),
        KeyCode::Esc => dash.filter_state = FilterState::Closed,
        _ => {}
    }
}

fn edit_copy(dash: &mut Dash, code: KeyCode) {
    match code {
        KeyCode::Char(c) if c.is_ascii_digit() => dash.copy_count.push(c),
        KeyCode::Backspace => {
            dash.copy_count.pop();
        }
        KeyCode::Enter => finish_copy(dash),
        KeyCode::Esc => {
            dash.copy_count.clear();
            dash.copying = false;
        }
        _ => {}
    }
}

fn finish_copy(dash: &mut Dash) {
    dash.copying = false;
    let count = dash.copy_count.parse::<usize>().ok().filter(|&n| n > 0);
    dash.copy_count.clear();
    let Some(count) = count else {
        return;
    };
    let text = newest_lines(dash, count);
    let message = match host_facade::copy_to_clipboard(&text) {
        Ok(()) => format!("copied {} lines to clipboard", text.lines().count()),
        Err(error) => format!("copy failed: {error}"),
    };
    dash.notice = Some((Instant::now(), message));
}

fn newest_lines(dash: &Dash, count: usize) -> String {
    let ring = match dash.view {
        View::Trace => Some(&dash.trace.ring),
        View::EmuDetail => emu_detail_ring(dash),
        View::Dashboard
        | View::Logs
        | View::Doctor
        | View::Plugins
        | View::Emu
        | View::Endpoints => Some(&dash.logs.ring),
    };
    let Some(ring) = ring else {
        return String::new();
    };
    let filtered: Vec<&String> = ring
        .lines
        .iter()
        .filter(|line| line_matches_filters(line, dash.active_filters()))
        .collect();
    let start = filtered.len().saturating_sub(count);
    filtered[start..]
        .iter()
        .map(|line| strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_ansi(raw: &str) -> String {
    use ansi_to_tui::IntoText;
    let Ok(text) = raw.into_text() else {
        return raw.to_string();
    };
    text.lines
        .iter()
        .flat_map(|line| line.spans.iter().map(|span| span.content.as_ref()))
        .collect()
}

fn copy_highlight(dash: &Dash) -> Option<usize> {
    if !dash.copying {
        return None;
    }
    dash.copy_count.parse::<usize>().ok().filter(|&n| n > 0)
}

fn poll_key() -> Result<Option<(KeyCode, KeyModifiers)>> {
    if !event::poll(TICK)? {
        return Ok(None);
    }
    let Event::Key(key) = event::read()? else {
        return Ok(None);
    };
    if key.kind != KeyEventKind::Press {
        return Ok(None);
    }
    Ok(Some((key.code, key.modifiers)))
}

fn apply_action(dash: &mut Dash, action: Action, modified: bool) {
    let page = dash.log_height.max(1);
    match action {
        Action::ToggleView => {
            dash.view = match dash.view {
                View::Logs => View::Dashboard,
                _ => View::Logs,
            };
            dash.scroll_offset = 0;
            dash.close_filters();
        }
        Action::ToggleKeys => {
            dash.keys_hidden = !dash.keys_hidden;
            dash.mark_state_dirty();
        }
        Action::ToggleArm => dash.armed = !dash.armed,
        Action::FeatureFlags => dash.toggle_feature_flags_panel(),
        Action::Rebuild => {
            trigger_rebuild(dash);
            trigger_reload(dash);
        }
        Action::Doctor => open_doctor(dash),
        Action::ToggleTraceDetails => toggle_trace_details(dash),
        Action::ToggleTraceRate => toggle_trace_rate(dash),
        Action::Activate => match dash.view {
            View::Dashboard => act_row(dash, modified),
            View::Emu => act_emu(dash, modified),
            View::Plugins => act_plugin(dash),
            View::Logs | View::Doctor | View::Trace | View::Endpoints | View::EmuDetail => {}
        },
        Action::Dive => match dash.view {
            View::Dashboard => dive_row(dash),
            View::Emu => open_emu_detail(dash),
            View::Logs
            | View::Doctor
            | View::Plugins
            | View::Trace
            | View::Endpoints
            | View::EmuDetail => {}
        },
        Action::Back => {
            dash.view = if dash.view == View::EmuDetail {
                View::Emu
            } else {
                View::Dashboard
            };
            dash.emu_detail = None;
            dash.scroll_offset = 0;
            dash.close_filters();
        }
        Action::ScrollUp => match dash.view {
            View::Dashboard => dash.cursor = dash.cursor.saturating_sub(1),
            View::Emu => dash.emu_cursor = dash.emu_cursor.saturating_sub(1),
            View::Plugins => dash.plugin_cursor = dash.plugin_cursor.saturating_sub(1),
            View::Logs | View::Doctor | View::Trace | View::Endpoints | View::EmuDetail => {
                dash.scroll_offset = dash.scroll_offset.saturating_add(1)
            }
        },
        Action::ScrollDown => match dash.view {
            View::Dashboard => dash.cursor = (dash.cursor + 1).min(ROWS.len() - 1),
            View::Emu => {
                let total = emu_env_count(dash) + dash.emu_candidates.len();
                dash.emu_cursor = (dash.emu_cursor + 1).min(total.saturating_sub(1));
            }
            View::Plugins => {
                let total = plugin_row_count(dash);
                dash.plugin_cursor = (dash.plugin_cursor + 1).min(total.saturating_sub(1));
            }
            View::Logs | View::Doctor | View::Trace | View::Endpoints | View::EmuDetail => {
                dash.scroll_offset = dash.scroll_offset.saturating_sub(1)
            }
        },
        Action::PageUp => dash.scroll_offset = dash.scroll_offset.saturating_add(page),
        Action::PageDown => dash.scroll_offset = dash.scroll_offset.saturating_sub(page),
        Action::Follow => dash.scroll_offset = 0,
        Action::Filter => {
            if filterable_view(dash.view) {
                dash.open_filter_manager();
            }
        }
        Action::Copy => {
            if filterable_view(dash.view) {
                dash.copying = true;
                dash.copy_count.clear();
                dash.scroll_offset = 0;
            }
        }
        Action::OpenCurrentLogFolder => open_current_log_folder(dash),
        Action::OpenCurrentLogEditor => open_current_log_editor(dash, false),
        Action::OpenCurrentLogRaw => open_current_log_editor(dash, true),
        Action::OpenEmuDir => {
            if dash.view == View::Emu {
                open_emu_dir();
            }
        }
        Action::ToggleArch => {
            if dash.view == View::Emu {
                let notice = selected_candidate_mut(dash).map(|candidate| {
                    candidate.arch = candidate.arch.toggled();
                    candidate.firmware =
                        crate::commands::emu::Firmware::for_arch(candidate.arch.arch());
                    format!("{} arch {}", candidate.id, candidate.arch.as_str())
                });
                if let Some(notice) = notice {
                    dash.notice = Some((Instant::now(), notice));
                }
            }
        }
        Action::Confirm => {
            if dash.view == View::Emu {
                confirm_selected_candidate(dash);
            }
        }
        Action::Quit | Action::Ignore => {}
    }
    let len = if dash.view == View::Trace {
        dash.trace.len()
    } else if dash.view == View::EmuDetail {
        emu_detail_ring(dash).map_or(0, LogRing::len)
    } else {
        dash.logs.len()
    };
    dash.scroll_offset = clamp_offset(len, dash.log_height, dash.scroll_offset);
}

fn act_row(dash: &mut Dash, modified: bool) {
    match ROWS[dash.cursor] {
        Row::Tray => {
            if !modified {
                trigger_rebuild(dash);
            }
        }
        Row::Web => {
            if !modified {
                host_facade::open_url(WEBSITE_URL);
            }
        }
        Row::Plugins => {
            if !modified {
                trigger_reload(dash);
            }
        }
        Row::Emu => {
            if !modified {
                open_emu(dash);
            }
        }
        Row::Doctor => {
            if modified {
                dash.start_doctor(DoctorMode::Fix);
            } else {
                dash.start_doctor(DoctorMode::Check);
            }
        }
        Row::Logs | Row::Trace => {}
    }
}

fn dive_row(dash: &mut Dash) {
    match ROWS[dash.cursor] {
        Row::Tray => {}
        Row::Web => open_endpoints(dash),
        Row::Plugins => {
            dash.view = View::Plugins;
            dash.scroll_offset = 0;
            dash.plugin_cursor = 0;
            dash.pokes.links = true;
        }
        Row::Emu => open_emu(dash),
        Row::Doctor => open_doctor(dash),
        Row::Logs => {
            dash.view = View::Logs;
            dash.scroll_offset = 0;
        }
        Row::Trace => open_trace(dash),
    }
}

fn open_endpoints(dash: &mut Dash) {
    dash.view = View::Endpoints;
    dash.scroll_offset = 0;
}

fn open_emu(dash: &mut Dash) {
    dash.view = View::Emu;
    dash.scroll_offset = 0;
    dash.pokes.emu = true;
}

fn health_state(up: bool) -> Health {
    if up {
        Health::Up
    } else {
        Health::Down
    }
}

fn apply_doctor_outcome(dash: &mut Dash, outcome: Result<DoctorRun, String>) {
    match outcome {
        Ok(run) => {
            dash.doctor.last = Some(run);
            dash.doctor.last_at_ms = Some(now_unix_ms());
            dash.doctor.error = None;
        }
        Err(error) => dash.doctor.error = Some(error),
    }
}

fn apply_health(dash: &mut Dash, snapshot: HealthSnapshot) {
    let was_up = dash.health == Health::Up;
    dash.health = health_state(snapshot.api);
    dash.web = health_state(snapshot.web);
    if !was_up && dash.health == Health::Up {
        dash.pokes.links = true;
        dash.pokes.doctor = true;
    }
}

fn start_trace(dash: &mut Dash) {
    if dash.trace.is_live() {
        return;
    }
    let renderer = dash.trace_renderer();
    match spawn_trace(renderer, dash.trace_details_enabled()) {
        Some((child, rx)) => {
            dash.trace.attach(child, rx);
            dash.trace_unavailable = false;
        }
        None => {
            if !dash.trace_unavailable {
                dash.trace.push(format!(
                    "[qol dev] could not start {} tracer ({})",
                    renderer.name(),
                    renderer.missing_hint()
                ));
            }
            dash.trace_unavailable = true;
        }
    }
}

fn open_trace(dash: &mut Dash) {
    dash.view = View::Trace;
    dash.scroll_offset = 0;
    start_trace(dash);
}

fn trace_args(details: bool) -> Vec<&'static str> {
    let mut args = vec!["--no-header"];
    if details {
        args.push("--details");
    } else {
        args.push("--no-ghosts");
        args.push("--no-opacity");
    }
    args
}

fn spawn_trace(renderer: TraceRenderer, details: bool) -> Option<(Child, Receiver<String>)> {
    let root = crate::workspace::repo_root().ok()?;
    let mut cmd = trace_command(renderer, &root)?;
    cmd.args(trace_args(details));
    let mut child = cmd
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let rx = spawn_forwarders(&mut child);
    Some((child, rx))
}

fn trace_command(renderer: TraceRenderer, root: &Path) -> Option<Command> {
    let _ = root;
    Some(match renderer {
        TraceRenderer::Rust => {
            let mut cmd = Command::new(std::env::current_exe().ok()?);
            cmd.arg("trace-rs");
            cmd
        }
    })
}

fn stop_trace(dash: &mut Dash) {
    dash.trace.stop();
}

fn toggle_trace_details(dash: &mut Dash) {
    set_trace_details(dash, !dash.trace_details_enabled());
}

fn toggle_trace_rate(dash: &mut Dash) {
    dash.trace_rate = dash.trace_rate.toggled();
    dash.trace.set_collapse(!dash.trace_rate.is_realtime());
    dash.mark_state_dirty();
    dash.notice = Some((Instant::now(), format!("trace {}", dash.trace_rate.label())));
}

fn toggle_feature_flag(flag: FeatureFlag) {
    match flag {}
}

fn set_trace_details(dash: &mut Dash, enabled: bool) {
    if dash.trace_details_enabled() == enabled {
        return;
    }
    dash.trace_details = enabled;
    dash.mark_state_dirty();
    if !dash.trace.is_live() {
        return;
    }
    stop_trace(dash);
    start_trace(dash);
}

const DEFAULT_TRACE_LOG_FILE: &str = "/tmp/qol-altmon.log";

struct LogSourceInfo {
    kind: &'static str,
    file: Option<PathBuf>,
    folder: PathBuf,
    stream_note: &'static str,
}

fn current_log_source(dash: &Dash) -> Option<LogSourceInfo> {
    match dash.view {
        View::Trace => {
            let path = trace_log_file();
            Some(LogSourceInfo {
                kind: "trace",
                folder: path
                    .parent()
                    .unwrap_or_else(|| Path::new("/tmp"))
                    .to_path_buf(),
                file: Some(path),
                stream_note: "raw probe file",
            })
        }
        View::Logs => {
            let file = dash.log_file.as_ref().map(|log_file| log_file.path.clone());
            let folder = file
                .as_ref()
                .and_then(|path| path.parent().map(Path::to_path_buf))
                .unwrap_or_else(dev_log_dir);
            Some(LogSourceInfo {
                kind: "log",
                file,
                folder,
                stream_note: "qol dev stdout/stderr tee",
            })
        }
        _ => None,
    }
}

fn trace_log_file() -> PathBuf {
    std::env::var_os("QOL_TRACE_LOG_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_TRACE_LOG_FILE))
}

fn core_log_dir() -> PathBuf {
    match crate::host_facade::os_name() {
        "macos" => dirs::home_dir()
            .map(|home| home.join("Library/Logs/qol-tray"))
            .unwrap_or_else(|| PathBuf::from("/tmp/qol-tray/logs")),
        "windows" => dirs::data_local_dir()
            .map(|dir| dir.join("qol-tray/logs"))
            .unwrap_or_else(|| PathBuf::from("C:/Temp/qol-tray/logs")),
        _ => qol_config::data_dir()
            .map(|dir| dir.join("logs"))
            .unwrap_or_else(|| PathBuf::from("/tmp/qol-tray/logs")),
    }
}

fn open_current_log_folder(dash: &mut Dash) {
    let Some(source) = current_log_source(dash) else {
        return;
    };
    crate::host_facade::open_path(&source.folder);
    dash.notice = Some((
        Instant::now(),
        format!("opened {} folder {}", source.kind, source.folder.display()),
    ));
}

fn open_current_log_editor(dash: &mut Dash, raw: bool) {
    let Some(source) = current_log_source(dash) else {
        return;
    };
    let Some(ref file) = source.file else {
        dash.notice = Some((
            Instant::now(),
            format!(
                "no persisted {} file yet in {}",
                source.kind,
                source.folder.display()
            ),
        ));
        return;
    };
    if !file.exists() {
        dash.notice = Some((Instant::now(), format!("{} does not exist", file.display())));
        return;
    }
    let target = match editor_target_for_source(dash, &source, file, raw) {
        Ok(target) => target,
        Err(error) => {
            dash.notice = Some((Instant::now(), error));
            return;
        }
    };
    let message = if open_with_os_default(&target) {
        format!("opened {}", target.display())
    } else {
        format!("could not open {}", target.display())
    };
    dash.notice = Some((Instant::now(), message));
}

fn editor_target_for_source(
    dash: &Dash,
    source: &LogSourceInfo,
    file: &Path,
    raw: bool,
) -> Result<PathBuf, String> {
    if source.kind != "trace" || raw {
        return Ok(file.to_path_buf());
    }
    render_pretty_trace_snapshot(file, dash.trace_renderer())
}

fn render_pretty_trace_snapshot(file: &Path, renderer: TraceRenderer) -> Result<PathBuf, String> {
    let root = crate::workspace::repo_root().map_err(|error| error.to_string())?;
    let pretty = pretty_trace_file(file);
    let mut cmd = trace_command(renderer, &root)
        .ok_or_else(|| format!("could not create {} trace renderer", renderer.name()))?;
    cmd.arg("--replay")
        .arg("--details")
        .env("QOL_TRACE_LOG_FILE", file);
    let output = cmd
        .current_dir(&root)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("could not render pretty trace: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        let suffix = if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        };
        return Err(format!("pretty trace failed{suffix}"));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    fs::write(&pretty, strip_ansi_codes(&text))
        .map_err(|error| format!("could not write {}: {error}", pretty.display()))?;
    Ok(pretty)
}

fn pretty_trace_file(file: &Path) -> PathBuf {
    let folder = file.parent().unwrap_or_else(|| Path::new("."));
    let stem = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("trace");
    folder.join(format!("{stem}.pretty.log"))
}

fn strip_ansi_codes(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for code in chars.by_ref() {
                if ('@'..='~').contains(&code) {
                    break;
                }
            }
            continue;
        }
        output.push(ch);
    }
    output
}

fn open_with_os_default(path: &Path) -> bool {
    match crate::host_facade::os_name() {
        "macos" => Command::new("open")
            .arg("-t")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok(),
        "windows" => Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .spawn()
            .is_ok(),
        _ => Command::new("xdg-open")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok(),
    }
}

fn emu_env_count(dash: &Dash) -> usize {
    match &dash.emu {
        EmuState::Done(statuses) => statuses.len(),
        EmuState::Probing | EmuState::Failed(_) => 0,
    }
}

fn selected_emu_status(dash: &Dash) -> Option<&EnvironmentStatus> {
    match &dash.emu {
        EmuState::Done(statuses) => statuses.get(dash.emu_cursor),
        EmuState::Probing | EmuState::Failed(_) => None,
    }
}

fn selected_candidate_mut(dash: &mut Dash) -> Option<&mut ImageCandidate> {
    dash.emu_cursor
        .checked_sub(emu_env_count(dash))
        .and_then(|index| dash.emu_candidates.get_mut(index))
}

fn selected_candidate(dash: &Dash) -> Option<&ImageCandidate> {
    dash.emu_cursor
        .checked_sub(emu_env_count(dash))
        .and_then(|index| dash.emu_candidates.get(index))
}

fn is_running(dash: &Dash, id: &str) -> bool {
    dash.active_runs.get(id).is_some_and(LogPane::is_live)
}

fn act_emu(dash: &mut Dash, modified: bool) {
    if let Some((id, ready)) = selected_emu_status(dash)
        .map(|status| (status.id.clone(), status.state == ResolveState::Ready))
    {
        if is_running(dash, &id) {
            fire_emu_down(dash, &id);
        } else if ready {
            let verb = if modified { "check" } else { "up" };
            launch_emu(dash, verb, id);
        }
        return;
    }
    let Some(id) = selected_candidate(dash).map(|candidate| candidate.id.clone()) else {
        return;
    };
    if is_running(dash, &id) {
        fire_emu_down(dash, &id);
    } else {
        launch_emu(dash, "up", id);
    }
}

fn launch_emu(dash: &mut Dash, verb: &'static str, id: String) {
    let mut pane = LogPane::new();
    match spawn_emu_verb(verb, &id) {
        Some((child, rx)) => pane.attach(child, rx),
        None => pane.push(emu_run_line(
            "error",
            &format!("could not launch qol emu {verb} {id}"),
        )),
    }
    dash.active_runs.insert(id, pane);
}

fn emu_run_line(verb: &str, detail: &str) -> String {
    format!("  {verb:<9}{detail}")
}

fn keep_emu_line(line: &str) -> bool {
    let trimmed = line.trim();
    !(trimmed.is_empty() || trimmed.starts_with("qol emu") || trimmed.starts_with("hint:"))
}

fn spawn_emu_verb(verb: &str, id: &str) -> Option<(Child, Receiver<String>)> {
    let exe = std::env::current_exe().ok()?;
    let root = crate::workspace::repo_root().ok()?;
    let mut child = Command::new(exe)
        .args(["emu", verb, id])
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let rx = spawn_forwarders(&mut child);
    Some((child, rx))
}

fn fire_emu_down(dash: &mut Dash, id: &str) {
    let line = match spawn_emu_verb("down", id) {
        Some((mut child, _)) => {
            let _ = child.wait();
            emu_run_line("down", &format!("sent to {id}"))
        }
        None => emu_run_line("error", &format!("could not send down to {id}")),
    };
    if let Some(pane) = dash.active_runs.get_mut(id) {
        pane.push(line);
    }
}

fn open_emu_dir() {
    let Some(dir) = emu_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    crate::host_facade::open_path(&dir);
}

fn confirm_selected_candidate(dash: &mut Dash) {
    let Some(emu_toml) = emu_config_path() else {
        return;
    };
    let Some(candidate) = selected_candidate(dash).cloned() else {
        return;
    };
    let qemu_img = crate::commands::emu::find_on_path("qemu-img");
    match crate::commands::emu::register_image(&emu_toml, &candidate, qemu_img.as_deref()) {
        Ok(id) => {
            dash.notice = Some((Instant::now(), format!("registered {id}")));
            dash.pokes.emu = true;
        }
        Err(error) => {
            dash.notice = Some((Instant::now(), candidate_register_error(&error.to_string())));
        }
    }
}

fn candidate_register_error(message: &str) -> String {
    if message == "arch unconfirmed" {
        return "arch unconfirmed · press t to set arch, then a".to_string();
    }
    message.to_string()
}

fn drain_emu_runs(dash: &mut Dash) {
    let mut finished = false;
    for pane in dash.active_runs.values_mut() {
        if pane.poll_finished(keep_emu_line) {
            finished = true;
        }
    }
    if finished {
        dash.pokes.emu = true;
        dash.pokes.doctor = true;
    }
}

fn stop_emu_runs(dash: &mut Dash) {
    let live: Vec<String> = dash
        .active_runs
        .iter()
        .filter(|(_, pane)| pane.is_live())
        .map(|(id, _)| id.clone())
        .collect();
    for id in live {
        fire_emu_down(dash, &id);
        if let Some(pane) = dash.active_runs.get_mut(&id) {
            pane.stop_graceful();
        }
    }
}

fn open_emu_detail(dash: &mut Dash) {
    if let Some(status) = selected_emu_status(dash).cloned() {
        let detail = newest_run_detail(&status.id);
        let info = emu_info_lines(&status, detail.as_ref());
        set_emu_detail(dash, status.id, info, detail);
        return;
    }
    let Some(candidate) = selected_candidate(dash).cloned() else {
        return;
    };
    let detail = newest_run_detail(&candidate.id);
    let info = candidate_info_lines(&candidate, detail.as_ref());
    set_emu_detail(dash, candidate.id, info, detail);
}

fn set_emu_detail(
    dash: &mut Dash,
    id: String,
    info: Vec<Line<'static>>,
    detail: Option<RunDetail>,
) {
    let replay = if dash.active_runs.contains_key(&id) {
        None
    } else {
        detail.as_ref().map(|d| LogPane::replay(&d.run_log()))
    };
    dash.emu_detail = Some(EmuDetail { id, info, replay });
    dash.view = View::EmuDetail;
    dash.scroll_offset = 0;
    dash.close_filters();
}

fn emu_detail_ring(dash: &Dash) -> Option<&LogRing> {
    let detail = dash.emu_detail.as_ref()?;
    if let Some(pane) = dash.active_runs.get(&detail.id) {
        return Some(&pane.ring);
    }
    detail.replay.as_ref().map(|pane| &pane.ring)
}

fn live_verb(dash: &Dash, id: &str) -> Option<String> {
    let pane = dash.active_runs.get(id)?;
    if !pane.is_live() {
        return None;
    }
    let latest = pane.ring.lines.back()?;
    Some(
        latest
            .split_whitespace()
            .next()
            .unwrap_or("running")
            .to_string(),
    )
}

fn state_color(state: ResolveState) -> Color {
    match state {
        ResolveState::Ready => Color::Green,
        ResolveState::Missing => Color::Yellow,
        ResolveState::Unsupported => Color::Red,
    }
}

fn emu_info_lines(status: &EnvironmentStatus, detail: Option<&RunDetail>) -> Vec<Line<'static>> {
    let color = state_color(status.state);
    let mut head = vec![
        "● ".fg(color).bold(),
        status.state.as_str().fg(color).bold(),
        format!(" · {}", status.backend).fg(Color::DarkGray),
    ];
    if let Some(detail) = detail {
        head.push(format!(" · {}", detail.arch).fg(Color::DarkGray));
    }
    head.extend(last_run_spans(status.last_run.as_ref()));
    let mut lines = vec![Line::from(head)];
    if status.state != ResolveState::Ready {
        lines.push(Line::from(vec![
            "  ".into(),
            status.reason.clone().fg(Color::DarkGray),
        ]));
    }
    match detail {
        Some(detail) => {
            lines.push(info_row("image", &detail.image_path));
            lines.push(info_row("accel", &detail.acceleration));
            lines.push(info_row("run dir", &detail.run_dir.display().to_string()));
        }
        None => lines.push(Line::from("  no runs yet".fg(Color::DarkGray))),
    }
    lines
}

fn candidate_info_lines(
    candidate: &ImageCandidate,
    detail: Option<&RunDetail>,
) -> Vec<Line<'static>> {
    let mut head = vec![
        "○ ".fg(Color::Green).bold(),
        "ready".fg(Color::Green).bold(),
        " · candidate".fg(Color::DarkGray),
    ];
    if let Some(detail) = detail {
        head.push(format!(" · {}", detail.arch).fg(Color::DarkGray));
    }
    let mut lines = vec![Line::from(head)];
    match candidate.arch {
        crate::commands::emu::ArchGuess::Known(arch) => {
            lines.push(info_row("arch", arch.as_str()));
        }
        crate::commands::emu::ArchGuess::Assumed(arch) => {
            lines.push(info_row("arch", &format!("assumed {}", arch.as_str())));
            lines.push(Line::from(
                "  press t to set arch, then a to add".fg(Color::DarkGray),
            ));
        }
    }
    match detail {
        Some(detail) => {
            lines.push(info_row("image", &detail.image_path));
            lines.push(info_row("accel", &detail.acceleration));
            lines.push(info_row("run dir", &detail.run_dir.display().to_string()));
        }
        None => {
            lines.push(info_row("image", &candidate.path.display().to_string()));
            lines.push(Line::from("  no runs yet".fg(Color::DarkGray)));
        }
    }
    lines
}

fn info_row(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        format!("  {label:<8} ").fg(Color::White),
        value.to_string().fg(Color::DarkGray),
    ])
}

fn start_reload(dash: &mut Dash) {
    if dash.is_reloading() {
        return;
    }
    match spawn_reload() {
        Some((child, rx)) => {
            dash.push_log("[qol dev] reloading: qol setup");
            dash.reload = Reload::Running { child, rx };
        }
        None => dash.push_log("[qol dev] reload failed to start"),
    }
}

fn spawn_reload() -> Option<(Child, Receiver<String>)> {
    let exe = std::env::current_exe().ok()?;
    let root = crate::workspace::repo_root().ok()?;
    let mut child = Command::new(exe)
        .arg("setup")
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let rx = spawn_forwarders(&mut child);
    Some((child, rx))
}

fn poll_reload(dash: &mut Dash) -> ReloadOutcome {
    let mut drained = Vec::new();
    let status = match &mut dash.reload {
        Reload::Idle => return ReloadOutcome::Pending,
        Reload::Running { child, rx, .. } => {
            while let Ok(line) = rx.try_recv() {
                drained.push(line);
            }
            match child.try_wait() {
                Ok(Some(status)) => status,
                _ => {
                    for line in drained {
                        dash.push_log(line);
                    }
                    return ReloadOutcome::Pending;
                }
            }
        }
    };
    for line in drained {
        dash.push_log(line);
    }
    dash.reload = Reload::Idle;
    if status.success() {
        return ReloadOutcome::Ready;
    }
    dash.push_log(format!("[qol dev] reload aborted: qol setup {status}"));
    ReloadOutcome::Pending
}

fn trigger_rebuild(dash: &mut Dash) {
    dash.rebuild = match post_recompile_current() {
        Ok(()) => {
            dash.pokes.doctor = true;
            RebuildState::Requested(Instant::now())
        }
        Err(error) => RebuildState::Failed(format!("{error:#}")),
    };
}

fn trigger_reload(dash: &mut Dash) {
    dash.plugin_reload = match post_reload_plugins() {
        Ok(()) => {
            dash.pokes.links = true;
            dash.pokes.doctor = true;
            RebuildState::Requested(Instant::now())
        }
        Err(error) => RebuildState::Failed(format!("{error:#}")),
    };
}

fn open_doctor(dash: &mut Dash) {
    dash.view = View::Doctor;
    dash.scroll_offset = 0;
    dash.pokes.doctor = true;
}

fn draw(frame: &mut Frame, dash: &mut Dash) {
    let accent = frame_accent(dash);
    let [_, body] =
        Layout::vertical([Constraint::Length(TITLE_CAP), Constraint::Min(0)]).areas(frame.area());
    let block = Block::bordered()
        .border_style(Style::new().fg(accent))
        .padding(PANEL_PADDING);
    let inner = block.inner(body);
    frame.render_widget(block, body);
    let content = page_header(frame, dash.view, inner);
    match dash.view {
        View::Dashboard => draw_dashboard(frame, dash, content),
        View::Logs => draw_logs(frame, dash, content),
        View::Doctor => draw_doctor(frame, dash, content),
        View::Plugins => draw_plugins(frame, dash, content),
        View::Emu => draw_emu(frame, dash, content),
        View::EmuDetail => draw_emu_detail(frame, dash, content),
        View::Trace => draw_trace(frame, dash, content),
        View::Endpoints => draw_endpoints(frame, dash, content),
    }
    draw_filter_panel(frame, dash, inner, accent);
    draw_feature_flags_panel(frame, dash, inner, accent);
    Sign {
        content: breadcrumb(dash, accent),
    }
    .render(frame, body, accent);
    draw_keys_hud(frame, dash, inner);
}

fn page_header(frame: &mut Frame, view: View, inner: Rect) -> Rect {
    let Some(desc) = page_description(view) else {
        return inner;
    };
    frame.render_widget(
        Paragraph::new(Line::from(format!("  {desc}").fg(Color::DarkGray))),
        Rect { height: 1, ..inner },
    );
    Rect {
        y: inner.y + 2,
        height: inner.height.saturating_sub(2),
        ..inner
    }
}

fn page_description(view: View) -> Option<&'static str> {
    match view {
        View::Logs => Some("live daemon logs"),
        View::Trace => Some("runtime trace events"),
        View::Doctor => Some("install health checks"),
        View::Plugins => Some("workspace plugins · enter to link/unlink"),
        View::Emu => Some("clean-os test envs"),
        View::Endpoints => Some("local service endpoints"),
        View::Dashboard | View::EmuDetail => None,
    }
}

fn filterable_view(view: View) -> bool {
    filter_scope(view).is_some()
}

fn filters_visible(dash: &Dash) -> bool {
    filterable_view(dash.view) && !dash.active_filters().is_empty()
}

fn breadcrumb(dash: &Dash, accent: Color) -> Line<'static> {
    let trail: Vec<String> = match dash.view {
        View::Dashboard => Vec::new(),
        View::Logs => vec!["logs".to_string()],
        View::Trace => vec!["trace".to_string()],
        View::Doctor => vec!["doctor".to_string()],
        View::Plugins => vec!["plugins".to_string()],
        View::Emu => vec!["emu".to_string()],
        View::Endpoints => vec!["endpoints".to_string()],
        View::EmuDetail => {
            let id = dash
                .emu_detail
                .as_ref()
                .map(|detail| detail.id.clone())
                .unwrap_or_default();
            vec!["emu".to_string(), id]
        }
    };
    let mut spans: Vec<Span<'static>> = Vec::new();
    if trail.is_empty() {
        spans.push("qol dev".fg(accent).bold());
    } else {
        spans.push("qol dev".fg(Color::DarkGray));
        let last = trail.len() - 1;
        for (index, segment) in trail.into_iter().enumerate() {
            spans.push(" · ".fg(Color::DarkGray));
            if index == last {
                spans.push(segment.fg(accent).bold());
            } else {
                spans.push(segment.fg(Color::DarkGray));
            }
        }
    }
    if filters_visible(dash) {
        spans.push(" · FILTERED".fg(Color::Yellow).bold());
    }
    if dash.is_reloading() {
        spans.push(" · RELOADING".fg(Color::Red).bold());
    } else if dash.armed {
        spans.push(" · ARMED".fg(Color::Yellow).bold());
    }
    Line::from(spans)
}

const KEYS_HUD_WIDTH: u16 = 34;

fn context_keys(dash: &Dash) -> Vec<KeyHint> {
    if dash.copying {
        return vec![
            KeyHint {
                key: "digits",
                desc: "line count",
            },
            KeyHint {
                key: "enter",
                desc: "copy",
            },
            KeyHint {
                key: "esc",
                desc: "cancel",
            },
        ];
    }
    if dash.feature_panel.is_active() {
        return vec![
            KeyHint {
                key: "←↑↓→",
                desc: "select flag",
            },
            KeyHint {
                key: "space",
                desc: "toggle",
            },
            KeyHint {
                key: "enter",
                desc: "toggle",
            },
            KeyHint {
                key: "esc",
                desc: "close",
            },
        ];
    }
    match &dash.filter_state {
        FilterState::Managing => {
            return vec![
                KeyHint {
                    key: "←↑↓→",
                    desc: "select filter",
                },
                KeyHint {
                    key: "enter",
                    desc: "add",
                },
                KeyHint {
                    key: "e",
                    desc: "edit",
                },
                KeyHint {
                    key: "d",
                    desc: "delete",
                },
                KeyHint {
                    key: "esc",
                    desc: "close",
                },
            ];
        }
        FilterState::Editing { .. } => {
            return vec![
                KeyHint {
                    key: "type",
                    desc: "filter text",
                },
                KeyHint {
                    key: "↑/↓",
                    desc: "strategy + / -",
                },
                KeyHint {
                    key: "enter",
                    desc: "save",
                },
                KeyHint {
                    key: "esc",
                    desc: "cancel",
                },
            ];
        }
        FilterState::Closed => {}
    }
    unique_hints(context_action_bindings(dash))
}

fn global_keys(armed: bool) -> Vec<KeyHint> {
    unique_hints(global_action_bindings(armed))
}

fn key_lines(keys: &[KeyHint]) -> Vec<Line<'static>> {
    keys.iter()
        .map(|hint| {
            Line::from(vec![
                format!(" {:<9} ", hint.key).fg(Color::White).bold(),
                format!("{} ", hint.desc).fg(Color::DarkGray),
            ])
        })
        .collect()
}

fn section_label(label: &'static str) -> Line<'static> {
    Line::from(format!(" {label}").fg(Color::Green).bold())
}

fn keys_rows(dash: &Dash) -> Vec<Line<'static>> {
    let mut rows = vec![section_label("global")];
    rows.push(Line::from(""));
    rows.extend(key_lines(&global_keys(dash.armed)));
    rows.push(Line::from(""));
    rows.push(Line::from(""));
    rows.push(section_label("context"));
    rows.push(Line::from(""));
    rows.extend(key_lines(&context_keys(dash)));
    rows
}

fn draw_keys_hud(frame: &mut Frame, dash: &Dash, area: Rect) {
    if dash.keys_hidden {
        return;
    }
    let rows = keys_rows(dash);
    let height = (rows.len() as u16 + SignBox::CHROME_ROWS).min(area.height);
    if height == 0 {
        return;
    };
    let rect = Rect {
        x: area.x + area.width.saturating_sub(KEYS_HUD_WIDTH),
        y: area.y,
        width: KEYS_HUD_WIDTH.min(area.width),
        height,
    };
    frame.render_widget(Clear, rect);
    SignBox {
        title: "keys · k",
        rows,
    }
    .render(frame, rect, frame_accent(dash));
}

fn draw_filter_panel(frame: &mut Frame, dash: &mut Dash, area: Rect, accent: Color) {
    if !dash.filter_state.is_active() {
        return;
    }
    let width = if area.width <= FILTER_PANEL_MIN_WIDTH {
        area.width
    } else {
        area.width
            .saturating_sub(4)
            .clamp(FILTER_PANEL_MIN_WIDTH, FILTER_PANEL_MAX_WIDTH)
    };
    dash.filter_layout_width = width.saturating_sub(2) as usize;
    let mut rows = filter_panel_rows(dash);
    let height = (rows.len() as u16 + SignBox::CHROME_ROWS).min(area.height);
    if width == 0 || height == 0 {
        return;
    }
    rows.truncate(SignBox::capacity(height));
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + area.height.saturating_sub(height + 1),
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    SignBox {
        title: "filters",
        rows,
    }
    .render(frame, rect, accent);
}

fn draw_feature_flags_panel(frame: &mut Frame, dash: &mut Dash, area: Rect, accent: Color) {
    if !dash.feature_panel.is_active() {
        return;
    }
    let width = if area.width <= FILTER_PANEL_MIN_WIDTH {
        area.width
    } else {
        area.width
            .saturating_sub(4)
            .clamp(FILTER_PANEL_MIN_WIDTH, FILTER_PANEL_MAX_WIDTH)
    };
    dash.feature_panel.layout_width = width.saturating_sub(2) as usize;
    let mut rows = feature_flag_panel_rows(dash);
    let height = (rows.len() as u16 + SignBox::CHROME_ROWS).min(area.height);
    if width == 0 || height == 0 {
        return;
    }
    rows.truncate(SignBox::capacity(height));
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + area.height.saturating_sub(height + 1),
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    SignBox {
        title: "feature flags",
        rows,
    }
    .render(frame, rect, accent);
}

fn filter_panel_rows(dash: &Dash) -> Vec<Line<'static>> {
    match &dash.filter_state {
        FilterState::Closed => Vec::new(),
        FilterState::Managing if dash.active_filters().is_empty() => vec![
            Line::from(" no filters".fg(Color::DarkGray)),
            Line::from(" enter add".fg(Color::DarkGray)),
        ],
        FilterState::Managing => filter_brick_rows(
            dash.active_filters(),
            dash.filter_index,
            dash.filter_layout_width,
        ),
        FilterState::Editing {
            index,
            draft,
            strategy,
        } => {
            let label = if index.is_some() { " edit" } else { " add" };
            vec![Line::from(vec![
                label.fg(Color::DarkGray),
                " ".into(),
                strategy.symbol().fg(strategy.color()).bold(),
                " ".into(),
                format!("{draft}_").fg(Color::White),
            ])]
        }
    }
}

fn feature_flag_panel_rows(dash: &Dash) -> Vec<Line<'static>> {
    if FEATURE_FLAGS.is_empty() {
        return vec![Line::from(" no feature flags".fg(Color::DarkGray))];
    }
    let mut rows = feature_flag_brick_rows(dash);
    if let Some(def) = FEATURE_FLAGS.get(dash.feature_panel.selected) {
        let state = if dash.features.enabled(def.flag) {
            "on"
        } else {
            "off"
        };
        rows.push(Line::from(""));
        rows.push(Line::from(vec![
            format!(" {state:<3} ")
                .fg(feature_flag_color(dash, def))
                .bold(),
            def.label.fg(Color::White),
        ]));
    }
    rows
}

fn feature_flag_brick_rows(dash: &Dash) -> Vec<Line<'static>> {
    let layout = feature_flag_brick_layout(dash.feature_panel.layout_width);
    let Some(max_row) = layout.iter().map(|brick| brick.row).max() else {
        return Vec::new();
    };
    (0..=max_row)
        .map(|row| feature_flag_brick_row(dash, &layout, row))
        .collect()
}

fn feature_flag_brick_row(dash: &Dash, layout: &[PickerBrick], row: usize) -> Line<'static> {
    let mut spans = Vec::new();
    let mut x = 0;
    for brick in layout.iter().filter(|brick| brick.row == row) {
        if brick.x > x {
            spans.push(Span::raw(" ".repeat(brick.x - x)));
        }
        let def = &FEATURE_FLAGS[brick.index];
        spans.extend(feature_flag_brick_spans(
            dash,
            def,
            brick.index == dash.feature_panel.selected,
            brick.width,
        ));
        x = brick.x + brick.width;
    }
    Line::from(spans)
}

fn filter_brick_rows(
    filters: &[LogFilter],
    selected_index: usize,
    width: usize,
) -> Vec<Line<'static>> {
    let layout = filter_brick_layout(filters, width);
    let Some(max_row) = layout.iter().map(|brick| brick.row).max() else {
        return Vec::new();
    };
    (0..=max_row)
        .map(|row| filter_brick_row(filters, selected_index, &layout, row))
        .collect()
}

fn filter_brick_row(
    filters: &[LogFilter],
    selected_index: usize,
    layout: &[PickerBrick],
    row: usize,
) -> Line<'static> {
    let mut spans = Vec::new();
    let mut x = 0;
    for brick in layout.iter().filter(|brick| brick.row == row) {
        if brick.x > x {
            spans.push(Span::raw(" ".repeat(brick.x - x)));
        }
        let filter = &filters[brick.index];
        spans.extend(filter_brick_spans(
            filter,
            brick.index == selected_index,
            brick.width,
        ));
        x = brick.x + brick.width;
    }
    Line::from(spans)
}

fn filter_brick_layout(filters: &[LogFilter], width: usize) -> Vec<PickerBrick> {
    picker_brick_layout(filters, width, filter_brick_width)
}

fn feature_flag_brick_layout(width: usize) -> Vec<PickerBrick> {
    picker_brick_layout(FEATURE_FLAGS, width, feature_flag_brick_width)
}

fn picker_brick_layout<T>(
    items: &[T],
    width: usize,
    mut item_width: impl FnMut(&T, usize) -> usize,
) -> Vec<PickerBrick> {
    let width = width.max(1);
    let mut row = 0;
    let mut x = 0;
    let mut layout = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let brick_width = item_width(item, width);
        if x > 0 && x + FILTER_BRICK_GAP + brick_width > width {
            row += 1;
            x = 0;
        }
        let gap = if x == 0 { 0 } else { FILTER_BRICK_GAP };
        let brick_x = x + gap;
        layout.push(PickerBrick {
            index,
            row,
            x: brick_x,
            width: brick_width,
        });
        x = brick_x + brick_width;
    }
    layout
}

fn move_picker_selection(
    selected: &mut usize,
    item_count: usize,
    direction: PickerMove,
    layout: &[PickerBrick],
) {
    if item_count == 0 {
        *selected = 0;
        return;
    }
    if matches!(direction, PickerMove::Left | PickerMove::Right) {
        let len = item_count as isize;
        let delta = if matches!(direction, PickerMove::Left) {
            -1
        } else {
            1
        };
        *selected = (*selected as isize + delta).rem_euclid(len) as usize;
        return;
    }
    let Some(current) = layout.iter().find(|brick| brick.index == *selected) else {
        *selected = 0;
        return;
    };
    let Some(max_row) = layout.iter().map(|brick| brick.row).max() else {
        return;
    };
    let target_row = match direction {
        PickerMove::Up if current.row == 0 => max_row,
        PickerMove::Up => current.row - 1,
        PickerMove::Down if current.row == max_row => 0,
        PickerMove::Down => current.row + 1,
        PickerMove::Left | PickerMove::Right => current.row,
    };
    let center = brick_center(current);
    let Some(target) = layout
        .iter()
        .filter(|brick| brick.row == target_row)
        .min_by_key(|brick| {
            (
                brick_center(brick).abs_diff(center),
                brick.index.abs_diff(current.index),
            )
        })
    else {
        return;
    };
    *selected = target.index;
}

fn filter_brick_width(filter: &LogFilter, row_width: usize) -> usize {
    let max_text_width = filter_text_width(row_width);
    FILTER_BRICK_CHROME + filter_text(&filter.text, max_text_width).chars().count()
}

fn feature_flag_brick_width(flag: &FeatureFlagDef, row_width: usize) -> usize {
    let max_text_width = filter_text_width(row_width);
    FILTER_BRICK_CHROME + filter_text(flag.id, max_text_width).chars().count()
}

fn feature_flag_brick_spans(
    dash: &Dash,
    flag: &FeatureFlagDef,
    selected: bool,
    width: usize,
) -> Vec<Span<'static>> {
    let text = filter_text(flag.id, filter_text_width(width));
    let enabled = dash.features.enabled(flag.flag);
    let text_style = if selected {
        Style::new().fg(Color::White).bg(Color::Rgb(38, 44, 74))
    } else if enabled {
        Style::new().fg(Color::White)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    let symbol_style = if selected {
        Style::new()
            .fg(feature_flag_color(dash, flag))
            .bg(Color::Rgb(38, 44, 74))
            .bold()
    } else {
        Style::new().fg(feature_flag_color(dash, flag)).bold()
    };
    let edge = if selected { ("[", "]") } else { (" ", " ") };
    let symbol = if enabled { "*" } else { "." };
    vec![
        Span::styled(edge.0.to_string(), text_style),
        Span::styled(symbol.to_string(), symbol_style),
        Span::styled(" ".to_string(), text_style),
        Span::styled(text, text_style),
        Span::styled(edge.1.to_string(), text_style),
    ]
}

fn feature_flag_color(dash: &Dash, flag: &FeatureFlagDef) -> Color {
    if dash.features.enabled(flag.flag) {
        Color::Green
    } else {
        Color::DarkGray
    }
}

fn filter_brick_spans(filter: &LogFilter, selected: bool, width: usize) -> Vec<Span<'static>> {
    let text = filter_text(&filter.text, filter_text_width(width));
    let text_style = if selected {
        Style::new().fg(Color::White).bg(Color::Rgb(38, 44, 74))
    } else {
        Style::new().fg(Color::DarkGray)
    };
    let symbol_style = if selected {
        Style::new()
            .fg(filter.strategy.color())
            .bg(Color::Rgb(38, 44, 74))
            .bold()
    } else {
        Style::new().fg(filter.strategy.color()).bold()
    };
    let edge = if selected { ("[", "]") } else { (" ", " ") };
    vec![
        Span::styled(edge.0.to_string(), text_style),
        Span::styled(filter.strategy.symbol().to_string(), symbol_style),
        Span::styled(" ".to_string(), text_style),
        Span::styled(text, text_style),
        Span::styled(edge.1.to_string(), text_style),
    ]
}

fn filter_text(raw: &str, max_width: usize) -> String {
    raw.chars().take(max_width.max(1)).collect()
}

fn filter_text_width(row_width: usize) -> usize {
    row_width.saturating_sub(FILTER_BRICK_CHROME).max(1)
}

fn brick_center(brick: &PickerBrick) -> usize {
    brick.x.saturating_mul(2) + brick.width
}

fn default_filter_layout_width() -> usize {
    FILTER_PANEL_MAX_WIDTH.saturating_sub(2) as usize
}

struct SignBox<'a> {
    title: &'a str,
    rows: Vec<Line<'a>>,
}

impl SignBox<'_> {
    const CHROME_ROWS: u16 = 4;

    fn capacity(height: u16) -> usize {
        height.saturating_sub(Self::CHROME_ROWS) as usize
    }

    fn render(self, frame: &mut Frame, area: Rect, accent: Color) {
        let body = Rect {
            y: area.y + 1,
            height: area.height.saturating_sub(1),
            ..area
        };
        let block = Block::bordered().border_style(Style::new().fg(accent));
        let inner = block.inner(body);
        frame.render_widget(block, body);
        let rows_area = Rect {
            y: inner.y + 1,
            height: inner.height.saturating_sub(1),
            ..inner
        };
        frame.render_widget(Paragraph::new(self.rows), rows_area);
        Sign {
            content: Line::from(self.title.to_string().fg(accent).bold()),
        }
        .render(frame, body, accent);
    }
}

const ITEM_GAP: u16 = 1;

fn space_rows(rows: Vec<Line>, gap: u16) -> Vec<Line> {
    if gap == 0 || rows.len() <= 1 {
        return rows;
    }
    let last = rows.len() - 1;
    let mut spaced = Vec::with_capacity(rows.len() + last * gap as usize);
    for (index, row) in rows.into_iter().enumerate() {
        spaced.push(row);
        if index != last {
            spaced.extend((0..gap).map(|_| Line::from("")));
        }
    }
    spaced
}

fn spaced_height(items: usize, gap: u16) -> u16 {
    items as u16 + items.saturating_sub(1) as u16 * gap
}

fn list_capacity(height: u16) -> usize {
    (height as usize + ITEM_GAP as usize) / (1 + ITEM_GAP as usize)
}

fn view_content(frame: &mut Frame, area: Rect, lines: Vec<Line>) {
    frame.render_widget(Paragraph::new(space_rows(lines, ITEM_GAP)), area);
}

fn draw_dashboard(frame: &mut Frame, dash: &Dash, area: Rect) {
    let (tray_color, tray_value) = tray_status(dash);
    let (web_color, web_value) = web_status(dash.web);
    let (plugins_color, plugins_value) =
        plugins_status(&dash.plugin_reload, dash.plugin_names.len(), &dash.links);
    let (emu_color, emu_value) = emu_status(&dash.emu);
    let (doctor_color, doctor_value) = doctor_status(&dash.doctor, now_unix_ms());

    let rows = vec![
        dash_row(dash.cursor == 0, tray_color, "tray", tray_value),
        dash_row(dash.cursor == 1, web_color, "web", web_value),
        dash_row(dash.cursor == 2, plugins_color, "plugins", plugins_value),
        dash_row(dash.cursor == 3, emu_color, "emu", emu_value),
        dash_row(dash.cursor == 4, doctor_color, "doctor", doctor_value),
        dash_row(
            dash.cursor == 5,
            Color::DarkGray,
            "logs",
            vec![format!("{} buffered", dash.logs.len()).fg(Color::DarkGray)],
        ),
        dash_row(
            dash.cursor == 6,
            Color::DarkGray,
            "trace",
            trace_value(dash),
        ),
    ];

    view_content(frame, area, rows);
}

struct Sign {
    content: Line<'static>,
}

impl Sign {
    fn render(self, frame: &mut Frame, body: Rect, accent: Color) {
        let span = self.content.width() as u16 + 2;
        let width = span + 2;
        if width + 2 > body.width {
            return;
        }
        let x = body.x + (body.width - width) / 2;
        let bar = "─".repeat(span as usize);
        render_overlay(
            frame,
            x,
            body.y.saturating_sub(1),
            Line::from(format!("╭{bar}╮").fg(accent)),
        );
        let mut middle = vec!["┤ ".fg(accent)];
        middle.extend(self.content.spans);
        middle.push(" ├".fg(accent));
        render_overlay(frame, x, body.y, Line::from(middle));
        render_overlay(
            frame,
            x,
            body.y + 1,
            Line::from(format!("╰{bar}╯").fg(accent)),
        );
    }
}

fn render_overlay(frame: &mut Frame, x: u16, y: u16, line: Line<'static>) {
    let width = line.width() as u16;
    frame.render_widget(Paragraph::new(line), Rect::new(x, y, width, 1));
}

fn tray_status(dash: &Dash) -> (Color, Vec<Span<'static>>) {
    let (text, color) = match dash.health {
        Health::Checking => ("starting", Color::Yellow),
        Health::Up => ("running", Color::Green),
        Health::Down => ("down", Color::Red),
    };
    let mut value = vec![
        text.fg(color).bold(),
        format!(" · up {}", format_duration(dash.started.elapsed())).fg(Color::DarkGray),
    ];
    if dash.health == Health::Up {
        value.push(" · api ✓".fg(Color::Green));
    }
    match &dash.rebuild {
        RebuildState::Requested(at) if at.elapsed() < ACK_TTL => {
            value.push(" · rebuild sent".fg(Color::Yellow))
        }
        RebuildState::Idle | RebuildState::Requested(_) => {}
        RebuildState::Failed(error) => {
            value.push(" · rebuild ".fg(Color::DarkGray));
            value.push("failed".fg(Color::Red).bold());
            value.push(format!(" · {error}").fg(Color::DarkGray));
        }
    }
    (color, value)
}

fn web_status(web: Health) -> (Color, Vec<Span<'static>>) {
    match web {
        Health::Checking => (Color::Yellow, vec!["checking".fg(Color::Yellow)]),
        Health::Up => (
            Color::Green,
            vec![
                "up".fg(Color::Green).bold(),
                " · localhost:42700".fg(Color::DarkGray),
            ],
        ),
        Health::Down => (Color::Red, vec!["down".fg(Color::Red).bold()]),
    }
}

fn dash_row(selected: bool, color: Color, label: &str, value: Vec<Span<'static>>) -> Line<'static> {
    let caret: Span<'static> = if selected {
        "▸ ".fg(Color::Green).bold()
    } else {
        "  ".into()
    };
    let label_span = if selected {
        format!(" {label:<8} ").fg(Color::White).bold()
    } else {
        format!(" {label:<8} ").fg(Color::DarkGray)
    };
    let mut spans: Vec<Span<'static>> = vec![caret, "●".fg(color).bold(), label_span];
    spans.extend(value);
    Line::from(spans)
}

fn frame_accent(dash: &Dash) -> Color {
    if dash.is_reloading() {
        Color::Red
    } else if dash.armed {
        Color::Yellow
    } else {
        Color::Green
    }
}

const PANEL_PADDING: Padding = Padding {
    left: 1,
    right: 1,
    top: 2,
    bottom: 1,
};

const TITLE_CAP: u16 = 1;

fn plugins_status(
    state: &RebuildState,
    boot_count: usize,
    links: &LinksState,
) -> (Color, Vec<Span<'static>>) {
    let (live_color, mut value) = match links {
        LinksState::Live(plugins) => {
            let linked = plugins.iter().filter(|plugin| plugin.linked).count();
            let stale = plugins
                .iter()
                .filter(|plugin| plugin.linked && plugin.needs_rebuild)
                .count();
            if stale > 0 {
                (
                    Color::Yellow,
                    vec![
                        format!("{linked} linked").fg(Color::Green),
                        format!(" · {stale} stale").fg(Color::Yellow).bold(),
                    ],
                )
            } else {
                (
                    Color::Green,
                    vec![format!("{linked} linked").fg(Color::Green)],
                )
            }
        }
        LinksState::Unknown => (
            Color::Green,
            vec![format!("{boot_count} linked").fg(Color::DarkGray)],
        ),
        LinksState::Unreachable => (
            Color::Yellow,
            vec![
                format!("{boot_count} linked").fg(Color::DarkGray),
                " · api down".fg(Color::DarkGray),
            ],
        ),
    };
    let color = match state {
        RebuildState::Requested(at) if at.elapsed() < ACK_TTL => {
            value.push(" · reload sent".fg(Color::Yellow));
            live_color
        }
        RebuildState::Failed(error) => {
            value.push(" · reload ".fg(Color::DarkGray));
            value.push("failed".fg(Color::Red).bold());
            value.push(format!(" · {error}").fg(Color::DarkGray));
            Color::Red
        }
        RebuildState::Idle | RebuildState::Requested(_) => live_color,
    };
    (color, value)
}

fn emu_status(state: &EmuState) -> (Color, Vec<Span<'static>>) {
    let statuses = match state {
        EmuState::Probing => {
            return (
                Color::Yellow,
                vec![
                    "scanning".fg(Color::Yellow).bold(),
                    " · → open".fg(Color::DarkGray),
                ],
            )
        }
        EmuState::Done(statuses) => statuses,
        EmuState::Failed(error) => {
            return (
                Color::Red,
                vec![
                    "registry error".fg(Color::Red).bold(),
                    format!(" · {error}").fg(Color::DarkGray),
                ],
            )
        }
    };
    if statuses.is_empty() {
        return (
            Color::Yellow,
            vec![
                "no envs".fg(Color::Yellow).bold(),
                " · → open".fg(Color::DarkGray),
            ],
        );
    }
    let ready = statuses
        .iter()
        .filter(|status| status.state == ResolveState::Ready)
        .count();
    let missing = statuses
        .iter()
        .filter(|status| status.state == ResolveState::Missing)
        .count();
    let unsupported = statuses
        .iter()
        .filter(|status| status.state == ResolveState::Unsupported)
        .count();
    if ready > 0 {
        return (
            Color::Green,
            vec![
                format!("{} envs · {ready} ready", statuses.len())
                    .fg(Color::Green)
                    .bold(),
                " · → open".fg(Color::DarkGray),
            ],
        );
    }
    let color = if unsupported == statuses.len() {
        Color::Red
    } else {
        Color::Yellow
    };
    (
        color,
        vec![
            format!(
                "{} envs · {missing} missing · {unsupported} unsupported",
                statuses.len()
            )
            .fg(color)
            .bold(),
            " · → open".fg(Color::DarkGray),
        ],
    )
}

fn plugin_row_count(dash: &Dash) -> usize {
    match &dash.links {
        LinksState::Live(rows) => rows.len(),
        LinksState::Unknown | LinksState::Unreachable => 0,
    }
}

fn draw_plugins(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let height = list_capacity(area.height);
    dash.log_height = height;
    let total = plugin_row_count(dash);
    if total == 0 {
        let message = match &dash.links {
            LinksState::Unreachable => "  api down",
            LinksState::Unknown => "  loading plugins…",
            LinksState::Live(_) => "  no workspace plugins found",
        };
        view_content(frame, area, vec![Line::from(message.fg(Color::DarkGray))]);
        return;
    }
    if dash.plugin_cursor >= total {
        dash.plugin_cursor = total - 1;
    }
    let cursor = dash.plugin_cursor;
    let start = cursor_window_start(total, height, cursor);
    let LinksState::Live(rows) = &dash.links else {
        return;
    };
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, row)| plugin_row_line(row, index == cursor))
        .collect();
    view_content(frame, area, lines);
}

fn cursor_window_start(total: usize, height: usize, cursor: usize) -> usize {
    if height == 0 || total <= height {
        return 0;
    }
    let max_start = total - height;
    if cursor >= height {
        (cursor + 1 - height).min(max_start)
    } else {
        0
    }
}

fn plugin_row_line(row: &WorkspacePlugin, selected: bool) -> Line<'static> {
    let caret: Span<'static> = if selected {
        "▸ ".fg(Color::Green).bold()
    } else {
        "  ".into()
    };
    let (dot, status) = if !row.linked {
        (
            "○".fg(Color::DarkGray).bold(),
            " · linkable".fg(Color::DarkGray),
        )
    } else if row.needs_rebuild {
        ("●".fg(Color::Yellow).bold(), " · stale".fg(Color::Yellow))
    } else {
        ("●".fg(Color::Green).bold(), " · linked".fg(Color::DarkGray))
    };
    let name = format!(" {}", row.name);
    let name_span = if selected {
        name.fg(Color::White).bold()
    } else {
        name.fg(Color::White)
    };
    let mut spans = vec![caret, dot, name_span];
    if !row.version.is_empty() {
        spans.push(format!(" v{}", row.version).fg(Color::DarkGray));
    }
    spans.push(status);
    if row.linked && row.needs_rebuild && !row.rebuild_reason.is_empty() {
        spans.push(" · ".fg(Color::Yellow));
        spans.push(row.rebuild_reason.clone().fg(Color::DarkGray));
    }
    Line::from(spans)
}

fn act_plugin(dash: &mut Dash) {
    let selected = match &dash.links {
        LinksState::Live(rows) => rows.get(dash.plugin_cursor).cloned(),
        LinksState::Unknown | LinksState::Unreachable => None,
    };
    let Some(plugin) = selected else {
        return;
    };
    let message = match toggle_dev_link(&plugin) {
        Ok(LinkToggle::Linked) => format!("linked {}", plugin.name),
        Ok(LinkToggle::Unlinked) => format!("unlinked {}", plugin.name),
        Err(error) => format!("link failed · {error:#}"),
    };
    dash.notice = Some((Instant::now(), message));
    dash.pokes.links = true;
}

fn emu_empty_lines(config: &str) -> Vec<Line<'static>> {
    vec![
        Line::from("  no emus found".fg(Color::DarkGray)),
        Line::from(vec![
            "  config ".fg(Color::DarkGray),
            config.to_string().fg(Color::White),
        ]),
    ]
}

fn candidate_line(
    candidate: &ImageCandidate,
    selected: bool,
    live_verb: Option<String>,
) -> Line<'static> {
    let caret: Span<'static> = if selected {
        "▸ ".fg(Color::Green).bold()
    } else {
        "  ".into()
    };
    let id_span = if selected {
        candidate.id.clone().fg(Color::White).bold()
    } else {
        candidate.id.clone().fg(Color::White)
    };
    let mut spans = vec![caret, "○ ".fg(Color::DarkGray), id_span];
    match live_verb {
        Some(verb) => {
            spans.push(format!("  {verb}").fg(Color::Yellow).bold());
            spans.push(" · → log".fg(Color::DarkGray));
        }
        None => {
            spans.push("  ready".fg(Color::Green));
            if let crate::commands::emu::ArchGuess::Assumed(arch) = candidate.arch {
                spans.push(format!(" · arch assumed {}", arch.as_str()).fg(Color::DarkGray));
            }
        }
    }
    Line::from(spans)
}

fn draw_emu(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let lines = match &dash.emu {
        EmuState::Probing => vec![Line::from("  scanning emus".fg(Color::Yellow))],
        EmuState::Done(statuses) if statuses.is_empty() => {
            let config = emu_config_path()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "~/.config/qol-tray/emu.toml".to_string());
            emu_empty_lines(&config)
        }
        EmuState::Done(statuses) => statuses
            .iter()
            .enumerate()
            .flat_map(|(index, status)| {
                let selected = index == dash.emu_cursor;
                let color = state_color(status.state);
                let caret: Span<'static> = if selected {
                    "▸ ".fg(Color::Green).bold()
                } else {
                    "  ".into()
                };
                let id_span = if selected {
                    status.id.clone().fg(Color::White).bold()
                } else {
                    status.id.clone().fg(Color::White)
                };
                let mut header = vec![
                    caret,
                    "● ".fg(color).bold(),
                    id_span,
                    format!("  {}", status.state.as_str()).fg(color).bold(),
                    format!(" · {}", status.backend).fg(Color::DarkGray),
                ];
                match live_verb(dash, &status.id) {
                    Some(verb) => {
                        header.push(format!(" · {verb}").fg(Color::Yellow).bold());
                        header.push(" · → log".fg(Color::DarkGray));
                    }
                    None => header.extend(last_run_spans(status.last_run.as_ref())),
                }
                let mut entry = vec![Line::from(header)];
                if status.state != ResolveState::Ready {
                    entry.push(Line::from(vec![
                        "    ".into(),
                        status.reason.clone().fg(Color::DarkGray),
                    ]));
                }
                entry
            })
            .collect(),
        EmuState::Failed(error) => vec![Line::from(vec![
            "  registry error ".fg(Color::Red).bold(),
            error.clone().fg(Color::DarkGray),
        ])],
    };
    let mut lines = lines;
    let env_count = emu_env_count(dash);
    for (index, candidate) in dash.emu_candidates.iter().enumerate() {
        lines.push(candidate_line(
            candidate,
            env_count + index == dash.emu_cursor,
            live_verb(dash, &candidate.id),
        ));
    }
    let total = lines.len();
    let (start, height) = list_window(dash, area, total);
    let visible: Vec<Line> = lines.into_iter().skip(start).take(height).collect();
    view_content(frame, area, visible);
}

fn draw_emu_detail(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let accent = frame_accent(dash);
    let Some((id, info)) = dash
        .emu_detail
        .as_ref()
        .map(|detail| (detail.id.clone(), detail.info.clone()))
    else {
        return;
    };
    let info_height = spaced_height(info.len(), ITEM_GAP).min(area.height);
    view_content(
        frame,
        Rect {
            height: info_height,
            ..area
        },
        info,
    );
    let used = info_height.saturating_add(1);
    if used >= area.height {
        return;
    }
    let log_area = Rect {
        y: area.y + used,
        height: area.height - used,
        ..area
    };
    let highlight = copy_highlight(dash);
    if let Some(pane) = dash.active_runs.get(&id) {
        draw_run_log(
            frame,
            log_area,
            &pane.ring,
            &dash.filters.emu,
            &mut dash.scroll_offset,
            &mut dash.log_height,
            accent,
            highlight,
        );
        return;
    }
    match dash
        .emu_detail
        .as_ref()
        .and_then(|detail| detail.replay.as_ref())
    {
        Some(pane) => draw_run_log(
            frame,
            log_area,
            &pane.ring,
            &dash.filters.emu,
            &mut dash.scroll_offset,
            &mut dash.log_height,
            accent,
            highlight,
        ),
        None => view_content(
            frame,
            log_area,
            vec![Line::from(
                "  no run.log yet · boot to create one".fg(Color::DarkGray),
            )],
        ),
    }
}

fn last_run_spans(last_run: Option<&LastRun>) -> Vec<Span<'static>> {
    let Some(run) = last_run else {
        return Vec::new();
    };
    let color = match run.status.as_str() {
        "pass" => Color::Green,
        "failed" => Color::Red,
        "running" => Color::Yellow,
        _ => Color::DarkGray,
    };
    vec![
        " · ".fg(Color::DarkGray),
        run.status.clone().fg(color),
        format!(" {}", relative_age(now_unix_ms(), run.finished_at_unix_ms)).fg(Color::DarkGray),
    ]
}

fn relative_age(now_ms: u64, then_ms: u64) -> String {
    let seconds = now_ms.saturating_sub(then_ms) / 1000;
    match seconds {
        0..=9 => "just now".to_string(),
        10..=59 => format!("{seconds}s ago"),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86399 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86400),
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn doctor_status(panel: &DoctorPanel, now_ms: u64) -> (Color, Vec<Span<'static>>) {
    if let Some((mode, _)) = &panel.manual {
        return (Color::Yellow, vec![mode.gerund().fg(Color::Yellow)]);
    }
    let Some(run) = &panel.last else {
        let detail = panel
            .error
            .clone()
            .unwrap_or_else(|| "waiting for first check".to_string());
        return (Color::Yellow, vec![detail.fg(Color::DarkGray)]);
    };
    let report = run.report;
    let (color, mut value) = if report.divergences() == 0 {
        (
            Color::Green,
            vec![
                "all good".fg(Color::Green).bold(),
                format!(" · {} checks", report.ok).fg(Color::DarkGray),
            ],
        )
    } else {
        let color = if report.error + report.crash > 0 {
            Color::Red
        } else {
            Color::Yellow
        };
        (
            color,
            vec![
                format!("{} divergences", report.divergences())
                    .fg(color)
                    .bold(),
                format!(
                    " · {} warn · {} err",
                    report.warn,
                    report.error + report.crash
                )
                .fg(Color::DarkGray),
            ],
        )
    };
    if let Some(at) = panel.last_at_ms {
        value.push(format!(" · {}", relative_age(now_ms, at)).fg(Color::DarkGray));
    }
    if panel.error.is_some() {
        value.push(" · probe failed".fg(Color::DarkGray));
    }
    (color, value)
}

fn draw_logs(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let highlight = copy_highlight(dash);
    let header = log_source_header(dash);
    let body_height = area.height.saturating_sub(header.len() as u16) as usize;
    let (rows, _) = stream_rows(
        &dash.logs.ring,
        &dash.filters.logs,
        &mut dash.scroll_offset,
        &mut dash.log_height,
        body_height,
        highlight,
        area.width as usize,
    );
    frame.render_widget(Paragraph::new(join_header_rows(header, rows)), area);
}

fn draw_trace(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let header = log_source_header(dash);
    let body_height = area.height.saturating_sub(header.len() as u16) as usize;
    let highlight = copy_highlight(dash);
    let rows = if dash.trace.ring.lines.is_empty() && dash.active_filters().is_empty() {
        dash.log_height = body_height;
        vec![Line::from("  waiting for trace events")]
    } else {
        stream_rows(
            &dash.trace.ring,
            &dash.filters.trace,
            &mut dash.scroll_offset,
            &mut dash.log_height,
            body_height,
            highlight,
            area.width as usize,
        )
        .0
    };
    frame.render_widget(Paragraph::new(join_header_rows(header, rows)), area);
}

fn join_header_rows<'a>(header: Vec<Line<'static>>, body: Vec<Line<'a>>) -> Vec<Line<'a>> {
    header.into_iter().chain(body).collect()
}

fn log_source_header(dash: &Dash) -> Vec<Line<'static>> {
    let Some(source) = current_log_source(dash) else {
        return Vec::new();
    };
    let file = source
        .file
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "(none yet)".to_string());
    let mut meta = vec![
        "  folder ".fg(Color::DarkGray),
        source.folder.display().to_string().fg(Color::White),
    ];
    meta.push(" · ".fg(Color::DarkGray));
    meta.push(source.stream_note.fg(Color::DarkGray));

    vec![
        Line::from(vec![
            format!("  {} file ", source.kind).fg(Color::DarkGray),
            file.fg(Color::White),
        ]),
        Line::from(meta),
        Line::from(""),
    ]
}

#[allow(clippy::too_many_arguments)]
fn stream_rows<'a>(
    ring: &'a LogRing,
    filters: &[LogFilter],
    scroll_offset: &mut usize,
    log_height: &mut usize,
    height: usize,
    highlight_tail: Option<usize>,
    inner_width: usize,
) -> (Vec<Line<'a>>, usize) {
    *log_height = height;
    let filtered: Vec<&'a String> = ring
        .lines
        .iter()
        .filter(|line| line_matches_filters(line, filters))
        .collect();
    let total = filtered.len();
    *scroll_offset = clamp_offset(total, height, *scroll_offset);
    let start = window_start(total, height, *scroll_offset);
    let highlight_from = highlight_tail.map(|n| total.saturating_sub(n));
    let rows = filtered
        .into_iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, line)| {
            let styled = styled_line(line);
            match highlight_from {
                Some(from) if index >= from => highlight_bar(styled, inner_width),
                _ => styled,
            }
        })
        .collect();
    (rows, total)
}

#[allow(clippy::too_many_arguments)]
fn draw_run_log(
    frame: &mut Frame,
    area: Rect,
    ring: &LogRing,
    filters: &[LogFilter],
    scroll_offset: &mut usize,
    log_height: &mut usize,
    accent: Color,
    highlight_tail: Option<usize>,
) {
    let height = SignBox::capacity(area.height);
    let inner_width = area.width.saturating_sub(2) as usize;
    let (rows, total) = stream_rows(
        ring,
        filters,
        scroll_offset,
        log_height,
        height,
        highlight_tail,
        inner_width,
    );
    let title = format!("run.log · {}", list_status(total, *scroll_offset));
    SignBox {
        title: &title,
        rows,
    }
    .render(frame, area, accent);
}

fn highlight_bar(line: Line<'_>, inner_width: usize) -> Line<'_> {
    let pad = inner_width.saturating_sub(line.width());
    let mut spans = line.spans;
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    Line::from(spans).style(Style::new().bg(Color::Rgb(38, 44, 74)))
}

fn trace_value(dash: &Dash) -> Vec<Span<'static>> {
    if dash.trace.is_live() {
        return vec![format!("{} lines", dash.trace.len()).fg(Color::DarkGray)];
    }
    if dash.trace_unavailable {
        return vec!["tracer unavailable".fg(Color::DarkGray)];
    }
    vec!["idle · → open".fg(Color::DarkGray)]
}

fn draw_endpoints(frame: &mut Frame, dash: &Dash, area: Rect) {
    let lines: Vec<Line> = match &dash.endpoints {
        EndpointsState::Probing => vec![Line::from("  probing endpoints".fg(Color::DarkGray))],
        EndpointsState::Done(items) => items.iter().map(endpoint_line).collect(),
    };
    view_content(frame, area, lines);
}

fn endpoint_line(status: &EndpointStatus) -> Line<'static> {
    let (symbol, color) = if status.ok {
        ("✓", Color::Green)
    } else {
        ("✗", Color::Red)
    };
    Line::from(vec![
        format!("  {symbol} ").fg(color).bold(),
        format!("{:<8}", status.label).fg(Color::White),
        format!("  {}", status.url).fg(Color::DarkGray),
    ])
}

fn draw_doctor(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let lines = doctor_view_lines(&dash.doctor);
    if lines.is_empty() {
        let message = match &dash.doctor.manual {
            Some((mode, _)) => mode.progress_message(),
            None => "  no checks reported · press d to run",
        };
        view_content(frame, area, vec![Line::from(message)]);
        return;
    }
    let total = lines.len();
    let (start, height) = list_window(dash, area, total);
    let render = if dash.armed {
        styled_doctor_line
    } else {
        friendly_doctor_line
    };
    let visible: Vec<Line> = lines
        .iter()
        .skip(start)
        .take(height)
        .map(|line| render(line))
        .collect();
    view_content(frame, area, visible);
}

fn doctor_view_lines(panel: &DoctorPanel) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(error) = &panel.error {
        lines.push(format!("[ERR] doctor: {error}"));
    }
    if let Some(run) = &panel.last {
        lines.extend(run.lines.iter().cloned());
    }
    lines
}

fn list_window(dash: &mut Dash, area: Rect, total: usize) -> (usize, usize) {
    let height = list_capacity(area.height);
    dash.log_height = height;
    dash.scroll_offset = clamp_offset(total, height, dash.scroll_offset);
    (window_start(total, height, dash.scroll_offset), height)
}

fn list_status(total: usize, scroll_offset: usize) -> String {
    let mode = if scroll_offset == 0 {
        "follow"
    } else {
        "scroll"
    };
    format!("{total} lines · {mode}")
}

fn doctor_line_style(raw: &str) -> Option<(&'static str, Color)> {
    let trimmed = raw.trim_start();
    if trimmed.starts_with("[OK]") {
        return Some(("✓", Color::Green));
    }
    if trimmed.starts_with("[WARN]") {
        return Some(("▲", Color::Yellow));
    }
    if trimmed.starts_with("[ERR]") {
        return Some(("✗", Color::Red));
    }
    if trimmed.starts_with("[CRASH]") {
        return Some(("✗", Color::Magenta));
    }
    None
}

fn styled_doctor_line(raw: &str) -> Line<'static> {
    let Some((symbol, color)) = doctor_line_style(raw) else {
        return Line::from(format!("  {}", raw.trim_start()));
    };
    let rest = raw
        .trim_start()
        .split_once(']')
        .map(|(_, rest)| rest)
        .unwrap_or("")
        .trim_start();
    Line::from(vec![
        format!("  {symbol} ").fg(color).bold(),
        rest.to_string().into(),
    ])
}

fn friendly_doctor_line(raw: &str) -> Line<'static> {
    let Some((symbol, color)) = doctor_line_style(raw) else {
        return Line::from(format!("  {}", raw.trim_start()));
    };
    let rest = raw
        .trim_start()
        .split_once(']')
        .map(|(_, rest)| rest.trim_start())
        .unwrap_or("");
    let Some((id, message)) = rest.split_once(": ") else {
        return Line::from(vec![
            format!("  {symbol} ").fg(color).bold(),
            rest.to_string().into(),
        ]);
    };
    let head = format!("  {symbol} {}", humanize_check_id(id));
    if symbol == "✓" {
        return Line::from(head.fg(color).bold());
    }
    let detail = message
        .split_once('\u{2014}')
        .map_or(message, |(_, tail)| tail)
        .trim();
    Line::from(vec![
        format!("{head} - ").fg(color).bold(),
        detail.to_string().into(),
    ])
}

fn humanize_check_id(id: &str) -> String {
    let spaced = id.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => spaced,
    }
}

fn styled_line(raw: &str) -> Line<'_> {
    use ansi_to_tui::IntoText;
    let Ok(text) = raw.into_text() else {
        return Line::from(raw);
    };
    text.lines
        .into_iter()
        .next()
        .unwrap_or_else(|| Line::from(raw))
}

fn format_duration(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    let (minutes, seconds) = (total / 60, total % 60);
    if minutes >= 60 {
        return format!("{}h{:02}m", minutes / 60, minutes % 60);
    }
    format!("{minutes}m{seconds:02}s")
}

fn try_wait(child: &mut Child) -> Result<Option<ExitStatus>> {
    child
        .try_wait()
        .context("failed polling qol-tray dev process")
}

fn stop_child(child: &mut Child) -> Result<()> {
    host_facade::stop_qol_tray()?;
    let deadline = Instant::now() + STOP_GRACE;
    while Instant::now() < deadline {
        if try_wait(child)?.is_some() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    child.wait().context("failed to reap qol-tray after kill")?;
    Ok(())
}

fn spawn_forwarders(child: &mut Child) -> Receiver<String> {
    let (tx, rx) = channel();
    if let Some(stdout) = child.stdout.take() {
        spawn_forwarder(stdout, tx.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_forwarder(stderr, tx);
    }
    rx
}

fn spawn_forwarder(reader: impl Read + Send + 'static, tx: Sender<String>) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) => return,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end_matches(['\n', '\r']);
                    if tx.send(line.to_string()).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let _ = tx.send(format!("[qol dev] log stream error: {error}"));
                    return;
                }
            }
        }
    });
}

#[derive(Clone, Copy)]
enum DoctorMode {
    Check,
    Fix,
}

impl DoctorMode {
    fn arg(self) -> &'static str {
        match self {
            DoctorMode::Check => "check",
            DoctorMode::Fix => "fix",
        }
    }

    fn gerund(self) -> &'static str {
        match self {
            DoctorMode::Check => "checking",
            DoctorMode::Fix => "fixing",
        }
    }

    fn progress_message(self) -> &'static str {
        match self {
            DoctorMode::Check => "  running checks",
            DoctorMode::Fix => "  applying fixes",
        }
    }
}

fn spawn_doctor(mode: DoctorMode) -> Receiver<Result<DoctorRun, String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(run_doctor(mode));
    });
    rx
}

fn run_doctor(mode: DoctorMode) -> Result<DoctorRun, String> {
    let root = crate::workspace::repo_root().map_err(|error| format!("{error:#}"))?;
    build_doctor(&root)?;
    run_doctor_binary(&doctor_binary(&root), &root, mode)
}

fn run_doctor_prebuilt() -> Result<DoctorRun, String> {
    let root = crate::workspace::repo_root().map_err(|error| format!("{error:#}"))?;
    let binary = doctor_binary(&root);
    if !binary.exists() {
        return Err("doctor binary not built · press d".to_string());
    }
    run_doctor_binary(&binary, &root, DoctorMode::Check)
}

fn doctor_binary(root: &std::path::Path) -> std::path::PathBuf {
    root.join("target")
        .join("debug")
        .join(host_facade::exe_name("qol-tray-doctor"))
}

fn run_doctor_binary(
    binary: &std::path::Path,
    root: &std::path::Path,
    mode: DoctorMode,
) -> Result<DoctorRun, String> {
    let output = Command::new(binary)
        .current_dir(root)
        .arg(mode.arg())
        .output()
        .map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout);
    let report =
        parse_doctor_summary(&text).ok_or_else(|| "could not read doctor summary".to_string())?;
    Ok(DoctorRun {
        report,
        lines: doctor_lines(&text, mode),
    })
}

fn build_doctor(root: &std::path::Path) -> Result<(), String> {
    let output = Command::new("cargo")
        .current_dir(root)
        .args([
            "build",
            "-p",
            "qol-tray",
            "--features",
            "dev",
            "--bin",
            "qol-tray-doctor",
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr
        .lines()
        .rfind(|line| line.contains("error"))
        .unwrap_or("no cargo error line captured");
    Err(format!("doctor build failed ({}): {detail}", output.status))
}

fn doctor_lines(text: &str, mode: DoctorMode) -> Vec<String> {
    let relevant = match mode {
        DoctorMode::Check => text,
        DoctorMode::Fix => text.rsplit("Doctor Check (After)").next().unwrap_or(text),
    };
    relevant
        .lines()
        .filter(|line| line.trim_start().starts_with('['))
        .map(|line| line.to_string())
        .collect()
}

fn parse_doctor_summary(text: &str) -> Option<DoctorReport> {
    let line = text
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with("Summary:"))?;
    Some(DoctorReport {
        ok: parse_summary_field(line, "ok=")?,
        warn: parse_summary_field(line, "warn=")?,
        error: parse_summary_field(line, "error=")?,
        crash: parse_summary_field(line, "crash=")?,
    })
}

fn parse_summary_field(line: &str, key: &str) -> Option<usize> {
    let rest = line.split(key).nth(1)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_age_buckets_seconds_minutes_hours_days() {
        let cases = [
            (5_000, "just now"),
            (59_000, "59s ago"),
            (60_000, "1m ago"),
            (3_599_000, "59m ago"),
            (3_600_000, "1h ago"),
            (86_399_000, "23h ago"),
            (86_400_000, "1d ago"),
            (0, "just now"),
        ];
        for (elapsed_ms, expected) in cases {
            assert_eq!(
                relative_age(1_000_000_000 + elapsed_ms, 1_000_000_000),
                expected,
                "elapsed_ms: {elapsed_ms}"
            );
        }
    }

    fn span_text(spans: &[Span<'static>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    fn log_filter(strategy: FilterStrategy, text: &str) -> LogFilter {
        LogFilter {
            strategy,
            text: text.to_string(),
        }
    }

    fn set_active_filters(dash: &mut Dash, filters: Vec<LogFilter>) {
        let view = dash.view;
        *dash
            .filters
            .for_view_mut(view)
            .expect("active view is filterable") = filters;
    }

    #[test]
    fn relative_age_gains_a_seconds_bucket() {
        let cases = [
            (5_000, "just now"),
            (10_000, "10s ago"),
            (59_000, "59s ago"),
            (60_000, "1m ago"),
        ];
        for (elapsed_ms, expected) in cases {
            assert_eq!(
                relative_age(1_000_000_000 + elapsed_ms, 1_000_000_000),
                expected,
                "elapsed_ms: {elapsed_ms}"
            );
        }
    }

    #[test]
    fn doctor_status_covers_panel_states() {
        let report = DoctorReport {
            ok: 11,
            warn: 0,
            error: 0,
            crash: 0,
        };
        let run = DoctorRun {
            report,
            lines: Vec::new(),
        };
        let warn_report = DoctorReport {
            ok: 9,
            warn: 2,
            error: 0,
            crash: 0,
        };
        let warn_run = DoctorRun {
            report: warn_report,
            lines: Vec::new(),
        };
        let now = 1_000_000_000;
        let cases = [
            (
                DoctorPanel {
                    last: None,
                    last_at_ms: None,
                    manual: None,
                    error: None,
                },
                Color::Yellow,
                "waiting for first check",
            ),
            (
                DoctorPanel {
                    last: Some(run.clone()),
                    last_at_ms: Some(now - 15_000),
                    manual: None,
                    error: None,
                },
                Color::Green,
                "all good · 11 checks · 15s ago",
            ),
            (
                DoctorPanel {
                    last: Some(warn_run.clone()),
                    last_at_ms: Some(now - 5_000),
                    manual: None,
                    error: None,
                },
                Color::Yellow,
                "2 divergences · 2 warn · 0 err · just now",
            ),
            (
                DoctorPanel {
                    last: Some(run.clone()),
                    last_at_ms: Some(now - 15_000),
                    manual: None,
                    error: Some("boom".to_string()),
                },
                Color::Green,
                "all good · 11 checks · 15s ago · probe failed",
            ),
            (
                DoctorPanel {
                    last: None,
                    last_at_ms: None,
                    manual: None,
                    error: Some("doctor binary not built · press d".to_string()),
                },
                Color::Yellow,
                "doctor binary not built · press d",
            ),
        ];
        for (panel, expected_color, expected_text) in cases {
            let (color, spans) = doctor_status(&panel, now);
            assert_eq!(color, expected_color, "text: {expected_text}");
            assert_eq!(span_text(&spans), expected_text);
        }
    }

    fn workspace_plugin(name: &str, linked: bool, needs_rebuild: bool) -> WorkspacePlugin {
        WorkspacePlugin {
            id: name.to_string(),
            name: name.to_string(),
            version: if linked { "1.0.0" } else { "" }.to_string(),
            path: format!("/ws/{name}"),
            linked,
            needs_rebuild,
            rebuild_reason: if needs_rebuild { "Source changed" } else { "" }.to_string(),
        }
    }

    #[test]
    fn plugins_status_counts_only_linked_among_workspace_plugins() {
        let fresh = workspace_plugin("foo", true, false);
        let stale = workspace_plugin("bar", true, true);
        let linkable = workspace_plugin("baz", false, false);
        let cases = [
            (
                LinksState::Live(vec![fresh.clone(), stale.clone(), linkable.clone()]),
                Color::Yellow,
                "2 linked · 1 stale",
            ),
            (
                LinksState::Live(vec![fresh.clone(), linkable.clone()]),
                Color::Green,
                "1 linked",
            ),
            (LinksState::Unknown, Color::Green, "3 linked"),
            (
                LinksState::Unreachable,
                Color::Yellow,
                "3 linked · api down",
            ),
        ];
        for (links, expected_color, expected_text) in cases {
            let (color, spans) = plugins_status(&RebuildState::Idle, 3, &links);
            assert_eq!(color, expected_color, "text: {expected_text}");
            assert_eq!(span_text(&spans), expected_text);
        }
    }

    #[test]
    fn plugin_row_line_marks_linked_and_linkable_states() {
        let cases = [
            (workspace_plugin("a", true, false), "● a v1.0.0 · linked"),
            (workspace_plugin("b", false, false), "○ b · linkable"),
        ];
        for (row, expected) in cases {
            let text = span_text(&plugin_row_line(&row, false).spans);
            assert!(
                text.contains(expected),
                "row line must show {expected:?}, got {text:?}"
            );
        }
    }

    #[test]
    fn plugin_row_line_carets_the_selected_row() {
        let row = workspace_plugin("a", true, false);
        let selected = span_text(&plugin_row_line(&row, true).spans);
        let unselected = span_text(&plugin_row_line(&row, false).spans);
        assert!(
            selected.starts_with("▸"),
            "selected row gets a caret: {selected:?}"
        );
        assert!(
            !unselected.starts_with("▸"),
            "unselected row has no caret: {unselected:?}"
        );
    }

    #[test]
    fn cursor_window_keeps_selection_visible() {
        let cases = [
            (10, 4, 0, 0),
            (10, 4, 2, 0),
            (10, 4, 4, 1),
            (10, 4, 9, 6),
            (3, 5, 2, 0),
        ];
        for (total, height, cursor, expected) in cases {
            assert_eq!(
                cursor_window_start(total, height, cursor),
                expected,
                "total={total} height={height} cursor={cursor}"
            );
        }
    }

    #[test]
    fn plugins_status_appends_reload_failure() {
        let (color, spans) = plugins_status(
            &RebuildState::Failed("boom".to_string()),
            3,
            &LinksState::Unknown,
        );
        assert_eq!(color, Color::Red, "failed reload turns the row red");
        assert!(
            span_text(&spans).contains("reload failed · boom"),
            "spans: {}",
            span_text(&spans)
        );
    }

    #[test]
    fn diving_into_emu_requests_an_emu_poke() {
        let mut dash = Dash::new(Vec::new());
        dash.cursor = 3;
        apply_action(&mut dash, Action::Dive, false);
        assert!(dash.pokes.emu, "emu dive marks the emu probe dirty");
        assert!(matches!(dash.view, View::Emu), "dive opened the emu view");
    }

    #[test]
    fn action_for_maps_keys() {
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        let mut dash = Dash::new(Vec::new());
        let cases = [
            (KeyCode::Char('l'), none, Action::ToggleView),
            (KeyCode::Char('L'), none, Action::ToggleView),
            (KeyCode::Char('d'), none, Action::Doctor),
            (KeyCode::Char('D'), none, Action::Doctor),
            (KeyCode::Esc, none, Action::Back),
            (KeyCode::Left, none, Action::Back),
            (KeyCode::Enter, none, Action::Activate),
            (KeyCode::Right, none, Action::Dive),
            (KeyCode::Char('r'), ctrl, Action::Rebuild),
            (KeyCode::Char('f'), ctrl, Action::FeatureFlags),
            (KeyCode::Char('p'), ctrl, Action::Ignore),
            (KeyCode::Char('u'), ctrl, Action::Ignore),
            (KeyCode::Char('c'), ctrl, Action::Quit),
            (KeyCode::Char('q'), none, Action::Quit),
            (KeyCode::Up, none, Action::ScrollUp),
            (KeyCode::Down, none, Action::ScrollDown),
            (KeyCode::Char('k'), none, Action::ToggleKeys),
            (KeyCode::Char('K'), none, Action::ToggleKeys),
            (KeyCode::Char(' '), none, Action::ToggleArm),
            (KeyCode::Char('r'), none, Action::Ignore),
            (KeyCode::Char('p'), none, Action::Ignore),
            (KeyCode::Char('x'), none, Action::Ignore),
            (KeyCode::Char('u'), none, Action::Ignore),
        ];
        for (code, mods, expected) in cases {
            assert_eq!(action_for(&dash, code, mods), expected, "{code:?} {mods:?}");
        }
        dash.view = View::Trace;
        assert_eq!(
            action_for(&dash, KeyCode::Char('d'), none),
            Action::ToggleTraceDetails,
            "d toggles trace details in the trace view"
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char('o'), none),
            Action::OpenCurrentLogFolder,
            "o opens trace folder in the trace view"
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char('e'), none),
            Action::OpenCurrentLogEditor,
            "e opens the prettified trace in the trace view"
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char('r'), none),
            Action::OpenCurrentLogRaw,
            "r opens the raw trace file in the trace view"
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char(' '), none),
            Action::ToggleArm,
            "space arms the reload in the trace view"
        );
        for (code, expected) in [
            (KeyCode::PageUp, Action::PageUp),
            (KeyCode::PageDown, Action::PageDown),
            (KeyCode::End, Action::Follow),
            (KeyCode::Char('f'), Action::Follow),
            (KeyCode::Char('/'), Action::Filter),
            (KeyCode::Char('c'), Action::Copy),
            (KeyCode::Char('C'), Action::Copy),
        ] {
            assert_eq!(
                action_for(&dash, code, none),
                expected,
                "stream key: {code:?}"
            );
        }
        dash.view = View::Logs;
        assert_eq!(
            action_for(&dash, KeyCode::Char('d'), none),
            Action::Ignore,
            "d is not doctor outside its owning contexts"
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char('o'), none),
            Action::OpenCurrentLogFolder,
            "o opens log folder in the logs view"
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char('e'), none),
            Action::OpenCurrentLogEditor,
            "e opens log file in the logs view"
        );
    }

    #[test]
    fn dashboard_cursor_moves_and_clamps() {
        let mut dash = Dash::new(Vec::new());
        assert_eq!(dash.cursor, 0);
        apply_action(&mut dash, Action::ScrollUp, false);
        assert_eq!(dash.cursor, 0, "clamps at top");
        for _ in 0..10 {
            apply_action(&mut dash, Action::ScrollDown, false);
        }
        assert_eq!(dash.cursor, ROWS.len() - 1, "clamps at bottom");
        apply_action(&mut dash, Action::ScrollUp, false);
        assert_eq!(dash.cursor, ROWS.len() - 2);
    }

    #[test]
    fn emu_row_opens_emu_view() {
        let mut dash = Dash::new(Vec::new());
        dash.cursor = 3;
        apply_action(&mut dash, Action::Activate, false);
        assert!(matches!(dash.view, View::Emu));
    }

    fn emu_env(id: &str, state: ResolveState) -> EnvironmentStatus {
        EnvironmentStatus {
            id: id.to_string(),
            backend: "qemu".to_string(),
            state,
            reason: String::new(),
            last_run: None,
        }
    }

    #[test]
    fn emu_cursor_moves_and_clamps_without_scrolling() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![
            emu_env("foo", ResolveState::Ready),
            emu_env("bar", ResolveState::Ready),
        ]);
        let moves = [
            (Action::ScrollDown, 1),
            (Action::ScrollDown, 1),
            (Action::ScrollUp, 0),
            (Action::ScrollUp, 0),
        ];
        for (action, expected) in moves {
            apply_action(&mut dash, action, false);
            assert_eq!(dash.emu_cursor, expected, "after {action:?}");
            assert_eq!(dash.scroll_offset, 0, "after {action:?}");
        }
    }

    fn emu_candidate(id: &str) -> ImageCandidate {
        use crate::commands::emu::{ArchGuess, BootMedia, Firmware, GuestArch};
        ImageCandidate {
            id: id.to_string(),
            path: std::path::PathBuf::from(format!("/a/b/{id}.qcow2")),
            display_name: id.to_string(),
            arch: ArchGuess::assumed(GuestArch::X86_64),
            firmware: Firmware::Uefi,
            media: BootMedia::Disk,
        }
    }

    fn known_emu_candidate(id: &str) -> ImageCandidate {
        use crate::commands::emu::{ArchGuess, GuestArch};
        let mut candidate = emu_candidate(id);
        candidate.arch = ArchGuess::known(GuestArch::X86_64);
        candidate
    }

    #[test]
    fn emu_cursor_extends_into_candidate_rows_and_clamps() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![
            emu_env("foo", ResolveState::Ready),
            emu_env("bar", ResolveState::Ready),
        ]);
        dash.emu_candidates = vec![emu_candidate("baz"), emu_candidate("qux")];
        let moves = [
            (Action::ScrollDown, 1),
            (Action::ScrollDown, 2),
            (Action::ScrollDown, 3),
            (Action::ScrollDown, 3),
        ];
        for (action, expected) in moves {
            apply_action(&mut dash, action, false);
            assert_eq!(dash.emu_cursor, expected, "after {action:?}");
        }
    }

    #[test]
    fn candidate_line_uses_plain_ready_label() {
        let line = candidate_line(&known_emu_candidate("plain"), false, None);
        assert_eq!(span_text(&line.spans), "  ○ plain  ready");
    }

    #[test]
    fn candidate_line_marks_assumed_arch() {
        let line = candidate_line(&emu_candidate("plain"), false, None);
        assert_eq!(
            span_text(&line.spans),
            "  ○ plain  ready · arch assumed x86_64"
        );
    }

    #[test]
    fn candidate_line_marks_live_run_with_log_hint() {
        let line = candidate_line(&emu_candidate("plain"), true, Some("boot".to_string()));
        assert_eq!(span_text(&line.spans), "▸ ○ plain  boot · → log");
    }

    #[test]
    fn emu_keys_map_open_toggle_and_confirm() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        let cases = [
            (KeyCode::Char('o'), Action::OpenEmuDir),
            (KeyCode::Char('t'), Action::ToggleArch),
            (KeyCode::Char('a'), Action::Confirm),
        ];
        for (code, expected) in cases {
            assert_eq!(
                action_for(&dash, code, KeyModifiers::NONE),
                expected,
                "code: {code:?}"
            );
        }
    }

    #[test]
    fn toggle_arch_flips_selected_candidate_only() {
        use crate::commands::emu::{ArchGuess, Firmware, GuestArch};
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![
            emu_env("foo", ResolveState::Ready),
            emu_env("bar", ResolveState::Ready),
        ]);
        dash.emu_candidates = vec![emu_candidate("baz"), emu_candidate("qux")];

        dash.emu_cursor = 0;
        apply_action(&mut dash, Action::ToggleArch, false);
        assert_eq!(
            dash.emu_candidates
                .iter()
                .map(|candidate| candidate.arch)
                .collect::<Vec<_>>(),
            vec![
                ArchGuess::assumed(GuestArch::X86_64),
                ArchGuess::assumed(GuestArch::X86_64)
            ],
            "cursor on an env row must not mutate any candidate"
        );

        dash.emu_cursor = 3;
        apply_action(&mut dash, Action::ToggleArch, false);
        assert_eq!(
            dash.emu_candidates[0].arch,
            ArchGuess::assumed(GuestArch::X86_64),
            "untouched"
        );
        assert_eq!(
            dash.emu_candidates[1].arch,
            ArchGuess::known(GuestArch::Aarch64),
            "selected candidate becomes known"
        );
        assert_eq!(
            dash.emu_candidates[1].firmware,
            Firmware::Uefi,
            "toggle refreshes firmware for the selected arch"
        );
    }

    #[test]
    fn confirm_refuses_assumed_candidate_without_refreshing_emu_list() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![emu_env("foo", ResolveState::Ready)]);
        dash.emu_candidates = vec![emu_candidate("tokenless")];
        dash.emu_cursor = 1;

        apply_action(&mut dash, Action::Confirm, false);

        assert!(!dash.pokes.emu, "refusal must not refresh registered envs");
        let notice = dash.notice.as_ref().map(|(_, message)| message.as_str());
        assert_eq!(
            notice,
            Some("arch unconfirmed · press t to set arch, then a")
        );
    }

    #[test]
    fn act_emu_refuses_envs_that_are_not_ready() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![emu_env("foo", ResolveState::Missing)]);
        act_emu(&mut dash, false);
        assert!(
            dash.active_runs.is_empty(),
            "a not-ready emu does not start a run"
        );
    }

    fn live_pane(line: &str) -> LogPane {
        let mut pane = LogPane::new();
        let child = Command::new("true").spawn().unwrap();
        let (_tx, rx) = channel::<String>();
        pane.attach(child, rx);
        pane.push(line.to_string());
        pane
    }

    #[test]
    fn diving_into_an_emu_opens_its_detail() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![emu_env("foo", ResolveState::Ready)]);
        apply_action(&mut dash, Action::Dive, false);
        assert!(
            matches!(dash.view, View::EmuDetail),
            "dive opened the detail"
        );
        assert_eq!(
            dash.emu_detail.as_ref().map(|detail| detail.id.as_str()),
            Some("foo")
        );
        apply_action(&mut dash, Action::Back, false);
        assert!(matches!(dash.view, View::Emu), "back returns to the list");
        assert!(dash.emu_detail.is_none(), "back clears the detail");
    }

    #[test]
    fn diving_into_candidate_opens_its_live_log() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![emu_env("foo", ResolveState::Ready)]);
        dash.emu_candidates = vec![emu_candidate("linuxmint")];
        dash.emu_cursor = 1;
        dash.active_runs.insert(
            "linuxmint".to_string(),
            live_pane("  boot     linuxmint · qmp"),
        );

        apply_action(&mut dash, Action::Dive, false);

        assert!(matches!(dash.view, View::EmuDetail));
        assert_eq!(
            dash.emu_detail.as_ref().map(|detail| detail.id.as_str()),
            Some("linuxmint")
        );
        assert_eq!(
            emu_detail_ring(&dash)
                .and_then(|ring| ring.lines.back())
                .map(String::as_str),
            Some("  boot     linuxmint · qmp")
        );
    }

    #[test]
    fn live_run_state_exposes_running_detail() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Emu;
        dash.emu = EmuState::Done(vec![emu_env("foo", ResolveState::Ready)]);
        dash.active_runs
            .insert("foo".to_string(), live_pane("  boot     foo · qmp"));
        assert!(is_running(&dash, "foo"));
        assert_eq!(live_verb(&dash, "foo").as_deref(), Some("boot"));
    }

    fn render_text(dash: &mut Dash) -> String {
        use ratatui::backend::TestBackend;
        let mut terminal = ratatui::Terminal::new(TestBackend::new(110, 30)).unwrap();
        terminal.draw(|frame| draw(frame, dash)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn render_rows(dash: &mut Dash) -> Vec<String> {
        use ratatui::backend::TestBackend;
        let mut terminal = ratatui::Terminal::new(TestBackend::new(110, 30)).unwrap();
        terminal.draw(|frame| draw(frame, dash)).unwrap();
        let backend = terminal.backend();
        let buffer = backend.buffer();
        let width = buffer.area.width as usize;
        buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }

    fn row_bounds(rows: &[String], needle: &str) -> (usize, usize) {
        let row = rows
            .iter()
            .find(|row| row.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} not rendered"));
        let start = row.find(needle).expect("needle already found");
        let left = row[..start].rfind('│').expect("missing left border");
        let right = row[start..]
            .find('│')
            .map(|index| start + index)
            .expect("missing right border");
        (left, right)
    }

    #[test]
    fn every_view_shows_its_page_as_a_breadcrumb_on_the_qol_dev_sign() {
        let cases = [
            (View::Endpoints, "qol dev · endpoints"),
            (View::Plugins, "qol dev · plugins"),
            (View::Doctor, "qol dev · doctor"),
            (View::Emu, "qol dev · emu"),
        ];
        for (view, crumb) in cases {
            let mut dash = Dash::new(Vec::new());
            dash.view = view;
            let rows = render_rows(&mut dash);
            let sign = rows
                .iter()
                .position(|row| row.contains(crumb))
                .unwrap_or_else(|| panic!("{crumb} not rendered on the shell sign"));
            assert!(
                rows[sign].contains('┤') && rows[sign].contains('├'),
                "{crumb}: breadcrumb not framed as a sign"
            );
            assert!(
                rows[sign - 1].contains('╭') && rows[sign - 1].contains('╮'),
                "{crumb}: missing poke-up cap above the sign"
            );
            assert!(
                rows[sign + 1].contains('╰') && rows[sign + 1].contains('╯'),
                "{crumb}: missing sign base below"
            );
            assert!(
                !rows.iter().any(|row| row.contains("┤ menu ├")),
                "{crumb}: page should not nest its own sign-box"
            );
            let desc = page_description(view).expect("listed views carry a description");
            assert!(
                rows.iter().any(|row| row.contains(desc)),
                "{crumb}: description {desc:?} not rendered under the title"
            );
        }
    }

    #[test]
    fn filterable_log_views_mark_saved_filters_in_the_breadcrumb() {
        for view in [View::Logs, View::Trace, View::EmuDetail] {
            let mut dash = Dash::new(Vec::new());
            dash.view = view;
            set_active_filters(
                &mut dash,
                vec![log_filter(FilterStrategy::Include, "shortcut")],
            );
            if view == View::EmuDetail {
                dash.emu_detail = Some(EmuDetail {
                    id: "foo".to_string(),
                    info: Vec::new(),
                    replay: None,
                });
            }
            let text = span_text(&breadcrumb(&dash, Color::Green).spans);
            assert!(
                text.contains("FILTERED"),
                "filterable view should mark active filters: {text}"
            );
        }
    }

    #[test]
    fn non_filtering_views_do_not_mark_filters_in_the_breadcrumb() {
        let mut logs = Dash::new(Vec::new());
        logs.view = View::Logs;
        assert!(!span_text(&breadcrumb(&logs, Color::Green).spans).contains("FILTERED"));

        let mut dashboard = Dash::new(Vec::new());
        dashboard.filters.logs = vec![log_filter(FilterStrategy::Include, "shortcut")];
        assert!(!span_text(&breadcrumb(&dashboard, Color::Green).spans).contains("FILTERED"));
    }

    #[test]
    fn qol_dev_shell_sign_is_centered() {
        let mut dash = Dash::new(Vec::new());
        let rows = render_rows(&mut dash);
        let border = rows
            .iter()
            .position(|row| row.contains("┤ qol dev ├"))
            .expect("shell sign present");
        let row = &rows[border];
        let left = row.chars().take_while(|&c| c == '┌' || c == '─').count() - 1;
        let right = row
            .chars()
            .skip_while(|&c| c != '├')
            .skip(1)
            .take_while(|&c| c == '─')
            .count();
        assert!(
            left.abs_diff(right) <= 1,
            "shell sign not centered ({left} dashes left, {right} right)"
        );
    }

    #[test]
    fn keys_hud_renders_with_view_keys_and_globals() {
        let cases = [
            (View::Dashboard, "arm, then enter"),
            (View::Emu, "boot · stop"),
        ];
        for (view, expected) in cases {
            let mut dash = Dash::new(Vec::new());
            dash.view = view;
            let text = render_text(&mut dash);
            assert!(text.contains(expected), "missing {expected:?}");
            assert!(text.contains("ctrl+r"), "missing globals");
            assert!(text.contains("rebuild tray+plugins"), "missing globals");
            assert!(
                !text.contains("reload qol dev"),
                "reload shown while unarmed"
            );
            assert!(!text.contains("armed ctrl+r"), "stale armed label rendered");
            assert!(!text.contains("ctrl+u"), "stale reload shortcut rendered");
            assert!(text.contains("keys · k"), "missing keys badge");
            assert!(text.contains("global"), "missing global section");
            assert!(text.contains("context"), "missing context section");
            assert!(
                text.find("global") < text.find("context"),
                "global section should render before context"
            );
            assert!(
                !text.contains("context · k"),
                "stale context title rendered"
            );
            if matches!(view, View::Emu) {
                assert!(text.contains("set arch"), "missing emu o/t/a keys");
            }
        }
    }

    #[test]
    fn keys_hud_swaps_ctrl_r_action_when_armed() {
        let mut dash = Dash::new(Vec::new());
        dash.armed = true;
        let text = render_text(&mut dash);
        assert!(text.contains("ctrl+r"), "missing ctrl+r key");
        assert!(
            text.contains("reload qol dev"),
            "missing armed reload action"
        );
        assert!(
            !text.contains("rebuild tray+plugins"),
            "unarmed rebuild action rendered"
        );
        assert!(text.contains("keys · k"), "missing keys badge");
        assert!(text.contains("global"), "missing global section");
        assert!(text.contains("context"), "missing context section");
        assert!(!text.contains("armed ctrl+r"), "stale armed label rendered");
        assert!(!text.contains("ctrl+u"), "stale reload shortcut rendered");
    }

    #[test]
    fn keys_rows_space_sections() {
        let dash = Dash::new(Vec::new());
        let rows: Vec<String> = keys_rows(&dash)
            .into_iter()
            .map(|line| span_text(&line.spans))
            .collect();
        assert_eq!(rows[0], " global");
        assert_eq!(rows[1], "");
        assert_eq!(rows[2], " ctrl+r    rebuild tray+plugins ");
        assert_eq!(rows[3], " l         logs ");
        assert_eq!(rows[4], " k         keys ");
        assert_eq!(rows[5], " ctrl+f    feature flags ");
        assert_eq!(rows[6], " q / ctrl+c quit ");
        assert_eq!(rows[7], "");
        assert_eq!(rows[8], "");
        assert_eq!(rows[9], " context");
        assert_eq!(rows[10], "");
        assert_eq!(rows[11], " ↑/↓       move ");
    }

    #[test]
    fn trace_keys_include_detail_toggle_not_doctor() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Trace;
        let rows: Vec<String> = keys_rows(&dash)
            .into_iter()
            .map(|line| span_text(&line.spans))
            .collect();
        let text = rows.join("\n");
        assert!(
            text.contains(" d         details "),
            "missing trace detail key"
        );
        assert!(
            text.contains(" o         open folder "),
            "missing open folder key"
        );
        assert!(
            text.contains(" e         open in editor "),
            "missing editor open key"
        );
        assert!(
            text.contains(" r         open raw "),
            "missing raw open key"
        );
        assert!(
            text.contains(" space     arm: reload "),
            "missing reload arm key"
        );
        assert!(
            !text.contains("arm: raw"),
            "trace context must not show the legacy raw arm key"
        );
        assert!(
            !text.contains("refresh checks"),
            "trace context must not show doctor binding"
        );
    }

    #[test]
    fn armed_ctrl_r_requests_reload_from_dashboard() {
        let mut dash = Dash::new(Vec::new());
        assert!(
            dash.view == View::Dashboard,
            "dashboard is the landing view"
        );

        assert_eq!(
            handle_key(&mut dash, KeyCode::Char(' '), KeyModifiers::NONE),
            KeyOutcome::Handled
        );
        assert!(dash.armed, "space arms in the dashboard");

        assert_eq!(
            handle_key(&mut dash, KeyCode::Char('r'), KeyModifiers::CONTROL),
            KeyOutcome::Reload,
            "armed ctrl+r reloads instead of rebuilding"
        );
        assert!(!dash.armed, "reload consumes the armed state");
    }

    #[test]
    fn armed_ctrl_r_requests_reload_from_trace() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Trace;

        assert_eq!(
            handle_key(&mut dash, KeyCode::Char(' '), KeyModifiers::NONE),
            KeyOutcome::Handled
        );
        assert!(dash.armed, "space arms in the trace view");

        assert_eq!(
            handle_key(&mut dash, KeyCode::Char('r'), KeyModifiers::CONTROL),
            KeyOutcome::Reload,
            "armed ctrl+r reloads from the trace view too"
        );
        assert!(!dash.armed, "reload consumes the armed state");
    }

    #[test]
    fn esc_disarms_without_quitting() {
        let mut dash = Dash::new(Vec::new());
        dash.armed = true;
        assert_eq!(
            handle_key(&mut dash, KeyCode::Esc, KeyModifiers::NONE),
            KeyOutcome::Handled
        );
        assert!(!dash.armed, "esc clears the armed state");
    }

    #[test]
    fn trace_pretty_snapshot_helpers_keep_utf8_and_strip_ansi() {
        assert_eq!(
            pretty_trace_file(Path::new("/tmp/qol-altmon.log")),
            PathBuf::from("/tmp/qol-altmon.pretty.log")
        );
        assert_eq!(
            strip_ansi_codes("\x1b[2m[08:00]\x1b[0m ┌── \x1b[1;32mok\x1b[0m\n"),
            "[08:00] ┌── ok\n"
        );
    }

    #[test]
    fn logs_and_trace_render_source_metadata() {
        let mut trace = Dash::new(Vec::new());
        trace.view = View::Trace;
        let trace_text = render_text(&mut trace);
        assert!(
            trace_text.contains(DEFAULT_TRACE_LOG_FILE),
            "trace pane should show trace file path"
        );
        assert!(
            !trace_text.contains("open -t"),
            "trace pane should not expose macOS opener fallback"
        );

        let mut logs = Dash::new(Vec::new());
        logs.view = View::Logs;
        logs.log_file = Some(DevLogFile::path_only(PathBuf::from(
            "/tmp/qol-dev-test.log",
        )));
        let source = current_log_source(&logs).expect("logs source");
        assert_eq!(source.stream_note, "qol dev stdout/stderr tee");
        let logs_text = render_text(&mut logs);
        assert!(
            logs_text.contains("qol-dev-test.log"),
            "logs pane should show the current session log file"
        );
        assert!(
            !logs_text.contains("open -t"),
            "logs pane should not expose macOS opener fallback"
        );
    }

    #[test]
    fn keys_box_width_stays_fixed_when_armed() {
        let mut unarmed = Dash::new(Vec::new());
        let unarmed_rows = render_rows(&mut unarmed);
        let mut armed = Dash::new(Vec::new());
        armed.armed = true;
        let armed_rows = render_rows(&mut armed);

        assert_eq!(
            row_bounds(&unarmed_rows, "rebuild tray+plugins"),
            row_bounds(&armed_rows, "reload qol dev")
        );
    }

    #[test]
    fn keys_hud_and_panel_follow_filter_state() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Logs;
        dash.open_filter_manager();
        dash.filter_layout_width = 24;
        set_active_filters(
            &mut dash,
            vec![
                log_filter(FilterStrategy::Include, "shortcut"),
                log_filter(FilterStrategy::Exclude, "success"),
                log_filter(FilterStrategy::Include, "trace"),
            ],
        );

        let text = render_text(&mut dash);
        assert!(text.contains("select filter"), "manager keys missing");
        assert!(text.contains("delete"), "delete key missing");
        dash.filter_layout_width = 24;
        let rows: Vec<String> = filter_panel_rows(&dash)
            .into_iter()
            .map(|line| span_text(&line.spans))
            .collect();
        assert_eq!(rows[0], "[+ shortcut]  - success ");
        assert_eq!(rows[1], " + trace ");

        dash.start_filter_add();
        let text = render_text(&mut dash);
        assert!(
            text.contains("strategy + / -"),
            "editing strategy key missing"
        );
        assert!(text.contains("save"), "editing save key missing");
        let rows: Vec<String> = filter_panel_rows(&dash)
            .into_iter()
            .map(|line| span_text(&line.spans))
            .collect();
        assert_eq!(rows, vec![" add + _"]);
    }

    #[test]
    fn feature_flags_panel_reuses_picker_controls() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Logs;
        dash.open_filter_manager();
        dash.toggle_feature_flags_panel();

        assert!(dash.feature_panel.is_active());
        assert!(
            matches!(dash.filter_state, FilterState::Closed),
            "feature flags supersede filter modal"
        );
        let text = render_text(&mut dash);
        assert!(text.contains("feature flags"), "missing feature panel");
        assert!(text.contains("select flag"), "missing feature keys");
        assert!(text.contains("no feature flags"), "missing empty state");
        assert_eq!(dash.trace_renderer(), TraceRenderer::Rust);
        assert!(
            !dash.trace_details_enabled(),
            "details are not a feature flag"
        );

        edit_feature_flags(&mut dash, KeyCode::Enter);
        assert_eq!(dash.trace_renderer(), TraceRenderer::Rust);
        assert!(
            !dash.trace_details_enabled(),
            "renderer flag must not toggle details"
        );
        edit_feature_flags(&mut dash, KeyCode::Char(' '));
        assert_eq!(dash.trace_renderer(), TraceRenderer::Rust);
        edit_feature_flags(&mut dash, KeyCode::Esc);
        assert!(!dash.feature_panel.is_active(), "esc closes feature panel");
    }

    #[test]
    fn filter_manager_arrows_follow_brick_rows() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Logs;
        dash.open_filter_manager();
        dash.filter_layout_width = 24;
        set_active_filters(
            &mut dash,
            vec![
                log_filter(FilterStrategy::Include, "shortcut"),
                log_filter(FilterStrategy::Exclude, "success"),
                log_filter(FilterStrategy::Include, "trace"),
            ],
        );

        edit_filters(&mut dash, KeyCode::Right);
        assert_eq!(dash.filter_index, 1, "right selects next brick");
        edit_filters(&mut dash, KeyCode::Down);
        assert_eq!(dash.filter_index, 2, "down selects nearest brick below");
        edit_filters(&mut dash, KeyCode::Up);
        assert_eq!(dash.filter_index, 0, "up selects nearest brick above");
        edit_filters(&mut dash, KeyCode::Left);
        assert_eq!(dash.filter_index, 2, "left wraps to previous row tail");
    }

    #[test]
    fn toggle_keys_hides_and_restores_the_hud() {
        let mut dash = Dash::new(Vec::new());
        apply_action(&mut dash, Action::ToggleKeys, false);
        assert!(dash.keys_hidden);
        let text = render_text(&mut dash);
        assert!(!text.contains("rebuild tray+plugins"), "hud still rendered");
        apply_action(&mut dash, Action::ToggleKeys, false);
        assert!(!dash.keys_hidden);
        let text = render_text(&mut dash);
        assert!(
            text.contains("rebuild tray+plugins"),
            "hud did not come back"
        );
    }

    #[test]
    fn reloading_state_drives_red_accent_and_status() {
        let mut dash = Dash::new(Vec::new());
        let child = Command::new("true").spawn().unwrap();
        let (_tx, rx) = channel();
        dash.reload = Reload::Running { child, rx };
        assert!(dash.is_reloading());
        assert_eq!(frame_accent(&dash), Color::Red);
        if let Reload::Running { mut child, .. } = dash.reload {
            let _ = child.wait();
        }
    }

    #[test]
    fn shell_uses_last_terminal_row_after_footer_removal() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Logs;
        dash.open_filter_manager();
        let rows = render_rows(&mut dash);
        let last = rows.last().expect("render produced rows");
        assert!(
            last.contains('└') && last.contains('┘'),
            "main panel should own the last terminal row: {last}"
        );
        assert!(
            !last.contains("filter") && !last.contains("enter"),
            "footer text leaked onto the last row: {last}"
        );
    }

    #[test]
    fn keep_emu_line_drops_noise_lines() {
        let cases = [
            ("qol emu up", false),
            ("  hint: use -v/--verbose for detailed output", false),
            ("", false),
            ("   ", false),
            ("  boot     foo · qmp 127.0.0.1:1234", true),
            ("  verdict  pass · no qol traces survive", true),
        ];
        for (line, kept) in cases {
            assert_eq!(keep_emu_line(line), kept, "line: {line:?}");
        }
    }

    #[test]
    fn humanize_check_id_titlecases_first_word_only() {
        let cases = [
            ("plugin_staleness", "Plugin staleness"),
            ("install_identity", "Install identity"),
            ("rust_formatting", "Rust formatting"),
        ];
        for (id, expected) in cases {
            assert_eq!(humanize_check_id(id), expected, "id: {id}");
        }
    }

    #[test]
    fn friendly_doctor_line_hides_detail_for_ok_and_keeps_it_for_warn() {
        let ok = friendly_doctor_line("[OK] install_identity: marker and id are aligned (x)");
        assert_eq!(
            ok.to_string(),
            "  ✓ Install identity",
            "ok hides the detail"
        );

        let warn = friendly_doctor_line(
            "[WARN] plugin_staleness: plugin staleness detected \u{2014} rebuild required: a, b",
        );
        assert_eq!(
            warn.to_string(),
            "  ▲ Plugin staleness - rebuild required: a, b",
            "warn humanizes the name and keeps the post-dash detail",
        );
    }

    #[test]
    fn doctor_lines_fix_mode_keeps_only_after_block() {
        let text = "Doctor Check (Before)\n[WARN] a: x\nSummary: ok=0\n\nDoctor Check (After)\n[OK] a: x\nSummary: ok=1\n";
        assert_eq!(
            doctor_lines(text, DoctorMode::Check).len(),
            2,
            "check keeps every bracket line"
        );
        assert_eq!(
            doctor_lines(text, DoctorMode::Fix),
            vec!["[OK] a: x".to_string()],
            "fix keeps only the after block"
        );
    }

    #[test]
    fn edit_filters_adds_cycles_edits_and_deletes() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Logs;
        dash.open_filter_manager();
        edit_filters(&mut dash, KeyCode::Enter);
        for c in "focus".chars() {
            edit_filters(&mut dash, KeyCode::Char(c));
        }
        edit_filters(&mut dash, KeyCode::Backspace);
        edit_filters(&mut dash, KeyCode::Down);
        edit_filters(&mut dash, KeyCode::Enter);
        assert_eq!(
            dash.filters.logs,
            vec![log_filter(FilterStrategy::Exclude, "focu")]
        );
        assert!(matches!(dash.filter_state, FilterState::Managing));

        edit_filters(&mut dash, KeyCode::Char('e'));
        edit_filters(&mut dash, KeyCode::Backspace);
        edit_filters(&mut dash, KeyCode::Up);
        edit_filters(&mut dash, KeyCode::Enter);
        assert_eq!(
            dash.filters.logs,
            vec![log_filter(FilterStrategy::Include, "foc")]
        );

        edit_filters(&mut dash, KeyCode::Char('d'));
        assert!(
            dash.active_filters().is_empty(),
            "d deletes the selected filter"
        );
        edit_filters(&mut dash, KeyCode::Esc);
        assert!(matches!(dash.filter_state, FilterState::Closed));
    }

    #[test]
    fn filters_are_per_view_and_survive_navigation() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Logs;
        set_active_filters(
            &mut dash,
            vec![log_filter(FilterStrategy::Exclude, "GHOSTWIN")],
        );
        dash.view = View::Trace;
        set_active_filters(
            &mut dash,
            vec![log_filter(FilterStrategy::Include, "focus")],
        );

        assert_eq!(
            dash.filters.logs,
            vec![log_filter(FilterStrategy::Exclude, "GHOSTWIN")],
            "logs view keeps its own filter set"
        );
        assert_eq!(
            dash.filters.trace,
            vec![log_filter(FilterStrategy::Include, "focus")],
            "trace view keeps a separate filter set"
        );

        dash.view = View::Logs;
        apply_action(&mut dash, Action::ToggleView, false);
        apply_action(&mut dash, Action::ToggleView, false);
        assert_eq!(dash.view, View::Logs);
        assert_eq!(
            dash.filters.logs,
            vec![log_filter(FilterStrategy::Exclude, "GHOSTWIN")],
            "navigating away and back must not wipe per-view filters"
        );
        assert_eq!(
            dash.filters.trace,
            vec![log_filter(FilterStrategy::Include, "focus")],
            "other views keep their filters through navigation"
        );
    }

    #[test]
    fn console_state_round_trips_through_json() {
        let state = ConsoleState {
            filters: ViewFilters {
                logs: vec![log_filter(FilterStrategy::Exclude, "noise")],
                trace: vec![log_filter(FilterStrategy::Include, "focus")],
                emu: Vec::new(),
            },
            trace_details: true,
            trace_rate: TraceRate::Realtime,
            keys_hidden: true,
            feature_flags: Vec::new(),
        };
        let json = serde_json::to_string(&state).expect("serialize console state");
        let back: ConsoleState = serde_json::from_str(&json).expect("deserialize console state");
        assert_eq!(back.filters, state.filters);
        assert!(back.trace_details);
        assert_eq!(back.trace_rate, TraceRate::Realtime);
        assert!(back.keys_hidden);
    }

    #[test]
    fn console_state_defaults_every_missing_field() {
        let state: ConsoleState = serde_json::from_str("{}").expect("empty object deserializes");
        assert!(state.filters.logs.is_empty());
        assert!(state.filters.trace.is_empty());
        assert!(!state.trace_details);
        assert_eq!(state.trace_rate, TraceRate::Relaxed);
        assert!(!state.keys_hidden);
        assert!(state.feature_flags.is_empty());
    }

    #[test]
    fn dash_applies_saved_state_without_marking_dirty_and_exports_it_back() {
        let mut dash = Dash::new(Vec::new());
        let state = ConsoleState {
            filters: ViewFilters {
                logs: Vec::new(),
                trace: vec![log_filter(FilterStrategy::Include, "focus")],
                emu: Vec::new(),
            },
            trace_details: true,
            trace_rate: TraceRate::Realtime,
            keys_hidden: true,
            feature_flags: Vec::new(),
        };
        dash.apply_state(state);

        assert_eq!(
            dash.filters.trace,
            vec![log_filter(FilterStrategy::Include, "focus")]
        );
        assert!(dash.trace_details);
        assert_eq!(dash.trace_rate, TraceRate::Realtime);
        assert!(dash.keys_hidden);
        assert!(
            !dash.state_dirty,
            "loading saved state must not schedule a redundant save"
        );

        let exported = dash.to_state();
        assert_eq!(exported.filters.trace, dash.filters.trace);
        assert!(exported.trace_details);
        assert_eq!(exported.trace_rate, TraceRate::Realtime);
        assert!(exported.keys_hidden);
    }

    #[test]
    fn persistable_mutations_mark_state_dirty() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Logs;
        dash.open_filter_manager();
        edit_filters(&mut dash, KeyCode::Enter);
        for c in "focus".chars() {
            edit_filters(&mut dash, KeyCode::Char(c));
        }
        edit_filters(&mut dash, KeyCode::Enter);
        assert!(dash.state_dirty, "saving a filter should mark state dirty");

        let mut dash = Dash::new(Vec::new());
        set_trace_details(&mut dash, true);
        assert!(dash.state_dirty, "toggling trace detail should mark dirty");

        let mut dash = Dash::new(Vec::new());
        apply_action(&mut dash, Action::ToggleKeys, false);
        assert!(dash.state_dirty, "toggling the keys HUD should mark dirty");
    }

    #[test]
    fn trace_view_legend_exposes_the_rate_toggle() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Trace;
        let hints = unique_hints(context_action_bindings(&dash));
        let rate = hints
            .iter()
            .find(|h| h.key == "s")
            .expect("trace legend must surface the rate toggle on 's'");
        assert!(
            rate.desc.contains("relaxed"),
            "relaxed is the default, shown in the legend: {}",
            rate.desc
        );
        assert_eq!(
            action_for(&dash, KeyCode::Char('s'), KeyModifiers::NONE),
            Action::ToggleTraceRate,
            "'s' in the trace view toggles the reporting rate"
        );
    }

    #[test]
    fn rate_toggle_flips_relaxed_and_realtime_and_persists() {
        let mut dash = Dash::new(Vec::new());
        assert_eq!(dash.trace_rate, TraceRate::Relaxed, "relaxed by default");
        apply_action(&mut dash, Action::ToggleTraceRate, false);
        assert_eq!(dash.trace_rate, TraceRate::Realtime);
        assert!(dash.state_dirty, "rate change must persist");
        apply_action(&mut dash, Action::ToggleTraceRate, false);
        assert_eq!(dash.trace_rate, TraceRate::Relaxed);
    }

    #[test]
    fn relaxed_folds_repeats_while_realtime_shows_every_event() {
        // relaxed (collapse on): distinct events all show; identical repeats fold to one xN
        let mut pane = LogPane::collapsing();
        pane.push("[10:00:00.001] AMC poll".to_string());
        pane.push("[10:00:00.002] FOCUS bar".to_string());
        for ms in 3..=10 {
            pane.push(format!("[10:00:00.0{ms:02}] AMC poll"));
        }
        assert_eq!(
            pane.ring.len(),
            3,
            "distinct lines stay; the trailing identical burst folds to one"
        );
        assert!(strip_ansi(&pane.ring.lines[2]).contains("(\u{d7}8)"));

        // realtime (collapse off): every individual event is shown
        let mut pane = LogPane::new();
        pane.set_collapse(false);
        for ms in 1..=8 {
            pane.push(format!("[10:00:00.0{ms:02}] AMC poll"));
        }
        assert_eq!(pane.ring.len(), 8, "realtime keeps every occurrence");
    }

    #[test]
    fn rate_toggle_switches_collapse_and_resets_fold_tracking() {
        let mut dash = Dash::new(Vec::new());
        assert!(dash.trace.ring.collapse, "relaxed default folds repeats");
        dash.trace.push("[10:00:00.001] AMC poll".to_string());
        dash.trace.push("[10:00:00.002] AMC poll".to_string());
        assert_eq!(dash.trace.ring.len(), 1, "repeats fold while relaxed");

        apply_action(&mut dash, Action::ToggleTraceRate, false);
        assert!(!dash.trace.ring.collapse, "realtime stops folding");
        dash.trace.push("[10:00:00.003] AMC poll".to_string());
        dash.trace.push("[10:00:00.004] AMC poll".to_string());
        assert_eq!(
            dash.trace.ring.len(),
            3,
            "each event becomes its own line in realtime"
        );
    }

    #[test]
    fn trace_args_relax_by_default_and_expand_when_detailed() {
        assert_eq!(
            trace_args(false),
            ["--no-header", "--no-ghosts", "--no-opacity"],
            "the console trace defaults to a relaxed, suppressed firehose"
        );
        assert_eq!(
            trace_args(true),
            ["--no-header", "--details"],
            "toggling detail opts into the full expanded trace"
        );
    }

    #[test]
    fn collapse_key_ignores_timestamp_and_ansi() {
        let a = collapse_key("\u{1b}[90m[10:00:00.001]\u{1b}[0m GHOSTWIN foo");
        let b = collapse_key("\u{1b}[90m[10:00:00.999]\u{1b}[0m GHOSTWIN foo");
        assert_eq!(a, b, "the same event at different times shares a key");
        assert_eq!(a, "GHOSTWIN foo");
        assert_ne!(
            a,
            collapse_key("[10:00:00.001] FOCUS bar"),
            "distinct events have distinct keys"
        );
    }

    #[test]
    fn collapsing_ring_folds_identical_consecutive_lines_with_a_count() {
        let mut ring = LogRing::collapsing();
        for ms in ["001", "050", "120"] {
            ring.push(format!("\u{1b}[90m[10:00:00.{ms}]\u{1b}[0m GHOSTWIN foo"));
        }
        assert_eq!(
            ring.len(),
            1,
            "events differing only by timestamp collapse to one line"
        );
        let folded = strip_ansi(&ring.lines[0]);
        assert!(folded.contains("GHOSTWIN foo"), "keeps the text: {folded}");
        assert!(
            folded.contains("(\u{d7}3)"),
            "shows the repeat count: {folded}"
        );

        ring.push("[10:00:00.200] FOCUS bar".to_string());
        assert_eq!(ring.len(), 2, "a distinct event starts a new line");

        ring.push("[10:00:00.260] FOCUS bar".to_string());
        assert_eq!(ring.len(), 2, "the new event collapses on repeat too");
        assert!(strip_ansi(&ring.lines[1]).contains("(\u{d7}2)"));
    }

    #[test]
    fn non_collapsing_ring_keeps_every_line() {
        let mut ring = LogRing::new();
        ring.push("[10:00:00.001] GHOSTWIN foo".to_string());
        ring.push("[10:00:00.050] GHOSTWIN foo".to_string());
        assert_eq!(ring.len(), 2, "the logs ring never folds repeats");
    }

    #[test]
    fn line_matches_filters_combines_include_and_exclude_rules() {
        let filters = vec![
            log_filter(FilterStrategy::Include, "shortcut"),
            log_filter(FilterStrategy::Include, "trace"),
            log_filter(FilterStrategy::Exclude, "success"),
        ];
        assert!(line_matches_filters("shortcut failed", &filters));
        assert!(line_matches_filters("trace emitted", &filters));
        assert!(!line_matches_filters("profile synced", &filters));
        assert!(!line_matches_filters("shortcut success", &filters));
    }

    #[test]
    fn exclude_only_filters_keep_non_matching_lines() {
        let filters = vec![log_filter(FilterStrategy::Exclude, "noise")];
        assert!(line_matches_filters("important trace", &filters));
        assert!(!line_matches_filters("noise trace", &filters));
    }

    #[test]
    fn edit_copy_accepts_only_digits_and_cancels() {
        let mut dash = Dash::new(Vec::new());
        dash.copying = true;
        for code in [KeyCode::Char('4'), KeyCode::Char('x'), KeyCode::Char('2')] {
            edit_copy(&mut dash, code);
        }
        assert_eq!(dash.copy_count, "42", "non-digits are ignored");
        edit_copy(&mut dash, KeyCode::Backspace);
        assert_eq!(dash.copy_count, "4", "backspace deletes last digit");
        edit_copy(&mut dash, KeyCode::Esc);
        assert!(dash.copy_count.is_empty(), "esc clears the count");
        assert!(!dash.copying, "esc exits copy mode");
    }

    #[test]
    fn newest_lines_takes_filtered_tail_and_strips_ansi() {
        let mut dash = Dash::new(Vec::new());
        dash.view = View::Trace;
        for line in ["alpha", "\u{1b}[31mbeta\u{1b}[0m", "gamma", "delta"] {
            dash.trace.push(line.to_string());
        }
        assert_eq!(
            newest_lines(&dash, 2),
            "gamma\ndelta",
            "tail of N newest lines"
        );
        set_active_filters(&mut dash, vec![log_filter(FilterStrategy::Include, "a")]);
        assert_eq!(
            newest_lines(&dash, 2),
            "gamma\ndelta",
            "filter keeps only matching lines before taking the tail"
        );
        set_active_filters(&mut dash, vec![log_filter(FilterStrategy::Include, "beta")]);
        assert_eq!(
            newest_lines(&dash, 5),
            "beta",
            "ansi codes are stripped from the copied text"
        );
    }

    #[test]
    fn doctor_line_style_colors_by_status() {
        let cases = [
            ("[OK] x: y", Some(("✓", Color::Green))),
            ("[WARN] x: y", Some(("▲", Color::Yellow))),
            ("[ERR] x: y", Some(("✗", Color::Red))),
            ("[CRASH] x: y", Some(("✗", Color::Magenta))),
            ("Summary: ok=1", None),
        ];
        for (input, expected) in cases {
            assert_eq!(doctor_line_style(input), expected, "input: {input}");
        }
    }

    #[test]
    fn parse_doctor_summary_reads_counts() {
        let report = parse_doctor_summary(
            "Doctor Check\n[OK] x: y\nSummary: ok=9, warn=2, error=1, crash=0",
        )
        .expect("summary present");
        assert_eq!(
            (report.ok, report.warn, report.error, report.crash),
            (9, 2, 1, 0)
        );
        assert_eq!(report.divergences(), 3);
        assert!(parse_doctor_summary("no summary line here").is_none());
    }

    #[test]
    fn forwarder_decodes_non_utf8_lossily_and_ends_on_eof() {
        let (tx, rx) = channel();
        let data = b"ok\n\xFF\xFEbad\nlast".to_vec();
        spawn_forwarder(std::io::Cursor::new(data), tx);
        let lines: Vec<String> = rx.iter().collect();
        assert_eq!(lines.len(), 3, "all lines delivered: {lines:?}");
        assert_eq!(lines[0], "ok");
        assert!(lines[1].contains("bad"), "lossy line kept: {:?}", lines[1]);
        assert_eq!(lines[2], "last", "no trailing newline still delivered");
    }

    #[test]
    fn log_ring_caps_and_evicts_oldest() {
        let mut ring = LogRing::new();
        for i in 0..(LOG_CAP + 3) {
            ring.push(format!("line-{i}"));
        }
        assert_eq!(ring.len(), LOG_CAP);
        assert_eq!(ring.lines.front().map(String::as_str), Some("line-3"));
    }

    #[test]
    fn window_math_keeps_tail_and_clamps() {
        let cases = [
            ("follow shows tail", 100, 10, 0, 90),
            ("scrolled up shifts window", 100, 10, 25, 65),
            ("short log starts at zero", 5, 10, 0, 0),
            ("offset beyond start clamps to zero", 100, 10, 500, 0),
        ];
        for (label, len, height, offset, want_start) in cases {
            assert_eq!(window_start(len, height, offset), want_start, "{label}");
        }
        assert_eq!(clamp_offset(100, 10, 500), 90, "clamp to len-height");
        assert_eq!(clamp_offset(5, 10, 3), 0, "short log clamps to zero");
    }

    #[test]
    fn emu_empty_lines_list_config_path() {
        let lines = emu_empty_lines("~/.config/qol-tray/emu.toml");
        assert_eq!(lines.len(), 2, "lines: {lines:?}");
    }
}
