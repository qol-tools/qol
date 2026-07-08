use super::*;

pub(super) fn detail_control_hint(replay: bool, details: bool) -> &'static str {
    if replay {
        return if details {
            "details enabled"
        } else {
            "use --details to expand"
        };
    }
    if std::io::stdin().is_terminal() {
        "press d to toggle, ctrl+c exits"
    } else if details {
        "details enabled"
    } else {
        "use --details to expand"
    }
}

pub(super) struct DetailToggleInput {
    pub(super) enabled: bool,
    pub(super) pipe_flag: Option<Arc<AtomicBool>>,
    #[cfg(unix)]
    pub(super) _guard: Option<CbreakGuard>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TailControl {
    Continue,
    Exit,
}

impl DetailToggleInput {
    pub(super) fn new() -> Self {
        if !std::io::stdin().is_terminal() {
            return Self::piped();
        }
        #[cfg(unix)]
        let guard = CbreakGuard::new();
        #[cfg(unix)]
        let enabled = guard.is_some();
        #[cfg(not(unix))]
        let enabled = false;
        Self {
            enabled,
            pipe_flag: None,
            #[cfg(unix)]
            _guard: guard,
        }
    }

    pub(super) fn piped() -> Self {
        let flag = Arc::new(AtomicBool::new(false));
        let writer = Arc::clone(&flag);
        std::thread::spawn(move || {
            let mut byte = [0u8; 1];
            let mut stdin = std::io::stdin().lock();
            while let Ok(read) = stdin.read(&mut byte) {
                if read == 0 {
                    break;
                }
                if matches!(byte[0], b'd' | b'D') {
                    writer.store(true, Ordering::SeqCst);
                }
            }
        });
        Self {
            enabled: false,
            pipe_flag: Some(flag),
            #[cfg(unix)]
            _guard: None,
        }
    }

    pub(super) fn poll(&mut self, runner: &mut TraceRunner) -> TailControl {
        if let Some(flag) = self.pipe_flag.as_ref() {
            if flag.swap(false, Ordering::SeqCst) {
                println!("{}\n", runner.toggle_details());
                runner.flush();
            }
            return TailControl::Continue;
        }
        if !self.enabled {
            return TailControl::Continue;
        }
        let has_event = match event::poll(Duration::ZERO) {
            Ok(has_event) => has_event,
            Err(_) => {
                self.enabled = false;
                return TailControl::Continue;
            }
        };
        if !has_event {
            return TailControl::Continue;
        }
        let Ok(TerminalEvent::Key(key)) = event::read() else {
            return TailControl::Continue;
        };
        Self::handle_key(runner, key.code, key.modifiers, key.kind)
    }

    pub(super) fn handle_key(
        runner: &mut TraceRunner,
        code: KeyCode,
        modifiers: KeyModifiers,
        kind: KeyEventKind,
    ) -> TailControl {
        if kind != KeyEventKind::Press {
            return TailControl::Continue;
        }
        if is_ctrl_c(code, modifiers) {
            return TailControl::Exit;
        }
        if matches!(code, KeyCode::Char('d') | KeyCode::Char('D')) {
            println!("{}\n", runner.toggle_details());
            runner.flush();
        }
        TailControl::Continue
    }
}

pub(super) fn is_ctrl_c(code: KeyCode, modifiers: KeyModifiers) -> bool {
    match code {
        KeyCode::Char('c') | KeyCode::Char('C') => modifiers.contains(KeyModifiers::CONTROL),
        KeyCode::Char('\u{3}') => true,
        _ => false,
    }
}

#[cfg(unix)]
pub(super) struct CbreakGuard {
    pub(super) fd: i32,
    pub(super) original: libc::termios,
}

#[cfg(unix)]
impl CbreakGuard {
    pub(super) fn new() -> Option<Self> {
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

#[cfg(unix)]
impl Drop for CbreakGuard {
    fn drop(&mut self) {
        // SAFETY: original was captured from this fd and remains a valid termios value.
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_input_maps_detail_toggle_and_ctrl_c() {
        let args = Args::parse(&[]).unwrap();
        let mut runner = TraceRunner::new(args, PathBuf::from(DEFAULT_LOG_FILE));

        let toggle = DetailToggleInput::handle_key(
            &mut runner,
            KeyCode::Char('d'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        );
        assert_eq!(toggle, TailControl::Continue);
        assert!(runner.args.details);

        let release = DetailToggleInput::handle_key(
            &mut runner,
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            KeyEventKind::Release,
        );
        assert_eq!(release, TailControl::Continue);

        let exit = DetailToggleInput::handle_key(
            &mut runner,
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        );
        assert_eq!(exit, TailControl::Exit);
    }
}
