use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::host::{project_of, Pane};
use crate::signal::screen::{codex_banner, codex_working, has_prompt_markers};
use crate::signal::title::title_working;
use crate::strategy::{Ctx, Phase, Reading, Strategy};

pub struct CodexSession {
    pub name: Option<String>,
    pub touched: bool,
}

pub trait CodexStore {
    fn session(&self, pane: &Pane) -> Option<CodexSession>;
}

pub struct NoCodexStore;

impl CodexStore for NoCodexStore {
    fn session(&self, _pane: &Pane) -> Option<CodexSession> {
        None
    }
}

pub struct Codex<'a> {
    store: &'a dyn CodexStore,
}

impl<'a> Codex<'a> {
    pub fn new(store: &'a dyn CodexStore) -> Self {
        Self { store }
    }
}

impl Strategy for Codex<'_> {
    fn wants_screen(&self, _pane: &Pane) -> bool {
        true
    }

    fn read(&self, ctx: &Ctx) -> Reading {
        let screen = ctx.screen.unwrap_or("");
        let session = self.store.session(ctx.pane);
        let phase = if title_working(&ctx.pane.title) || codex_working(screen) {
            Phase::Busy
        } else if has_prompt_markers(screen) {
            Phase::Blocked
        } else if turn_taken(&session, screen) {
            Phase::Done
        } else {
            Phase::Idle
        };
        let label = session
            .and_then(|s| s.name)
            .or_else(|| Some(project_of(&ctx.pane.cwd)));
        Reading { phase, label }
    }
}

fn turn_taken(session: &Option<CodexSession>, screen: &str) -> bool {
    match session {
        Some(s) => s.touched,
        None => !codex_banner(screen),
    }
}

pub struct DiskCodexStore;

impl CodexStore for DiskCodexStore {
    fn session(&self, pane: &Pane) -> Option<CodexSession> {
        let rollout = rollout_path_for(pane)?;
        let uuid = uuid_from_path(&rollout)?;
        Some(CodexSession {
            name: thread_name(&uuid),
            touched: touched(&rollout),
        })
    }
}

fn rollout_path_for(pane: &Pane) -> Option<PathBuf> {
    pane.foreground_pids
        .iter()
        .find_map(|pid| open_rollout(*pid))
}

fn open_rollout(pid: i32) -> Option<PathBuf> {
    let out = Command::new("lsof")
        .args(["-p", &pid.to_string(), "-Fn"])
        .output()
        .ok()?;
    String::from_utf8(out.stdout)
        .ok()?
        .lines()
        .filter_map(|l| l.strip_prefix('n'))
        .find(|p| p.contains("/sessions/") && p.contains("/rollout-") && p.ends_with(".jsonl"))
        .map(PathBuf::from)
}

fn uuid_from_path(path: &Path) -> Option<String> {
    let stem = path.file_name()?.to_str()?.strip_suffix(".jsonl")?;
    let parts: Vec<&str> = stem.split('-').collect();
    (parts.len() >= 5).then(|| parts[parts.len() - 5..].join("-"))
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex"))
}

fn thread_name(uuid: &str) -> Option<String> {
    let content = fs::read_to_string(codex_home()?.join("session_index.jsonl")).ok()?;
    let mut name = None;
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("id").and_then(Value::as_str) == Some(uuid) {
            if let Some(n) = v.get("thread_name").and_then(Value::as_str) {
                if !n.is_empty() {
                    name = Some(n.to_string());
                }
            }
        }
    }
    name
}

fn touched(path: &Path) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let mut non_empty = 0;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if !line.trim().is_empty() {
            non_empty += 1;
            if non_empty > 1 {
                return true;
            }
        }
    }
    false
}
