use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::status::Status;
use crate::tool::Tool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub window_id: u64,
    pub root_pid: i32,
    pub project: String,
    pub name: Option<String>,
    pub cwd: String,
    pub branch: Option<String>,
    pub tool: Tool,
    pub status: Status,
    pub summary: String,
    pub last_activity: u64,
    #[serde(default)]
    pub screen_hash: Option<u64>,
    #[serde(default)]
    pub running_since: Option<u64>,
}

impl SessionState {
    pub fn acknowledge(&mut self) {
        if self.status == Status::YourTurn {
            self.status = Status::Acknowledged;
            self.summary = "acknowledged".into();
        }
    }
}

pub fn summary_for(status: Status, tool: Tool) -> String {
    match status {
        Status::Working => match tool {
            Tool::Generic => "running",
            Tool::Claude | Tool::Codex => "working",
        },
        Status::YourTurn => "your turn",
        Status::NeedsYou => "needs you",
        Status::Unknown => "idle",
        Status::Acknowledged => "acknowledged",
    }
    .to_string()
}

#[derive(Default)]
pub struct Registry {
    sessions: HashMap<u64, SessionState>,
}

impl Registry {
    pub fn upsert(&mut self, state: SessionState) {
        self.sessions.insert(state.window_id, state);
    }

    pub fn restore(&mut self, states: Vec<SessionState>) {
        for state in states {
            self.sessions.entry(state.window_id).or_insert(state);
        }
    }

    pub fn remove(&mut self, window_id: u64) {
        self.sessions.remove(&window_id);
    }

    pub fn contains_window(&self, window_id: u64) -> bool {
        self.sessions.contains_key(&window_id)
    }

    pub fn prune(&mut self, is_alive: impl Fn(i32) -> bool) {
        self.sessions.retain(|_, s| is_alive(s.root_pid));
    }

    pub fn get(&self, window_id: u64) -> Option<&SessionState> {
        self.sessions.get(&window_id)
    }

    pub fn get_mut(&mut self, window_id: u64) -> Option<&mut SessionState> {
        self.sessions.get_mut(&window_id)
    }

    pub fn sorted(&self) -> Vec<SessionState> {
        let mut out: Vec<SessionState> = self.sessions.values().cloned().collect();
        out.sort_by(|a, b| {
            rank(a.status)
                .cmp(&rank(b.status))
                .then(b.last_activity.cmp(&a.last_activity))
                .then(a.window_id.cmp(&b.window_id))
        });
        out
    }
}

fn rank(status: Status) -> u8 {
    match status {
        Status::NeedsYou => 0,
        Status::YourTurn => 1,
        Status::Working => 2,
        Status::Unknown => 3,
        Status::Acknowledged => 4,
    }
}
