pub mod kitty;
mod platform;

use std::sync::Arc;

pub use qol_terminal_sessions::SessionFacts as Pane;
use qol_terminal_sessions::{SessionBinding, SessionId};

pub const CONSOLE_PROBE: &str = "console-probe";

pub trait TerminalHost {
    fn discover(&self) -> Vec<Pane>;
    fn get_text(&self, target: &SessionBinding) -> Option<String>;
    fn visual_screen_is_current(&self, _target: &SessionBinding, _screen: &str) -> Option<bool> {
        Some(true)
    }
    fn focus(&self, target: &SessionBinding) -> anyhow::Result<()>;
}

pub fn system() -> Arc<dyn TerminalHost + Send + Sync> {
    platform::system()
}

pub fn console_probe(args: &[String]) -> anyhow::Result<String> {
    platform::console_probe(args)
}

pub fn kitty_session_id(window_id: u64) -> SessionId {
    SessionId::new(
        qol_terminal_sessions::kitty::backend_id().clone(),
        window_id.to_string(),
    )
    .expect("numeric Kitty ids are valid terminal session identities")
}

pub fn kitty_binding(window_id: u64, root_pid: i32) -> anyhow::Result<SessionBinding> {
    SessionBinding::new(kitty_session_id(window_id), root_pid).map_err(Into::into)
}

pub fn project_of(cwd: &str) -> String {
    let project = cwd
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(cwd);
    if project.chars().any(|character| character.is_control()) {
        return String::new();
    }
    project.to_owned()
}

#[cfg(test)]
mod tests {
    use super::project_of;

    #[test]
    fn project_fallback_rejects_corrupted_terminal_cwds() {
        let cases = [
            ("/work/project", "project"),
            ("/work/project/", "project"),
            (r"C:\Users\me\project\", "project"),
            (r"C:\", "C:"),
            ("/Users/kaho/\u{1}", ""),
        ];
        for (cwd, expected) in cases {
            assert_eq!(project_of(cwd), expected, "{cwd:?}");
        }
    }
}
