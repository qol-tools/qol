use std::path::PathBuf;

use serde::Deserialize;

use crate::{BackendId, SessionCapabilities, SessionFacts, SessionId, TerminalError};

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

fn basename(cmdline: &[String]) -> Option<String> {
    let program = cmdline.first()?;
    std::path::Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
}

impl KittyWindow {
    fn into_session(self, backend_id: &BackendId, instance: Option<&str>) -> SessionFacts {
        let foreground_basenames = self
            .foreground_processes
            .iter()
            .filter_map(|process| basename(&process.cmdline))
            .collect();
        let foreground_pids = self
            .foreground_processes
            .iter()
            .map(|process| process.pid)
            .collect();
        let reported = self.last_reported_cmdline.trim();
        let reported_cmd = if reported.is_empty() {
            None
        } else {
            basename(&[reported.to_owned()]).or_else(|| Some(reported.to_owned()))
        };
        let native_id = match instance {
            Some(instance) => format!("{instance}.{}", self.id),
            None => self.id.to_string(),
        };
        SessionFacts {
            id: SessionId::new(backend_id.clone(), native_id)
                .expect("Kitty endpoint and window ids are valid terminal session identities"),
            root_pid: self.pid,
            cwd: self.cwd.to_string_lossy().into_owned(),
            title: self.title,
            at_prompt: self.at_prompt,
            reported_cmd,
            foreground_basenames,
            foreground_pids,
            capabilities: SessionCapabilities::ALL,
        }
    }
}

impl KittyLs {
    pub fn sessions(self, backend_id: &BackendId) -> Vec<SessionFacts> {
        self.sessions_for(backend_id, None)
    }

    pub(super) fn sessions_for(
        self,
        backend_id: &BackendId,
        instance: Option<&str>,
    ) -> Vec<SessionFacts> {
        self.0
            .into_iter()
            .flat_map(|window| window.tabs)
            .flat_map(|tab| tab.windows)
            .map(|window| window.into_session(backend_id, instance))
            .collect()
    }
}

pub fn parse_ls(body: &str, backend_id: &BackendId) -> Result<KittyLs, TerminalError> {
    serde_json::from_str(body).map_err(|source| TerminalError::InvalidResponse {
        backend: backend_id.clone(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_ls;
    use crate::kitty::backend_id;

    const SAMPLE: &str = r#"[{"id":1,"tabs":[{"id":1,"windows":[
{"id":10,"title":"Agent","cwd":"/a/proj","pid":100,"at_prompt":false,"last_reported_cmdline":"claude","foreground_processes":[{"pid":100,"cmdline":["/bin/zsh"]},{"pid":101,"cmdline":["/usr/bin/claude"]}]},
{"id":11,"title":"Shell","cwd":"/a/sh","pid":200,"at_prompt":true,"last_reported_cmdline":"","foreground_processes":[{"pid":200,"cmdline":["/bin/zsh"]}]}
]}]}]"#;

    #[test]
    fn parser_flattens_windows_and_shell_integration_facts() {
        let sessions = parse_ls(SAMPLE, backend_id())
            .unwrap()
            .sessions(backend_id());

        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].id.native(), "10");
        assert_eq!(sessions[0].reported_cmd.as_deref(), Some("claude"));
        assert_eq!(sessions[0].foreground_basenames, ["zsh", "claude"]);
        assert_eq!(sessions[0].foreground_pids, [100, 101]);
        assert!(sessions[1].at_prompt);
        assert_eq!(sessions[1].reported_cmd, None);
    }

    #[test]
    fn parser_qualifies_window_ids_with_the_terminal_instance() {
        let sessions = parse_ls(SAMPLE, backend_id())
            .unwrap()
            .sessions_for(backend_id(), Some("k1_2"));

        assert_eq!(sessions[0].id.native(), "k1_2.10");
        assert_eq!(sessions[1].id.native(), "k1_2.11");
    }
}
