/// SIGINT handler that kills daemon process groups before exiting.
///
/// When daemons run in their own session (`setsid`), terminal SIGINT only reaches
/// qol-tray. This module tracks daemon PIDs and kills their process groups in the
/// signal handler so they don't become orphans.
#[cfg(unix)]
mod unix {
    use std::sync::atomic::{AtomicI32, Ordering};

    const MAX_DAEMONS: usize = 16;

    /// 0 = empty slot. Stores pid as i32 (matches libc::pid_t).
    static DAEMON_PIDS: [AtomicI32; MAX_DAEMONS] = {
        const EMPTY: AtomicI32 = AtomicI32::new(0);
        [EMPTY; MAX_DAEMONS]
    };

    pub fn register_daemon_pid(pid: u32) {
        let pid = pid as i32;
        for slot in &DAEMON_PIDS {
            if slot.compare_exchange(0, pid, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                return;
            }
        }
        log::warn!("Signal handler PID table full, daemon pid {} not tracked", pid);
    }

    pub fn unregister_daemon_pid(pid: u32) {
        let pid = pid as i32;
        for slot in &DAEMON_PIDS {
            let _ = slot.compare_exchange(pid, 0, Ordering::AcqRel, Ordering::Relaxed);
        }
    }

    pub fn install_signal_handler() {
        unsafe {
            libc::signal(libc::SIGINT, sigint_handler as *const () as libc::sighandler_t);
        }
    }

    extern "C" fn sigint_handler(_sig: libc::c_int) {
        // All calls here are async-signal-safe: kill() and _exit().
        for slot in &DAEMON_PIDS {
            let pid = slot.load(Ordering::Relaxed);
            if pid > 0 {
                unsafe {
                    // Kill the entire process group (setsid makes pid == pgid).
                    libc::kill(-pid, libc::SIGTERM);
                }
            }
        }
        unsafe {
            libc::_exit(130); // 128 + SIGINT(2), standard convention
        }
    }
}

#[cfg(unix)]
pub use unix::{install_signal_handler, register_daemon_pid, unregister_daemon_pid};

#[cfg(not(unix))]
pub fn install_signal_handler() {}

#[cfg(not(unix))]
pub fn register_daemon_pid(_pid: u32) {}

#[cfg(not(unix))]
pub fn unregister_daemon_pid(_pid: u32) {}
