mod daemon;
mod daemon_run;

use anyhow::Result;

pub fn run() -> Result<()> {
    daemon_run::run()
}

pub fn open_settings() -> Result<()> {
    crate::settings::open()
}

pub fn toggle_theme() -> Result<crate::theme::ColorScheme> {
    crate::theme::toggle()
}

pub fn kill() -> Result<()> {
    daemon::send_kill();
    Ok(())
}
