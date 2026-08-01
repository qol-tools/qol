use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::{IdentityError, TerminalError};

const TOKEN_VERSION: &str = "v1";

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct BackendId(String);

impl BackendId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if valid_component(&value) {
            return Ok(Self(value));
        }
        Err(IdentityError::component("terminal backend", value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for BackendId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SessionId {
    backend: BackendId,
    native: String,
}

impl SessionId {
    pub fn new(backend: BackendId, native: impl Into<String>) -> Result<Self, IdentityError> {
        let native = native.into();
        if valid_component(&native) {
            return Ok(Self { backend, native });
        }
        Err(IdentityError::component("terminal session", native))
    }

    pub fn backend(&self) -> &BackendId {
        &self.backend
    }

    pub fn native(&self) -> &str {
        &self.native
    }

    pub fn native_u64(&self) -> Option<u64> {
        self.native.parse().ok()
    }
}

impl Display for SessionId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}", self.backend, self.native)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SessionBinding {
    session_id: SessionId,
    root_pid: i32,
}

impl SessionBinding {
    pub fn new(session_id: SessionId, root_pid: i32) -> Result<Self, IdentityError> {
        if root_pid > 0 {
            return Ok(Self {
                session_id,
                root_pid,
            });
        }
        Err(IdentityError::root_pid(root_pid))
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn root_pid(&self) -> i32 {
        self.root_pid
    }

    pub fn token(&self) -> String {
        format!(
            "{TOKEN_VERSION}:{}:{}:{}",
            self.session_id.backend, self.session_id.native, self.root_pid
        )
    }
}

impl Display for SessionBinding {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        self.token().fmt(formatter)
    }
}

impl FromStr for SessionBinding {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parts = value.split(':').collect::<Vec<_>>();
        if parts.len() != 4 || parts[0] != TOKEN_VERSION {
            return Err(IdentityError::binding(value));
        }
        let backend = BackendId::new(parts[1])?;
        let session_id = SessionId::new(backend, parts[2])?;
        let root_pid = parts[3]
            .parse::<i32>()
            .map_err(|_| IdentityError::binding(value))?;
        Self::new(session_id, root_pid)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    #[default]
    Insert,
    Submit,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionCapabilities(u8);

impl SessionCapabilities {
    pub const NONE: Self = Self(0);
    pub const SCREEN_READING: Self = Self(1);
    pub const FOCUS: Self = Self(2);
    pub const TEXT_INPUT: Self = Self(4);
    pub const ALL: Self = Self(7);

    pub fn contains(self, capability: Self) -> bool {
        self.0 & capability.0 == capability.0
    }
}

impl std::ops::BitOr for SessionCapabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for SessionCapabilities {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionFacts {
    pub id: SessionId,
    pub root_pid: i32,
    pub cwd: String,
    pub title: String,
    pub at_prompt: bool,
    pub reported_cmd: Option<String>,
    pub foreground_basenames: Vec<String>,
    pub foreground_pids: Vec<i32>,
    pub capabilities: SessionCapabilities,
}

impl SessionFacts {
    pub fn binding(&self) -> Result<SessionBinding, IdentityError> {
        SessionBinding::new(self.id.clone(), self.root_pid)
    }
}

#[derive(Clone)]
pub struct TerminalSnapshot {
    sessions: Arc<[SessionFacts]>,
    screens: Arc<Mutex<HashMap<SessionBinding, String>>>,
    created_at: Instant,
}

impl TerminalSnapshot {
    pub(crate) fn new(sessions: Vec<SessionFacts>) -> Self {
        Self {
            sessions: Arc::from(sessions),
            screens: Arc::new(Mutex::new(HashMap::new())),
            created_at: Instant::now(),
        }
    }

    pub fn sessions(&self) -> &[SessionFacts] {
        &self.sessions
    }

    pub(crate) fn validate_screen_target(
        &self,
        target: &SessionBinding,
    ) -> Result<(), TerminalError> {
        let session = self
            .sessions()
            .iter()
            .find(|session| session.id == *target.session_id())
            .ok_or_else(|| TerminalError::TargetMissing(target.clone()))?;
        if session.root_pid != target.root_pid() {
            return Err(TerminalError::TargetChanged {
                target: session.id.clone(),
                expected_root_pid: target.root_pid(),
                actual_root_pid: session.root_pid,
            });
        }
        if session
            .capabilities
            .contains(SessionCapabilities::SCREEN_READING)
        {
            return Ok(());
        }
        Err(TerminalError::Unsupported {
            target: target.session_id().clone(),
            capability: "screen reading",
        })
    }

    pub(crate) fn age_ms(&self) -> u128 {
        self.created_at.elapsed().as_millis()
    }

    pub(crate) fn cached_screen(&self, target: &SessionBinding) -> Option<String> {
        self.screens.lock().ok()?.get(target).cloned()
    }

    pub(crate) fn cache_screen(&self, target: SessionBinding, screen: String) {
        if let Ok(mut screens) = self.screens.lock() {
            screens.insert(target, screen);
        }
    }
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{BackendId, SessionBinding, SessionId};

    #[test]
    fn binding_tokens_round_trip() {
        let session_id = SessionId::new(BackendId::new("kitty").unwrap(), "42").unwrap();
        let binding = SessionBinding::new(session_id, 1234).unwrap();

        assert_eq!(SessionBinding::from_str(&binding.token()).unwrap(), binding);
    }

    #[test]
    fn identities_reject_token_delimiters_and_invalid_pids() {
        let backend = BackendId::new("kitty").unwrap();
        let cases = [
            SessionId::new(backend.clone(), "pane:4").is_err(),
            SessionId::new(backend.clone(), "").is_err(),
            SessionBinding::new(SessionId::new(backend, "4").unwrap(), 0).is_err(),
        ];

        assert!(cases.into_iter().all(|rejected| rejected));
    }
}
