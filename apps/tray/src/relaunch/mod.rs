use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

const ENV_RELAUNCH_AFTER_PID: &str = "QOL_TRAY_RELAUNCH_AFTER_PID";
const PREDECESSOR_EXIT_TIMEOUT: Duration = Duration::from_secs(10);
/// Time a predecessor may spend on cleanup after spawning its successor, kept
/// well under `PREDECESSOR_EXIT_TIMEOUT` so the successor never gives up first.
pub const PREDECESSOR_CLEANUP_BUDGET: Duration = Duration::from_secs(5);
const PREDECESSOR_POLL_INTERVAL: Duration = Duration::from_millis(25);

pub fn spawn_successor_and_exit(binary: &Path, args: &[OsString]) -> std::io::Error {
    match spawn_successor(binary, args) {
        Ok(pid) => exit_to_successor(pid),
        Err(error) => error,
    }
}

pub fn spawn_successor(binary: &Path, args: &[OsString]) -> std::io::Result<u32> {
    std::process::Command::new(binary)
        .args(args)
        .env(ENV_RELAUNCH_AFTER_PID, std::process::id().to_string())
        .spawn()
        .map(|child| child.id())
}

pub fn exit_to_successor(pid: u32) -> ! {
    log::info!("relaunched as pid {pid}, exiting");
    std::process::exit(0);
}

pub fn wait_for_predecessor() {
    let Some(pid) = take_predecessor_pid() else {
        return;
    };
    let started = Instant::now();
    if wait_until_gone(pid, PREDECESSOR_EXIT_TIMEOUT, crate::process::is_pid_alive) {
        log::info!(
            "predecessor pid {pid} exited after {} ms",
            started.elapsed().as_millis()
        );
    } else {
        log::warn!("predecessor pid {pid} still alive after {PREDECESSOR_EXIT_TIMEOUT:?}");
    }
}

fn take_predecessor_pid() -> Option<i32> {
    let raw = std::env::var(ENV_RELAUNCH_AFTER_PID).ok()?;
    std::env::remove_var(ENV_RELAUNCH_AFTER_PID);
    parse_pid(&raw)
}

fn parse_pid(raw: &str) -> Option<i32> {
    raw.trim().parse::<i32>().ok().filter(|pid| *pid > 1)
}

fn wait_until_gone(pid: i32, timeout: Duration, is_alive: impl Fn(i32) -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while is_alive(pid) {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(PREDECESSOR_POLL_INTERVAL);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn parse_pid_rejects_garbage_and_init() {
        let cases = [
            ("4242", Some(4242)),
            (" 4242\n", Some(4242)),
            ("1", None),
            ("0", None),
            ("-5", None),
            ("", None),
            ("abc", None),
        ];
        for (raw, expected) in cases {
            assert_eq!(parse_pid(raw), expected, "raw={raw:?}");
        }
    }

    #[test]
    fn wait_until_gone_returns_once_the_pid_disappears() {
        let polls = Cell::new(0);
        let gone = wait_until_gone(4242, Duration::from_secs(5), |_| {
            polls.set(polls.get() + 1);
            polls.get() < 3
        });
        assert!(gone);
        assert_eq!(polls.get(), 3);
    }

    #[test]
    fn wait_until_gone_gives_up_at_the_deadline() {
        let gone = wait_until_gone(4242, Duration::from_millis(60), |_| true);
        assert!(!gone);
    }
}
