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
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub master: bool,
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

    pub fn is_master_ready(&self) -> bool {
        master_ready(self.master, self.status)
    }

    pub fn wants_attention(&self) -> bool {
        self.status.is_attention() && !self.is_master_ready()
    }
}

pub fn master_ready(master: bool, status: Status) -> bool {
    master
        && matches!(
            status,
            Status::YourTurn | Status::Acknowledged | Status::Unknown
        )
}

pub fn is_master_key(key: Option<&str>) -> bool {
    key == Some(qol_sessions_engine::SWITCHBOARD_KEY)
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

fn rank(state: &SessionState) -> (u8, u8, u8) {
    let status = state.status.definition().priority;
    (
        u8::from(!state.master),
        status,
        u8::from(is_generic(&state.tool)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_terminal_sessions::cli::{claude_tool, generic_tool};

    fn state(window_id: u64, status: Status, master: bool) -> SessionState {
        SessionState {
            id: crate::host::kitty_session_id(window_id),
            root_pid: window_id as i32,
            project: "proj".into(),
            name: None,
            cwd: "/a/b/proj".into(),
            branch: None,
            tool: claude_tool(),
            status,
            summary: "x".into(),
            last_activity: 1,
            screen_hash: None,
            key: master.then(|| qol_sessions_engine::SWITCHBOARD_KEY.to_owned()),
            master,
            working_since: None,
            settled_since: None,
            bridged: false,
            driving: Vec::new(),
            runtime_status: None,
        }
    }

    #[test]
    fn the_master_row_is_pinned_first_whatever_its_status() {
        for status in Status::ALL {
            let mut registry = Registry::default();
            registry.upsert(state(1, Status::NeedsYou, false));
            registry.upsert(state(2, Status::YourTurn, false));
            registry.upsert(state(9, status, true));
            let mut generic = state(3, Status::NeedsYou, false);
            generic.tool = generic_tool();
            registry.upsert(generic);
            let order: Vec<_> = registry.sorted().into_iter().map(|row| row.id).collect();
            assert_eq!(
                order,
                [9, 1, 3, 2].map(crate::host::kitty_session_id).to_vec(),
                "{status:?}"
            );
        }
    }

    #[test]
    fn the_resting_master_is_ready_and_never_asks_for_attention() {
        for status in Status::ALL {
            let master = state(9, status, true);
            let resting = matches!(
                status,
                Status::YourTurn | Status::Acknowledged | Status::Unknown
            );
            assert_eq!(master.is_master_ready(), resting, "{status:?}");
            assert_eq!(
                master.wants_attention(),
                status == Status::NeedsYou,
                "{status:?}"
            );
            let lane = state(1, status, false);
            assert!(!lane.is_master_ready(), "{status:?}");
            assert_eq!(lane.wants_attention(), status.is_attention(), "{status:?}");
        }
    }

    #[test]
    fn only_the_switchboard_key_marks_the_master() {
        assert!(is_master_key(Some(qol_sessions_engine::SWITCHBOARD_KEY)));
        assert!(!is_master_key(Some("switchboard-2")));
        assert!(!is_master_key(Some("park-1-2")));
        assert!(!is_master_key(None));
    }

    #[test]
    fn sessions_saved_before_the_master_flag_still_load() {
        let mut saved = serde_json::to_value(state(1, Status::YourTurn, false)).unwrap();
        let object = saved.as_object_mut().unwrap();
        object.remove("master");
        object.remove("key");
        let loaded: SessionState = serde_json::from_value(saved).unwrap();
        assert!(!loaded.master);
        assert_eq!(loaded.key, None);
        let master = state(9, Status::Unknown, true);
        let round_trip: SessionState =
            serde_json::from_value(serde_json::to_value(&master).unwrap()).unwrap();
        assert_eq!(round_trip, master);
    }
}
