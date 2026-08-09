use std::collections::HashMap;

use qol_terminal_sessions::{SessionBinding, SessionId};
use serde::{Deserialize, Serialize};

use crate::session::status::Status;
use crate::session::tool::Tool;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub id: SessionId,
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
    #[serde(default, skip)]
    pub working_since: Option<u64>,
    #[serde(default, skip)]
    pub settled_since: Option<u64>,
    #[serde(default, skip)]
    pub bridged: bool,
    #[serde(default, skip)]
    pub driving: Vec<SessionId>,
}

impl SessionState {
    pub fn binding(&self) -> Option<SessionBinding> {
        SessionBinding::new(self.id.clone(), self.root_pid).ok()
    }

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
            Tool::Claude | Tool::Codex | Tool::Kimi | Tool::Pi => "working",
        },
        Status::Service => "live",
        Status::YourTurn => "your turn",
        Status::NeedsYou => "needs you",
        Status::Unknown => "idle",
        Status::Acknowledged => "acknowledged",
    }
    .to_string()
}

#[derive(Default)]
pub struct Registry {
    sessions: HashMap<SessionId, SessionState>,
}

impl Registry {
    pub fn upsert(&mut self, state: SessionState) {
        self.sessions.insert(state.id.clone(), state);
    }

    pub fn restore(&mut self, states: Vec<SessionState>) {
        for state in states {
            self.sessions.entry(state.id.clone()).or_insert(state);
        }
    }

    pub fn remove(&mut self, id: &SessionId) {
        self.sessions.remove(id);
    }

    pub fn contains(&self, id: &SessionId) -> bool {
        self.sessions.contains_key(id)
    }

    pub fn prune(&mut self, is_alive: impl Fn(i32) -> bool) {
        self.sessions.retain(|_, s| is_alive(s.root_pid));
    }

    pub fn get(&self, id: &SessionId) -> Option<&SessionState> {
        self.sessions.get(id)
    }

    pub fn get_mut(&mut self, id: &SessionId) -> Option<&mut SessionState> {
        self.sessions.get_mut(id)
    }

    pub fn sorted(&self) -> Vec<SessionState> {
        let mut out: Vec<SessionState> = self.sessions.values().cloned().collect();
        out.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.id.cmp(&b.id)));
        out
    }
}

fn rank(state: &SessionState) -> u8 {
    let status = match state.status {
        Status::NeedsYou => 0,
        Status::YourTurn => 1,
        Status::Working => 2,
        Status::Service => 4,
        Status::Acknowledged => 5,
        Status::Unknown => 6,
    };
    if state.bridged {
        status.min(3)
    } else {
        status
    }
}
