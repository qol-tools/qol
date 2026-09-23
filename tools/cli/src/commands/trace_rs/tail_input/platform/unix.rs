use std::io::IsTerminal;
use std::mem::MaybeUninit;
use std::os::fd::AsRawFd;

pub(in crate::commands::trace_rs::tail_input) struct CbreakGuard {
    fd: i32,
    original: libc::termios,
}

impl CbreakGuard {
    pub(in crate::commands::trace_rs::tail_input) fn new() -> Option<Self> {
        let stdin = std::io::stdin();
        if !stdin.is_terminal() {
            return None;
        }
        let fd = stdin.as_raw_fd();
        let mut original = MaybeUninit::<libc::termios>::uninit();
        // SAFETY: fd is stdin's live file descriptor and the pointer is valid for tcgetattr.
        if unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: tcgetattr returned success, so original has been fully initialized.
        let original = unsafe { original.assume_init() };
        let mut cbreak = original;
        cbreak.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
        cbreak.c_cc[libc::VMIN] = 1;
        cbreak.c_cc[libc::VTIME] = 0;
        // SAFETY: cbreak is derived from a valid termios struct for this fd.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &cbreak) } != 0 {
            return None;
        }
        Some(Self { fd, original })
    }
}

impl Drop for CbreakGuard {
    fn drop(&mut self) {
        // SAFETY: original was captured from this fd and remains a valid termios value.
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
        }
    }
}
