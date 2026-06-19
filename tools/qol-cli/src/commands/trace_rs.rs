use anyhow::{bail, Context, Result};
use chrono::{Local, TimeZone};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_LOG_FILE: &str = "/tmp/qol-altmon.log";
const REPLAY_GAP_MS: u64 = 120;
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

const ANOMALY_MARKERS: [&str; 7] = [
    "MISDIRECTED",
    "FOCUS FAILURE",
    "SUPERSEDED",
    "DIVERGENCE",
    "Timed out",
    "THRASH",
    "REVERT",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Topic {
    All,
    Focus,
    Monitor,
    Boot,
    Opacity,
    Ui,
    Preview,
}

impl Topic {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "all" => Ok(Self::All),
            "focus" => Ok(Self::Focus),
            "monitor" => Ok(Self::Monitor),
            "boot" => Ok(Self::Boot),
            "opacity" => Ok(Self::Opacity),
            "ui" => Ok(Self::Ui),
            "preview" => Ok(Self::Preview),
            _ => bail!("unknown trace topic `{value}`"),
        }
    }

    fn matches(self, tag: &str) -> bool {
        match self {
            Self::All => true,
            Self::Ui => tag.starts_with("LAUNCHER_") || tag.starts_with("WORLD_"),
            Self::Preview => {
                tag.starts_with("PREVIEW_")
                    || tag.starts_with("REFRESH_")
                    || tag.starts_with("CAPTURE")
                    || matches!(
                        tag,
                        "SHOW_RECV" | "SHOW_TIMING" | "SHOW_PAINTED" | "FOCUS_WIN"
                    )
            }
            Self::Focus => matches!(
                tag,
                "FOCUS"
                    | "FOCUS_WIN"
                    | "ACTIVATE"
                    | "ACTIVATE_WIN"
                    | "WM_RECEIVE"
                    | "ALT_POLL_START"
                    | "DISMISS"
            ),
            Self::Monitor => {
                matches!(
                    tag,
                    "PUBLISH"
                        | "SUBSCRIBE"
                        | "RECV"
                        | "LEGEND"
                        | "AMC"
                        | "HOST_EMIT_AMC"
                        | "PLUGIN_RECV_AMC"
                )
            }
            Self::Boot => matches!(tag, "PUBLISH" | "SUBSCRIBE" | "RECV" | "LEGEND"),
            Self::Opacity => matches!(
                tag,
                "SHOW_WIN" | "HIDE_WIN" | "GHOSTWIN" | "GHOSTDUMP" | "SUMMARY"
            ),
        }
    }
}

struct Args {
    plugin: Option<String>,
    topic: Topic,
    grep: Option<String>,
    since: Option<Duration>,
    mark: Option<String>,
    replay: bool,
    details: bool,
    anomalies: bool,
    no_ghosts: bool,
    no_opacity: bool,
}

impl Args {
    fn parse(args: &[OsString]) -> Result<Self> {
        let mut plugin = None;
        let mut topic = Topic::All;
        let mut grep = None;
        let mut since = None;
        let mut mark = None;
        let mut replay = false;
        let mut details = false;
        let mut anomalies = false;
        let mut no_ghosts = false;
        let mut no_opacity = false;
        let mut focus_only = false;

        let mut iter = args.iter().peekable();
        while let Some(arg) = iter.next() {
            let value = arg
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("trace-rs argument is not valid UTF-8"))?;
            match value {
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                "-f" | "--focus-only" => focus_only = true,
                "-g" | "--no-ghosts" => no_ghosts = true,
                "-o" | "--no-opacity" => no_opacity = true,
                "-d" | "--details" => details = true,
                "--replay" => replay = true,
                "--anomalies" => anomalies = true,
                "--stats" => {}
                "--topic" => topic = Topic::parse(next_value(&mut iter, "--topic")?)?,
                "--grep" => grep = Some(next_value(&mut iter, "--grep")?.to_string()),
                "--since" => since = Some(parse_duration(next_value(&mut iter, "--since")?)?),
                "--mark" => mark = Some(next_value(&mut iter, "--mark")?.to_string()),
                _ if value.starts_with("--topic=") => {
                    topic = Topic::parse(value.trim_start_matches("--topic="))?
                }
                _ if value.starts_with("--grep=") => {
                    grep = Some(value.trim_start_matches("--grep=").to_string())
                }
                _ if value.starts_with("--since=") => {
                    since = Some(parse_duration(value.trim_start_matches("--since="))?)
                }
                _ if value.starts_with("--mark=") => {
                    mark = Some(value.trim_start_matches("--mark=").to_string())
                }
                "focus" => focus_only = true,
                positional if positional.starts_with('-') => {
                    bail!("unknown trace-rs flag `{positional}`")
                }
                positional => {
                    if plugin.is_some() {
                        bail!("usage: qol trace-rs [plugin|focus] [flags]");
                    }
                    plugin = Some(positional.to_string());
                }
            }
        }

        if focus_only {
            topic = Topic::Focus;
        }
        if plugin.as_deref() == Some("runtime") {
            plugin = None;
        }

        Ok(Self {
            plugin,
            topic,
            grep,
            since,
            mark,
            replay,
            details,
            anomalies,
            no_ghosts,
            no_opacity,
        })
    }
}

fn next_value<'a>(
    iter: &mut std::iter::Peekable<std::slice::Iter<'a, OsString>>,
    flag: &str,
) -> Result<&'a str> {
    iter.next()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))
}

fn parse_duration(value: &str) -> Result<Duration> {
    let digits = value
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        bail!("invalid duration `{value}`");
    }
    let amount = digits.parse::<u64>()?;
    let suffix = &value[digits.len()..];
    let seconds = match suffix {
        "" | "s" => amount,
        "m" => amount * 60,
        "h" => amount * 60 * 60,
        _ => bail!("invalid duration unit `{suffix}`"),
    };
    Ok(Duration::from_secs(seconds))
}

fn print_help() {
    println!(
        "qol trace-rs [plugin|focus] [--replay] [--details] [--since 10s] [--grep text]\n\
         \n\
         Rust formatter for /tmp/qol-altmon.log. `qol trace` still uses the Python formatter."
    );
}

pub(crate) fn run(args: &[OsString]) -> Result<()> {
    let args = Args::parse(args)?;
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

fn log_file() -> PathBuf {
    std::env::var_os("QOL_TRACE_LOG_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_LOG_FILE))
}

fn write_mark(path: &PathBuf, message: &str) -> Result<()> {
    let ts = now_ms();
    let pid = std::process::id();
    let escaped = message.replace('"', "\\\"");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open trace log {}", path.display()))?;
    writeln!(file, "{ts} pid={pid} MARK message=\"{escaped}\"")?;
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

struct RawLine {
    ts_ms: u64,
    pid: String,
    tag: String,
    msg: String,
}

fn parse_raw_line(line: &str) -> Option<RawLine> {
    let (ts, rest) = line.split_once(" pid=")?;
    let ts_ms = ts.parse().ok()?;
    let (pid, rest) = rest.split_once(' ')?;
    let (tag, msg) = rest.split_once(' ')?;
    Some(RawLine {
        ts_ms,
        pid: pid.to_string(),
        tag: tag.to_string(),
        msg: msg.to_string(),
    })
}

struct Event {
    ts_ms: u64,
    ts: String,
    tag: String,
    source: String,
    text: String,
}

struct TraceRunner {
    args: Args,
    path: PathBuf,
    pid_names: HashMap<String, String>,
    buffer: Vec<Event>,
    last_event_at: Option<Instant>,
    last_pick: Option<(String, String, String, String, String)>,
    monitors: Vec<(i64, i64, i64, i64)>,
    last_summary: Option<String>,
}

impl TraceRunner {
    fn new(args: Args, path: PathBuf) -> Self {
        Self {
            args,
            path,
            pid_names: HashMap::new(),
            buffer: Vec::new(),
            last_event_at: None,
            last_pick: None,
            monitors: Vec::new(),
            last_summary: None,
        }
    }

    fn run(&mut self) -> Result<()> {
        self.print_header();
        let start_ts = self.start_ts();
        let file = File::open(&self.path)
            .with_context(|| format!("failed to open trace log {}", self.path.display()))?;
        let mut reader = BufReader::new(file);

        if self.args.replay {
            println!("{COLOR_DIM}Replaying full log...{COLOR_RESET}\n");
            self.replay(&mut reader, start_ts)?;
            self.flush();
            return Ok(());
        }

        if start_ts.is_none() {
            reader.seek(SeekFrom::End(0))?;
        }
        self.tail(reader, start_ts)
    }

    fn print_header(&self) {
        let mode = if self.args.details {
            "expanded"
        } else {
            "collapsed"
        };
        match self.args.plugin.as_deref() {
            Some(plugin) => println!(
                "{COLOR_HEADER}Tailing {} filtering for {COLOR_OK}{plugin}{COLOR_HEADER} with Rust...{COLOR_RESET}",
                self.path.display()
            ),
            None => println!(
                "{COLOR_HEADER}Tailing {} (Rust runtime trace)...{COLOR_RESET}",
                self.path.display()
            ),
        }
        println!("{COLOR_DIM}Aggregating transitions into {mode} trace groups.{COLOR_RESET}\n");
    }

    fn start_ts(&self) -> Option<u64> {
        let since = self.args.since?;
        Some(now_ms().saturating_sub(since.as_millis() as u64))
    }

    fn replay(&mut self, reader: &mut BufReader<File>, start_ts: Option<u64>) -> Result<()> {
        let mut prev_ts = None;
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            let Some(raw) = parse_raw_line(line.trim_end()) else {
                continue;
            };
            if start_ts.is_some_and(|start| raw.ts_ms < start) {
                continue;
            }
            if prev_ts.is_some_and(|prev| raw.ts_ms.saturating_sub(prev) > REPLAY_GAP_MS) {
                self.flush();
            }
            prev_ts = Some(raw.ts_ms);
            self.process_raw(raw);
        }
        Ok(())
    }

    fn tail(&mut self, mut reader: BufReader<File>, start_ts: Option<u64>) -> Result<()> {
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                if self
                    .last_event_at
                    .is_some_and(|at| at.elapsed() > TAIL_FLUSH_AFTER)
                {
                    self.flush();
                }
                std::thread::sleep(TAIL_IDLE_SLEEP);
                continue;
            }
            let Some(raw) = parse_raw_line(line.trim_end()) else {
                continue;
            };
            if start_ts.is_some_and(|start| raw.ts_ms < start) {
                continue;
            }
            self.process_raw(raw);
        }
    }

    fn process_raw(&mut self, raw: RawLine) {
        if !self.args.topic.matches(&raw.tag) {
            return;
        }
        if self.args.no_ghosts && matches!(raw.tag.as_str(), "GHOSTDUMP" | "GHOSTWIN" | "SUMMARY") {
            return;
        }
        if self.args.no_opacity && matches!(raw.tag.as_str(), "HIDE_WIN" | "SHOW_WIN") {
            return;
        }
        self.register_monitors(&raw.msg);

        let Some(mut event) = self.format_event(raw) else {
            return;
        };
        if let Some(plugin) = self.args.plugin.as_deref() {
            if event.source != plugin {
                return;
            }
        }
        if let Some(grep) = self.args.grep.as_deref() {
            if !event.text.to_lowercase().contains(&grep.to_lowercase()) {
                return;
            }
        }
        if self.args.anomalies
            && !ANOMALY_MARKERS
                .iter()
                .any(|marker| event.text.contains(marker))
        {
            return;
        }
        if event.tag == "SUMMARY" && self.last_summary.as_ref() == Some(&event.text) {
            return;
        }
        if event.tag == "SUMMARY" {
            self.last_summary = Some(event.text.clone());
        }

        event.ts = format_timestamp(event.ts_ms);
        self.buffer.push(event);
        self.last_event_at = Some(Instant::now());
    }

    fn format_event(&mut self, raw: RawLine) -> Option<Event> {
        let source = self.source_for(&raw);
        let text = match raw.tag.as_str() {
            "PICK" => self.format_pick(&raw)?,
            "HOST_EMIT_AMC" => {
                let new_idx = field(&raw.msg, "new_idx").unwrap_or("?");
                let is_boot = field(&raw.msg, "is_boot").unwrap_or("?");
                format!("HOST_EMIT_AMC: new_idx={new_idx} (is_boot={is_boot})")
            }
            "PLUGIN_RECV_AMC" => {
                let idx = field(&raw.msg, "monitor_idx").unwrap_or("?");
                format!("PLUGIN_RECV_AMC: monitor_idx={idx}")
            }
            "SUBSCRIBE" => format!("SUBSCRIBE {}", raw.msg),
            "PUBLISH" => format!("PUBLISH {}", raw.msg),
            "MARK" => quoted_field(&raw.msg, "message")
                .map(|message| format!("MARK: {message}"))
                .unwrap_or_else(|| format!("MARK: {}", raw.msg)),
            _ => {
                let color = if raw.msg.contains("DIVERGENCE") {
                    COLOR_FAIL
                } else {
                    ""
                };
                let reset = if color.is_empty() { "" } else { COLOR_RESET };
                format!("{color}{}: {}{reset}", raw.tag, raw.msg)
            }
        };
        Some(Event {
            ts_ms: raw.ts_ms,
            ts: String::new(),
            tag: raw.tag,
            source,
            text,
        })
    }

    fn format_pick(&mut self, raw: &RawLine) -> Option<String> {
        if self.args.plugin.is_some() {
            return None;
        }
        let cursor = tuple_field(&raw.msg, "cursor")?;
        let focus = tuple_field(&raw.msg, "focus")?;
        let cursor_age = field(&raw.msg, "cursor_age_ms")?.to_string();
        let focus_age = field(&raw.msg, "focus_age_ms")?.to_string();
        let winner = field(&raw.msg, "winner")?.to_string();
        let key = (
            winner.clone(),
            cursor.0.to_string(),
            cursor.1.to_string(),
            focus.0.to_string(),
            focus.1.to_string(),
        );
        if self.last_pick.as_ref() == Some(&key) {
            return None;
        }
        self.last_pick = Some(key);

        let cursor_age_ms = cursor_age.parse::<u64>().unwrap_or(0);
        let focus_age_ms = focus_age.parse::<u64>().unwrap_or(0);
        let cursor_status = active_status(cursor_age_ms);
        let focus_status = active_status(focus_age_ms);
        let winner_color = if winner == "cursor" {
            COLOR_OK
        } else {
            COLOR_FOCUS
        };
        Some(format!(
            "Winner -> {winner_color}{}{COLOR_RESET} | Cursor: {} (age: {:.2}s {cursor_status}) | Focus: {} (age: {:.2}s {focus_status})",
            winner.to_uppercase(),
            self.monitor_name(cursor.0, cursor.1),
            cursor_age_ms as f64 / 1000.0,
            self.monitor_name(focus.0, focus.1),
            focus_age_ms as f64 / 1000.0,
        ))
    }

    fn source_for(&mut self, raw: &RawLine) -> String {
        match raw.tag.as_str() {
            "PICK" | "HOST_EMIT_AMC" | "PUBLISH" | "LEGEND" | "MARK" => "host".to_string(),
            "SUBSCRIBE" => field(&raw.msg, "plugin")
                .map(ToString::to_string)
                .unwrap_or_else(|| self.process_name(&raw.pid)),
            tag if tag.starts_with("PROFILE_") => "profile".to_string(),
            tag if tag.starts_with("WORLD_") => "world".to_string(),
            tag if tag.starts_with("WINACT_") => "window-actions".to_string(),
            tag if tag.starts_with("CLI_SESSIONS") => {
                fallback_process(&self.process_name(&raw.pid), "cli-sessions")
            }
            tag if tag.starts_with("PREVIEW_")
                || tag.starts_with("REFRESH_")
                || tag == "CAPTURE"
                || tag == "SHOW_LIST" =>
            {
                fallback_process(&self.process_name(&raw.pid), "alt-tab")
            }
            tag if tag.starts_with("LAUNCHER_") => {
                fallback_process(&self.process_name(&raw.pid), "launcher")
            }
            _ => self.process_name(&raw.pid),
        }
    }

    fn process_name(&mut self, pid: &str) -> String {
        if let Some(name) = self.pid_names.get(pid) {
            return name.clone();
        }
        let name = Command::new("ps")
            .args(["-p", pid, "-o", "ucomm="])
            .output()
            .ok()
            .and_then(|output| {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                (!text.is_empty()).then_some(text)
            })
            .unwrap_or_else(|| pid.to_string());
        self.pid_names.insert(pid.to_string(), name.clone());
        name
    }

    fn register_monitors(&mut self, msg: &str) {
        for token in msg.split_whitespace() {
            if let Some(bounds) = parse_at_bounds(token) {
                self.push_monitor(bounds);
            }
        }
    }

    fn push_monitor(&mut self, bounds: (i64, i64, i64, i64)) {
        if self.monitors.contains(&bounds) {
            return;
        }
        self.monitors.push(bounds);
        self.monitors.sort_by_key(|(x, y, _, _)| (*x, *y));
    }

    fn monitor_name(&self, x: i64, y: i64) -> String {
        self.monitors
            .iter()
            .enumerate()
            .find(|(_, (mx, my, w, h))| *mx <= x && x < *mx + *w && *my <= y && y < *my + *h)
            .map(|(idx, _)| format!("Mon {idx}"))
            .unwrap_or_else(|| format!("({x},{y})"))
    }

    fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        let mut seen = HashSet::new();
        let mut unique = Vec::new();
        for event in self.buffer.drain(..) {
            if !event.tag.starts_with("WINACT_") && !seen.insert(event.text.clone()) {
                continue;
            }
            unique.push(event);
        }
        if unique.is_empty() {
            return;
        }
        render_group(&unique, self.args.details);
        println!();
        self.last_event_at = None;
    }
}

fn render_group(events: &[Event], details: bool) {
    let root = &events[0];
    let span_ms = events
        .last()
        .map(|last| last.ts_ms.saturating_sub(root.ts_ms))
        .unwrap_or(0);
    let latency = if span_ms > 0 {
        format!(" {COLOR_TIME}(span: {span_ms}ms){COLOR_RESET}")
    } else {
        String::new()
    };
    let src_tag = format!(
        "{}[{}]{COLOR_RESET} ",
        hash_color(&root.source),
        root.source
    );
    if events.len() == 1 {
        println!(
            "{COLOR_TIME}[{}]{COLOR_RESET} ── {src_tag}{}{}",
            root.ts, root.text, latency
        );
        return;
    }
    if !details {
        println!(
            "{COLOR_TIME}[{}]{COLOR_RESET} ── {src_tag}{}{} {}",
            root.ts,
            root.text,
            latency,
            detail_suffix(events.len() - 1)
        );
        return;
    }
    println!(
        "{COLOR_TIME}[{}]{COLOR_RESET} ┌── {src_tag}{}{}",
        root.ts, root.text, latency
    );
    for (idx, event) in events.iter().enumerate().skip(1) {
        let connector = if idx == events.len() - 1 {
            "└── "
        } else {
            "├── "
        };
        let src_tag = format!(
            "{}[{}]{COLOR_RESET} ",
            hash_color(&event.source),
            event.source
        );
        println!(
            "{COLOR_TIME}[{}]{COLOR_RESET} │   {connector}{src_tag}{}",
            event.ts, event.text
        );
    }
}

fn detail_suffix(hidden_count: usize) -> String {
    let noun = if hidden_count == 1 {
        "detail"
    } else {
        "details"
    };
    format!("{COLOR_DIM}(+{hidden_count} {noun}){COLOR_RESET}")
}

fn format_timestamp(ms: u64) -> String {
    let Some(dt) = Local.timestamp_millis_opt(ms as i64).single() else {
        return ms.to_string();
    };
    dt.format("%H:%M:%S.%3f").to_string()
}

fn hash_color(name: &str) -> &'static str {
    const COLORS: [&str; 8] = [
        "\x1b[1;34m",
        "\x1b[1;35m",
        "\x1b[1;36m",
        "\x1b[1;32m",
        "\x1b[1;94m",
        "\x1b[1;95m",
        "\x1b[1;96m",
        "\x1b[1;92m",
    ];
    if matches!(name, "host" | "qol-tray" | "tray") {
        return COLOR_WARN;
    }
    let mut hash = 0i64;
    for ch in name.chars() {
        hash = ch as i64 + ((hash << 5) - hash);
    }
    COLORS[hash.unsigned_abs() as usize % COLORS.len()]
}

fn field<'a>(msg: &'a str, name: &str) -> Option<&'a str> {
    msg.split_whitespace().find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == name).then_some(value)
    })
}

fn quoted_field(msg: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = msg.find(&needle)? + needle.len();
    let rest = &msg[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn tuple_field(msg: &str, name: &str) -> Option<(i64, i64)> {
    let needle = format!("{name}=(");
    let start = msg.find(&needle)? + needle.len();
    let rest = &msg[start..];
    let end = rest.find(')')?;
    let (x, y) = rest[..end].split_once(',')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

fn active_status(age_ms: u64) -> String {
    if age_ms >= 1500 {
        format!("{COLOR_FAIL}(STALE){COLOR_RESET}")
    } else {
        format!("{COLOR_OK}(ACTIVE){COLOR_RESET}")
    }
}

fn fallback_process(actual: &str, fallback: &str) -> String {
    if actual.chars().all(|ch| ch.is_ascii_digit()) {
        fallback.to_string()
    } else {
        actual.to_string()
    }
}

fn parse_at_bounds(token: &str) -> Option<(i64, i64, i64, i64)> {
    let (_, rest) = token.split_once('@')?;
    let (x, rest) = rest.split_once(',')?;
    let (y, rest) = rest.split_once(',')?;
    let rest = rest.trim_end_matches(|ch: char| !ch.is_ascii_digit());
    let (w, h) = rest.split_once('x')?;
    Some((
        x.parse().ok()?,
        y.parse().ok()?,
        w.parse().ok()?,
        h.parse().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_raw_line_reads_probe_shape() {
        let raw = parse_raw_line(
            "1781848506980 pid=62404 CLI_SESSIONS_KITTEN args=[\"@\", \"ls\"] ok=true",
        )
        .expect("raw line");
        assert_eq!(raw.ts_ms, 1781848506980);
        assert_eq!(raw.pid, "62404");
        assert_eq!(raw.tag, "CLI_SESSIONS_KITTEN");
        assert_eq!(raw.msg, "args=[\"@\", \"ls\"] ok=true");
    }

    #[test]
    fn parse_args_accepts_legacy_focus() {
        let args = Args::parse(&["focus".into(), "--details".into()]).unwrap();
        assert_eq!(args.topic, Topic::Focus);
        assert!(args.details);
    }

    #[test]
    fn pick_events_are_deduped_until_winner_or_positions_change() {
        let args = Args::parse(&[]).unwrap();
        let mut runner = TraceRunner::new(args, PathBuf::from(DEFAULT_LOG_FILE));
        let raw = RawLine {
            ts_ms: 1,
            pid: "1".to_string(),
            tag: "PICK".to_string(),
            msg: "cursor=(0,0) cursor_age_ms=1 focus=(0,0) focus_age_ms=2 winner=cursor"
                .to_string(),
        };
        assert!(runner.format_pick(&raw).is_some());
        assert!(runner.format_pick(&raw).is_none());
    }

    #[test]
    fn duration_parser_handles_units() {
        assert_eq!(parse_duration("5s").unwrap(), Duration::from_secs(5));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn parses_monitor_bounds_from_trace_token() {
        assert_eq!(
            parse_at_bounds("target=0,0@0,0,1800x1169"),
            Some((0, 0, 1800, 1169))
        );
    }
}
