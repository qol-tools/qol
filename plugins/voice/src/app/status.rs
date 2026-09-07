use serde::{Deserialize, Serialize};

use crate::turn::{AssistantOutputState, TurnSnapshot, UserActivityState};

use super::routing::RouteStatus;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Idle,
    Listening,
    Stopping,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantActivity {
    Idle,
    Starting,
    Playing,
    Ducked,
    Paused,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UserActivity {
    Idle,
    Candidate,
    Confirmed,
    Finalizing,
    Interrupted,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionStatus {
    pub state: LifecycleState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sequence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant_state: Option<AssistantActivity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_state: Option<UserActivity>,
    pub routing: RouteStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Default for SessionStatus {
    fn default() -> Self {
        Self {
            state: LifecycleState::Idle,
            session_id: None,
            input_device: None,
            provider: None,
            last_sequence: None,
            assistant_state: None,
            user_state: None,
            routing: RouteStatus::default(),
            error: None,
        }
    }
}

impl AssistantActivity {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Starting => "starting",
            Self::Playing => "playing",
            Self::Ducked => "ducked",
            Self::Paused => "paused",
        }
    }
}

impl UserActivity {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Candidate => "candidate",
            Self::Confirmed => "confirmed",
            Self::Finalizing => "finalizing",
            Self::Interrupted => "interrupted",
        }
    }
}

pub(super) fn assistant_state(snapshot: &TurnSnapshot) -> AssistantActivity {
    match snapshot.assistant {
        AssistantOutputState::Idle => AssistantActivity::Idle,
        AssistantOutputState::Starting { .. } => AssistantActivity::Starting,
        AssistantOutputState::Playing { .. } => AssistantActivity::Playing,
        AssistantOutputState::Ducked { .. } => AssistantActivity::Ducked,
        AssistantOutputState::Paused { .. } => AssistantActivity::Paused,
    }
}

pub(super) fn user_state(snapshot: &TurnSnapshot) -> UserActivity {
    match &snapshot.user {
        UserActivityState::Idle => UserActivity::Idle,
        UserActivityState::Candidate { .. } => UserActivity::Candidate,
        UserActivityState::Confirmed { .. } => UserActivity::Confirmed,
        UserActivityState::Finalizing { .. } => UserActivity::Finalizing,
        UserActivityState::Interrupted { .. } => UserActivity::Interrupted,
    }
}
