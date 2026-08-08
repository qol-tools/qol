use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{
    BackendId, DeliveryMode, SessionBinding, SessionFacts, SessionId, TerminalError,
    TerminalSnapshot,
};

pub trait SessionInventory {
    fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError>;
}

pub trait ScreenReader {
    fn read_screen(&self, target: &SessionBinding) -> Result<String, TerminalError>;
}

pub trait SessionFocus {
    fn focus(&self, target: &SessionBinding) -> Result<(), TerminalError>;
}

pub trait TextInput {
    fn send_text(
        &self,
        target: &SessionBinding,
        text: &str,
        mode: DeliveryMode,
    ) -> Result<(), TerminalError>;
}

pub trait TerminalBackend:
    SessionInventory + ScreenReader + SessionFocus + TextInput + Send + Sync
{
    fn read_screen_from_snapshot(
        &self,
        snapshot: &TerminalSnapshot,
        target: &SessionBinding,
    ) -> Result<String, TerminalError>;

    fn id(&self) -> &BackendId;

    fn current_session_id(&self) -> Option<SessionId> {
        None
    }
}

pub struct TerminalSessionService {
    backends: BTreeMap<BackendId, Arc<dyn TerminalBackend>>,
}

impl TerminalSessionService {
    pub fn system() -> Self {
        Self::from_backends([
            Arc::new(crate::kitty::KittyBackend::default()) as Arc<dyn TerminalBackend>
        ])
        .expect("built-in terminal backend ids are unique")
    }

    pub fn from_backends(
        backends: impl IntoIterator<Item = Arc<dyn TerminalBackend>>,
    ) -> Result<Self, TerminalError> {
        let mut registered = BTreeMap::new();
        for backend in backends {
            let id = backend.id().clone();
            if registered.insert(id.clone(), backend).is_some() {
                return Err(TerminalError::DuplicateBackend(id));
            }
        }
        Ok(Self {
            backends: registered,
        })
    }

    fn backend_for(&self, session_id: &SessionId) -> Result<&dyn TerminalBackend, TerminalError> {
        self.backends
            .get(session_id.backend())
            .map(Arc::as_ref)
            .ok_or_else(|| TerminalError::UnknownBackend(session_id.backend().clone()))
    }
}

impl Default for TerminalSessionService {
    fn default() -> Self {
        Self::system()
    }
}

impl SessionInventory for TerminalSessionService {
    fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError> {
        Ok(self.snapshot()?.sessions().to_vec())
    }
}

impl TerminalSessionService {
    pub fn snapshot(&self) -> Result<TerminalSnapshot, TerminalError> {
        let mut sessions = Vec::new();
        let mut first_error = None;
        for backend in self.backends.values() {
            match backend.discover() {
                Ok(mut discovered) => sessions.append(&mut discovered),
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
        if sessions.is_empty() {
            if let Some(error) = first_error {
                return Err(error);
            }
        }
        sessions.sort_by(|left, right| left.id.cmp(&right.id));
        let snapshot = TerminalSnapshot::new(sessions);
        qol_runtime::probe!(
            "TERMINAL_SESSIONS",
            "operation=snapshot cache=load age_ms=0 sessions={}",
            snapshot.sessions().len()
        );
        Ok(snapshot)
    }

    pub fn read_screen_from(
        &self,
        snapshot: &TerminalSnapshot,
        target: &SessionBinding,
    ) -> Result<String, TerminalError> {
        snapshot.validate_screen_target(target)?;
        if let Some(screen) = snapshot.cached_screen(target) {
            qol_runtime::probe!(
                "TERMINAL_SESSIONS",
                "operation=read_screen cache=hit age_ms={}",
                snapshot.age_ms()
            );
            return Ok(screen);
        }
        let screen = self
            .backend_for(target.session_id())?
            .read_screen_from_snapshot(snapshot, target)?;
        snapshot.cache_screen(target.clone(), screen.clone());
        qol_runtime::probe!(
            "TERMINAL_SESSIONS",
            "operation=read_screen cache=load age_ms={}",
            snapshot.age_ms()
        );
        Ok(screen)
    }

    pub fn is_current(&self, target: &SessionBinding) -> Result<bool, TerminalError> {
        Ok(self
            .backend_for(target.session_id())?
            .current_session_id()
            .as_ref()
            == Some(target.session_id()))
    }
}

impl ScreenReader for TerminalSessionService {
    fn read_screen(&self, target: &SessionBinding) -> Result<String, TerminalError> {
        self.backend_for(target.session_id())?.read_screen(target)
    }
}

impl SessionFocus for TerminalSessionService {
    fn focus(&self, target: &SessionBinding) -> Result<(), TerminalError> {
        self.backend_for(target.session_id())?.focus(target)
    }
}

impl TextInput for TerminalSessionService {
    fn send_text(
        &self,
        target: &SessionBinding,
        text: &str,
        mode: DeliveryMode,
    ) -> Result<(), TerminalError> {
        self.backend_for(target.session_id())?
            .send_text(target, text, mode)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::kitty::KittyBackend;

    use super::{TerminalBackend, TerminalSessionService};

    #[test]
    fn duplicate_backend_ids_are_rejected() {
        let backends = [
            Arc::new(KittyBackend::default()) as Arc<dyn TerminalBackend>,
            Arc::new(KittyBackend::default()) as Arc<dyn TerminalBackend>,
        ];

        let error = TerminalSessionService::from_backends(backends)
            .err()
            .expect("duplicate ids must fail");

        assert!(error.to_string().contains("registered twice"));
    }
}
