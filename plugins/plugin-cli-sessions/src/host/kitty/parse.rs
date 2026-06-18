use std::path::PathBuf;

use serde::Deserialize;

use crate::host::Pane;

#[derive(Debug, Clone, Deserialize)]
pub struct KittyLs(pub Vec<OsWindow>);

#[derive(Debug, Clone, Deserialize)]
pub struct OsWindow {
    pub tabs: Vec<Tab>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tab {
    pub windows: Vec<KittyWindow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KittyWindow {
    pub id: u64,
    pub title: String,
    pub cwd: PathBuf,
    pub pid: i32,
    #[serde(default)]
    pub at_prompt: bool,
    #[serde(default)]
    pub last_reported_cmdline: String,
    #[serde(default)]
    pub foreground_processes: Vec<ForegroundProcess>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForegroundProcess {
    pub pid: i32,
    pub cmdline: Vec<String>,
}

#[derive(Debug)]
pub enum LsParseError {
    Json(serde_json::Error),
}

impl std::fmt::Display for LsParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LsParseError::Json(e) => write!(f, "kitty @ ls JSON parse error: {e}"),
        }
    }
}

impl std::error::Error for LsParseError {}

fn basename(cmdline: &[String]) -> Option<String> {
    let prog = cmdline.first()?;
    std::path::Path::new(prog)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

impl KittyWindow {
    fn into_pane(self) -> Pane {
        let foreground_basenames = self
            .foreground_processes
            .iter()
            .filter_map(|p| basename(&p.cmdline))
            .collect();
        let foreground_pids = self.foreground_processes.iter().map(|p| p.pid).collect();
        let reported = self.last_reported_cmdline.trim();
        let reported_cmd = if reported.is_empty() {
            None
        } else {
            basename(&[reported.to_string()]).or_else(|| Some(reported.to_string()))
        };
        Pane {
            window_id: self.id,
            root_pid: self.pid,
            cwd: self.cwd.to_string_lossy().into_owned(),
            title: self.title,
            at_prompt: self.at_prompt,
            reported_cmd,
            foreground_basenames,
            foreground_pids,
        }
    }
}

impl KittyLs {
    pub fn panes(self) -> Vec<Pane> {
        self.0
            .into_iter()
            .flat_map(|os| os.tabs)
            .flat_map(|t| t.windows)
            .map(KittyWindow::into_pane)
            .collect()
    }
}

pub fn parse_ls(body: &str) -> Result<KittyLs, LsParseError> {
    serde_json::from_str(body).map_err(LsParseError::Json)
}
