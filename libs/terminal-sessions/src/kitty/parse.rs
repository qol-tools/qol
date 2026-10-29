use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::spawn::identity_from_user_vars;
use crate::{BackendId, SessionCapabilities, SessionFacts, SessionId, TerminalError};

#[derive(Debug, Clone, Deserialize)]
pub struct KittyLs(pub Vec<OsWindow>);

#[derive(Debug, Clone, Deserialize)]
pub struct OsWindow {
    #[serde(default)]
    pub is_focused: bool,
    #[serde(default)]
    pub last_focused: Option<bool>,
    pub tabs: Vec<Tab>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tab {
    #[serde(default)]
    pub is_focused: bool,
    #[serde(default)]
    pub is_active: bool,
    pub windows: Vec<KittyWindow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KittyWindow {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    pub pid: i32,
    #[serde(default)]
    pub is_focused: bool,
    #[serde(default)]
    pub is_active: bool,
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
    pub cwd: Option<PathBuf>,
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
    fn into_session(
        self,
        backend_id: &BackendId,
        instance: Option<&str>,
        tab_focused: bool,
    ) -> SessionFacts {
        let cwd = self
            .foreground_processes
            .iter()
            .rev()
            .find_map(|process| process.cwd.as_deref().and_then(usable_cwd))
            .or_else(|| self.cwd.as_deref().and_then(usable_cwd))
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
            focused: tab_focused && self.is_focused,
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
            .flat_map(|os_window| {
                os_window
                    .tabs
                    .into_iter()
                    .map(move |tab| (os_window.is_focused && tab.is_focused, tab))
            })
            .flat_map(|(tab_focused, tab)| {
                tab.windows
                    .into_iter()
                    .map(move |window| window.into_session(backend_id, instance, tab_focused))
            })
            .collect()
    }
}

impl KittyLs {
    pub fn has_focus(&self) -> bool {
        self.0.iter().any(|os_window| os_window.is_focused)
    }

    pub fn active_window_id(&self) -> Option<u64> {
        let os_window = self
            .0
            .iter()
            .find(|os_window| os_window.is_focused)
            .or_else(|| {
                self.0
                    .iter()
                    .find(|os_window| os_window.last_focused == Some(true))
            })
            .or_else(|| self.0.first())?;
        let tab = os_window
            .tabs
            .iter()
            .find(|tab| tab.is_active)
            .or_else(|| os_window.tabs.first())?;
        tab.windows
            .iter()
            .find(|window| window.is_active)
            .or_else(|| tab.windows.first())
            .map(|window| window.id)
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
    fn only_the_active_window_of_the_active_tab_of_a_focused_os_window_is_focused() {
        let cases = [
            (true, true, true, true, "the window holding keyboard focus"),
            (false, true, true, false, "kitty itself is not focused"),
            (
                true,
                false,
                true,
                false,
                "the window sits in a background tab",
            ),
            (
                true,
                true,
                false,
                false,
                "another split of the tab is active",
            ),
        ];
        for (os_window, tab, window, expected, note) in cases {
            let body = format!(
                r#"[{{"is_focused":{os_window},"tabs":[{{"is_focused":{tab},"windows":[
{{"id":10,"title":"Agent","pid":100,"is_focused":{window}}}]}}]}}]"#
            );
            let sessions = parse_ls(&body, backend_id())
                .unwrap()
                .sessions(backend_id());
            assert_eq!(sessions[0].focused, expected, "{note}");
        }
    }

    #[test]
    fn active_window_is_the_active_split_of_the_active_tab_of_the_last_used_os_window() {
        let cases = [
            (
                r#"[{"is_focused":false,"last_focused":false,"tabs":[{"is_active":true,"windows":[{"id":11,"title":"a","pid":1,"is_active":true}]}]},
{"is_focused":false,"last_focused":true,"tabs":[{"is_active":false,"windows":[{"id":15,"title":"b","pid":2,"is_active":true}]},{"is_active":true,"windows":[{"id":16,"title":"c","pid":3,"is_active":false},{"id":17,"title":"d","pid":4,"is_active":true}]}]}]"#,
                Some(17),
                "kitty is in the background, so the last focused os window wins",
            ),
            (
                r#"[{"is_focused":true,"last_focused":false,"tabs":[{"is_active":true,"windows":[{"id":11,"title":"a","pid":1,"is_active":true}]}]},
{"is_focused":false,"last_focused":true,"tabs":[{"is_active":true,"windows":[{"id":15,"title":"b","pid":2,"is_active":true}]}]}]"#,
                Some(11),
                "the focused os window wins over a stale last focused flag",
            ),
            (
                r#"[{"last_focused":null,"tabs":[{"windows":[{"id":20,"title":"a","pid":1},{"id":21,"title":"b","pid":2}]}]}]"#,
                Some(20),
                "an older kitty without activity flags falls back to the first window",
            ),
            (r#"[]"#, None, "no kitty window is open"),
        ];
        for (body, expected, note) in cases {
            let ls = parse_ls(body, backend_id()).unwrap();
            assert_eq!(ls.active_window_id(), expected, "{note}");
        }
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
    fn parser_tolerates_null_cwd_on_fresh_windows_and_processes() {
        let body = r#"[{"id":1,"tabs":[{"windows":[
{"id":10,"title":"Fresh","cwd":null,"pid":100,"foreground_processes":[{"pid":101,"cwd":null,"cmdline":["/usr/bin/pi"]}]},
{"id":11,"title":"Shell","pid":200,"foreground_processes":[{"pid":201,"cmdline":["/bin/zsh"]}]}
]}]}]"#;

        let sessions = parse_ls(body, backend_id()).unwrap().sessions(backend_id());

        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].cwd, "", "a null window cwd falls back to empty");
        assert_eq!(sessions[0].title, "Fresh");
        assert_eq!(sessions[1].cwd, "", "a missing cwd stays empty");
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
