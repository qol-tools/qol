use std::collections::HashMap;

use qol_terminal_sessions::{SessionBinding, SessionId};
use serde::{Deserialize, Serialize};

use crate::session::status::Status;
use crate::session::tool::{is_generic, Tool};

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
    #[serde(default, skip)]
    pub runtime_status: Option<Status>,
}

impl SessionState {
    pub fn binding(&self) -> Option<SessionBinding> {
        SessionBinding::new(self.id.clone(), self.root_pid).ok()
    }

    pub fn acknowledge(&mut self) {
        if self.status == Status::YourTurn {
            self.status = Status::Acknowledged;
            self.runtime_status = Some(Status::Acknowledged);
            self.summary = self.status.definition().label.into();
        }
    }
}

pub fn meaningful_name(value: Option<&str>) -> Option<&str> {
    let value = value?.trim();
    if value.is_empty() || value.chars().any(|character| character.is_control()) {
        return None;
    }
    Some(value)
}

pub fn summary_for(status: Status, tool: &Tool) -> String {
    if status == Status::Working && is_generic(tool) {
        return "running".into();
    }
    status.definition().label.into()
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

fn rank(state: &SessionState) -> (u8, u8) {
    let status = state.status.definition().priority;
    (status, u8::from(is_generic(&state.tool)))
}
