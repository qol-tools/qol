use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::spawn::identity_from_user_vars;
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
    #[serde(default)]
    pub user_vars: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForegroundProcess {
    pub pid: i32,
    pub cmdline: Vec<String>,
    #[serde(default)]
    pub cwd: PathBuf,
}

fn basename(cmdline: &[String]) -> Option<String> {
    let program = cmdline.first()?;
    std::path::Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
}

fn usable_cwd(path: &Path) -> Option<String> {
    let value = path.to_string_lossy();
    if value.is_empty() || value.chars().any(|character| character.is_control()) {
        return None;
    }
    Some(value.into_owned())
}

impl KittyWindow {
    fn into_session(self, backend_id: &BackendId, instance: Option<&str>) -> SessionFacts {
        let cwd = self
            .foreground_processes
            .iter()
            .rev()
            .find_map(|process| usable_cwd(&process.cwd))
            .or_else(|| usable_cwd(&self.cwd))
            .unwrap_or_default();
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
        let spawn_identity = identity_from_user_vars(&self.user_vars);
        SessionFacts {
            id: SessionId::new(backend_id.clone(), native_id)
                .expect("Kitty endpoint and window ids are valid terminal session identities"),
            root_pid: self.pid,
            cwd,
            title: self.title,
            at_prompt: self.at_prompt,
            reported_cmd,
            foreground_basenames,
            foreground_pids,
            capabilities: SessionCapabilities::ALL,
            spawn_identity,
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
    use crate::cli::CliToolId;
    use crate::kitty::backend_id;
    use crate::{SpawnIdentity, SpawnKey, SpawnSurface};

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

    #[test]
    fn parser_prefers_a_valid_foreground_cwd_over_corrupted_window_metadata() {
        let body = r#"[{"id":1,"tabs":[{"windows":[
{"id":10,"title":"Agent","cwd":"/Users/kaho/\u0001","pid":100,"foreground_processes":[{"pid":101,"cwd":"/work/project","cmdline":["/usr/bin/codex"]}]}
]}]}]"#;

        let sessions = parse_ls(body, backend_id()).unwrap().sessions(backend_id());

        assert_eq!(sessions[0].cwd, "/work/project");
    }

    #[test]
    fn parser_reads_spawn_identity_only_from_complete_valid_user_vars() {
        const TAGGED: &str = r#"[{"id":1,"tabs":[{"windows":[
{"id":20,"title":"Codex","cwd":"/a","pid":300,"user_vars":{"qol_session_key":"lane-1","qol_session_tool":"codex","qol_session_surface":"tab"}},
{"id":21,"title":"Partial","cwd":"/a","pid":301,"user_vars":{"qol_session_key":"partial","qol_session_surface":"tab"}},
{"id":22,"title":"Malformed","cwd":"/a","pid":302,"user_vars":{"qol_session_key":"has space","qol_session_tool":"codex","qol_session_surface":"tab"}},
{"id":23,"title":"Unknown","cwd":"/a","pid":303,"user_vars":{"qol_session_key":"lane-2","qol_session_tool":"codex","qol_session_surface":"banana"}},
{"id":24,"title":"Untagged","cwd":"/a","pid":304}
]}]}]"#;

        let sessions = parse_ls(TAGGED, backend_id())
            .unwrap()
            .sessions(backend_id());

        assert_eq!(
            sessions[0].spawn_identity,
            Some(SpawnIdentity {
                key: SpawnKey::new("lane-1").unwrap(),
                tool: CliToolId::new("codex").unwrap(),
                surface: SpawnSurface::Tab,
            })
        );
        for session in &sessions[1..] {
            assert_eq!(session.spawn_identity, None, "window: {}", session.id);
        }
    }
}
