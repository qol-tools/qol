pub(in crate::commands::check::command) fn fallback_alive(pid: u32) -> bool {
    qol_process::is_pid_alive(pid)
}

pub(in crate::commands::check::command) fn fallback_request_stop(pid: u32) -> std::io::Result<()> {
    qol_process::signal_term_pid(pid)
}

pub(in crate::commands::check::command) fn fallback_force_stop(pid: u32) -> std::io::Result<()> {
    qol_process::kill_pid(pid)
}
