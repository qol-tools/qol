mod checkout;
mod config;
mod platform;
mod server;
mod takeover;

pub(crate) use checkout::find_executable;
pub(crate) use config::{inspect as inspect_config, Config};

pub(crate) fn is_executable(path: &std::path::Path) -> bool {
    platform::is_executable(path)
}

pub(crate) fn open_settings() -> Result<(), String> {
    platform::open_settings()
}

pub fn run() -> u8 {
    platform::spawn_host_death_watchdog();
    let config = config::Config::load();
    match server::serve(daemon_port(), config) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("[task-runner] failed to start daemon: {error}");
            1
        }
    }
}

pub(crate) fn daemon_port() -> u16 {
    env!("QOL_DAEMON_PORT")
        .parse()
        .expect("QOL_DAEMON_PORT must be a valid u16")
}
