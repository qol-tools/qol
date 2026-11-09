use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use super::adopt_session::RESUMED_FROM;
use super::fork_state::{self, ForkState};

const MODEL_COLOR: &str = "\x1b[33m";
const COLOR_OFF: &str = "\x1b[39m";
const BOLD: &str = "\x1b[1m";
const BOLD_OFF: &str = "\x1b[22m";
const DIM: &str = "\x1b[2m";
const DIM_OFF: &str = "\x1b[22m";
const HOME_LABEL: &str = "~";
const ROOT_LABEL: &str = "/";
const FOLDER_SEPARATOR: char = '/';
const FOLDER_DEPTH: usize = 2;
const CWD_KEY: &str = "cwd";
const WORKING_LABEL: &str = "working";
const WAITING_LABEL: &str = "waiting";
const YOUR_TURN_LABEL: &str = "your turn";
const IDLE_LABEL: &str = "idle";
const WAITS_ON_LABEL: &str = "waits on";
const ASKS_LABEL: &str = "asks you";
const FRESH_MARK: char = '◉';
const FRESH_LOOK: &str = "\x1b[1m\x1b[38;5;147m";
const PARENT_KEY: &str = "parentAgentId";
const MARKS_EXTENSION: &str = ".json";
const ELLIPSIS: char = '…';
const GAP: &str = "  ";
const TAIL_SEPARATOR: &str = " · ";
const CLAUDE_PREFIX: &str = "claude-";
const KEY_SEPARATOR: &str = ": ";
const KEY_NAME_LIMIT: usize = 64;
const NAME_SHARE: usize = 3;
const NAME_MIN: usize = 12;
const SUMMARY_MIN: usize = 4;
const STARTED_FORMAT: &str = "%b %-d %H:%M";
const FIRST_ROW_LIMIT: u64 = 256 * 1024;
const COLOR_TAIL: u64 = 256 * 1024;
const COLOR_ROW_TAG: &str = "\"type\":\"agent-color\"";
pub(super) const CONTINUE_LEAD: &str = "Its transcript is ";
pub(super) const CONTINUE_END: &str = ": read the end of it";
const SESSION_COLORS: [(&str, &str); 8] = [
    ("red", "\x1b[31m"),
    ("green", "\x1b[32m"),
    ("yellow", "\x1b[33m"),
    ("blue", "\x1b[34m"),
    ("purple", "\x1b[35m"),
    ("cyan", "\x1b[36m"),
    ("orange", "\x1b[38;5;208m"),
    ("pink", "\x1b[38;5;211m"),
];
const KEY_COLORS: [&str; 7] = ["red", "blue", "green", "purple", "orange", "pink", "cyan"];

struct StateLook {
    state: &'static str,
    color: &'static str,
    bold: bool,
}

static STATE_LOOKS: [StateLook; 3] = [
    StateLook {
        state: WORKING_LABEL,
        color: "green",
        bold: false,
    },
    StateLook {
        state: WAITING_LABEL,
        color: "blue",
        bold: false,
    },
    StateLook {
        state: YOUR_TURN_LABEL,
        color: "yellow",
        bold: true,
    },
];

pub(super) fn state_looks() -> String {
    Value::Array(
        STATE_LOOKS
            .iter()
            .map(|look| json!({"state": look.state, "color": look.color, "bold": look.bold}))
            .collect(),
    )
    .to_string()
}

pub(super) fn state_word(state: &ForkState) -> &'static str {
    match state {
        ForkState::Working => WORKING_LABEL,
        ForkState::Waiting(_) => WAITING_LABEL,
        ForkState::YourTurn => YOUR_TURN_LABEL,
        ForkState::Idle => IDLE_LABEL,
    }
}

fn state_paint(state: &str) -> Option<String> {
    let look = STATE_LOOKS.iter().find(|look| look.state == state)?;
    let color = session_color(look.color)?;
    Some(if look.bold {
        format!("{BOLD}{color}")
    } else {
        color.to_owned()
    })
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Rows {
    columns: Option<usize>,
    transcript_path: Option<String>,
    tasks: Vec<Task>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Task {
    id: String,
    name: Option<String>,
    agent_type: Option<String>,
    description: String,
    label: Option<String>,
    status: Option<String>,
    start_time: Option<u64>,
    model: Option<String>,
    effort: Option<Value>,
    token_count: u64,
}

#[derive(Default)]
struct Fork {
    started_ms: Option<u64>,
    color: Option<String>,
    folder: Option<String>,
    parent: Option<String>,
    fresh: bool,
    asks: bool,
    picked: Picked,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct Picked {
    model: Option<String>,
    effort: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Marks {
    fresh: Vec<String>,
    asks: Vec<String>,
    models: HashMap<String, Picked>,
}

impl Marks {
    fn marked(&self, id: &str, fork: Fork) -> Fork {
        Fork {
            fresh: self.fresh.iter().any(|one| one == id),
            asks: self.asks.iter().any(|one| one == id),
            picked: self.models.get(id).cloned().unwrap_or_default(),
            ..fork
        }
    }
}

fn read_marks(folder: &Path, session: &Path) -> Marks {
    let Some(stem) = session.file_stem() else {
        return Marks::default();
    };
    let mut name = stem.to_os_string();
    name.push(MARKS_EXTENSION);
    std::fs::read(folder.join(name))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn session_marks(session: Option<&Path>) -> Marks {
    let folder = std::env::var_os(super::MARKS_ENV).filter(|folder| !folder.is_empty());
    match (session, folder) {
        (Some(session), Some(folder)) => read_marks(Path::new(&folder), session),
        _ => Marks::default(),
    }
}

struct Clock<'a> {
    now_ms: u64,
    fork: &'a dyn Fn(&str) -> Fork,
    state: &'a dyn Fn(&str, Option<&str>) -> Option<ForkState>,
    started_label: &'a dyn Fn(u64) -> Option<String>,
}

fn session_color(name: &str) -> Option<&'static str> {
    SESSION_COLORS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, code)| *code)
}

fn key_color(name: &str) -> &'static str {
    let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    let folded = hash ^ (hash >> 32);
    let pick = KEY_COLORS[(folded % KEY_COLORS.len() as u64) as usize];
    session_color(pick).unwrap_or(COLOR_OFF)
}

fn painted(text: &str, color: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    format!("{color}{text}{COLOR_OFF}")
}

fn model_label(model: &str) -> String {
    let (id, suffix) = match model.find('[') {
        Some(at) => (&model[..at], &model[at..]),
        None => (model, ""),
    };
    let Some(rest) = id.strip_prefix(CLAUDE_PREFIX) else {
        return model.to_owned();
    };
    let mut words = Vec::new();
    let mut version = Vec::new();
    for part in rest.split('-') {
        if part.is_empty() || (part.len() == 8 && part.chars().all(|c| c.is_ascii_digit())) {
            continue;
        }
        if part.chars().all(|c| c.is_ascii_digit()) {
            version.push(part);
        } else {
            let mut chars = part.chars();
            let first = chars.next().map(|c| c.to_ascii_uppercase());
            words.push(first.into_iter().chain(chars).collect::<String>());
        }
    }
    if words.is_empty() {
        return model.to_owned();
    }
    let mut label = words.join(" ");
    if !version.is_empty() {
        label.push(' ');
        label.push_str(&version.join("."));
    }
    label.push_str(suffix);
    label
}

fn effort_label(effort: &Value) -> Option<String> {
    match effort {
        Value::String(level) if !level.is_empty() => Some(level.clone()),
        Value::Number(budget) => Some(budget.to_string()),
        _ => None,
    }
}

fn count_label(count: u64) -> String {
    if count < 1000 {
        return count.to_string();
    }
    format!("{:.1}k", count as f64 / 1000.0)
}

fn tokens_label(count: u64) -> String {
    format!("{} tokens", count_label(count))
}

fn elapsed_label(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {}s", seconds / 60, seconds % 60),
        _ => format!("{}h {}m", seconds / 3600, seconds % 3600 / 60),
    }
}

fn local_started_label(ms: u64) -> Option<String> {
    let ms = i64::try_from(ms).ok()?;
    let time = chrono::DateTime::from_timestamp_millis(ms)?.with_timezone(&chrono::Local);
    Some(time.format(STARTED_FORMAT).to_string())
}

fn width(text: &str) -> usize {
    text.chars().count()
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut cut: String = text.chars().take(width - 1).collect();
    cut.push(ELLIPSIS);
    cut
}

fn plain(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn filled(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|text| !text.is_empty())
}

fn is_key_name(text: &str) -> bool {
    text.len() <= KEY_NAME_LIMIT
        && text.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn key_and_friendly(description: &str) -> (Option<&str>, Option<&str>) {
    let description = description.trim();
    if let Some((key, friendly)) = description.split_once(KEY_SEPARATOR) {
        if is_key_name(key.trim()) {
            return (Some(key.trim()), filled(Some(friendly)));
        }
    }
    if description.contains('-') && is_key_name(description) {
        return (Some(description), None);
    }
    (None, filled(Some(description)))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HeadFit {
    Full,
    NoEffort,
    NameFolder,
    NameShortFolder,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TailFit {
    Full,
    NoStarted,
    NoElapsed,
    ShortTokens,
    NoTokens,
}

const LADDER: [(HeadFit, TailFit); 8] = [
    (HeadFit::Full, TailFit::Full),
    (HeadFit::Full, TailFit::NoStarted),
    (HeadFit::Full, TailFit::NoElapsed),
    (HeadFit::NoEffort, TailFit::NoElapsed),
    (HeadFit::NameFolder, TailFit::NoElapsed),
    (HeadFit::NameShortFolder, TailFit::NoElapsed),
    (HeadFit::NameShortFolder, TailFit::ShortTokens),
    (HeadFit::NameShortFolder, TailFit::NoTokens),
];

struct Drawn {
    text: String,
    width: usize,
}

fn short_folder(label: &str) -> &str {
    match label.rsplit_once(FOLDER_SEPARATOR) {
        Some((_, last)) if !last.is_empty() => last,
        _ => label,
    }
}

fn looked(text: &str, paint: Option<&str>) -> String {
    match paint {
        Some(paint) => format!("{paint}{text}{BOLD_OFF}{COLOR_OFF}"),
        None => text.to_owned(),
    }
}

fn lead(fresh: bool) -> Drawn {
    if !fresh {
        return Drawn {
            text: GAP.to_owned(),
            width: width(GAP),
        };
    }
    Drawn {
        text: format!("{} ", looked(&FRESH_MARK.to_string(), Some(FRESH_LOOK))),
        width: width(&format!("{FRESH_MARK} ")),
    }
}

struct Head<'a> {
    name: &'a str,
    color: &'a str,
    lead: Drawn,
    badge: Drawn,
    model: Option<&'a str>,
    effort: Option<String>,
    folder: Option<&'a str>,
}

impl Head<'_> {
    fn drawn(&self, name: &str, fit: HeadFit) -> Drawn {
        let mut text = format!(
            "{}{}{}",
            self.lead.text,
            painted(name, self.color),
            self.badge.text
        );
        let mut width = self.lead.width + width(name) + self.badge.width;
        if matches!(fit, HeadFit::Full | HeadFit::NoEffort) {
            if let Some(model) = self.model.filter(|model| !model.is_empty()) {
                let label = plain(&model_label(model));
                width += GAP.len() + label.chars().count();
                text.push_str(&format!("{GAP}{MODEL_COLOR}{label}{COLOR_OFF}"));
                if fit == HeadFit::Full {
                    if let Some(effort) = self.effort.as_deref().filter(|effort| !effort.is_empty())
                    {
                        let effort = plain(effort);
                        width += 1 + effort.chars().count();
                        text.push(' ');
                        text.push_str(&effort);
                    }
                }
            }
        }
        if let Some(folder) = self.folder.filter(|folder| !folder.is_empty()) {
            let shown = match fit {
                HeadFit::NameShortFolder => short_folder(folder),
                _ => folder,
            };
            let shown = plain(shown);
            width += GAP.len() + shown.chars().count();
            text.push_str(&format!("{GAP}{DIM}{shown}{DIM_OFF}"));
        }
        Drawn { text, width }
    }

    fn named(&self, room: usize) -> Drawn {
        let name = |room: usize| truncate(self.name, room);
        let (lead, name, badge) = if self.lead.width + self.badge.width < room {
            (
                Some(&self.lead),
                name(room - self.lead.width - self.badge.width),
                Some(&self.badge),
            )
        } else if self.lead.width < room {
            (Some(&self.lead), name(room - self.lead.width), None)
        } else {
            (None, name(room), None)
        };
        let parts = [lead, badge].into_iter().flatten();
        Drawn {
            text: format!(
                "{}{}{}",
                lead.map_or("", |lead| lead.text.as_str()),
                painted(&name, self.color),
                badge.map_or("", |badge| badge.text.as_str())
            ),
            width: width(&name) + parts.map(|part| part.width).sum::<usize>(),
        }
    }
}

struct StateLabel {
    word: String,
    color: Option<String>,
}

fn state_label(state: &ForkState) -> StateLabel {
    let look = state_word(state);
    let word = match state {
        ForkState::Waiting(prs) if !prs.is_empty() => {
            let numbers = prs
                .iter()
                .filter_map(|pr| pr.rsplit('/').next())
                .map(|number| format!("#{number}"))
                .collect::<Vec<_>>();
            format!("{look} {}", numbers.join(","))
        }
        _ => look.to_owned(),
    };
    StateLabel {
        word,
        color: state_paint(look),
    }
}

#[derive(Default)]
struct Family {
    count: usize,
    asking: usize,
    busy: usize,
}

impl Family {
    fn of(id: &str, all: &[Shown]) -> Self {
        let mut family = Self::default();
        for one in all
            .iter()
            .filter(|one| one.fork.parent.as_deref() == Some(id))
        {
            let busy = one.fork.asks
                || matches!(one.state, Some(ForkState::Working | ForkState::Waiting(_)));
            family.count += 1;
            family.asking += usize::from(one.fork.asks);
            family.busy += usize::from(busy);
        }
        family
    }

    fn badge(&self) -> Drawn {
        if self.count == 0 {
            return Drawn {
                text: String::new(),
                width: 0,
            };
        }
        let (text, look) = if self.asking == 0 {
            (format!(" +{}", self.count), WAITING_LABEL)
        } else {
            (
                format!(
                    " +{}{TAIL_SEPARATOR}{} {ASKS_LABEL}",
                    self.count, self.asking
                ),
                YOUR_TURN_LABEL,
            )
        };
        Drawn {
            width: width(&text),
            text: looked(&text, state_paint(look).as_deref()),
        }
    }

    fn state(&self, own: Option<&ForkState>, asks: bool) -> Option<StateLabel> {
        if self.busy == 0 || asks || own == Some(&ForkState::Working) {
            return own.map(state_label);
        }
        Some(StateLabel {
            word: format!("{WAITS_ON_LABEL} {}", self.busy),
            color: state_paint(WAITING_LABEL),
        })
    }
}

struct Tail {
    state: Option<StateLabel>,
    started: Option<String>,
    elapsed: Option<String>,
    tokens: u64,
}

impl Tail {
    fn drawn(&self, fit: TailFit) -> Drawn {
        Drawn {
            text: self.text(fit, true),
            width: width(&self.text(fit, false)),
        }
    }

    fn text(&self, fit: TailFit, paint: bool) -> String {
        let mut parts = Vec::new();
        if let Some(state) = &self.state {
            parts.push(looked(
                &state.word,
                state.color.as_deref().filter(|_| paint),
            ));
        }
        if fit == TailFit::Full {
            parts.extend(self.started.clone());
        }
        if matches!(fit, TailFit::Full | TailFit::NoStarted) {
            parts.extend(self.elapsed.clone());
        }
        match fit {
            TailFit::Full | TailFit::NoStarted | TailFit::NoElapsed => {
                parts.push(tokens_label(self.tokens));
            }
            TailFit::ShortTokens => parts.push(count_label(self.tokens)),
            TailFit::NoTokens => {}
        }
        parts.join(TAIL_SEPARATOR)
    }
}

fn fitted(head: &Head, summary: &str, tail: &Tail, columns: usize) -> String {
    let name = head.name;
    let name_floor = width(name).min(NAME_MIN);
    for (head_fit, tail_fit) in LADDER {
        let Drawn {
            text: tail,
            width: tail_width,
        } = tail.drawn(tail_fit);
        let fixed = head.drawn("", head_fit).width + GAP.len() + tail_width;
        if name_floor + fixed > columns {
            continue;
        }
        if head_fit == HeadFit::Full && tail_fit == TailFit::Full && !summary.is_empty() {
            let capped = head.drawn(
                &truncate(
                    name,
                    (columns / NAME_SHARE).max(NAME_MIN).min(columns - fixed),
                ),
                head_fit,
            );
            let room = columns.saturating_sub(capped.width + GAP.len() * 2 + tail_width);
            if room > 0 && room >= SUMMARY_MIN.min(width(summary)) {
                let summary = truncate(summary, room);
                let pad = room - width(&summary) + GAP.len();
                return format!("{}{GAP}{summary}{}{tail}", capped.text, " ".repeat(pad));
            }
        }
        let shown = head.drawn(&truncate(name, columns - fixed), head_fit);
        let pad = columns - shown.width - tail_width;
        return format!("{}{}{tail}", shown.text, " ".repeat(pad));
    }
    let shown = tail.drawn(TailFit::NoTokens);
    if shown.width == 0 || shown.width + 2 > columns {
        return head.named(columns).text;
    }
    let named = head.named(columns - shown.width - 1);
    let pad = columns - named.width - shown.width;
    format!("{}{}{}", named.text, " ".repeat(pad), shown.text)
}

struct Shown<'a> {
    task: &'a Task,
    fork: Fork,
    state: Option<ForkState>,
}

fn row(one: &Shown, all: &[Shown], columns: Option<usize>, clock: &Clock) -> Option<String> {
    let task = one.task;
    let fork = &one.fork;
    let (key, friendly) = key_and_friendly(&task.description);
    let name = plain(
        filled(task.name.as_deref())
            .or(key)
            .or(friendly)
            .or(filled(task.agent_type.as_deref()))
            .unwrap_or("agent"),
    );
    let progress = filled(task.label.as_deref())
        .filter(|label| *label != task.description.trim() && Some(*label) != friendly);
    let summary = [
        friendly.map(plain).filter(|friendly| *friendly != name),
        progress.map(plain),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let summary = summary.trim();
    let color = fork
        .color
        .as_deref()
        .and_then(session_color)
        .unwrap_or_else(|| key_color(&name));
    let started = fork.started_ms.or(task.start_time);
    let family = Family::of(&task.id, all);
    let tail = Tail {
        state: family.state(one.state.as_ref(), fork.asks),
        started: started
            .and_then(clock.started_label)
            .map(|label| plain(&label)),
        elapsed: started.map(|start| elapsed_label(clock.now_ms.saturating_sub(start) / 1000)),
        tokens: task.token_count,
    };
    let head = Head {
        name: &name,
        color,
        lead: lead(fork.fresh),
        badge: family.badge(),
        model: fork.picked.model.as_deref().or(task.model.as_deref()),
        effort: fork
            .picked
            .effort
            .clone()
            .or_else(|| task.effort.as_ref().and_then(effort_label)),
        folder: fork.folder.as_deref(),
    };
    match columns {
        Some(0) => None,
        Some(columns) => Some(fitted(&head, summary, &tail, columns)),
        None => {
            let shown = head.drawn(&name, HeadFit::Full);
            let line = [
                shown.text.as_str(),
                summary,
                &tail.drawn(TailFit::Full).text,
            ]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(GAP);
            Some(line)
        }
    }
}

struct FirstRow {
    started_ms: Option<u64>,
    source: Option<PathBuf>,
}

fn prompt_text(row: &Value) -> String {
    match row.pointer("/message/content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn continued_from(prompt: &str) -> Option<PathBuf> {
    let (_, rest) = prompt.split_once(CONTINUE_LEAD)?;
    let (path, _) = rest.split_once(CONTINUE_END)?;
    let path = path.trim();
    (!path.is_empty() && !path.contains('\n')).then(|| PathBuf::from(path))
}

fn meta_of(transcript: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(transcript.with_extension("meta.json")).ok()?).ok()
}

fn meta_path(meta: &Value, key: &str) -> Option<PathBuf> {
    meta.get(key)
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn folder_names(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn last_names(names: &[String]) -> String {
    names[names.len().saturating_sub(FOLDER_DEPTH)..].join(&FOLDER_SEPARATOR.to_string())
}

fn folder_label(path: &Path, home: Option<&Path>) -> Option<String> {
    if let Some(home) = home.filter(|home| !folder_names(home).is_empty()) {
        if let Ok(below) = path.strip_prefix(home) {
            let names = folder_names(below);
            return Some(match names.as_slice() {
                [] => HOME_LABEL.to_owned(),
                [only] => format!("{HOME_LABEL}{FOLDER_SEPARATOR}{only}"),
                _ => last_names(&names),
            });
        }
    }
    let names = folder_names(path);
    if names.is_empty() {
        return (!path.as_os_str().is_empty()).then(|| ROOT_LABEL.to_owned());
    }
    Some(last_names(&names))
}

fn resumed_from(meta: &Value) -> Option<PathBuf> {
    meta_path(meta, RESUMED_FROM)
}

fn first_row(transcript: &Path) -> Option<FirstRow> {
    let file = File::open(transcript).ok()?;
    let mut line = String::new();
    BufReader::new(file.take(FIRST_ROW_LIMIT))
        .read_line(&mut line)
        .ok()?;
    let row: Value = serde_json::from_str(&line).ok()?;
    let started_ms = row
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|stamp| chrono::DateTime::parse_from_rfc3339(stamp).ok())
        .and_then(|stamp| u64::try_from(stamp.timestamp_millis()).ok());
    Some(FirstRow {
        started_ms,
        source: continued_from(&prompt_text(&row)),
    })
}

fn last_color(transcript: &Path) -> Option<String> {
    let mut file = File::open(transcript).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(COLOR_TAIL)))
        .ok()?;
    let mut tail = Vec::new();
    file.take(COLOR_TAIL).read_to_end(&mut tail).ok()?;
    String::from_utf8_lossy(&tail)
        .lines()
        .rev()
        .filter(|line| line.contains(COLOR_ROW_TAG))
        .find_map(|line| {
            let row: Value = serde_json::from_str(line).ok()?;
            row.get("agentColor")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

fn fork_facts(transcript: &Path) -> Fork {
    let meta = meta_of(transcript);
    let parent = meta
        .as_ref()
        .and_then(|meta| meta.get(PARENT_KEY))
        .and_then(Value::as_str)
        .filter(|parent| !parent.is_empty())
        .map(str::to_owned);
    let Some(first) = first_row(transcript) else {
        return Fork {
            parent,
            ..Fork::default()
        };
    };
    let home = qol_platform::launch_working_dir();
    Fork {
        parent,
        started_ms: first.started_ms,
        color: meta
            .as_ref()
            .and_then(resumed_from)
            .or(first.source)
            .as_deref()
            .and_then(last_color),
        folder: meta
            .as_ref()
            .and_then(|meta| meta_path(meta, CWD_KEY))
            .and_then(|cwd| folder_label(&cwd, home.as_deref())),
        ..Fork::default()
    }
}

fn render(raw: &str, clock: &Clock) -> Result<Vec<String>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let rows: Rows =
        serde_json::from_str(raw).context("the agent rows payload is not valid JSON")?;
    let shown: Vec<Shown> = rows
        .tasks
        .iter()
        .filter(|task| !task.id.is_empty())
        .map(|task| Shown {
            task,
            fork: (clock.fork)(&task.id),
            state: (clock.state)(&task.id, task.status.as_deref()),
        })
        .collect();
    Ok(shown
        .iter()
        .filter_map(|one| {
            let content = row(one, &shown, rows.columns, clock)?;
            Some(json!({"id": one.task.id, "content": content}).to_string())
        })
        .collect())
}

fn session_transcript(raw: &str) -> Option<PathBuf> {
    let rows: Rows = serde_json::from_str(raw).ok()?;
    filled(rows.transcript_path.as_deref()).map(PathBuf::from)
}

fn agent_transcript(session: Option<&Path>, id: &str) -> Option<PathBuf> {
    fork_state::transcript_of(session?, id)
}

fn session_state(session: Option<&Path>, id: &str, status: Option<&str>) -> Option<ForkState> {
    fork_state::state(status, agent_transcript(session, id).as_deref())
}

pub(super) fn run() -> Result<()> {
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .context("failed to read the agent rows payload")?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or_default();
    let session = session_transcript(&raw);
    let marks = session_marks(session.as_deref());
    let fork = |id: &str| {
        let facts = agent_transcript(session.as_deref(), id)
            .map(|transcript| fork_facts(&transcript))
            .unwrap_or_default();
        marks.marked(id, facts)
    };
    let state = |id: &str, status: Option<&str>| session_state(session.as_deref(), id, status);
    let clock = Clock {
        now_ms,
        fork: &fork,
        state: &state,
        started_label: &local_started_label,
    };
    for line in render(&raw, &clock)? {
        crate::cli::line(line);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000_000;
    const STARTED: &str = "Nov 1 07:58";

    const CYAN: &str = "\x1b[36m";

    fn cyan_fork(_id: &str) -> Fork {
        Fork {
            color: Some("cyan".to_owned()),
            ..Fork::default()
        }
    }

    fn fixed_label(_ms: u64) -> Option<String> {
        Some(STARTED.to_owned())
    }

    fn status_state(_id: &str, status: Option<&str>) -> Option<ForkState> {
        fork_state::state(status, None)
    }

    fn content(raw: Value) -> Vec<String> {
        content_with(raw, &status_state)
    }

    fn content_in(raw: Value, folder: &str) -> Vec<String> {
        let fork = |_id: &str| Fork {
            color: Some("cyan".to_owned()),
            folder: Some(folder.to_owned()),
            ..Fork::default()
        };
        let clock = Clock {
            now_ms: NOW,
            fork: &fork,
            state: &status_state,
            started_label: &fixed_label,
        };
        render(&raw.to_string(), &clock)
            .unwrap()
            .into_iter()
            .map(|line| {
                let line: Value = serde_json::from_str(&line).unwrap();
                line["content"].as_str().unwrap().to_owned()
            })
            .collect()
    }

    fn content_with(
        raw: Value,
        state: &dyn Fn(&str, Option<&str>) -> Option<ForkState>,
    ) -> Vec<String> {
        let clock = Clock {
            now_ms: NOW,
            fork: &cyan_fork,
            state,
            started_label: &fixed_label,
        };
        render(&raw.to_string(), &clock)
            .unwrap()
            .into_iter()
            .map(|line| {
                let line: Value = serde_json::from_str(&line).unwrap();
                line["content"].as_str().unwrap().to_owned()
            })
            .collect()
    }

    fn visible(row: &str) -> String {
        let mut shown = String::new();
        let mut chars = row.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' && chars.peek() == Some(&'[') {
                for skipped in chars.by_ref() {
                    if skipped == 'm' {
                        break;
                    }
                }
                continue;
            }
            shown.push(c);
        }
        shown
    }

    fn squeezed(rows: &[String]) -> Vec<String> {
        rows.iter()
            .map(|row| {
                visible(row)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn model_ids_read_as_family_and_version() {
        for (id, label) in [
            ("claude-opus-5-5", "Opus 5.5"),
            ("claude-sonnet-5-5", "Sonnet 5.5"),
            ("claude-haiku-4-5-20251001", "Haiku 4.5"),
            ("claude-3-5-sonnet-20241022", "Sonnet 3.5"),
            ("claude-opus-4-6[1m]", "Opus 4.6[1m]"),
            ("claude-fable", "Fable"),
            ("gpt-5", "gpt-5"),
            ("claude-", "claude-"),
        ] {
            assert_eq!(model_label(id), label, "{id}");
        }
    }

    #[test]
    fn a_row_names_the_agent_its_model_and_effort_before_the_summary() {
        let rows = content(json!({
            "columns": 120,
            "tasks": [{
                "id": "a1",
                "agentType": "project-fork",
                "description": "RTX 3080 limits research",
                "label": "Checking gpu-unlock RTX 3080 support",
                "startTime": NOW - 125_000,
                "model": "claude-opus-5-5",
                "effort": "high",
                "tokenCount": 31_234,
            }],
        }));
        let row = &rows[0];
        assert!(
            row.starts_with(
                "  \x1b[36mRTX 3080 limits research\x1b[39m  \x1b[33mOpus 5.5\x1b[39m high  Checking gpu-unlock RTX 3080 support"
            ),
            "{row:?}"
        );
        assert!(
            row.ends_with(&format!("  {STARTED} · 2m 5s · 31.2k tokens")),
            "{row:?}"
        );
        assert_eq!(visible(row).chars().count(), 120, "{row:?}");
    }

    #[test]
    fn a_narrow_row_cuts_the_summary_and_keeps_the_model() {
        let rows = content(json!({
            "columns": 60,
            "tasks": [{
                "id": "a1",
                "name": "fork-a",
                "description": "A very long description of the task at hand here",
                "model": "claude-sonnet-5-5",
                "effort": 4096,
                "tokenCount": 512,
            }],
        }));
        let visible = visible(&rows[0]);
        assert!(
            visible.starts_with("  fork-a  Sonnet 5.5 4096  A very"),
            "{visible:?}"
        );
        assert!(visible.contains('…'), "{visible:?}");
        assert!(visible.ends_with("  512 tokens"), "{visible:?}");
        assert_eq!(visible.chars().count(), 60, "{visible:?}");
    }

    #[test]
    fn the_start_date_shows_on_every_row_even_idle_and_without_a_width() {
        let task = json!({
            "id": "a1",
            "name": "fork-a",
            "description": "work",
            "status": "completed",
            "startTime": NOW - 5_000,
            "tokenCount": 10,
        });
        let wide = content(json!({"columns": 80, "tasks": [task.clone()]}));
        assert!(
            wide[0].ends_with(&format!("  idle · {STARTED} · 5s · 10 tokens")),
            "{wide:?}"
        );
        let unbounded = content(json!({"tasks": [task]}));
        assert_eq!(
            unbounded,
            [format!(
                "  {CYAN}fork-a{COLOR_OFF}  work  idle · {STARTED} · 5s · 10 tokens"
            )]
        );
    }

    #[test]
    fn the_transcript_start_outranks_the_run_start() {
        let fork = |id: &str| Fork {
            started_ms: (id == "a1").then_some(NOW - 3_600_000),
            ..Fork::default()
        };
        let label = |ms: u64| Some(format!("at {ms}"));
        let clock = Clock {
            now_ms: NOW,
            fork: &fork,
            state: &status_state,
            started_label: &label,
        };
        let raw = json!({
            "tasks": [
                {"id": "a1", "name": "old", "startTime": NOW - 1_000},
                {"id": "a2", "name": "new", "startTime": NOW - 1_000},
            ],
        });
        let rows = render(&raw.to_string(), &clock).unwrap();
        assert!(
            rows[0].contains(&format!("at {} · 1h 0m", NOW - 3_600_000)),
            "{rows:?}"
        );
        assert!(
            rows[1].contains(&format!("at {} · 1s", NOW - 1_000)),
            "{rows:?}"
        );
    }

    #[test]
    fn the_model_and_effort_the_board_picked_outrank_the_ones_the_fork_started_with() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("c0ab.json"),
            json!({"models": {"a1": {"model": "Sonnet", "effort": "max"}, "a2": {"effort": "low"}}, "fresh": []}).to_string(),
        )
        .unwrap();
        let marks = read_marks(dir.path(), Path::new("/p/proj/c0ab.jsonl"));
        let fork = |id: &str| marks.marked(id, Fork::default());
        let clock = Clock {
            now_ms: NOW,
            fork: &fork,
            state: &status_state,
            started_label: &fixed_label,
        };
        let raw = json!({
            "columns": 120,
            "tasks": [
                {"id": "a1", "name": "picked", "startTime": NOW - 1_000, "model": "claude-opus-5-5", "effort": "high"},
                {"id": "a2", "name": "effort", "startTime": NOW - 1_000, "model": "claude-opus-5-5", "effort": "high"},
                {"id": "a3", "name": "kept", "startTime": NOW - 1_000, "model": "claude-opus-5-5", "effort": "high"},
            ],
        });
        let rows: Vec<String> = render(&raw.to_string(), &clock)
            .unwrap()
            .into_iter()
            .map(|line| {
                let line: Value = serde_json::from_str(&line).unwrap();
                line["content"].as_str().unwrap().to_owned()
            })
            .collect();
        assert!(rows[0].contains("\x1b[33mSonnet\x1b[39m max"), "{rows:?}");
        assert!(rows[1].contains("\x1b[33mOpus 5.5\x1b[39m low"), "{rows:?}");
        assert!(
            rows[2].contains("\x1b[33mOpus 5.5\x1b[39m high"),
            "{rows:?}"
        );
    }

    #[test]
    fn no_row_is_ever_wider_than_its_columns() {
        let task = json!({
            "id": "a1",
            "description": "session-lookup-bug: Blocket session lookup",
            "label": "Creating pull request with gh",
            "status": "completed",
            "startTime": NOW - 7_300_000,
            "model": "claude-opus-5-5",
            "effort": "high",
            "tokenCount": 153_900,
        });
        for columns in 1..=140 {
            let rows = content(json!({"columns": columns, "tasks": [task.clone()]}));
            let shown = visible(&rows[0]);
            assert!(
                shown.chars().count() <= columns,
                "{columns}: {shown:?} is {} wide",
                shown.chars().count()
            );
            if columns >= 40 {
                assert_eq!(shown.chars().count(), columns, "{columns}: {shown:?}");
                assert!(shown.contains(IDLE_LABEL), "{columns}: {shown:?}");
            }
            let rows = content_in(
                json!({"columns": columns, "tasks": [task.clone()]}),
                "Git/qol-monorepo",
            );
            let shown = visible(&rows[0]);
            assert!(
                shown.chars().count() <= columns,
                "{columns}: {shown:?} is {} wide",
                shown.chars().count()
            );
            if columns >= 40 {
                assert_eq!(shown.chars().count(), columns, "{columns}: {shown:?}");
                assert!(shown.contains("qol-monorepo"), "{columns}: {shown:?}");
                assert!(shown.contains(IDLE_LABEL), "{columns}: {shown:?}");
            }
        }
    }

    #[test]
    fn a_shrinking_row_drops_the_summary_effort_model_elapsed_and_tokens_in_turn() {
        let task = json!({
            "id": "a1",
            "description": "kcd2-modding: continue KCD2 modding",
            "startTime": NOW - 25_000,
            "model": "claude-opus-5-5",
            "effort": "high",
            "tokenCount": 53_900,
        });
        let at = |columns: usize| {
            let rows = content(json!({"columns": columns, "tasks": [task.clone()]}));
            squeezed(&[visible(&rows[0])])[0].clone()
        };
        assert_eq!(
            at(100),
            format!(
                "kcd2-modding Opus 5.5 high continue KCD2 modding {STARTED} · 25s · 53.9k tokens"
            )
        );
        assert_eq!(
            at(64),
            format!("kcd2-modding Opus 5.5 high {STARTED} · 25s · 53.9k tokens")
        );
        assert_eq!(at(52), "kcd2-modding Opus 5.5 high 25s · 53.9k tokens");
        assert_eq!(at(43), "kcd2-modding Opus 5.5 high 53.9k tokens");
        assert_eq!(at(42), "kcd2-modding Opus 5.5 53.9k tokens");
        assert_eq!(at(38), "kcd2-modding Opus 5.5 53.9k tokens");
        assert_eq!(at(37), "kcd2-modding 53.9k tokens");
        assert_eq!(at(28), "kcd2-modding 53.9k tokens");
        assert_eq!(at(21), "kcd2-modding 53.9k");
        assert_eq!(at(16), "kcd2-modding");
        assert_eq!(at(14), "kcd2-modding");
        assert_eq!(at(8), "kcd2-…");
    }

    #[test]
    fn a_shrinking_row_drops_date_elapsed_effort_model_and_shortens_the_folder_in_turn() {
        let task = json!({
            "id": "a1",
            "description": "kcd2-modding: continue KCD2 modding",
            "startTime": NOW - 25_000,
            "model": "claude-opus-5-5",
            "effort": "high",
            "tokenCount": 53_900,
        });
        let at = |columns: usize| {
            let rows = content_in(
                json!({"columns": columns, "tasks": [task.clone()]}),
                "Git/qol-monorepo",
            );
            assert!(visible(&rows[0]).chars().count() <= columns, "{rows:?}");
            squeezed(&[visible(&rows[0])])[0].clone()
        };
        let head = "kcd2-modding Opus 5.5 high Git/qol-monorepo";
        for (columns, shown) in [
            (
                110,
                format!("{head} continue KCD2 modding {STARTED} · 25s · 53.9k tokens"),
            ),
            (87, format!("{head} con… {STARTED} · 25s · 53.9k tokens")),
            (86, format!("{head} {STARTED} · 25s · 53.9k tokens")),
            (81, format!("{head} {STARTED} · 25s · 53.9k tokens")),
            (80, format!("{head} 25s · 53.9k tokens")),
            (67, format!("{head} 25s · 53.9k tokens")),
            (66, format!("{head} 53.9k tokens")),
            (61, format!("{head} 53.9k tokens")),
            (
                60,
                "kcd2-modding Opus 5.5 Git/qol-monorepo 53.9k tokens".to_owned(),
            ),
            (
                56,
                "kcd2-modding Opus 5.5 Git/qol-monorepo 53.9k tokens".to_owned(),
            ),
            (55, "kcd2-modding Git/qol-monorepo 53.9k tokens".to_owned()),
            (46, "kcd2-modding Git/qol-monorepo 53.9k tokens".to_owned()),
            (45, "kcd2-modding qol-monorepo 53.9k tokens".to_owned()),
            (42, "kcd2-modding qol-monorepo 53.9k tokens".to_owned()),
            (41, "kcd2-modding qol-monorepo 53.9k".to_owned()),
            (35, "kcd2-modding qol-monorepo 53.9k".to_owned()),
            (34, "kcd2-modding qol-monorepo".to_owned()),
            (30, "kcd2-modding qol-monorepo".to_owned()),
        ] {
            assert_eq!(at(columns), shown, "{columns} columns");
        }
        assert_eq!(
            at(20),
            "kcd2-modding",
            "below the name and folder the folder goes"
        );
    }

    #[test]
    fn a_row_shows_the_folder_dimmed_after_the_model_and_effort() {
        let rows = content_in(
            json!({
                "columns": 160,
                "tasks": [{
                    "id": "a1",
                    "name": "switchboard-flow-improvements",
                    "description": "orchestration revisions",
                    "label": "Committing changes",
                    "status": "running",
                    "startTime": NOW - 125_000,
                    "model": "claude-opus-5-5",
                    "effort": "high",
                    "tokenCount": 108_200,
                }],
            }),
            "Git/qol-monorepo",
        );
        let row = &rows[0];
        assert!(
            row.starts_with(&format!(
                "  {CYAN}switchboard-flow-improvements{COLOR_OFF}  \x1b[33mOpus 5.5\x1b[39m high  \x1b[2mGit/qol-monorepo\x1b[22m  orchestration revisions · Committing changes"
            )),
            "{row:?}"
        );
        assert!(
            visible(row).ends_with(&format!("  working · {STARTED} · 2m 5s · 108.2k tokens")),
            "{row:?}"
        );
        assert_eq!(visible(row).chars().count(), 160, "{row:?}");
        let unbounded = content_in(
            json!({"tasks": [{"id": "a1", "name": "fork-a", "description": "work"}]}),
            "~/qol",
        );
        assert_eq!(
            unbounded,
            [format!(
                "  {CYAN}fork-a{COLOR_OFF}  \x1b[2m~/qol\x1b[22m  work  0 tokens"
            )]
        );
        let sneaky = content_in(
            json!({"tasks": [{"id": "a1", "name": "fork-a", "description": "work"}]}),
            "a\x1b[2Jb",
        );
        assert!(!visible(&sneaky[0]).contains('\x1b'), "{sneaky:?}");
        assert!(sneaky[0].contains("a [2Jb"), "{sneaky:?}");
    }

    #[test]
    fn a_folder_label_is_the_last_two_names_with_home_written_as_a_tilde() {
        let home = Path::new("/home/me");
        for (path, label) in [
            ("/home/me", Some("~")),
            ("/home/me/", Some("~")),
            ("/home/me/Git", Some("~/Git")),
            ("/home/me/Git/qol-monorepo", Some("Git/qol-monorepo")),
            (
                "/home/me/Git/worktrees/loop-gate",
                Some("worktrees/loop-gate"),
            ),
            (
                "/home/me/Git/worktrees/loop-gate/qol",
                Some("loop-gate/qol"),
            ),
            ("/home/meow", Some("home/meow")),
            ("/home/meow/Git", Some("meow/Git")),
            ("/home", Some("home")),
            ("/", Some("/")),
            ("/tmp", Some("tmp")),
            ("/media/kmrh47/WD/Git", Some("WD/Git")),
            ("/media//kmrh47/./WD", Some("kmrh47/WD")),
            ("relative/path/here", Some("path/here")),
            ("", None),
        ] {
            assert_eq!(
                folder_label(Path::new(path), Some(home)).as_deref(),
                label,
                "{path:?}"
            );
        }
        for (path, label) in [
            ("/home/me", Some("home/me")),
            ("/home/me/Git", Some("me/Git")),
            ("/", Some("/")),
            ("", None),
        ] {
            assert_eq!(
                folder_label(Path::new(path), None).as_deref(),
                label,
                "{path:?} without a home"
            );
        }
        for (path, label) in [
            ("/", Some("/")),
            ("/usr", Some("usr")),
            ("/usr/lib", Some("usr/lib")),
        ] {
            assert_eq!(
                folder_label(Path::new(path), Some(Path::new("/"))).as_deref(),
                label,
                "{path:?} under a root home"
            );
        }
    }

    #[test]
    fn the_short_folder_is_the_last_name_and_a_lone_mark_stays() {
        for (label, short) in [
            ("Git/qol-monorepo", "qol-monorepo"),
            ("worktrees/loop-gate", "loop-gate"),
            ("~/Git", "Git"),
            ("~", "~"),
            ("/", "/"),
            ("tmp", "tmp"),
        ] {
            assert_eq!(short_folder(label), short, "{label}");
        }
    }

    #[test]
    fn a_zero_width_pane_leaves_the_row_to_claude_code() {
        let rows = content(json!({
            "columns": 0,
            "tasks": [{"id": "a1", "name": "fork-a", "description": "work"}],
        }));
        assert!(rows.is_empty(), "{rows:?}");
    }

    #[test]
    fn a_row_without_a_resolved_model_or_width_still_renders_as_plain_text() {
        let rows = content(json!({
            "tasks": [
                {"id": "a1", "agentType": "project-fork", "description": "Start\x1b[31ming\n", "label": " "},
                {"id": "", "description": "dropped"},
            ],
        }));
        assert_eq!(
            rows,
            [format!("  {CYAN}Start [31ming{COLOR_OFF}  0 tokens")]
        );
    }

    #[test]
    fn an_unnamed_fork_is_named_by_its_description_and_an_idle_one_says_so() {
        let rows = content(json!({
            "columns": 60,
            "tasks": [
                {"id": "a1", "agentType": "project-fork", "description": "kcd2-design", "label": "kcd2-design", "status": "completed"},
                {"id": "a2", "agentType": "project-fork", "description": "forks-persist", "label": "Probing fork resume", "status": "running"},
                {"id": "a3", "agentType": "project-fork", "description": " "},
            ],
        }));
        assert_eq!(
            squeezed(&rows),
            [
                "kcd2-design idle · 0 tokens",
                "forks-persist Probing fork resume working · 0 tokens",
                "project-fork 0 tokens",
            ]
        );
        for row in &rows {
            assert_eq!(visible(row).chars().count(), 60, "{row:?}");
        }
    }

    #[test]
    fn a_key_name_description_shows_the_key_then_the_friendly_words_and_the_progress() {
        let rows = content(json!({
            "columns": 100,
            "tasks": [
                {"id": "a1", "description": "session-lookup-bug: Blocket session lookup", "label": "Creating pull request with gh", "status": "running"},
                {"id": "a2", "description": "kcd2-design: KCD2 light-steer poses", "label": "kcd2-design: KCD2 light-steer poses", "status": "completed"},
                {"id": "a3", "description": "Fix qol shot open latency", "label": "Fix qol shot open latency"},
                {"id": "a4", "description": "Odd: Words Here", "label": "Odd: Words Here"},
            ],
        }));
        assert_eq!(
            squeezed(&rows),
            [
                "session-lookup-bug Blocket session lookup · Creating pull request with gh working · 0 tokens",
                "kcd2-design KCD2 light-steer poses idle · 0 tokens",
                "Fix qol shot open latency 0 tokens",
                "Odd: Words Here 0 tokens",
            ]
        );
    }

    #[test]
    fn a_long_name_with_a_model_still_fits_the_pane_and_keeps_the_idle_marker() {
        let rows = content(json!({
            "columns": 50,
            "tasks": [{
                "id": "a1",
                "description": "Investigate why nightly sync stops after midnight and fix it",
                "model": "claude-opus-5-5",
                "effort": "high",
                "status": "completed",
            }],
        }));
        let visible = visible(&rows[0]);
        assert_eq!(visible.chars().count(), 50, "{visible:?}");
        assert!(visible.starts_with("  Investigate why"), "{visible:?}");
        assert!(visible.contains("…  Opus 5.5 high"), "{visible:?}");
        assert!(visible.ends_with("  idle · 0 tokens"), "{visible:?}");
    }

    #[test]
    fn a_model_or_effort_carrying_escapes_reaches_the_row_as_plain_text() {
        let rows = content(json!({
            "tasks": [{
                "id": "a1",
                "name": "fork-a",
                "description": "work",
                "model": "evil\x1b[2J",
                "effort": "hi\x1b]0;x\x07gh",
            }],
        }));
        assert_eq!(
            rows,
            [format!(
                "  {CYAN}fork-a{COLOR_OFF}  \x1b[33mevil [2J\x1b[39m hi ]0;x gh  work  0 tokens"
            )]
        );
    }

    #[test]
    fn an_empty_payload_writes_nothing_and_bad_json_fails() {
        let clock = Clock {
            now_ms: NOW,
            fork: &cyan_fork,
            state: &status_state,
            started_label: &fixed_label,
        };
        assert!(render("  ", &clock).unwrap().is_empty());
        assert!(render("{", &clock).is_err());
    }

    #[test]
    fn each_fork_shows_its_cli_sessions_state_from_its_status_and_what_it_left_running() {
        use super::super::background::fixtures::waiting;
        use super::super::background::HOLD_COMMAND;
        let root = tempfile::tempdir().unwrap();
        let conversation = root.path().join("c0ab.jsonl");
        let subagents = root.path().join("c0ab").join("subagents");
        std::fs::create_dir_all(&subagents).unwrap();
        let watch = "node /x/bin/pr-watch.cjs https://github.com/qol-tools/qol/pull/407 --pretty";
        for (id, transcript) in [
            ("aturn", waiting("bhold", HOLD_COMMAND)),
            (
                "apark",
                [waiting("bhold", HOLD_COMMAND), waiting("bwatch", watch)].concat(),
            ),
            ("adone", String::new()),
            ("awork", waiting("bwatch", watch)),
        ] {
            std::fs::write(subagents.join(format!("agent-{id}.jsonl")), transcript).unwrap();
        }
        let raw = json!({"transcript_path": conversation.to_str().unwrap()}).to_string();
        let session = session_transcript(&raw);
        let state = |id: &str, status: Option<&str>| session_state(session.as_deref(), id, status);
        let task = |id: &str, status: &str| json!({"id": id, "description": format!("fork-{id}"), "status": status});
        let tasks = json!([
            task("aturn", "completed"),
            task("apark", "completed"),
            task("adone", "completed"),
            task("awork", "running"),
            task("amiss", "completed"),
            task("akill", "killed"),
        ]);
        let rows = content_with(json!({"columns": 40, "tasks": tasks.clone()}), &state);
        assert_eq!(
            squeezed(&rows),
            [
                "fork-aturn your turn · 0 tokens",
                "fork-apark waiting #407 · 0 tokens",
                "fork-adone idle · 0 tokens",
                "fork-awork working · 0 tokens",
                "fork-amiss idle · 0 tokens",
                "fork-akill 0 tokens",
            ]
        );
        for row in &rows {
            assert_eq!(visible(row).chars().count(), 40, "{row:?}");
        }
        assert!(
            rows[0].ends_with("\x1b[1m\x1b[33myour turn\x1b[22m\x1b[39m · 0 tokens"),
            "{:?}",
            rows[0]
        );
        assert!(
            rows[1].contains("\x1b[34mwaiting #407\x1b[22m\x1b[39m"),
            "{:?}",
            rows[1]
        );
        assert!(
            rows[3].contains("\x1b[32mworking\x1b[22m\x1b[39m"),
            "{:?}",
            rows[3]
        );
        let unbounded = content_with(json!({"tasks": tasks}), &state);
        assert_eq!(
            unbounded
                .iter()
                .map(String::as_str)
                .map(visible)
                .collect::<Vec<_>>(),
            [
                "  fork-aturn  your turn · 0 tokens",
                "  fork-apark  waiting #407 · 0 tokens",
                "  fork-adone  idle · 0 tokens",
                "  fork-awork  working · 0 tokens",
                "  fork-amiss  idle · 0 tokens",
                "  fork-akill  0 tokens",
            ]
        );
    }

    #[test]
    fn a_narrow_row_keeps_the_state_and_never_cuts_through_its_color() {
        let rows = content(json!({
            "columns": 12,
            "tasks": [{"id": "a1", "name": "fork-a", "description": "work", "status": "running"}],
        }));
        let shown = visible(&rows[0]);
        assert_eq!(shown, "  f… working");
        assert!(
            rows[0].ends_with("\x1b[32mworking\x1b[22m\x1b[39m"),
            "{:?}",
            rows[0]
        );
        let tiny = content(json!({
            "columns": 6,
            "tasks": [{"id": "a1", "name": "fork-a", "description": "work", "status": "running"}],
        }));
        assert_eq!(tiny, [format!("  {CYAN}for…{COLOR_OFF}")]);
    }

    #[test]
    fn the_session_transcript_comes_from_the_payload_and_a_fork_sits_beside_it() {
        assert_eq!(
            agent_transcript(Some(Path::new("/p/conv-1.jsonl")), "a9"),
            Some(PathBuf::from("/p/conv-1/subagents/agent-a9.jsonl"))
        );
        assert_eq!(agent_transcript(None, "a9"), None);
        assert_eq!(
            session_transcript(r#"{"transcript_path":"/p/c.jsonl"}"#),
            Some(PathBuf::from("/p/c.jsonl"))
        );
        assert_eq!(session_transcript(r#"{"transcript_path":" "}"#), None);
    }

    #[test]
    fn the_first_row_timestamp_is_read_and_anything_else_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.jsonl");
        std::fs::write(
            &good,
            "{\"type\":\"user\",\"timestamp\":\"2026-11-01T06:58:29.835Z\"}\n{\"type\":\"assistant\"}\n",
        )
        .unwrap();
        let started = |path: &Path| first_row(path).and_then(|row| row.started_ms);
        assert_eq!(started(&good), Some(1_793_516_309_835));
        let bad = dir.path().join("bad.jsonl");
        std::fs::write(&bad, "{\"type\":\"user\"}\n").unwrap();
        assert_eq!(started(&bad), None);
        assert_eq!(started(&dir.path().join("missing.jsonl")), None);
    }

    #[test]
    fn an_older_continued_fork_names_the_transcript_the_rows_read() {
        let prompt = format!(
            "Project: /w/app\ncontinue it\n\nCarry on the closed conversation titled T. {CONTINUE_LEAD}/h/p/c1.jsonl{CONTINUE_END} to learn where it stopped."
        );
        assert_eq!(
            continued_from(&prompt),
            Some(PathBuf::from("/h/p/c1.jsonl"))
        );
        assert_eq!(continued_from("Project: /w/app\nnew work"), None);
        let blocks = json!({"message": {"content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}});
        assert_eq!(prompt_text(&blocks), "a\nb");
    }

    #[test]
    fn a_continued_fork_takes_its_session_color_and_its_start() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("closed.jsonl");
        std::fs::write(
            &source,
            concat!(
                "{\"type\":\"agent-color\",\"agentColor\":\"blue\",\"sessionId\":\"s\"}\n",
                "{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n",
                "{\"type\":\"agent-color\",\"agentColor\":\"orange\",\"sessionId\":\"s\"}\n",
                "{\"type\":\"custom-title\",\"customTitle\":\"T\"}\n",
            ),
        )
        .unwrap();
        let fork = dir.path().join("agent-a1.jsonl");
        let prompt = format!(
            "Project: /w\nCarry on. {CONTINUE_LEAD}{}{CONTINUE_END}, then continue.",
            source.display()
        );
        let first = json!({
            "type": "user",
            "timestamp": "2026-11-01T06:58:29.835Z",
            "message": {"role": "user", "content": prompt},
        });
        std::fs::write(&fork, format!("{first}\n")).unwrap();
        let facts = fork_facts(&fork);
        assert_eq!(facts.started_ms, Some(1_793_516_309_835));
        assert_eq!(facts.color.as_deref(), Some("orange"));
        assert_eq!(last_color(&dir.path().join("missing.jsonl")), None);
        let fresh = dir.path().join("agent-a2.jsonl");
        std::fs::write(
            &fresh,
            "{\"type\":\"user\",\"message\":{\"content\":\"Project: /w\\nnew\"}}\n",
        )
        .unwrap();
        let facts = fork_facts(&fresh);
        assert_eq!((facts.started_ms, facts.color), (None, None));
        let resumed = dir.path().join("agent-a3.jsonl");
        std::fs::write(&resumed, format!("{}\n", json!({"type": "user", "timestamp": "2026-11-01T06:58:29.835Z", "message": {"content": format!("Project: /w\nIts own transcript, \"{}\", stays as it was.", dir.path().join("elsewhere.jsonl").display())}}))).unwrap();
        std::fs::write(
            dir.path().join("agent-a3.meta.json"),
            json!({RESUMED_FROM: source.to_string_lossy()}).to_string(),
        )
        .unwrap();
        let facts = fork_facts(&resumed);
        assert_eq!(
            facts.color.as_deref(),
            Some("orange"),
            "the meta file names the source, never the prompt"
        );
        assert_eq!(facts.folder, None, "no cwd in the meta file, no folder");
    }

    #[test]
    fn the_folder_comes_from_the_cwd_in_the_meta_file_of_the_rows_own_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let first = "{\"type\":\"user\",\"message\":{\"content\":\"Project: /w\"}}\n";
        let source = dir.path().join("agent-a1.jsonl");
        std::fs::write(&source, first).unwrap();
        std::fs::write(
            dir.path().join("agent-a1.meta.json"),
            json!({"cwd": "/qol-no-home/Git/old-project"}).to_string(),
        )
        .unwrap();
        let resumed = dir.path().join("agent-a2.jsonl");
        std::fs::write(&resumed, first).unwrap();
        std::fs::write(
            dir.path().join("agent-a2.meta.json"),
            json!({"cwd": "/qol-no-home/Git/qol-monorepo", RESUMED_FROM: source.to_string_lossy()})
                .to_string(),
        )
        .unwrap();
        assert_eq!(
            fork_facts(&source).folder.as_deref(),
            Some("Git/old-project")
        );
        assert_eq!(
            fork_facts(&resumed).folder.as_deref(),
            Some("Git/qol-monorepo"),
            "a resumed fork shows its own cwd, not its source's"
        );
        let bare = dir.path().join("agent-a3.jsonl");
        std::fs::write(&bare, first).unwrap();
        std::fs::write(dir.path().join("agent-a3.meta.json"), "{\"cwd\":\"\"}").unwrap();
        assert_eq!(fork_facts(&bare).folder, None);
    }

    #[test]
    fn the_board_gets_the_state_colors_the_agent_list_paints() {
        let looks: Value = serde_json::from_str(&state_looks()).unwrap();
        assert_eq!(
            looks,
            json!([
                {"state": "working", "color": "green", "bold": false},
                {"state": "waiting", "color": "blue", "bold": false},
                {"state": "your turn", "color": "yellow", "bold": true},
            ])
        );
        for look in &STATE_LOOKS {
            let paint = state_paint(look.state).unwrap();
            assert!(
                paint.ends_with(session_color(look.color).unwrap()),
                "{paint:?}"
            );
            assert_eq!(paint.starts_with(BOLD), look.bold, "{paint:?}");
        }
        assert_eq!(state_paint(IDLE_LABEL), None);
    }

    #[test]
    fn the_key_is_painted_in_the_session_color_or_a_steady_one_of_its_own() {
        let orange = |_id: &str| Fork {
            color: Some("orange".to_owned()),
            ..Fork::default()
        };
        let plain_fork = |_id: &str| Fork::default();
        let raw = json!({"columns": 40, "tasks": [{"id": "a1", "name": "kcd2-modding", "description": "x"}]}).to_string();
        let row_with = |fork: &dyn Fn(&str) -> Fork| {
            let clock = Clock {
                now_ms: NOW,
                fork,
                state: &status_state,
                started_label: &fixed_label,
            };
            let line: Value = serde_json::from_str(&render(&raw, &clock).unwrap()[0]).unwrap();
            line["content"].as_str().unwrap().to_owned()
        };
        assert!(
            row_with(&orange).starts_with("  \x1b[38;5;208mkcd2-modding\x1b[39m"),
            "{:?}",
            row_with(&orange)
        );
        let own = key_color("kcd2-modding");
        assert_eq!(own, key_color("kcd2-modding"));
        assert_ne!(own, MODEL_COLOR, "a key never borrows the model's yellow");
        assert!(
            row_with(&plain_fork).starts_with(&format!("  {own}kcd2-modding{COLOR_OFF}")),
            "{:?}",
            row_with(&plain_fork)
        );
        let unknown = |_id: &str| Fork {
            color: Some("default".to_owned()),
            ..Fork::default()
        };
        assert!(
            row_with(&unknown).starts_with(&format!("  {own}")),
            "{:?}",
            row_with(&unknown)
        );
        let spread: std::collections::BTreeSet<&str> = [
            "a-b",
            "kcd2-modding",
            "qol-bug-verify",
            "forks-persist",
            "statusline-perf",
            "tray-update",
            "x",
        ]
        .iter()
        .map(|name| key_color(name))
        .collect();
        assert!(spread.len() > 2, "{spread:?}");
    }

    #[test]
    fn the_local_start_label_reads_month_day_and_clock() {
        let label = local_started_label(1_793_516_309_835).unwrap();
        let parts: Vec<&str> = label.split(' ').collect();
        assert_eq!(parts.len(), 3, "{label}");
        assert_eq!(parts[0].len(), 3, "{label}");
        assert!(parts[1].parse::<u8>().is_ok(), "{label}");
        assert_eq!(parts[2].len(), 5, "{label}");
        assert_eq!(&parts[2][2..3], ":", "{label}");
    }

    const FRESH: &str = "\x1b[1m\x1b[38;5;147m◉\x1b[22m\x1b[39m ";
    const PULL: &str = "https://github.com/o/r/pull/9";

    fn content_of(
        raw: Value,
        fork: &dyn Fn(&str) -> Fork,
        state: &dyn Fn(&str, Option<&str>) -> Option<ForkState>,
    ) -> Vec<String> {
        let clock = Clock {
            now_ms: NOW,
            fork,
            state,
            started_label: &fixed_label,
        };
        render(&raw.to_string(), &clock)
            .unwrap()
            .into_iter()
            .map(|line| {
                let line: Value = serde_json::from_str(&line).unwrap();
                line["content"].as_str().unwrap().to_owned()
            })
            .collect()
    }

    fn family_fork(id: &str) -> Fork {
        Fork {
            color: Some("cyan".to_owned()),
            parent: id.starts_with("as").then(|| "ap".to_owned()),
            asks: id == "as3",
            fresh: id == "as3",
            ..Fork::default()
        }
    }

    fn family_state(id: &str, status: Option<&str>) -> Option<ForkState> {
        match id {
            "as1" => Some(ForkState::Working),
            "as3" => Some(ForkState::YourTurn),
            "as4" => Some(ForkState::Waiting(vec![PULL.to_owned()])),
            "ao" => None,
            _ => fork_state::state(status, None),
        }
    }

    fn family_task(id: &str, status: &str) -> Value {
        json!({"id": id, "description": format!("fork-{id}"), "status": status})
    }

    #[test]
    fn a_fresh_reply_puts_a_blue_mark_before_the_name_and_other_rows_keep_two_spaces() {
        let fork = |id: &str| Fork {
            color: Some("cyan".to_owned()),
            fresh: id == "a1",
            ..Fork::default()
        };
        let tasks = json!([
            {"id": "a1", "name": "fork-a", "description": "work", "status": "completed"},
            {"id": "a2", "name": "fork-b", "description": "work", "status": "completed"},
        ]);
        assert_eq!(
            content_of(json!({"tasks": tasks.clone()}), &fork, &status_state),
            [
                format!("{FRESH}{CYAN}fork-a{COLOR_OFF}  work  idle · 0 tokens"),
                format!("  {CYAN}fork-b{COLOR_OFF}  work  idle · 0 tokens"),
            ]
        );
        for columns in 1..=60 {
            let rows = content_of(
                json!({"columns": columns, "tasks": tasks.clone()}),
                &fork,
                &status_state,
            );
            for row in &rows {
                let shown = visible(row);
                assert!(shown.chars().count() <= columns, "{columns}: {shown:?}");
                if columns >= 20 {
                    assert_eq!(shown.chars().count(), columns, "{columns}: {shown:?}");
                }
            }
            if columns >= 20 {
                assert!(rows[0].starts_with(FRESH), "{columns}: {:?}", rows[0]);
                assert!(visible(&rows[0]).starts_with("◉ fork-a"), "{columns}");
                assert!(visible(&rows[1]).starts_with("  fork-b"), "{columns}");
            }
        }
    }

    #[test]
    fn a_fork_that_started_forks_counts_them_after_its_name_and_waits_on_the_busy_ones() {
        let tasks = json!([
            family_task("ap", "completed"),
            family_task("as1", "running"),
            family_task("as2", "completed"),
            family_task("as3", "completed"),
            family_task("as4", "completed"),
            family_task("ao", "completed"),
        ]);
        let rows = content_of(json!({"tasks": tasks.clone()}), &family_fork, &family_state);
        assert_eq!(
            rows.iter()
                .map(String::as_str)
                .map(visible)
                .collect::<Vec<_>>(),
            [
                "  fork-ap +4 · 1 asks you  waits on 3 · 0 tokens",
                "  fork-as1  working · 0 tokens",
                "  fork-as2  idle · 0 tokens",
                "◉ fork-as3  your turn · 0 tokens",
                "  fork-as4  waiting #9 · 0 tokens",
                "  fork-ao  0 tokens",
            ]
        );
        assert!(
            rows[0].starts_with(&format!(
                "  {CYAN}fork-ap{COLOR_OFF}\x1b[1m\x1b[33m +4 · 1 asks you\x1b[22m\x1b[39m"
            )),
            "{:?}",
            rows[0]
        );
        assert!(
            rows[0].contains("\x1b[34mwaits on 3\x1b[22m\x1b[39m"),
            "{:?}",
            rows[0]
        );
        let working =
            json!({"tasks": [family_task("ap", "running"), family_task("as1", "running")]});
        assert_eq!(
            visible(&content_of(working, &family_fork, &family_state)[0]),
            "  fork-ap +1  working · 0 tokens"
        );
        let asking = |id: &str| Fork {
            asks: id == "ap" || id == "as3",
            ..family_fork(id)
        };
        let asks =
            json!({"tasks": [family_task("ap", "completed"), family_task("as1", "running")]});
        assert_eq!(
            visible(&content_of(asks, &asking, &family_state)[0]),
            "  fork-ap +1  idle · 0 tokens"
        );
        let quiet =
            json!({"tasks": [family_task("ap", "completed"), family_task("as2", "completed")]});
        let rows = content_of(quiet, &family_fork, &family_state);
        assert_eq!(visible(&rows[0]), "  fork-ap +1  idle · 0 tokens");
        assert!(
            rows[0].starts_with(&format!(
                "  {CYAN}fork-ap{COLOR_OFF}\x1b[34m +1\x1b[22m\x1b[39m"
            )),
            "{:?}",
            rows[0]
        );
    }

    #[test]
    fn the_name_truncates_before_the_badge_and_no_family_row_is_wider_than_its_columns() {
        let tasks = json!([
            {"id": "ap", "description": "a-long-parent-name", "status": "completed"},
            family_task("as1", "running"),
        ]);
        let at = |columns: usize| {
            content_of(
                json!({"columns": columns, "tasks": tasks.clone()}),
                &family_fork,
                &family_state,
            )
        };
        assert_eq!(visible(&at(36)[0]), "  a-long-parent-… +1  waits on 1 · 0");
        assert_eq!(visible(&at(20)[0]), "  a-l… +1 waits on 1");
        assert_eq!(visible(&at(7)[0]), "  a… +1");
        assert!(
            at(36)[0].contains("\x1b[34m +1\x1b[22m\x1b[39m"),
            "{:?}",
            at(36)[0]
        );
        for columns in 1..=80 {
            for row in at(columns) {
                let shown = visible(&row);
                assert!(shown.chars().count() <= columns, "{columns}: {shown:?}");
            }
        }
        let wide = json!({"columns": 60, "tasks": [
            family_task("ap", "completed"),
            family_task("as1", "running"),
            family_task("as3", "completed"),
        ]});
        let rows = content_of(wide, &family_fork, &family_state);
        assert_eq!(
            squeezed(&rows),
            [
                "fork-ap +2 · 1 asks you waits on 2 · 0 tokens",
                "fork-as1 working · 0 tokens",
                "◉ fork-as3 your turn · 0 tokens",
            ]
        );
        for row in &rows {
            assert_eq!(visible(row).chars().count(), 60, "{row:?}");
        }
    }

    #[test]
    fn the_marks_file_is_named_by_the_switchboard_session_and_a_bad_one_marks_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("c0ab.json"),
            json!({"fresh": ["a1"], "asks": ["a2", "a3"]}).to_string(),
        )
        .unwrap();
        let marks = read_marks(dir.path(), Path::new("/p/proj/c0ab.jsonl"));
        assert_eq!(marks.fresh, ["a1"]);
        assert_eq!(marks.asks, ["a2", "a3"]);
        let asking = marks.marked(
            "a2",
            Fork {
                color: Some("cyan".to_owned()),
                ..Fork::default()
            },
        );
        assert!(asking.asks && !asking.fresh);
        assert_eq!(asking.color.as_deref(), Some("cyan"));
        let fresh = marks.marked("a1", Fork::default());
        assert!(fresh.fresh && !fresh.asks);
        std::fs::write(dir.path().join("bad.json"), "{").unwrap();
        std::fs::write(dir.path().join("odd.json"), "{\"fresh\": 3}").unwrap();
        for name in ["bad", "odd", "missing"] {
            let marks = read_marks(dir.path(), &Path::new("/p").join(format!("{name}.jsonl")));
            assert!(marks.fresh.is_empty() && marks.asks.is_empty(), "{name}");
        }
        assert!(session_marks(None).fresh.is_empty());
    }

    #[test]
    fn a_sub_fork_names_its_parent_in_its_meta_file() {
        let dir = tempfile::tempdir().unwrap();
        let first = "{\"type\":\"user\",\"message\":{\"content\":\"Project: /w\"}}\n";
        let sub = dir.path().join("agent-a1.jsonl");
        std::fs::write(&sub, first).unwrap();
        std::fs::write(
            dir.path().join("agent-a1.meta.json"),
            json!({PARENT_KEY: "a98", "cwd": "/qol-no-home/Git/app"}).to_string(),
        )
        .unwrap();
        let facts = fork_facts(&sub);
        assert_eq!(facts.parent.as_deref(), Some("a98"));
        assert_eq!(facts.folder.as_deref(), Some("Git/app"));
        let unread = dir.path().join("agent-a2.jsonl");
        std::fs::write(&unread, "").unwrap();
        std::fs::write(
            dir.path().join("agent-a2.meta.json"),
            json!({PARENT_KEY: "a98"}).to_string(),
        )
        .unwrap();
        assert_eq!(fork_facts(&unread).parent.as_deref(), Some("a98"));
        let top = dir.path().join("agent-a3.jsonl");
        std::fs::write(&top, first).unwrap();
        std::fs::write(
            dir.path().join("agent-a3.meta.json"),
            json!({PARENT_KEY: ""}).to_string(),
        )
        .unwrap();
        assert_eq!(fork_facts(&top).parent, None);
    }

    #[test]
    fn each_state_has_one_word_and_waiting_names_its_pull_requests() {
        for (state, word) in [
            (ForkState::Working, WORKING_LABEL),
            (ForkState::Waiting(Vec::new()), WAITING_LABEL),
            (ForkState::YourTurn, YOUR_TURN_LABEL),
            (ForkState::Idle, IDLE_LABEL),
        ] {
            assert_eq!(state_word(&state), word);
            assert_eq!(state_label(&state).word, word);
        }
        assert_eq!(WAITING_LABEL, "waiting");
        let both = ForkState::Waiting(vec![
            "https://github.com/o/r/pull/515".to_owned(),
            PULL.to_owned(),
        ]);
        assert_eq!(state_label(&both).word, "waiting #515,#9");
    }
}
