pub(in crate::commands::check::command) fn fallback_alive(pid: u32) -> bool {
    qol_process::is_group_alive(pid)
}

pub(in crate::commands::check::command) fn fallback_request_stop(pid: u32) -> std::io::Result<()> {
    qol_process::signal_term_group(pid)
}

pub(in crate::commands::check::command) fn fallback_force_stop(pid: u32) -> std::io::Result<()> {
    qol_process::kill_group(pid)
}

pub(in crate::commands::check) const FALLBACK_BACKEND: &str = "unix_process_group";

pub(in crate::commands::check) fn exit_signal(status: std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}
