use std::fs;

pub(super) fn process_cpu_micros(pid: i32) -> Option<u64> {
    if pid <= 0 {
        return None;
    }

    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let comm_end = stat.rfind(')')?;
    let fields = stat
        .get(comm_end + 2..)?
        .split_whitespace()
        .collect::<Vec<_>>();
    let user_ticks: u64 = fields.get(11)?.parse().ok()?;
    let system_ticks: u64 = fields.get(12)?.parse().ok()?;
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks_per_second <= 0 {
        return None;
    }

    let total_ticks = user_ticks as u128 + system_ticks as u128;
    Some((total_ticks * 1_000_000u128 / ticks_per_second as u128) as u64)
}
