pub(in crate::commands::check::command) fn fallback_alive(pid: u32) -> bool {
    qol_process::is_pid_alive(pid)
}

pub(in crate::commands::check::command) fn fallback_request_stop(pid: u32) -> std::io::Result<()> {
    qol_process::signal_term_pid(pid)
}

pub(in crate::commands::check::command) fn fallback_force_stop(pid: u32) -> std::io::Result<()> {
    qol_process::kill_pid(pid)
}

pub(in crate::commands::check) const FALLBACK_BACKEND: &str = "pid";

pub(in crate::commands::check) fn exit_signal(status: std::process::ExitStatus) -> Option<i32> {
    let _ = status;
    None
}
