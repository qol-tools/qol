use std::collections::BTreeSet;
use std::os::fd::RawFd;
use std::sync::{Mutex, MutexGuard, OnceLock};

const ENV_HANDOFF_FDS: &str = "QOL_TRAY_LIFELINE_HANDOFF_FDS";

#[derive(Default)]
struct HandoffRegistry {
    fds: Mutex<BTreeSet<RawFd>>,
}

impl HandoffRegistry {
    fn lock(&self) -> MutexGuard<'_, BTreeSet<RawFd>> {
        self.fds.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn register(&self, fd: RawFd) {
        self.lock().insert(fd);
    }

    fn unregister(&self, fd: RawFd) {
        self.lock().remove(&fd);
    }

    fn handoff_payload(&self) -> Option<String> {
        let fds: Vec<RawFd> = self.lock().iter().copied().collect();
        let inheritable = retain_flagged(&fds, false);
        if inheritable.is_empty() {
            return None;
        }
        Some(join_fds(&inheritable))
    }

    fn adopt_payload(&self, raw: &str) -> usize {
        let adopted = retain_flagged(&parse_fds(raw), true);
        for fd in &adopted {
            self.register(*fd);
        }
        adopted.len()
    }
}

fn global() -> &'static HandoffRegistry {
    static GLOBAL: OnceLock<HandoffRegistry> = OnceLock::new();
    GLOBAL.get_or_init(HandoffRegistry::default)
}

pub(crate) fn register(fd: RawFd) {
    global().register(fd);
}

pub(crate) fn unregister(fd: RawFd) {
    global().unregister(fd);
}

pub fn prepare_for_exec() {
    match global().handoff_payload() {
        Some(payload) => {
            log::info!(
                "[lifeline-handoff] handing {} lifeline fd(s) to the exec successor",
                payload.split(',').count()
            );
            std::env::set_var(ENV_HANDOFF_FDS, payload);
        }
        None => std::env::remove_var(ENV_HANDOFF_FDS),
    }
}

pub fn adopt_handed_off_fds() {
    let Ok(raw) = std::env::var(ENV_HANDOFF_FDS) else {
        return;
    };
    std::env::remove_var(ENV_HANDOFF_FDS);
    let adopted = global().adopt_payload(&raw);
    if adopted > 0 {
        log::info!(
                "[lifeline-handoff] re-secured {adopted} lifeline fd(s) inherited from the previous generation"
            );
    }
}

fn retain_flagged(fds: &[RawFd], cloexec: bool) -> Vec<RawFd> {
    fds.iter()
        .copied()
        .filter(|fd| set_cloexec(*fd, cloexec))
        .collect()
}

fn set_cloexec(fd: RawFd, cloexec: bool) -> bool {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags < 0 {
            return false;
        }
        let flags = if cloexec {
            flags | libc::FD_CLOEXEC
        } else {
            flags & !libc::FD_CLOEXEC
        };
        libc::fcntl(fd, libc::F_SETFD, flags) == 0
    }
}

fn join_fds(fds: &[RawFd]) -> String {
    fds.iter()
        .map(|fd| fd.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_fds(raw: &str) -> Vec<RawFd> {
    raw.split(',')
        .filter_map(|part| part.trim().parse::<RawFd>().ok())
        .filter(|fd| *fd > 2)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;

    fn cloexec_is_set(fd: RawFd) -> bool {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert!(flags >= 0, "fd {fd} must be open");
        flags & libc::FD_CLOEXEC != 0
    }

    #[test]
    fn handoff_payload_clears_cloexec_and_lists_registered_fds() {
        let registry = HandoffRegistry::default();
        let (a, _peer_a) = UnixStream::pair().expect("socketpair");
        let (b, _peer_b) = UnixStream::pair().expect("socketpair");
        registry.register(a.as_raw_fd());
        registry.register(b.as_raw_fd());
        assert!(
            cloexec_is_set(a.as_raw_fd()),
            "std sockets start close-on-exec"
        );

        let payload = registry
            .handoff_payload()
            .expect("payload for registered fds");

        let expected = join_fds(&[
            a.as_raw_fd().min(b.as_raw_fd()),
            a.as_raw_fd().max(b.as_raw_fd()),
        ]);
        assert_eq!(payload, expected, "payload lists every registered fd");
        assert!(
            !cloexec_is_set(a.as_raw_fd()),
            "handoff must make fd inheritable"
        );
        assert!(
            !cloexec_is_set(b.as_raw_fd()),
            "handoff must make fd inheritable"
        );
    }

    #[test]
    fn handoff_payload_skips_closed_and_unregistered_fds() {
        let registry = HandoffRegistry::default();
        registry.register(RawFd::MAX);
        assert_eq!(registry.handoff_payload(), None, "closed fds must drop out");

        registry.unregister(RawFd::MAX);
        let (alive, _peer) = UnixStream::pair().expect("socketpair");
        registry.register(alive.as_raw_fd());
        registry.unregister(alive.as_raw_fd());
        assert_eq!(
            registry.handoff_payload(),
            None,
            "unregistered fds must not be handed off"
        );
    }

    #[test]
    fn adopt_payload_restores_cloexec_and_reregisters_for_next_handoff() {
        let registry = HandoffRegistry::default();
        let (a, _peer) = UnixStream::pair().expect("socketpair");
        let fd = a.as_raw_fd();
        assert!(set_cloexec(fd, false), "simulate inherited non-cloexec fd");

        let adopted = registry.adopt_payload(&fd.to_string());

        assert_eq!(adopted, 1, "open fd must be adopted");
        assert!(cloexec_is_set(fd), "adoption must re-secure the fd");
        assert_eq!(
            registry.handoff_payload().as_deref(),
            Some(fd.to_string().as_str()),
            "adopted fd must be handed off again on the next exec"
        );
    }

    #[test]
    fn adopt_payload_ignores_junk_stdio_and_closed_entries() {
        let registry = HandoffRegistry::default();
        let cases = [
            ("", "empty payload"),
            ("0,1,2", "stdio fds"),
            ("garbage,-5, ,999999", "junk and closed fds"),
        ];
        for (raw, label) in cases {
            assert_eq!(registry.adopt_payload(raw), 0, "case: {label}");
        }
    }
}
