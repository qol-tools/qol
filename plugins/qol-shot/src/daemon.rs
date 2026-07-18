use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};

const CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

pub enum Command {
    Screenshot,
    Preview,
    Cli(String),
    Kill,
}

pub fn start_listener(tx: Sender<Command>) -> bool {
    let started = core_daemon::start_listener(&CONFIG, tx, parse_command);
    qol_runtime::probe!("SHOT_DAEMON_LISTEN", "started={started}");
    started
}

pub fn cleanup() {
    core_daemon::cleanup(&CONFIG);
}

pub fn wait_and_send_action(action: &str, timeout: Duration) -> bool {
    let started = Instant::now();
    loop {
        if core_daemon::send_action(&CONFIG, action, true) {
            qol_runtime::probe!(
                "SHOT_DAEMON_FORWARD",
                "action={action} result=sent ms={}",
                started.elapsed().as_millis()
            );
            return true;
        }
        if started.elapsed() >= timeout {
            qol_runtime::probe!(
                "SHOT_DAEMON_FORWARD",
                "action={action} result=timeout ms={}",
                started.elapsed().as_millis()
            );
            return false;
        }
        thread::sleep(Duration::from_millis(80));
    }
}

fn parse_command(cmd: &str) -> ReadResult<Command> {
    if cmd != "ping" {
        qol_runtime::probe!("CMD_RECV", "cmd={cmd}");
    }
    match cmd {
        "ping" => ReadResult::Handled,
        "kill" => ReadResult::Command(Command::Kill),
        "screenshot" => ReadResult::Command(Command::Screenshot),
        "preview" => ReadResult::Command(Command::Preview),
        "audio_sources" => audio_device_payload(crate::platform::list_audio_sources()),
        "audio_sinks" => audio_device_payload(crate::platform::list_audio_sinks()),
        other => ReadResult::Command(Command::Cli(other.to_string())),
    }
}

fn audio_device_payload(devices: Vec<crate::platform::AudioDevice>) -> ReadResult<Command> {
    match serde_json::to_value(devices) {
        Ok(payload) => ReadResult::HandledWithData(payload),
        Err(_) => ReadResult::HandledWithData(serde_json::json!([])),
    }
}
