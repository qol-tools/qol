pub(super) fn process_alive(pid: u32) -> bool {
    if matches!(qol_process::try_wait_pid(pid), Ok(Some(_))) {
        return false;
    }
    qol_process::is_pid_alive(pid)
}

pub(super) fn signal_process(pid: u32, signal: i32) -> anyhow::Result<()> {
    let pid = pid as libc::pid_t;
    if pid <= 0 {
        return Err(anyhow::anyhow!("invalid process pid {}", pid));
    }
    if unsafe { libc::kill(pid, signal) } == 0 || !process_alive(pid as u32) {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "failed to send signal {} to process pid {}",
        signal,
        pid
    ))
}
