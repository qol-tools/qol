pub mod kitty;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    pub window_id: u64,
    pub root_pid: i32,
    pub cwd: String,
    pub title: String,
    pub at_prompt: bool,
    pub reported_cmd: Option<String>,
    pub foreground_basenames: Vec<String>,
    pub foreground_pids: Vec<i32>,
}

pub trait TerminalHost {
    fn discover(&self) -> Vec<Pane>;
    fn get_text(&self, window_id: u64) -> Option<String>;
    fn focus(&self, window_id: u64) -> anyhow::Result<()>;
}

pub fn project_of(cwd: &str) -> String {
    cwd.rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(cwd)
        .to_string()
}
