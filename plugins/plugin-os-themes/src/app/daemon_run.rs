use std::sync::{mpsc, Arc};

use anyhow::{ensure, Result};

use crate::cursor::RunState;
use crate::{cursor, daemon};

pub fn run() -> Result<()> {
    if daemon::send_ping() {
        return Ok(());
    }

    cursor::install_signal_handlers();

    let control = Arc::new(RunState::new());
    let (tx, rx) = mpsc::channel();
    ensure!(daemon::start_listener(tx), "failed to start daemon listener");

    let listener_control = Arc::clone(&control);
    std::thread::spawn(move || handle_daemon_commands(rx, listener_control));

    supervise_effect(control)
}

fn supervise_effect(control: Arc<RunState>) -> Result<()> {
    let effect = cursor::create_effect();

    loop {
        control.reset();
        let config = crate::config::load();
        let result = effect.run(&config, control.as_ref());
        if control.reload_requested() {
            continue;
        }

        daemon::cleanup();
        return result;
    }
}

fn handle_daemon_commands(rx: mpsc::Receiver<daemon::Command>, control: Arc<RunState>) {
    while let Ok(command) = rx.recv() {
        match command {
            daemon::Command::Kill => {
                control.request_shutdown();
                break;
            }
            daemon::Command::Reload => control.request_reload(),
        }
    }
}
