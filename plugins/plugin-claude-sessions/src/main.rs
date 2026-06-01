//! plugin-claude-sessions binary.
//!
//! Two surfaces:
//! - `run` (default): print readiness and exit. The plugin does its real
//!   work via the `resolve` subcommand, not as a long-lived daemon.
//! - `resolve`: read one `PaneSnapshot` JSON from stdin, walk the
//!   foreground processes for `exe == "claude"`, resolve the active
//!   session jsonl via the platform fd resolver, and emit a single
//!   `RestoreClaim` JSON line on stdout. Empty stdout = no claim.
//!
//! This subcommand is the shell-out hop used by plugin-kitty until the
//! qol-tray AF_UNIX broker (`RUNTIME-1` / `TRAY-31`) replaces it with a
//! capability-gated IPC call.

use std::env;
use std::io::{self, Read};
use std::process::ExitCode;

use plugin_claude_sessions::claim::build_claim;
use plugin_claude_sessions::resolver::{resolve_session_jsonl, ResolveError};
use qol_plugin_api::restore::PaneSnapshot;

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        Some("resolve") => match run_resolve() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("plugin-claude-sessions resolve: {err:#}");
                ExitCode::from(1)
            }
        },
        None | Some("run") => {
            println!(
                "plugin-claude-sessions: ready; invoke `resolve` with a PaneSnapshot on stdin"
            );
            ExitCode::SUCCESS
        }
        Some(action) => {
            eprintln!("Unknown action: {action}");
            ExitCode::from(1)
        }
    }
}

fn run_resolve() -> anyhow::Result<()> {
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf)?;
    let pane: PaneSnapshot = serde_json::from_str(&buf)?;

    let Some(claude) = pane.foreground.iter().find(|p| p.exe == "claude") else {
        return Ok(());
    };

    match resolve_session_jsonl(claude.pid, &claude.exe) {
        Ok(jsonl) => {
            let claim = build_claim(&jsonl)?;
            println!("{}", serde_json::to_string(&claim)?);
            Ok(())
        }
        Err(ResolveError::NoSessionJsonl(_))
        | Err(ResolveError::NotClaude { .. })
        | Err(ResolveError::PidDead(_))
        | Err(ResolveError::PlatformUnsupported) => Ok(()),
        Err(err) => Err(err.into()),
    }
}
