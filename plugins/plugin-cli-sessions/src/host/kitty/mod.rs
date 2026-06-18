pub mod parse;

use std::process::Command;

use crate::host::{Pane, TerminalHost};
use parse::parse_ls;

pub struct Kitty;

fn run_kitten(args: &[&str]) -> Option<String> {
    let out = Command::new("kitten").args(args).output().ok()?;
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "CLI_SESSIONS_KITTEN",
        "args={:?} code={:?} ok={} out_len={} stderr={:?}",
        args,
        out.status.code(),
        out.status.success(),
        out.stdout.len(),
        diag::trunc(&out.stderr)
    );
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

impl TerminalHost for Kitty {
    fn discover(&self) -> Vec<Pane> {
        let Some(body) = run_kitten(&["@", "ls"]) else {
            #[cfg(debug_assertions)]
            qol_runtime::probe!("CLI_SESSIONS_DISCOVER", "ls=none panes=0");
            return Vec::new();
        };
        let panes = match parse_ls(&body) {
            Ok(ls) => ls.panes(),
            Err(_e) => {
                #[cfg(debug_assertions)]
                qol_runtime::probe!("CLI_SESSIONS_DISCOVER", "parse=err panes=0");
                return Vec::new();
            }
        };
        #[cfg(debug_assertions)]
        qol_runtime::probe!(
            "CLI_SESSIONS_DISCOVER",
            "panes={} ids={:?}",
            panes.len(),
            panes.iter().map(|p| p.window_id).collect::<Vec<_>>()
        );
        panes
    }

    fn get_text(&self, window_id: u64) -> Option<String> {
        let text = run_kitten(&[
            "@",
            "get-text",
            "--match",
            &format!("id:{window_id}"),
            "--extent",
            "screen",
        ]);
        #[cfg(debug_assertions)]
        qol_runtime::probe!(
            "CLI_SESSIONS_GETTEXT",
            "wid={window_id} got={} len={}",
            text.is_some(),
            text.as_deref().map(str::len).unwrap_or(0)
        );
        text
    }

    fn focus(&self, window_id: u64) -> anyhow::Result<()> {
        #[cfg(debug_assertions)]
        let front_before = diag::frontmost_app();
        let matcher = format!("id:{window_id}");
        let out = Command::new("kitten")
            .args(["@", "focus-window", "--match", &matcher])
            .output()?;
        #[cfg(debug_assertions)]
        {
            std::thread::sleep(std::time::Duration::from_millis(150));
            qol_runtime::probe!(
                "CLI_SESSIONS_FOCUS_CMD",
                "wid={window_id} code={:?} ok={} stdout={:?} stderr={:?} front_before={:?} front_after={:?}",
                out.status.code(),
                out.status.success(),
                diag::trunc(&out.stdout),
                diag::trunc(&out.stderr),
                front_before,
                diag::frontmost_app()
            );
        }
        anyhow::ensure!(out.status.success(), "kitten @ focus-window failed");
        Ok(())
    }
}

#[cfg(debug_assertions)]
mod diag {
    use std::process::Command;

    pub fn trunc(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes)
            .trim()
            .chars()
            .take(160)
            .map(|c| if c == '"' || c == '\n' { '_' } else { c })
            .collect()
    }

    pub fn frontmost_app() -> Option<String> {
        let out = Command::new("sh")
            .arg("-c")
            .arg("lsappinfo info -only name \"$(lsappinfo front)\"")
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let name = stdout
            .split('=')
            .nth(1)?
            .trim()
            .trim_matches('"')
            .to_string();
        (!name.is_empty()).then_some(name)
    }
}
