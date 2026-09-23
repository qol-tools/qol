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

    fn visual_screen_is_current(
        &self,
        target: &qol_terminal_sessions::SessionBinding,
        screen: &str,
    ) -> Option<bool> {
        let Ok(unscrolled) = self.sessions.read_screen_unscrolled(target) else {
            qol_runtime::probe!("CLI_SESSIONS_RECON", "phase=viewport outcome=unavailable");
            return None;
        };
        let current = visual_screen_is_current(screen, &unscrolled);
        qol_runtime::probe!(
            "CLI_SESSIONS_RECON",
            "phase=viewport outcome={} visual_bytes={} unscrolled_bytes={}",
            if current { "current" } else { "historical" },
            screen.len(),
            unscrolled.len()
        );
        Some(current)
    }

    fn focus(&self, target: &qol_terminal_sessions::SessionBinding) -> anyhow::Result<()> {
        self.sessions.focus(target).map_err(Into::into)
    }
}

fn visual_screen_is_current(screen: &str, unscrolled: &str) -> bool {
    let screen = screen.trim_end();
    !screen.is_empty() && unscrolled.trim_end().ends_with(screen)
}

#[cfg(test)]
mod tests {
    use super::visual_screen_is_current;

    #[test]
    fn old_dialog_in_scrollback_is_not_the_current_viewport() {
        let old_dialog = "Do you want to proceed?\n❯ 1. Yes\n  2. No\nEsc to cancel\n";
        let current = "Claude is working…\n❯ write tests\n";
        let unscrolled = format!("transcript\n{old_dialog}\n{current}");
        assert!(!visual_screen_is_current(old_dialog, &unscrolled));
        assert!(visual_screen_is_current(current, &unscrolled));
        assert!(visual_screen_is_current("❯ write tests\n", &unscrolled));
        assert!(!visual_screen_is_current("", &unscrolled));
    }
}
