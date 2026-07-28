use qol_terminal_sessions::{ScreenReader, SessionFocus, SessionInventory, TerminalSessionService};

use crate::host::{kitty_binding, Pane, TerminalHost};

#[derive(Default)]
pub struct Kitty {
    sessions: TerminalSessionService,
}

impl TerminalHost for Kitty {
    fn discover(&self) -> Vec<Pane> {
        self.sessions.discover().unwrap_or_else(|error| {
            qol_runtime::probe!("CLI_SESSIONS_DISCOVER", "outcome=error error={error}");
            Vec::new()
        })
    }

    fn get_text(&self, window_id: u64, root_pid: i32) -> Option<String> {
        let target = kitty_binding(window_id, root_pid).ok()?;
        self.sessions.read_screen(&target).ok()
    }

    fn focus(&self, window_id: u64, root_pid: i32) -> anyhow::Result<()> {
        let target = kitty_binding(window_id, root_pid)?;
        self.sessions.focus(&target).map_err(Into::into)
    }
}
