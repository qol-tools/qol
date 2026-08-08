use std::sync::{Arc, Mutex};

use qol_terminal_sessions::TerminalSnapshot;
use qol_terminal_sessions::{ScreenReader, SessionFocus, TerminalSessionService};

use crate::host::{Pane, TerminalHost};

#[derive(Default)]
pub struct Kitty {
    sessions: TerminalSessionService,
    snapshot: Mutex<Option<Arc<TerminalSnapshot>>>,
}

impl TerminalHost for Kitty {
    fn discover(&self) -> Vec<Pane> {
        let snapshot = match self.sessions.snapshot() {
            Ok(snapshot) => Arc::new(snapshot),
            Err(error) => {
                if let Ok(mut current) = self.snapshot.lock() {
                    *current = None;
                }
                qol_runtime::probe!("CLI_SESSIONS_DISCOVER", "outcome=error error={error}");
                return Vec::new();
            }
        };
        let panes = snapshot.sessions().to_vec();
        if let Ok(mut current) = self.snapshot.lock() {
            *current = Some(snapshot);
        }
        panes
    }

    fn get_text(&self, target: &qol_terminal_sessions::SessionBinding) -> Option<String> {
        let snapshot = self
            .snapshot
            .lock()
            .ok()
            .and_then(|current| current.clone());
        match snapshot {
            Some(snapshot) => self.sessions.read_screen_from(&snapshot, target).ok(),
            None => self.sessions.read_screen(target).ok(),
        }
    }

    fn focus(&self, target: &qol_terminal_sessions::SessionBinding) -> anyhow::Result<()> {
        self.sessions.focus(target).map_err(Into::into)
    }
}
