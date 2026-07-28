pub mod kitty;

pub use qol_terminal_sessions::SessionFacts as Pane;
use qol_terminal_sessions::{SessionBinding, SessionId};

pub trait TerminalHost {
    fn discover(&self) -> Vec<Pane>;
    fn get_text(&self, window_id: u64, root_pid: i32) -> Option<String>;
    fn focus(&self, window_id: u64, root_pid: i32) -> anyhow::Result<()>;
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

pub fn window_id(pane: &Pane) -> u64 {
    pane.id
        .native_u64()
        .expect("CLI Sessions currently consumes numeric Kitty session ids")
}

pub fn project_of(cwd: &str) -> String {
    cwd.rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(cwd)
        .to_string()
}
