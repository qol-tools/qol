use std::sync::atomic::{AtomicI32, Ordering};

const MAX_DAEMONS: usize = 16;

static DAEMON_PIDS: [AtomicI32; MAX_DAEMONS] = [const { AtomicI32::new(0) }; MAX_DAEMONS];

pub(super) fn register_daemon_pid(pid: u32) {
    let pid = pid as i32;
    for slot in &DAEMON_PIDS {
        if slot
            .compare_exchange(0, pid, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            return;
        }
    }
    log::warn!(
        "Signal handler PID table full, daemon pid {} not tracked",
        pid
    );
}

pub(super) fn unregister_daemon_pid(pid: u32) {
    let pid = pid as i32;
    for slot in &DAEMON_PIDS {
        let _ = slot.compare_exchange(pid, 0, Ordering::AcqRel, Ordering::Relaxed);
    }
}

pub(super) fn install_signal_handler() {
    unsafe {
        libc::signal(
            libc::SIGINT,
            sigint_handler as *const () as libc::sighandler_t,
        );
    }
}

extern "C" fn sigint_handler(_sig: libc::c_int) {
    for slot in &DAEMON_PIDS {
        let pid = slot.load(Ordering::Relaxed);
        if pid > 0 {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
        }
    }
    let grace = libc::timespec {
        tv_sec: 0,
        tv_nsec: 50_000_000,
    };
    unsafe {
        libc::nanosleep(&grace, std::ptr::null_mut());
    }
    for slot in &DAEMON_PIDS {
        let pid = slot.load(Ordering::Relaxed);
        if pid > 0 {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
    unsafe {
        libc::_exit(130);
    }
}
