mod containment;

use std::io;
use std::path::PathBuf;
use std::process::{Child, Command};

use crate::ProcessEntry;

pub(crate) use super::unix::{
    bind_to_host_lifetime, cancellation_requested, cancellation_signal_count,
    guard_current_process_tree, hide_console_window, install_cancellation_handler, is_group_alive,
    is_pid_alive, isolate_owned_command, kill_group, kill_pid, reload_group, signal_term_group,
    signal_term_pid, spawn_detached, terminate_group, terminate_owned, terminate_pid, try_wait_pid,
    wait_for_stop_request, wait_pid, CurrentProcessTreeGuard,
};
pub(crate) use containment::{
    is_pid_zombie, isolate_owned_session, own_current_process_tree_with_guardian, process_identity,
    process_identity_matches, process_tree_containment_support, run_process_tree_guardian_entry,
    PreparedSpawn, ProcessTreeGuard,
};

pub(crate) fn processes() -> io::Result<Vec<ProcessEntry>> {
    Ok(std::fs::read_dir("/proc")?
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            process_from_stat(pid, &stat)
        })
        .collect())
}

fn process_from_stat(pid: u32, stat: &str) -> Option<ProcessEntry> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let exe = stat.get(open + 1..close)?.to_string();
    let parent = stat
        .get(close + 1..)?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some(ProcessEntry { pid, parent, exe })
}

pub(crate) fn process_image_path(pid: u32) -> io::Result<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
}

pub(crate) fn spawn_owned(mut command: Command) -> io::Result<(Child, Option<ProcessTreeGuard>)> {
    isolate_owned_command(&mut command)?;
    Ok((command.spawn()?, None))
}

#[cfg(test)]
mod tests {
    use super::process_from_stat;
    use crate::ProcessEntry;

    #[test]
    fn process_from_stat_reads_the_name_and_parent() {
        let cases = [
            ("1 (systemd) S 0 1 1", Some((0, "systemd"))),
            ("42 (tmux: server) S 7 42 42", Some((7, "tmux: server"))),
            ("43 (a) b) R 9 43 43", Some((9, "a) b"))),
            ("44 broken", None),
        ];
        for (stat, expected) in cases {
            let pid = stat.split(' ').next().unwrap().parse().unwrap();
            assert_eq!(
                process_from_stat(pid, stat),
                expected.map(|(parent, exe)| ProcessEntry {
                    pid,
                    parent,
                    exe: exe.to_string(),
                }),
                "{stat}"
            );
        }
    }

    #[test]
    fn current_process_appears_in_the_snapshot() {
        let own = std::process::id();
        let table = super::processes().expect("reads /proc");
        assert!(table.iter().any(|entry| entry.pid == own));
    }
}
