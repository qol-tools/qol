use std::sync::atomic::Ordering;

use crate::plugins::daemon_tracker::registry::OWNED_DAEMON_PIDS;

pub(super) fn install_signal_handler() {
    let handler: extern "C" fn(libc::c_int) = sigint_handler;
    unsafe {
        libc::signal(libc::SIGINT, handler as libc::sighandler_t);
    }
}

extern "C" fn sigint_handler(_sig: libc::c_int) {
    for slot in &OWNED_DAEMON_PIDS {
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
    for slot in &OWNED_DAEMON_PIDS {
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
