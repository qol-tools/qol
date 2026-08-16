use std::os::fd::{AsRawFd, RawFd};

use super::PRIOR_TERMIOS_ENV;

const COOKED_LFLAG: libc::tcflag_t = libc::ICANON | libc::ECHO | libc::ISIG;
const COOKED_IFLAG: libc::tcflag_t = libc::IXON | libc::ICRNL;
const COOKED_OFLAG: libc::tcflag_t = libc::OPOST;

fn with_tty_fd<T>(f: impl FnOnce(RawFd) -> T) -> Option<T> {
    if unsafe { libc::isatty(libc::STDIN_FILENO) } == 1 {
        return Some(f(libc::STDIN_FILENO));
    }
    let dev_tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    Some(f(dev_tty.as_raw_fd()))
}

pub(crate) fn capture_prior_termios() {
    with_tty_fd(|fd| {
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut termios) } == 0 {
            std::env::set_var(PRIOR_TERMIOS_ENV, serialize_termios(&termios));
        }
    });
}

pub(crate) fn apply_prior_termios() {
    let termios = std::env::var(PRIOR_TERMIOS_ENV)
        .ok()
        .and_then(|raw| parse_termios(&raw))
        .or_else(cooked_fallback_termios);
    let Some(termios) = termios else {
        return;
    };
    with_tty_fd(|fd| {
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) };
    });
}

pub(crate) fn restore_resumed_tty() {
    apply_prior_termios();
    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::terminal::LeaveAlternateScreen,
        ratatui::crossterm::cursor::Show
    );
}

fn serialize_termios(termios: &libc::termios) -> String {
    format!(
        "{},{},{},{}",
        termios.c_iflag, termios.c_oflag, termios.c_cflag, termios.c_lflag
    )
}

fn parse_termios(raw: &str) -> Option<libc::termios> {
    let mut fields = raw.split(',');
    let c_iflag = fields.next()?.parse().ok()?;
    let c_oflag = fields.next()?.parse().ok()?;
    let c_cflag = fields.next()?.parse().ok()?;
    let c_lflag = fields.next()?.parse().ok()?;
    if fields.next().is_some() {
        return None;
    }
    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    termios.c_iflag = c_iflag;
    termios.c_oflag = c_oflag;
    termios.c_cflag = c_cflag;
    termios.c_lflag = c_lflag;
    Some(termios)
}

fn apply_cooked_flags(mut termios: libc::termios) -> libc::termios {
    termios.c_lflag |= COOKED_LFLAG;
    termios.c_iflag |= COOKED_IFLAG;
    termios.c_oflag |= COOKED_OFLAG;
    termios
}

fn cooked_fallback_termios() -> Option<libc::termios> {
    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    let rc = with_tty_fd(|fd| unsafe { libc::tcgetattr(fd, &mut termios) })?;
    if rc != 0 {
        return None;
    }
    Some(apply_cooked_flags(termios))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn termios_env_round_trips_the_four_flag_words() {
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        termios.c_iflag = libc::IXON | libc::ICRNL | 0x8000;
        termios.c_oflag = libc::OPOST | 0x1000;
        termios.c_cflag = 0x1234;
        termios.c_lflag = libc::ICANON | libc::ECHO | libc::ISIG;

        let parsed = parse_termios(&serialize_termios(&termios)).unwrap();

        assert_eq!(parsed.c_iflag, termios.c_iflag);
        assert_eq!(parsed.c_oflag, termios.c_oflag);
        assert_eq!(parsed.c_cflag, termios.c_cflag);
        assert_eq!(parsed.c_lflag, termios.c_lflag);
    }

    #[test]
    fn termios_parse_rejects_malformed_env_values() {
        let cases = ["", "1,2", "1,2,3", "1,2,3,4,5", "a,b,c,d", "1,2,3,x"];
        for raw in cases {
            assert!(parse_termios(raw).is_none(), "input: {raw:?}");
        }
    }

    #[test]
    fn cooked_fallback_sets_the_standard_mode_bits() {
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        termios.c_iflag = 0;
        termios.c_oflag = 0;
        termios.c_lflag = 0;

        let cooked = apply_cooked_flags(termios);

        assert_eq!(
            cooked.c_lflag & (libc::ICANON | libc::ECHO | libc::ISIG),
            libc::ICANON | libc::ECHO | libc::ISIG,
            "canonical mode, echo, and signals must be restored"
        );
        assert_eq!(
            cooked.c_iflag & (libc::IXON | libc::ICRNL),
            libc::IXON | libc::ICRNL,
            "flow control and CR-to-NL translation must be restored"
        );
        assert_eq!(
            cooked.c_oflag & libc::OPOST,
            libc::OPOST,
            "output processing must be restored"
        );
    }

    #[test]
    fn capture_prior_termios_does_not_panic_without_a_tty() {
        capture_prior_termios();
        apply_prior_termios();
        restore_resumed_tty();
    }
}
