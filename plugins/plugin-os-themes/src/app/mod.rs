mod daemon;
mod daemon_run;

use std::process::ExitCode;

use anyhow::Result;

use crate::cursor::{CursorPlatform, Platform};

pub fn run(action: Option<&str>) -> ExitCode {
    let result = match action {
        None | Some("run") => daemon_run::run(),
        Some("settings") => Platform.open_settings(),
        Some("toggle_theme") => toggle_theme(),
        Some("kill") => {
            daemon::send_kill();
            Ok(())
        }
        Some(action) => {
            eprintln!("Unknown action: {action}");
            return ExitCode::from(1);
        }
    };

    exit_code(result)
}

fn toggle_theme() -> Result<()> {
    let scheme = crate::theme::toggle()?;
    eprintln!("[os-themes] applied {scheme:?} theme");
    Ok(())
}

fn exit_code(result: Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::from(1)
        }
    }
}
